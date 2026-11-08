//! Spout2 sender (Windows), implemented natively: a D3D11 shared texture plus the
//! Spout shared-memory sender registry, so OBS & co. can receive the avatar with alpha.
//!
//! Frames come from wgpu via asynchronous readback (no stalls); the newest finished frame
//! is uploaded into the shared D3D11 texture.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::gpu::{copy_to_buffer, padded_row, Gpu, Offscreen};

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    use anyhow::{Context, Result};
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, INVALID_HANDLE_VALUE};
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Dxgi::IDXGIResource;
    use windows::Win32::System::Memory::*;
    use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

    const NAME_LEN: usize = 256;
    const DEFAULT_MAX_SENDERS: usize = 64;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct SharedMem {
        handle: HANDLE,
        view: MEMORY_MAPPED_VIEW_ADDRESS,
        pub size: usize,
    }

    impl SharedMem {
        pub fn open_or_create(name: &str, size: usize) -> Result<SharedMem> {
            unsafe {
                let w = wide(name);
                let handle = CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, size as u32, PCWSTR(w.as_ptr()))
                    .with_context(|| format!("CreateFileMapping {name}"))?;
                // Map the whole object (it may already exist with another size).
                let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, 0);
                if view.Value.is_null() {
                    let _ = CloseHandle(handle);
                    anyhow::bail!("MapViewOfFile {name} failed");
                }
                let mut info = MEMORY_BASIC_INFORMATION::default();
                VirtualQuery(Some(view.Value), &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
                Ok(SharedMem { handle, view, size: info.RegionSize.max(size) })
            }
        }

        pub fn bytes(&mut self) -> &mut [u8] {
            unsafe { std::slice::from_raw_parts_mut(self.view.Value as *mut u8, self.size) }
        }
    }

    impl Drop for SharedMem {
        fn drop(&mut self) {
            unsafe {
                let _ = UnmapViewOfFile(self.view);
                let _ = CloseHandle(self.handle);
            }
        }
    }

    pub struct NamedMutex(HANDLE);

    impl NamedMutex {
        pub fn new(name: &str) -> Result<NamedMutex> {
            let w = wide(name);
            Ok(NamedMutex(unsafe { CreateMutexW(None, false, PCWSTR(w.as_ptr()))? }))
        }
        pub fn lock<R>(&self, f: impl FnOnce() -> R) -> R {
            unsafe {
                let _ = WaitForSingleObject(self.0, 67);
                let r = f();
                let _ = ReleaseMutex(self.0);
                r
            }
        }
    }

    impl Drop for NamedMutex {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn read_names(b: &[u8]) -> Vec<String> {
        b.chunks_exact(NAME_LEN)
            .map(|c| {
                let end = c.iter().position(|&x| x == 0).unwrap_or(NAME_LEN);
                String::from_utf8_lossy(&c[..end]).into_owned()
            })
            .take_while(|s| !s.is_empty())
            .collect()
    }

    fn write_names(b: &mut [u8], names: &[String]) {
        let slots = b.len() / NAME_LEN;
        for (i, slot) in b.chunks_exact_mut(NAME_LEN).enumerate().take(slots) {
            slot.fill(0);
            if let Some(n) = names.get(i) {
                let s = n.as_bytes();
                let l = s.len().min(NAME_LEN - 1);
                slot[..l].copy_from_slice(&s[..l]);
            }
        }
    }

    pub struct Sender {
        pub name: String,
        pub w: u32,
        pub h: u32,
        _device: ID3D11Device,
        ctx: ID3D11DeviceContext,
        tex: ID3D11Resource,
        names: SharedMem,
        names_mutex: NamedMutex,
        _info: SharedMem,
        access: NamedMutex,
        _active: SharedMem,
    }

    impl Sender {
        pub fn new(name: &str, w: u32, h: u32, bgra: bool) -> Result<Sender> {
            unsafe {
                let mut device = None;
                let mut ctx = None;
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    Some(&[D3D_FEATURE_LEVEL_11_0]),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut ctx),
                )
                .context("D3D11CreateDevice")?;
                let device: ID3D11Device = device.context("no D3D11 device")?;
                let ctx = ctx.context("no D3D11 context")?;
                let format = if bgra { DXGI_FORMAT_B8G8R8A8_UNORM } else { DXGI_FORMAT_R8G8B8A8_UNORM };
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: w,
                    Height: h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
                };
                let mut tex = None;
                device.CreateTexture2D(&desc, None, Some(&mut tex)).context("CreateTexture2D (shared)")?;
                let tex: ID3D11Texture2D = tex.context("no texture")?;
                let share: HANDLE = tex.cast::<IDXGIResource>()?.GetSharedHandle()?;

                // Per-sender info block (SharedTextureInfo, 280 bytes).
                let mut info = SharedMem::open_or_create(name, 280)?;
                {
                    let b = info.bytes();
                    b[..280].fill(0);
                    b[0..4].copy_from_slice(&(share.0 as usize as u32).to_le_bytes());
                    b[4..8].copy_from_slice(&w.to_le_bytes());
                    b[8..12].copy_from_slice(&h.to_le_bytes());
                    b[12..16].copy_from_slice(&(format.0 as u32).to_le_bytes());
                    let desc = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    let d = desc.as_bytes();
                    let l = d.len().min(255);
                    b[20..20 + l].copy_from_slice(&d[..l]);
                }
                let access = NamedMutex::new(&format!("{name}_SpoutAccessMutex"))?;

                // Register in the global sender list.
                let mut names = SharedMem::open_or_create("SpoutSenderNames", DEFAULT_MAX_SENDERS * NAME_LEN)?;
                let names_mutex = NamedMutex::new("SpoutSenderNames_mutex")?;
                names_mutex.lock(|| {
                    let b = names.bytes();
                    let mut list = read_names(b);
                    if !list.iter().any(|n| n == name) {
                        list.push(name.to_string());
                        list.sort();
                    }
                    write_names(b, &list);
                });
                let mut active = SharedMem::open_or_create("ActiveSenderName", NAME_LEN)?;
                {
                    let b = active.bytes();
                    let cur = read_names(&b[..NAME_LEN]);
                    let registered = names_mutex.lock(|| read_names(names.bytes()));
                    if cur.first().map(|c| !registered.contains(c)).unwrap_or(true) {
                        write_names(&mut b[..NAME_LEN], &[name.to_string()]);
                    }
                }
                Ok(Sender {
                    name: name.to_string(),
                    w,
                    h,
                    _device: device,
                    ctx,
                    tex: tex.cast()?,
                    names,
                    names_mutex,
                    _info: info,
                    access,
                    _active: active,
                })
            }
        }

        pub fn send(&self, data: &[u8], row_pitch: u32) {
            self.access.lock(|| unsafe {
                self.ctx.UpdateSubresource(&self.tex, 0, None, data.as_ptr() as *const c_void, row_pitch, 0);
                self.ctx.Flush();
            });
        }
    }

    impl Drop for Sender {
        fn drop(&mut self) {
            let name = self.name.clone();
            let names = &mut self.names;
            self.names_mutex.lock(|| {
                let b = names.bytes();
                let list: Vec<String> = read_names(b).into_iter().filter(|n| *n != name).collect();
                write_names(b, &list);
            });
        }
    }
}

const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;

struct Slot {
    buf: wgpu::Buffer,
    state: Arc<AtomicU8>,
    seq: u64,
}

/// Where finished frames go.
pub enum Sink {
    #[cfg(windows)]
    Spout(win::Sender),
    Ndi(crate::ndi::NdiSender),
    Vcam(crate::vcam::Writer, crate::vcam::Camera, bool),
}

/// Off-screen video output (Spout2 or NDI): avatar + items on a transparent background,
/// read back asynchronously (ring of 3 buffers, no GPU stalls).
pub struct VideoOut {
    sink: Sink,
    pub offscreen: Offscreen,
    slots: Vec<Slot>,
    seq: u64,
    submitted: Option<usize>,
    pub frames_sent: u64,
    /// Draw the background too (virtual camera has no alpha).
    pub with_bg: bool,
}

impl VideoOut {
    fn with_sink(gpu: &Gpu, sink: Sink, w: u32, h: u32, mirror: bool) -> VideoOut {
        let size = padded_row(w) as u64 * h as u64;
        let slots = (0..3)
            .map(|_| Slot {
                buf: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("video-readback"),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
                seq: 0,
            })
            .collect();
        let offscreen = gpu.offscreen(w, h);
        offscreen.set_mirror(&gpu.queue, mirror);
        VideoOut { sink, offscreen, slots, seq: 0, submitted: None, frames_sent: 0, with_bg: false }
    }

    pub fn spout(gpu: &Gpu, name: &str, w: u32, h: u32, mirror: bool) -> Result<VideoOut> {
        if w == 0 || h == 0 {
            bail!("empty size");
        }
        #[cfg(not(windows))]
        {
            let _ = (gpu, name, mirror);
            bail!("Spout2 is Windows-only");
        }
        #[cfg(windows)]
        {
            let sender = win::Sender::new(name, w, h, gpu.is_bgra()).context("Spout2 sender")?;
            Ok(Self::with_sink(gpu, Sink::Spout(sender), w, h, mirror))
        }
    }

    pub fn ndi(gpu: &Gpu, name: &str, w: u32, h: u32, mirror: bool) -> Result<VideoOut> {
        if w == 0 || h == 0 {
            bail!("empty size");
        }
        let s = crate::ndi::NdiSender::new(name, gpu.is_bgra())?;
        Ok(Self::with_sink(gpu, Sink::Ndi(s), w, h, mirror))
    }

    pub fn vcam(gpu: &Gpu, mirror: bool) -> Result<VideoOut> {
        let cam = crate::vcam::Camera::start()?;
        let mut v = Self::with_sink(gpu, Sink::Vcam(crate::vcam::Writer::new(), cam, gpu.is_bgra()), crate::vcam::WIDTH, crate::vcam::HEIGHT, mirror);
        v.with_bg = true;
        Ok(v)
    }

    /// Virtual camera: is some program currently reading frames?
    pub fn vcam_connected(&self) -> bool {
        matches!(&self.sink, Sink::Vcam(w, _, _) if w.connected)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.offscreen.w, self.offscreen.h)
    }

    pub fn set_mirror(&self, gpu: &Gpu, mirror: bool) {
        self.offscreen.set_mirror(&gpu.queue, mirror);
    }

    /// Records the copy of the (already rendered + unpremultiplied) frame into a free slot.
    pub fn record_copy(&mut self, enc: &mut wgpu::CommandEncoder) {
        self.submitted = None;
        if let Some(i) = self.slots.iter().position(|s| s.state.load(Ordering::Acquire) == FREE) {
            copy_to_buffer(enc, &self.offscreen.out, &self.slots[i].buf, self.offscreen.w, self.offscreen.h);
            self.submitted = Some(i);
        }
    }

    /// Call right after `queue.submit`.
    pub fn after_submit(&mut self) {
        if let Some(i) = self.submitted.take() {
            self.seq += 1;
            let s = &mut self.slots[i];
            s.seq = self.seq;
            s.state.store(PENDING, Ordering::Release);
            let st = s.state.clone();
            s.buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                st.store(if r.is_ok() { READY } else { FREE }, Ordering::Release);
            });
        }
    }

    /// Sends the newest finished frame (call once per frame, after device.poll).
    pub fn pump(&mut self) {
        let mut newest: Option<usize> = None;
        for (i, s) in self.slots.iter().enumerate() {
            if s.state.load(Ordering::Acquire) == READY && newest.map(|n| self.slots[n].seq < s.seq).unwrap_or(true) {
                newest = Some(i);
            }
        }
        let (w, h) = (self.offscreen.w, self.offscreen.h);
        for (i, s) in self.slots.iter().enumerate() {
            if s.state.load(Ordering::Acquire) != READY {
                continue;
            }
            if Some(i) == newest {
                let data = s.buf.slice(..).get_mapped_range();
                // Dev hook: RIGRS_DUMP_OUT=file.png saves one output frame 12 s after start.
                static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
                static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                let start = *START.get_or_init(std::time::Instant::now);
                if start.elapsed().as_secs() >= 12 && !DONE.swap(true, Ordering::Relaxed) {
                    if let Some(path) = std::env::var_os("RIGRS_DUMP_OUT") {
                        let row = padded_row(w) as usize;
                        let mut px = Vec::with_capacity((w * h * 4) as usize);
                        for y in 0..h as usize {
                            px.extend_from_slice(&data[y * row..y * row + w as usize * 4]);
                        }
                        if let Some(img) = image::RgbaImage::from_raw(w, h, px) {
                            let _ = img.save(path);
                        }
                    }
                }
                match &mut self.sink {
                    #[cfg(windows)]
                    Sink::Spout(sender) => sender.send(&data, padded_row(w)),
                    Sink::Ndi(n) => n.send(&data, w, h, padded_row(w)),
                    Sink::Vcam(wr, _, bgra) => wr.write(&data, padded_row(w) as usize, *bgra),
                }
                drop(data);
                self.frames_sent += 1;
            }
            s.buf.unmap();
            s.state.store(FREE, Ordering::Release);
        }
    }
}
