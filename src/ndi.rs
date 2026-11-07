//! NDI video sender through the NDI Runtime (Processing.NDI.Lib.x64.dll, installed
//! separately from https://ndi.video). Loaded dynamically: no runtime → a clear error.

use std::ffi::{c_char, c_void, CString};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

#[repr(C)]
struct SendCreate {
    p_ndi_name: *const c_char,
    p_groups: *const c_char,
    clock_video: bool,
    clock_audio: bool,
}

#[repr(C)]
struct VideoFrameV2 {
    xres: i32,
    yres: i32,
    four_cc: u32,
    frame_rate_n: i32,
    frame_rate_d: i32,
    picture_aspect_ratio: f32,
    frame_format_type: i32,
    timecode: i64,
    p_data: *const u8,
    line_stride_in_bytes: i32,
    p_metadata: *const c_char,
    timestamp: i64,
}

const FOURCC_BGRA: u32 = u32::from_le_bytes(*b"BGRA");
const FOURCC_RGBA: u32 = u32::from_le_bytes(*b"RGBA");
const FRAME_PROGRESSIVE: i32 = 1;
const SEND_TIMECODE_SYNTHESIZE: i64 = i64::MAX;

struct Api {
    _lib: libloading::Library,
    initialize: unsafe extern "C" fn() -> bool,
    send_create: unsafe extern "C" fn(*const SendCreate) -> *mut c_void,
    send_destroy: unsafe extern "C" fn(*mut c_void),
    send_video_v2: unsafe extern "C" fn(*mut c_void, *const VideoFrameV2),
}

/// Candidate locations of the NDI runtime DLL.
pub fn runtime_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for var in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5", "NDI_RUNTIME_DIR_V4"] {
        if let Some(d) = std::env::var_os(var) {
            v.push(PathBuf::from(d).join("Processing.NDI.Lib.x64.dll"));
        }
    }
    v.push(PathBuf::from("Processing.NDI.Lib.x64.dll"));
    v
}

pub fn runtime_installed() -> bool {
    runtime_paths().iter().any(|p| p.is_file())
}

impl Api {
    fn load() -> Result<Api> {
        let path = runtime_paths().into_iter().find(|p| p.is_file()).context("NDI Runtime не установлен (нужна библиотека Processing.NDI.Lib.x64.dll)")?;
        unsafe {
            let lib = libloading::Library::new(&path).with_context(|| format!("load {}", path.display()))?;
            let api = Api {
                initialize: *lib.get(b"NDIlib_initialize\0")?,
                send_create: *lib.get(b"NDIlib_send_create\0")?,
                send_destroy: *lib.get(b"NDIlib_send_destroy\0")?,
                send_video_v2: *lib.get(b"NDIlib_send_send_video_v2\0")?,
                _lib: lib,
            };
            if !(api.initialize)() {
                bail!("NDIlib_initialize: процессор не поддерживается NDI");
            }
            Ok(api)
        }
    }
}

pub struct NdiSender {
    api: Api,
    inst: *mut c_void,
    _name: CString,
    four_cc: u32,
}

impl NdiSender {
    pub fn new(name: &str, bgra: bool) -> Result<NdiSender> {
        let api = Api::load()?;
        let cname = CString::new(name.replace('\0', "")).unwrap_or_default();
        let desc = SendCreate { p_ndi_name: cname.as_ptr(), p_groups: std::ptr::null(), clock_video: false, clock_audio: false };
        let inst = unsafe { (api.send_create)(&desc) };
        if inst.is_null() {
            bail!("NDIlib_send_create failed");
        }
        Ok(NdiSender { api, inst, _name: cname, four_cc: if bgra { FOURCC_BGRA } else { FOURCC_RGBA } })
    }

    /// Sends one straight-alpha frame (rows may be padded: `stride` bytes per row).
    pub fn send(&mut self, data: &[u8], w: u32, h: u32, stride: u32) {
        let frame = VideoFrameV2 {
            xres: w as i32,
            yres: h as i32,
            four_cc: self.four_cc,
            frame_rate_n: 60000,
            frame_rate_d: 1000,
            picture_aspect_ratio: w as f32 / h.max(1) as f32,
            frame_format_type: FRAME_PROGRESSIVE,
            timecode: SEND_TIMECODE_SYNTHESIZE,
            p_data: data.as_ptr(),
            line_stride_in_bytes: stride as i32,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        };
        // Synchronous send: NDI copies/encodes before returning, so `data` may be unmapped afterwards.
        unsafe { (self.api.send_video_v2)(self.inst, &frame) };
    }
}

impl Drop for NdiSender {
    fn drop(&mut self) {
        unsafe { (self.api.send_destroy)(self.inst) };
    }
}
