//! Webcam capture for the built-in tracker (Windows: Media Foundation source reader with
//! the built-in video processor, so MJPG / YUY2 / NV12 cameras all arrive as RGB32).

use anyhow::Result;

/// A camera mode: `index` is its position in the device's native type list.
#[derive(Clone, Debug)]
pub struct Mode {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub format: String,
}

pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{anyhow, Context};
    use windows::core::{GUID, PWSTR};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};

    const STREAM: u32 = 0xFFFF_FFFC; // MF_SOURCE_READER_FIRST_VIDEO_STREAM

    fn init() -> Result<()> {
        unsafe {
            // Already initialised with another model on this thread is fine.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup")?;
        }
        Ok(())
    }

    unsafe fn devices() -> Result<Vec<IMFActivate>> {
        let mut attr: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attr, 1)?;
        let attr = attr.ok_or_else(|| anyhow!("MFCreateAttributes"))?;
        attr.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        MFEnumDeviceSources(&attr, &mut list, &mut count)?;
        let mut out = Vec::new();
        if !list.is_null() {
            for i in 0..count as usize {
                if let Some(a) = (*list.add(i)).take() {
                    out.push(a);
                }
            }
            CoTaskMemFree(Some(list as *const _));
        }
        Ok(out)
    }

    unsafe fn name_of(a: &IMFActivate) -> String {
        let mut s = PWSTR::null();
        let mut len = 0u32;
        if a.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut s, &mut len).is_ok() && !s.is_null() {
            let name = s.to_string().unwrap_or_default();
            CoTaskMemFree(Some(s.0 as *const _));
            return name;
        }
        String::new()
    }

    pub fn list() -> Vec<(i32, String)> {
        if init().is_err() {
            return Vec::new();
        }
        unsafe { devices().map(|d| d.iter().enumerate().map(|(i, a)| (i as i32, name_of(a))).collect()).unwrap_or_default() }
    }

    fn fourcc(g: &GUID) -> String {
        if *g == MFVideoFormat_RGB32 {
            return "RGB32".into();
        }
        if *g == MFVideoFormat_RGB24 {
            return "RGB24".into();
        }
        let b = g.data1.to_le_bytes();
        if b.iter().all(|c| c.is_ascii_graphic()) {
            String::from_utf8_lossy(&b).into_owned()
        } else {
            format!("{:08X}", g.data1)
        }
    }

    unsafe fn reader_for(camera: i32) -> Result<IMFSourceReader> {
        let devs = devices()?;
        let dev = devs.get(camera.max(0) as usize).ok_or_else(|| anyhow!("camera #{camera} not found ({} connected)", devs.len()))?;
        let source: IMFMediaSource = dev.ActivateObject().context("cannot open the camera (busy in another app?)")?;
        let mut attr: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attr, 2)?;
        let attr = attr.ok_or_else(|| anyhow!("MFCreateAttributes"))?;
        attr.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        Ok(MFCreateSourceReaderFromMediaSource(&source, &attr)?)
    }

    unsafe fn modes_of(reader: &IMFSourceReader) -> Vec<Mode> {
        let mut out = Vec::new();
        for i in 0..512u32 {
            let Ok(t) = reader.GetNativeMediaType(STREAM, i) else { break };
            let size = t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
            let rate = t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
            let (num, den) = ((rate >> 32) as f32, (rate & 0xFFFF_FFFF) as f32);
            let sub = t.GetGUID(&MF_MT_SUBTYPE).unwrap_or_default();
            out.push(Mode { index: i, width: (size >> 32) as u32, height: (size & 0xFFFF_FFFF) as u32, fps: if den > 0.0 { num / den } else { 0.0 }, format: fourcc(&sub) });
        }
        out
    }

    pub fn modes(camera: i32) -> Vec<Mode> {
        if init().is_err() {
            return Vec::new();
        }
        unsafe { reader_for(camera).map(|r| modes_of(&r)).unwrap_or_default() }
    }

    pub struct Camera {
        reader: IMFSourceReader,
        pub width: usize,
        pub height: usize,
        /// Rows arrive bottom-up.
        flip: bool,
    }

    // Used from the tracker thread only.
    unsafe impl Send for Camera {}

    impl Camera {
        /// Opens the camera in the native mode `mode` (or the closest to the wanted size / fps).
        pub fn open(camera: i32, mode: Option<u32>, width: u32, height: u32, fps: u32) -> Result<Camera> {
            init()?;
            unsafe {
                let reader = reader_for(camera)?;
                let modes = modes_of(&reader);
                let pick = mode
                    .and_then(|m| modes.iter().find(|x| x.index == m))
                    .or_else(|| {
                        modes.iter().min_by_key(|m| {
                            let d = (m.width as i64 - width as i64).abs() + (m.height as i64 - height as i64).abs();
                            // Prefer modes that reach the wanted frame rate.
                            d * 4 + if m.fps + 0.5 >= fps as f32 { 0 } else { 2000 } + (m.fps - fps as f32).abs() as i64
                        })
                    })
                    .cloned();
                if let Some(m) = &pick {
                    if let Ok(t) = reader.GetNativeMediaType(STREAM, m.index) {
                        let _ = reader.SetCurrentMediaType(STREAM, None, &t);
                    }
                }
                let out = MFCreateMediaType()?;
                out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                reader.SetCurrentMediaType(STREAM, None, &out).context("camera cannot deliver RGB frames")?;
                let cur = reader.GetCurrentMediaType(STREAM)?;
                let size = cur.GetUINT64(&MF_MT_FRAME_SIZE)?;
                let (w, h) = ((size >> 32) as usize, (size & 0xFFFF_FFFF) as usize);
                if w == 0 || h == 0 {
                    return Err(anyhow!("camera reported an empty frame size"));
                }
                // A negative default stride means bottom-up rows.
                let flip = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| (s as i32) < 0).unwrap_or(false);
                Ok(Camera { reader, width: w, height: h, flip })
            }
        }

        /// Blocks until the next frame (None on a stream gap).
        pub fn read(&mut self) -> Result<Option<Frame>> {
            unsafe {
                let (mut actual, mut flags, mut ts) = (0u32, 0u32, 0i64);
                let mut sample: Option<IMFSample> = None;
                self.reader.ReadSample(STREAM, 0, Some(&mut actual), Some(&mut flags), Some(&mut ts), Some(&mut sample)).context("camera read failed (unplugged?)")?;
                // MF_SOURCE_READERF_ERROR = 1, ENDOFSTREAM = 2.
                if flags & 3 != 0 {
                    return Err(anyhow!("camera stream ended"));
                }
                let Some(sample) = sample else { return Ok(None) };
                let buf = sample.ConvertToContiguousBuffer()?;
                let mut ptr = std::ptr::null_mut();
                let mut len = 0u32;
                buf.Lock(&mut ptr, None, Some(&mut len))?;
                let (w, h) = (self.width, self.height);
                let mut rgb = vec![0u8; w * h * 3];
                if (len as usize) >= w * h * 4 && !ptr.is_null() {
                    let src = std::slice::from_raw_parts(ptr, w * h * 4);
                    for y in 0..h {
                        let sy = if self.flip { h - 1 - y } else { y };
                        let (row, out) = (&src[sy * w * 4..(sy + 1) * w * 4], &mut rgb[y * w * 3..(y + 1) * w * 3]);
                        for x in 0..w {
                            // BGRA → RGB.
                            out[x * 3] = row[x * 4 + 2];
                            out[x * 3 + 1] = row[x * 4 + 1];
                            out[x * 3 + 2] = row[x * 4];
                        }
                    }
                }
                let _ = buf.Unlock();
                Ok(Some(Frame { w, h, rgb }))
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn list() -> Vec<(i32, String)> {
        Vec::new()
    }

    pub fn modes(_camera: i32) -> Vec<Mode> {
        Vec::new()
    }

    /// No native capture off Windows yet: `RIGRS_CAMERA_IMAGE` (a photo) stands in for the
    /// camera, which is what the Linux tests and demos use.
    pub struct Camera {
        pub width: usize,
        pub height: usize,
        rgb: Vec<u8>,
        n: u32,
    }

    impl Camera {
        pub fn open(_camera: i32, _mode: Option<u32>, _width: u32, _height: u32, _fps: u32) -> Result<Camera> {
            let path = std::env::var("RIGRS_CAMERA_IMAGE").map_err(|_| anyhow::anyhow!("camera capture is only implemented on Windows"))?;
            let img = image::open(path)?.to_rgb8();
            Ok(Camera { width: img.width() as usize, height: img.height() as usize, rgb: img.into_raw(), n: 0 })
        }

        pub fn read(&mut self) -> Result<Option<Frame>> {
            std::thread::sleep(std::time::Duration::from_millis(33));
            // Sway the picture a little so the tracker output moves.
            self.n += 1;
            let (w, h) = (self.width, self.height);
            let dx = ((self.n as f32 * 0.15).sin() * 12.0) as i64;
            let mut rgb = vec![0u8; w * h * 3];
            for y in 0..h {
                for x in 0..w {
                    let sx = x as i64 + dx;
                    if sx >= 0 && (sx as usize) < w {
                        let (s, d) = ((y * w + sx as usize) * 3, (y * w + x) * 3);
                        rgb[d..d + 3].copy_from_slice(&self.rgb[s..s + 3]);
                    }
                }
            }
            Ok(Some(Frame { w, h, rgb }))
        }
    }
}

pub use imp::{list, modes, Camera};
