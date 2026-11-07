//! rigrs virtual camera — a Media Foundation media source that the Windows 11 Frame Server
//! loads for a virtual camera created with `MFCreateVirtualCamera`.
//!
//! COM objects: ClassFactory → Activator (IMFActivate) → MediaSource (IMFMediaSourceEx) →
//! Stream (IMFMediaStream2). Frames come from the rigrs app through a shared memory block.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{implement, w, Interface, IUnknown, Result, GUID, HRESULT, PCWSTR, PROPVARIANT, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Media::KernelStreaming::{IKsControl, IKsControl_Impl, KSIDENTIFIER};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Registry::*;

/// CLSID of the media source activator.
pub const CLSID_RIGRS_VCAM: GUID = GUID::from_u128(0x7a1f3c2e_5b4d_4e8a_9c61_2f0b8d3e4a17);
pub const CLSID_STR: &str = "{7A1F3C2E-5B4D-4E8A-9C61-2F0B8D3E4A17}";

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
pub const FPS: u32 = 30;
/// Shared frame block: 32-byte header + WIDTH*HEIGHT*4 BGRA, top-down.
pub const MAP_NAME_GLOBAL: PCWSTR = w!("Global\\rigrs_vcam_v1");
pub const MAP_NAME_LOCAL: PCWSTR = w!("Local\\rigrs_vcam_v1");
pub const HEADER: usize = 32;
pub const MAGIC: u32 = u32::from_le_bytes(*b"RGVC");
pub const MAP_SIZE: usize = HEADER + (WIDTH * HEIGHT * 4) as usize;

static OBJECTS: AtomicI32 = AtomicI32::new(0);
static LOCKS: AtomicI32 = AtomicI32::new(0);
static mut MODULE: HMODULE = HMODULE(std::ptr::null_mut());

/// KSCATEGORY / PINNAME_VIDEO_CAPTURE
const PINNAME_VIDEO_CAPTURE: GUID = GUID::from_u128(0xfb6c4281_0353_11d1_905f_0000c0cc16ba);

const ERROR_SET_NOT_FOUND_HR: HRESULT = HRESULT(0x80070492u32 as i32);

#[no_mangle]
extern "system" fn DllMain(module: HMODULE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == 1 {
        unsafe { MODULE = module };
    }
    TRUE
}

// ------------------------------------------------------------------ shared frames

struct SharedFrames {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

unsafe impl Send for SharedFrames {}

impl SharedFrames {
    /// Created by the camera side (the Frame Server service may create Global\ objects);
    /// the DACL lets the user's rigrs process write into it.
    fn create() -> Option<SharedFrames> {
        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            let _ = ConvertStringSecurityDescriptorToSecurityDescriptorW(w!("D:(A;;GA;;;WD)(A;;GA;;;SY)(A;;GA;;;AC)S:(ML;;NW;;;LW)"), SDDL_REVISION_1, &mut sd, None);
            let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: FALSE };
            let mut h = CreateFileMappingW(INVALID_HANDLE_VALUE, Some(&sa), PAGE_READWRITE, 0, MAP_SIZE as u32, MAP_NAME_GLOBAL);
            if h.is_err() {
                h = CreateFileMappingW(INVALID_HANDLE_VALUE, Some(&sa), PAGE_READWRITE, 0, MAP_SIZE as u32, MAP_NAME_LOCAL);
            }
            let handle = h.ok()?;
            let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, MAP_SIZE);
            if view.Value.is_null() {
                let _ = CloseHandle(handle);
                return None;
            }
            Some(SharedFrames { handle, view })
        }
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, MAP_SIZE) }
    }
}

impl Drop for SharedFrames {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

/// "No signal" picture shown while rigrs is not sending frames.
fn fill_idle(dst: &mut [u8], t: u64) {
    let shift = (t / 2) as u32;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let i = ((y * WIDTH + x) * 4) as usize;
            let band = ((x + shift) / 160) % 7;
            let (r, g, b) = match band {
                0 => (192, 192, 192),
                1 => (192, 192, 0),
                2 => (0, 192, 192),
                3 => (0, 192, 0),
                4 => (192, 0, 192),
                5 => (192, 0, 0),
                _ => (0, 0, 192),
            };
            let dim = if y > HEIGHT * 3 / 4 { 3 } else { 1 };
            dst[i] = b / dim;
            dst[i + 1] = g / dim;
            dst[i + 2] = r / dim;
            dst[i + 3] = 255;
        }
    }
}

// ------------------------------------------------------------------ media type helpers

fn media_type() -> Result<IMFMediaType> {
    unsafe {
        let mt = MFCreateMediaType()?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        mt.SetUINT64(&MF_MT_FRAME_SIZE, ((WIDTH as u64) << 32) | HEIGHT as u64)?;
        mt.SetUINT64(&MF_MT_FRAME_RATE, ((FPS as u64) << 32) | 1)?;
        mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        mt.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        mt.SetUINT32(&MF_MT_DEFAULT_STRIDE, WIDTH * 4)?;
        mt.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        mt.SetUINT32(&MF_MT_SAMPLE_SIZE, WIDTH * HEIGHT * 4)?;
        mt.SetUINT32(&MF_MT_AVG_BITRATE, WIDTH * HEIGHT * 4 * 8 * FPS)?;
        Ok(mt)
    }
}

// ------------------------------------------------------------------ stream

/// State shared between the source and its stream.
struct StreamShared {
    queue: IMFMediaEventQueue,
    state: Mutex<MF_STREAM_STATE>,
    source: Mutex<Option<IMFMediaSource>>,
}

#[implement(IMFMediaStream2, IKsControl)]
pub struct Stream {
    sh: Arc<StreamShared>,
    sd: IMFStreamDescriptor,
    frames: Mutex<Option<SharedFrames>>,
    counter: Mutex<u64>,
}

fn fill_frame(frames: &Option<SharedFrames>, dst: &mut [u8], n: u64) {
    if let Some(f) = frames {
        let b = f.bytes();
        let magic = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let w = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        let h = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        // b[16..24] = sequence, b[24..32] = writer tick (ms) — stale data counts as no signal.
        let tick = u64::from_le_bytes(b[24..32].try_into().unwrap());
        let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
        if magic == MAGIC && w == WIDTH && h == HEIGHT && now.saturating_sub(tick) < 2000 {
            dst.copy_from_slice(&b[HEADER..HEADER + dst.len()]);
            return;
        }
    }
    fill_idle(dst, n);
}

impl IMFMediaEventGenerator_Impl for Stream_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        unsafe { self.sh.queue.GetEvent(flags.0 as u32) }
    }
    fn BeginGetEvent(&self, cb: Option<&IMFAsyncCallback>, state: Option<&IUnknown>) -> Result<()> {
        unsafe { self.sh.queue.BeginGetEvent(cb, state) }
    }
    fn EndGetEvent(&self, r: Option<&IMFAsyncResult>) -> Result<IMFMediaEvent> {
        unsafe { self.sh.queue.EndGetEvent(r) }
    }
    fn QueueEvent(&self, met: u32, ext: *const GUID, hr: HRESULT, pv: *const PROPVARIANT) -> Result<()> {
        unsafe { self.sh.queue.QueueEventParamVar(met, ext, hr, pv) }
    }
}

impl IMFMediaStream_Impl for Stream_Impl {
    fn GetMediaSource(&self) -> Result<IMFMediaSource> {
        self.sh.source.lock().unwrap().clone().ok_or_else(|| MF_E_SHUTDOWN.into())
    }
    fn GetStreamDescriptor(&self) -> Result<IMFStreamDescriptor> {
        Ok(self.sd.clone())
    }
    fn RequestSample(&self, token: Option<&IUnknown>) -> Result<()> {
        if *self.sh.state.lock().unwrap() != MF_STREAM_STATE_RUNNING {
            return Err(MF_E_INVALIDREQUEST.into());
        }
        unsafe {
            let size = WIDTH * HEIGHT * 4;
            let buf = MFCreateMemoryBuffer(size)?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            buf.Lock(&mut ptr, None, None)?;
            let dst = std::slice::from_raw_parts_mut(ptr, size as usize);
            let n = {
                let mut c = self.counter.lock().unwrap();
                *c += 1;
                *c
            };
            fill_frame(&self.frames.lock().unwrap(), dst, n);
            buf.Unlock()?;
            buf.SetCurrentLength(size)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            sample.SetSampleTime(MFGetSystemTime())?;
            sample.SetSampleDuration(10_000_000 / FPS as i64)?;
            if let Some(t) = token {
                sample.SetUnknown(&MFSampleExtension_Token, t)?;
            }
            self.sh.queue.QueueEventParamUnk(MEMediaSample.0 as u32, &GUID::zeroed(), S_OK, &sample)?;
        }
        Ok(())
    }
}

impl IMFMediaStream2_Impl for Stream_Impl {
    fn SetStreamState(&self, value: MF_STREAM_STATE) -> Result<()> {
        *self.sh.state.lock().unwrap() = value;
        Ok(())
    }
    fn GetStreamState(&self) -> Result<MF_STREAM_STATE> {
        Ok(*self.sh.state.lock().unwrap())
    }
}

impl IKsControl_Impl for Stream_Impl {
    fn KsProperty(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND_HR.into())
    }
    fn KsMethod(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND_HR.into())
    }
    fn KsEvent(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND_HR.into())
    }
}

// ------------------------------------------------------------------ source

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl)]
pub struct Source {
    queue: IMFMediaEventQueue,
    attrs: IMFAttributes,
    pd: IMFPresentationDescriptor,
    sd_attrs: IMFAttributes,
    stream: IMFMediaStream2,
    sh: Arc<StreamShared>,
    stream_obj: Mutex<bool>,
}

impl Source {
    pub fn create() -> Result<IMFMediaSource> {
        unsafe {
            let mt = media_type()?;
            let sd = MFCreateStreamDescriptor(0, &[Some(mt.clone())])?;
            let handler = sd.GetMediaTypeHandler()?;
            handler.SetCurrentMediaType(&mt)?;
            let sd_attrs: IMFAttributes = sd.cast()?;
            sd_attrs.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
            sd_attrs.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
            sd_attrs.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
            sd_attrs.SetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color.0 as u32)?;
            let pd = MFCreatePresentationDescriptor(Some(&[Some(sd.clone())]))?;
            pd.SelectStream(0)?;
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 4)?;
            let attrs = attrs.unwrap();
            let sh = Arc::new(StreamShared { queue: MFCreateEventQueue()?, state: Mutex::new(MF_STREAM_STATE_STOPPED), source: Mutex::new(None) });
            let stream: IMFMediaStream2 = Stream { sh: sh.clone(), sd, frames: Mutex::new(SharedFrames::create()), counter: Mutex::new(0) }.into();
            let ex: IMFMediaSourceEx = Source { queue: MFCreateEventQueue()?, attrs, pd, sd_attrs, stream: stream.clone(), sh: sh.clone(), stream_obj: Mutex::new(false) }.into();
            let src: IMFMediaSource = ex.cast()?;
            // Give the stream its back-reference (broken again on Shutdown).
            *sh.source.lock().unwrap() = Some(src.clone());
            Ok(src)
        }
    }
}

impl IMFMediaEventGenerator_Impl for Source_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        unsafe { self.queue.GetEvent(flags.0 as u32) }
    }
    fn BeginGetEvent(&self, cb: Option<&IMFAsyncCallback>, state: Option<&IUnknown>) -> Result<()> {
        unsafe { self.queue.BeginGetEvent(cb, state) }
    }
    fn EndGetEvent(&self, r: Option<&IMFAsyncResult>) -> Result<IMFMediaEvent> {
        unsafe { self.queue.EndGetEvent(r) }
    }
    fn QueueEvent(&self, met: u32, ext: *const GUID, hr: HRESULT, pv: *const PROPVARIANT) -> Result<()> {
        unsafe { self.queue.QueueEventParamVar(met, ext, hr, pv) }
    }
}

impl IMFMediaSource_Impl for Source_Impl {
    fn GetCharacteristics(&self) -> Result<u32> {
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }
    fn CreatePresentationDescriptor(&self) -> Result<IMFPresentationDescriptor> {
        unsafe { self.pd.Clone() }
    }
    fn Start(&self, _pd: Option<&IMFPresentationDescriptor>, _fmt: *const GUID, _pos: *const PROPVARIANT) -> Result<()> {
        unsafe {
            let first = {
                let mut b = self.stream_obj.lock().unwrap();
                let f = !*b;
                *b = true;
                f
            };
            let met = if first { MENewStream } else { MEUpdatedStream };
            self.queue.QueueEventParamUnk(met.0 as u32, &GUID::zeroed(), S_OK, &self.stream)?;
            let s = &self.sh;
            *s.state.lock().unwrap() = MF_STREAM_STATE_RUNNING;
            let t = PROPVARIANT::from(MFGetSystemTime());
            s.queue.QueueEventParamVar(MEStreamStarted.0 as u32, &GUID::zeroed(), S_OK, &t)?;
            self.queue.QueueEventParamVar(MESourceStarted.0 as u32, &GUID::zeroed(), S_OK, &t)?;
        }
        Ok(())
    }
    fn Stop(&self) -> Result<()> {
        unsafe {
            let s = &self.sh;
            *s.state.lock().unwrap() = MF_STREAM_STATE_STOPPED;
            let empty = PROPVARIANT::default();
            s.queue.QueueEventParamVar(MEStreamStopped.0 as u32, &GUID::zeroed(), S_OK, &empty)?;
            self.queue.QueueEventParamVar(MESourceStopped.0 as u32, &GUID::zeroed(), S_OK, &empty)?;
        }
        Ok(())
    }
    fn Pause(&self) -> Result<()> {
        Err(MF_E_INVALID_STATE_TRANSITION.into())
    }
    fn Shutdown(&self) -> Result<()> {
        unsafe {
            let s = &self.sh;
            *s.state.lock().unwrap() = MF_STREAM_STATE_STOPPED;
            *s.source.lock().unwrap() = None;
            let _ = s.queue.Shutdown();
            let _ = self.queue.Shutdown();
        }
        Ok(())
    }
}

impl IMFMediaSourceEx_Impl for Source_Impl {
    fn GetSourceAttributes(&self) -> Result<IMFAttributes> {
        Ok(self.attrs.clone())
    }
    fn GetStreamAttributes(&self, id: u32) -> Result<IMFAttributes> {
        if id == 0 {
            Ok(self.sd_attrs.clone())
        } else {
            Err(E_INVALIDARG.into())
        }
    }
    fn SetD3DManager(&self, _: Option<&IUnknown>) -> Result<()> {
        Ok(())
    }
}

impl IMFGetService_Impl for Source_Impl {
    fn GetService(&self, _: *const GUID, _: *const GUID, _: *mut *mut c_void) -> Result<()> {
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}

impl IKsControl_Impl for Source_Impl {
    fn KsProperty(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND_HR.into())
    }
    fn KsMethod(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND_HR.into())
    }
    fn KsEvent(&self, _: *const KSIDENTIFIER, _: u32, _: *mut c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(ERROR_SET_NOT_FOUND_HR.into())
    }
}

// ------------------------------------------------------------------ activator (IMFActivate)

#[implement(IMFActivate)]
pub struct Activator {
    attrs: IMFAttributes,
    source: Mutex<Option<IMFMediaSource>>,
}

impl Activator {
    pub fn new() -> Result<Activator> {
        OBJECTS.fetch_add(1, Ordering::SeqCst);
        let mut attrs = None;
        unsafe { MFCreateAttributes(&mut attrs, 1)? };
        Ok(Activator { attrs: attrs.unwrap(), source: Mutex::new(None) })
    }
}

impl Drop for Activator {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IMFActivate_Impl for Activator_Impl {
    fn ActivateObject(&self, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        let mut src = self.source.lock().unwrap();
        if src.is_none() {
            *src = Some(Source::create()?);
        }
        unsafe { src.as_ref().unwrap().query(riid, ppv).ok() }
    }
    fn ShutdownObject(&self) -> Result<()> {
        if let Some(s) = self.source.lock().unwrap().take() {
            unsafe {
                let _ = s.Shutdown();
            }
        }
        Ok(())
    }
    fn DetachObject(&self) -> Result<()> {
        *self.source.lock().unwrap() = None;
        Ok(())
    }
}

// All IMFAttributes calls are forwarded to an inner attribute store.
impl IMFAttributes_Impl for Activator_Impl {
    fn GetItem(&self, k: *const GUID, v: *mut PROPVARIANT) -> Result<()> {
        unsafe { self.attrs.GetItem(k, Some(v)) }
    }
    fn GetItemType(&self, k: *const GUID) -> Result<MF_ATTRIBUTE_TYPE> {
        unsafe { self.attrs.GetItemType(k) }
    }
    fn CompareItem(&self, k: *const GUID, v: *const PROPVARIANT) -> Result<BOOL> {
        unsafe { self.attrs.CompareItem(k, v) }
    }
    fn Compare(&self, t: Option<&IMFAttributes>, m: MF_ATTRIBUTES_MATCH_TYPE) -> Result<BOOL> {
        unsafe { self.attrs.Compare(t, m) }
    }
    fn GetUINT32(&self, k: *const GUID) -> Result<u32> {
        unsafe { self.attrs.GetUINT32(k) }
    }
    fn GetUINT64(&self, k: *const GUID) -> Result<u64> {
        unsafe { self.attrs.GetUINT64(k) }
    }
    fn GetDouble(&self, k: *const GUID) -> Result<f64> {
        unsafe { self.attrs.GetDouble(k) }
    }
    fn GetGUID(&self, k: *const GUID) -> Result<GUID> {
        unsafe { self.attrs.GetGUID(k) }
    }
    fn GetStringLength(&self, k: *const GUID) -> Result<u32> {
        unsafe { self.attrs.GetStringLength(k) }
    }
    fn GetString(&self, k: *const GUID, s: PWSTR, n: u32, len: *mut u32) -> Result<()> {
        unsafe { self.attrs.GetString(k, std::slice::from_raw_parts_mut(s.0, n as usize), Some(len)) }
    }
    fn GetAllocatedString(&self, k: *const GUID, s: *mut PWSTR, len: *mut u32) -> Result<()> {
        unsafe { self.attrs.GetAllocatedString(k, s, len) }
    }
    fn GetBlobSize(&self, k: *const GUID) -> Result<u32> {
        unsafe { self.attrs.GetBlobSize(k) }
    }
    fn GetBlob(&self, k: *const GUID, b: *mut u8, n: u32, len: *mut u32) -> Result<()> {
        unsafe { self.attrs.GetBlob(k, std::slice::from_raw_parts_mut(b, n as usize), Some(len)) }
    }
    fn GetAllocatedBlob(&self, k: *const GUID, b: *mut *mut u8, n: *mut u32) -> Result<()> {
        unsafe { self.attrs.GetAllocatedBlob(k, b, n) }
    }
    fn GetUnknown(&self, k: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        unsafe { (Interface::vtable(&self.attrs).GetUnknown)(Interface::as_raw(&self.attrs), k, riid, ppv).ok() }
    }
    fn SetItem(&self, k: *const GUID, v: *const PROPVARIANT) -> Result<()> {
        unsafe { self.attrs.SetItem(k, v) }
    }
    fn DeleteItem(&self, k: *const GUID) -> Result<()> {
        unsafe { self.attrs.DeleteItem(k) }
    }
    fn DeleteAllItems(&self) -> Result<()> {
        unsafe { self.attrs.DeleteAllItems() }
    }
    fn SetUINT32(&self, k: *const GUID, v: u32) -> Result<()> {
        unsafe { self.attrs.SetUINT32(k, v) }
    }
    fn SetUINT64(&self, k: *const GUID, v: u64) -> Result<()> {
        unsafe { self.attrs.SetUINT64(k, v) }
    }
    fn SetDouble(&self, k: *const GUID, v: f64) -> Result<()> {
        unsafe { self.attrs.SetDouble(k, v) }
    }
    fn SetGUID(&self, k: *const GUID, v: *const GUID) -> Result<()> {
        unsafe { self.attrs.SetGUID(k, v) }
    }
    fn SetString(&self, k: *const GUID, v: &PCWSTR) -> Result<()> {
        unsafe { self.attrs.SetString(k, *v) }
    }
    fn SetBlob(&self, k: *const GUID, b: *const u8, n: u32) -> Result<()> {
        unsafe { self.attrs.SetBlob(k, std::slice::from_raw_parts(b, n as usize)) }
    }
    fn SetUnknown(&self, k: *const GUID, u: Option<&IUnknown>) -> Result<()> {
        unsafe { self.attrs.SetUnknown(k, u) }
    }
    fn LockStore(&self) -> Result<()> {
        unsafe { self.attrs.LockStore() }
    }
    fn UnlockStore(&self) -> Result<()> {
        unsafe { self.attrs.UnlockStore() }
    }
    fn GetCount(&self) -> Result<u32> {
        unsafe { self.attrs.GetCount() }
    }
    fn GetItemByIndex(&self, i: u32, k: *mut GUID, v: *mut PROPVARIANT) -> Result<()> {
        unsafe { self.attrs.GetItemByIndex(i, k, Some(v)) }
    }
    fn CopyAllItems(&self, d: Option<&IMFAttributes>) -> Result<()> {
        unsafe { self.attrs.CopyAllItems(d) }
    }
}

// ------------------------------------------------------------------ class factory + exports

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Option<&IUnknown>, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let a: IMFActivate = Activator::new()?.into();
        unsafe { a.query(riid, ppv).ok() }
    }
    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            LOCKS.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[no_mangle]
unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if ppv.is_null() || clsid.is_null() {
        return E_POINTER;
    }
    *ppv = std::ptr::null_mut();
    if *clsid != CLSID_RIGRS_VCAM {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let f: IClassFactory = Factory.into();
    f.query(riid, ppv)
}

#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_value(key: HKEY, name: Option<&str>, value: &str) -> windows::core::Result<()> {
    let v = wide(value);
    let n = name.map(wide);
    let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
    unsafe { RegSetValueExW(key, n.as_ref().map(|n| PCWSTR(n.as_ptr())).unwrap_or(PCWSTR::null()), 0, REG_SZ, Some(bytes)).ok() }
}

/// regsvr32: registers the class under HKLM (needs administrator rights).
#[no_mangle]
extern "system" fn DllRegisterServer() -> HRESULT {
    let mut path = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(MODULE, &mut path) } as usize;
    let dll = String::from_utf16_lossy(&path[..n]);
    let sub = wide(&format!("Software\\Classes\\CLSID\\{CLSID_STR}\\InprocServer32"));
    let mut key = HKEY::default();
    let r = unsafe { RegCreateKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(sub.as_ptr()), 0, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_ALL_ACCESS, None, &mut key, None) };
    if r.is_err() {
        return r.to_hresult();
    }
    let res = set_value(key, None, &dll).and_then(|_| set_value(key, Some("ThreadingModel"), "Both"));
    unsafe {
        let _ = RegCloseKey(key);
    }
    match res {
        Ok(_) => S_OK,
        Err(e) => e.code(),
    }
}

#[no_mangle]
extern "system" fn DllUnregisterServer() -> HRESULT {
    let sub = wide(&format!("Software\\Classes\\CLSID\\{CLSID_STR}"));
    unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(sub.as_ptr())).to_hresult() }
}
