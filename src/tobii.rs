//! Tobii eye tracker support through the Tobii Stream Engine C API (tobii_stream_engine.dll,
//! shipped with Tobii Experience / Tobii Game Hub). Loaded dynamically on a worker thread;
//! without the DLL or a device the card simply reports the status.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct TobiiState {
    pub status: String,
    pub connected: bool,
    /// Normalised display coordinates 0..1 (origin top-left).
    pub gaze: Option<[f32; 2]>,
    /// Head position in mm and rotation in radians (x = pitch, y = yaw, z = roll).
    pub head_pos: Option<[f32; 3]>,
    pub head_rot: Option<[f32; 3]>,
    pub updated: Option<Instant>,
}

#[repr(C)]
struct GazePoint {
    timestamp_us: i64,
    validity: i32,
    position_xy: [f32; 2],
}

#[repr(C)]
struct HeadPose {
    timestamp_us: i64,
    position_validity: i32,
    position_xyz: [f32; 3],
    rotation_validity_xyz: [i32; 3],
    rotation_xyz: [f32; 3],
}

const VALID: i32 = 1;
const FIELD_OF_USE_INTERACTIVE: i32 = 1;

type UrlCb = unsafe extern "C" fn(*const c_char, *mut c_void);
type GazeCb = unsafe extern "C" fn(*const GazePoint, *mut c_void);
type HeadCb = unsafe extern "C" fn(*const HeadPose, *mut c_void);

struct Api {
    _lib: libloading::Library,
    api_create: unsafe extern "C" fn(*mut *mut c_void, *const c_void, *const c_void) -> i32,
    api_destroy: unsafe extern "C" fn(*mut c_void) -> i32,
    enumerate: unsafe extern "C" fn(*mut c_void, UrlCb, *mut c_void) -> i32,
    device_create: unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut *mut c_void) -> i32,
    device_destroy: unsafe extern "C" fn(*mut c_void) -> i32,
    gaze_subscribe: unsafe extern "C" fn(*mut c_void, GazeCb, *mut c_void) -> i32,
    head_subscribe: unsafe extern "C" fn(*mut c_void, HeadCb, *mut c_void) -> i32,
    wait: unsafe extern "C" fn(i32, *const *mut c_void) -> i32,
    process: unsafe extern "C" fn(*mut c_void) -> i32,
}

fn dll_candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("tobii_stream_engine.dll")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            v.push(d.join("tobii_stream_engine.dll"));
        }
    }
    for base in [r"C:\Program Files\Tobii", r"C:\Program Files (x86)\Tobii"] {
        for sub in ["Tobii Experience", "Tobii EyeX", "Tobii Game Hub", "Service"] {
            v.push(PathBuf::from(base).join(sub).join("tobii_stream_engine.dll"));
        }
    }
    v
}

impl Api {
    fn load() -> Option<Api> {
        for p in dll_candidates() {
            let Ok(lib) = (unsafe { libloading::Library::new(&p) }) else { continue };
            let syms = unsafe {
                (|| -> Result<_, libloading::Error> {
                    Ok((
                        *lib.get(b"tobii_api_create\0")?,
                        *lib.get(b"tobii_api_destroy\0")?,
                        *lib.get(b"tobii_enumerate_local_device_urls\0")?,
                        *lib.get(b"tobii_device_create\0")?,
                        *lib.get(b"tobii_device_destroy\0")?,
                        *lib.get(b"tobii_gaze_point_subscribe\0")?,
                        *lib.get(b"tobii_head_pose_subscribe\0")?,
                        *lib.get(b"tobii_wait_for_callbacks\0")?,
                        *lib.get(b"tobii_device_process_callbacks\0")?,
                    ))
                })()
            };
            if let Ok((api_create, api_destroy, enumerate, device_create, device_destroy, gaze_subscribe, head_subscribe, wait, process)) = syms {
                return Some(Api { _lib: lib, api_create, api_destroy, enumerate, device_create, device_destroy, gaze_subscribe, head_subscribe, wait, process });
            }
        }
        None
    }
}

unsafe extern "C" fn url_cb(url: *const c_char, ud: *mut c_void) {
    let v = &mut *(ud as *mut Vec<String>);
    if !url.is_null() {
        v.push(CStr::from_ptr(url).to_string_lossy().into_owned());
    }
}

unsafe extern "C" fn gaze_cb(g: *const GazePoint, ud: *mut c_void) {
    let st = &*(ud as *const Mutex<TobiiState>);
    let g = &*g;
    if g.validity == VALID {
        let mut s = st.lock().unwrap();
        s.gaze = Some(g.position_xy);
        s.updated = Some(Instant::now());
    }
}

unsafe extern "C" fn head_cb(h: *const HeadPose, ud: *mut c_void) {
    let st = &*(ud as *const Mutex<TobiiState>);
    let h = &*h;
    let mut s = st.lock().unwrap();
    if h.position_validity == VALID {
        s.head_pos = Some(h.position_xyz);
    }
    if h.rotation_validity_xyz.iter().all(|&v| v == VALID) {
        s.head_rot = Some(h.rotation_xyz);
    }
    s.updated = Some(Instant::now());
}

pub struct Tobii {
    pub state: Arc<Mutex<TobiiState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Tobii {
    pub fn start() -> Tobii {
        let state = Arc::new(Mutex::new(TobiiState { status: "Запуск…".into(), ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        let thread = std::thread::Builder::new().name("tobii".into()).spawn(move || run(st, sp)).ok();
        Tobii { state, stop, thread }
    }

    pub fn snapshot(&self) -> TobiiState {
        self.state.lock().unwrap().clone()
    }
}

impl Drop for Tobii {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn set_status(st: &Mutex<TobiiState>, s: &str, connected: bool) {
    let mut g = st.lock().unwrap();
    g.status = s.to_string();
    g.connected = connected;
}

fn run(st: Arc<Mutex<TobiiState>>, stop: Arc<AtomicBool>) {
    let Some(api) = Api::load() else {
        set_status(&st, "Tobii Stream Engine не найден — установите Tobii Experience", false);
        return;
    };
    let mut ctx: *mut c_void = std::ptr::null_mut();
    if unsafe { (api.api_create)(&mut ctx, std::ptr::null(), std::ptr::null()) } != 0 || ctx.is_null() {
        set_status(&st, "Не удалось инициализировать Tobii API", false);
        return;
    }
    while !stop.load(Ordering::Relaxed) {
        let mut urls: Vec<String> = Vec::new();
        unsafe { (api.enumerate)(ctx, url_cb, &mut urls as *mut _ as *mut c_void) };
        let Some(url) = urls.first().cloned() else {
            set_status(&st, "Трекер Tobii не подключён", false);
            std::thread::sleep(Duration::from_secs(3));
            continue;
        };
        let curl = CString::new(url).unwrap_or_default();
        let mut dev: *mut c_void = std::ptr::null_mut();
        if unsafe { (api.device_create)(ctx, curl.as_ptr(), FIELD_OF_USE_INTERACTIVE, &mut dev) } != 0 || dev.is_null() {
            set_status(&st, "Не удалось подключиться к трекеру Tobii", false);
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }
        let ud = Arc::as_ptr(&st) as *mut c_void;
        unsafe {
            (api.gaze_subscribe)(dev, gaze_cb, ud);
            (api.head_subscribe)(dev, head_cb, ud);
        }
        set_status(&st, "Трекер подключён", true);
        let mut failures = 0;
        while !stop.load(Ordering::Relaxed) {
            let devs = [dev];
            let w = unsafe { (api.wait)(1, devs.as_ptr()) };
            let p = unsafe { (api.process)(dev) };
            if p != 0 && w != 0 {
                failures += 1;
                if failures > 50 {
                    break; // connection lost → reconnect
                }
                std::thread::sleep(Duration::from_millis(20));
            } else {
                failures = 0;
            }
        }
        unsafe { (api.device_destroy)(dev) };
        set_status(&st, "Соединение с трекером потеряно, переподключение…", false);
    }
    unsafe { (api.api_destroy)(ctx) };
}
