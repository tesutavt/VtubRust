//! Lighting overlay: samples the average colour of a monitor or window (GDI, ~15 Hz, a
//! 32x18 downscale) so the avatar can be tinted by what is on screen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub const THUMB_W: usize = 32;
pub const THUMB_H: usize = 18;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    Monitor(usize),
    Window(isize),
}

#[derive(Clone, Default)]
pub struct Sample {
    pub color: [f32; 3],
    /// RGBA thumbnail of the captured area (THUMB_W x THUMB_H).
    pub thumb: Vec<u8>,
    pub ok: bool,
}

pub struct Sampler {
    pub source: Source,
    sample: Arc<Mutex<Sample>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Sampler {
    pub fn start(source: Source) -> Sampler {
        let sample = Arc::new(Mutex::new(Sample::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, st2) = (sample.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("lighting".into())
            .spawn(move || {
                while !st2.load(Ordering::Relaxed) {
                    let r = imp::capture(source);
                    *s2.lock().unwrap() = r.unwrap_or_default();
                    std::thread::sleep(Duration::from_millis(66));
                }
            })
            .ok();
        Sampler { source, sample, stop, thread }
    }

    pub fn sample(&self) -> Sample {
        self.sample.lock().unwrap().clone()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Monitor rectangles (virtual-screen coordinates), in system order.
pub fn monitors() -> Vec<[i32; 4]> {
    imp::monitors()
}

/// Visible top-level windows with a title: (handle, title).
pub fn windows() -> Vec<(isize, String)> {
    imp::windows()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindowVisible};

    unsafe extern "system" fn mon_cb(_m: HMONITOR, _dc: HDC, r: *mut RECT, data: LPARAM) -> BOOL {
        let v = &mut *(data.0 as *mut Vec<[i32; 4]>);
        let r = &*r;
        v.push([r.left, r.top, r.right, r.bottom]);
        BOOL(1)
    }

    pub fn monitors() -> Vec<[i32; 4]> {
        let mut v: Vec<[i32; 4]> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(HDC::default(), None, Some(mon_cb), LPARAM(&mut v as *mut _ as isize));
        }
        v
    }

    unsafe extern "system" fn win_cb(h: HWND, data: LPARAM) -> BOOL {
        let v = &mut *(data.0 as *mut Vec<(isize, String)>);
        if IsWindowVisible(h).as_bool() && !IsIconic(h).as_bool() {
            let n = GetWindowTextLengthW(h);
            if n > 0 {
                let mut buf = vec![0u16; n as usize + 1];
                let got = GetWindowTextW(h, &mut buf);
                let title = String::from_utf16_lossy(&buf[..got.max(0) as usize]);
                if !title.trim().is_empty() {
                    v.push((h.0 as isize, title));
                }
            }
        }
        BOOL(1)
    }

    pub fn windows() -> Vec<(isize, String)> {
        let mut v: Vec<(isize, String)> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(win_cb), LPARAM(&mut v as *mut _ as isize));
        }
        v
    }

    pub fn capture(src: Source) -> Option<Sample> {
        let rect = match src {
            Source::Monitor(i) => monitors().get(i).copied()?,
            Source::Window(h) => unsafe {
                let mut r = RECT::default();
                GetWindowRect(HWND(h as *mut core::ffi::c_void), &mut r).ok()?;
                [r.left, r.top, r.right, r.bottom]
            },
        };
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        if w <= 0 || h <= 0 {
            return None;
        }
        unsafe {
            let screen = GetDC(HWND::default());
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, THUMB_W as i32, THUMB_H as i32);
            let old = SelectObject(mem, bmp);
            SetStretchBltMode(mem, HALFTONE);
            let ok = match src {
                // Monitor: copy straight from the desktop.
                Source::Monitor(_) => StretchBlt(mem, 0, 0, THUMB_W as i32, THUMB_H as i32, screen, rect[0], rect[1], w, h, SRCCOPY).as_bool(),
                // Window: let the window render itself (works when it is covered by other windows).
                Source::Window(hw) => {
                    let full_dc = CreateCompatibleDC(screen);
                    let full = CreateCompatibleBitmap(screen, w, h);
                    let old_full = SelectObject(full_dc, full);
                    const PW_RENDERFULLCONTENT: u32 = 2;
                    let printed = PrintWindow(HWND(hw as *mut core::ffi::c_void), full_dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
                    let ok = printed && StretchBlt(mem, 0, 0, THUMB_W as i32, THUMB_H as i32, full_dc, 0, 0, w, h, SRCCOPY).as_bool();
                    SelectObject(full_dc, old_full);
                    let _ = DeleteObject(full);
                    let _ = DeleteDC(full_dc);
                    ok
                }
            };
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = THUMB_W as i32;
            info.bmiHeader.biHeight = -(THUMB_H as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0;
            let mut px = vec![0u8; THUMB_W * THUMB_H * 4];
            let lines = GetDIBits(mem, bmp, 0, THUMB_H as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
            SelectObject(mem, old);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(mem);
            ReleaseDC(HWND::default(), screen);
            if !ok || lines == 0 {
                return None;
            }
            let mut acc = [0f32; 3];
            for p in px.chunks_exact_mut(4) {
                p.swap(0, 2); // BGRA -> RGBA
                p[3] = 255;
                acc[0] += p[0] as f32;
                acc[1] += p[1] as f32;
                acc[2] += p[2] as f32;
            }
            let n = (THUMB_W * THUMB_H) as f32 * 255.0;
            Some(Sample { color: [acc[0] / n, acc[1] / n, acc[2] / n], thumb: px, ok: true })
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn monitors() -> Vec<[i32; 4]> {
        Vec::new()
    }
    pub fn windows() -> Vec<(isize, String)> {
        Vec::new()
    }
    pub fn capture(_: Source) -> Option<Sample> {
        None
    }
}
