//! App side of the Windows 11 virtual camera "rigrsCam": registers a session virtual camera
//! backed by the rigrs_vcam.dll media source and feeds it frames through shared memory.

use anyhow::{bail, Context, Result};

/// Must match vcam/src/lib.rs.
pub const CLSID_STR: &str = "{7A1F3C2E-5B4D-4E8A-9C61-2F0B8D3E4A17}";
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
const HEADER: usize = 32;
const MAGIC: u32 = u32::from_le_bytes(*b"RGVC");
const MAP_SIZE: usize = HEADER + (WIDTH * HEIGHT * 4) as usize;

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::{w, Interface, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Media::MediaFoundation::{IMFVirtualCamera, MFStartup, MFSTARTUP_FULL, MF_VERSION};
    use windows::Win32::System::Memory::*;
    use windows::Win32::System::Registry::*;
    use windows::Win32::System::SystemInformation::GetTickCount64;

    pub fn installed() -> bool {
        let sub = HSTRING::from(format!("Software\\Classes\\CLSID\\{CLSID_STR}\\InprocServer32"));
        let mut key = HKEY::default();
        unsafe {
            let ok = RegOpenKeyExW(HKEY_LOCAL_MACHINE, &sub, 0, KEY_READ, &mut key).is_ok();
            if ok {
                let _ = RegCloseKey(key);
            }
            ok
        }
    }

    type CreateFn = unsafe extern "system" fn(i32, i32, i32, PCWSTR, PCWSTR, *const windows::core::GUID, u32, *mut *mut core::ffi::c_void) -> windows::core::HRESULT;

    pub struct Camera {
        cam: IMFVirtualCamera,
        _lib: libloading::Library,
    }

    impl Camera {
        pub fn start() -> Result<Camera> {
            if !installed() {
                bail!("камера rigrsCam не установлена — нажмите «Помощь с установкой»");
            }
            unsafe {
                MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup")?;
                let lib = libloading::Library::new("mfsensorgroup.dll").context("нужна Windows 11 (mfsensorgroup.dll не найден)")?;
                let create: CreateFn = *lib.get(b"MFCreateVirtualCamera\0").context("нужна Windows 11 (нет MFCreateVirtualCamera)")?;
                let source = HSTRING::from(CLSID_STR);
                let mut raw = std::ptr::null_mut();
                // SoftwareCameraSource, Session lifetime, CurrentUser access.
                create(0, 0, 0, w!("rigrsCam"), PCWSTR(source.as_ptr()), std::ptr::null(), 0, &mut raw).ok().context("MFCreateVirtualCamera")?;
                let cam = IMFVirtualCamera::from_raw(raw);
                cam.Start(None).context("запуск виртуальной камеры")?;
                Ok(Camera { cam, _lib: lib })
            }
        }
    }

    impl Drop for Camera {
        fn drop(&mut self) {
            unsafe {
                let _ = self.cam.Stop();
                let _ = self.cam.Remove();
                let _ = self.cam.Shutdown();
            }
        }
    }

    /// Writes frames into the block the camera DLL created (exists while a client uses the camera).
    pub struct Writer {
        map: Option<(HANDLE, MEMORY_MAPPED_VIEW_ADDRESS)>,
        last_try: std::time::Instant,
        seq: u64,
        pub connected: bool,
    }

    impl Writer {
        pub fn new() -> Writer {
            Writer { map: None, last_try: std::time::Instant::now() - std::time::Duration::from_secs(5), seq: 0, connected: false }
        }

        fn open(&mut self) {
            if self.map.is_some() || self.last_try.elapsed().as_secs_f32() < 1.0 {
                return;
            }
            self.last_try = std::time::Instant::now();
            unsafe {
                for name in [w!("Global\\rigrs_vcam_v1"), w!("Local\\rigrs_vcam_v1")] {
                    if let Ok(h) = OpenFileMappingW(FILE_MAP_WRITE.0, false, name) {
                        let v = MapViewOfFile(h, FILE_MAP_WRITE, 0, 0, MAP_SIZE);
                        if !v.Value.is_null() {
                            self.map = Some((h, v));
                            return;
                        }
                        let _ = CloseHandle(h);
                    }
                }
            }
        }

        /// `data`: rows of `stride` bytes, BGRA (or RGBA when `bgra` is false), straight alpha.
        pub fn write(&mut self, data: &[u8], stride: usize, bgra: bool) {
            self.open();
            let Some((_, v)) = &self.map else {
                self.connected = false;
                return;
            };
            self.connected = true;
            let dst = unsafe { std::slice::from_raw_parts_mut(v.Value as *mut u8, MAP_SIZE) };
            let row = (WIDTH * 4) as usize;
            for y in 0..HEIGHT as usize {
                let s = &data[y * stride..y * stride + row];
                let d = &mut dst[HEADER + y * row..HEADER + (y + 1) * row];
                if bgra {
                    d.copy_from_slice(s);
                } else {
                    for (o, i) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                        o.copy_from_slice(&[i[2], i[1], i[0], i[3]]);
                    }
                }
            }
            self.seq += 1;
            dst[0..4].copy_from_slice(&MAGIC.to_le_bytes());
            dst[4..8].copy_from_slice(&WIDTH.to_le_bytes());
            dst[8..12].copy_from_slice(&HEIGHT.to_le_bytes());
            dst[16..24].copy_from_slice(&self.seq.to_le_bytes());
            dst[24..32].copy_from_slice(&unsafe { GetTickCount64() }.to_le_bytes());
        }
    }

    impl Drop for Writer {
        fn drop(&mut self) {
            if let Some((h, v)) = self.map.take() {
                unsafe {
                    let _ = UnmapViewOfFile(v);
                    let _ = CloseHandle(h);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn installed() -> bool {
        false
    }
    pub struct Camera;
    impl Camera {
        pub fn start() -> Result<Camera> {
            bail!("только Windows 11")
        }
    }
    pub struct Writer {
        pub connected: bool,
    }
    impl Writer {
        pub fn new() -> Writer {
            Writer { connected: false }
        }
        pub fn write(&mut self, _: &[u8], _: usize, _: bool) {}
    }
}

pub use imp::{installed, Camera, Writer};

/// Prepares %APPDATA%\rigrs\vcam: the DLL plus install/uninstall scripts.
pub fn prepare_folder(dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let exe_dir = std::env::current_exe()?.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let dll = exe_dir.join("rigrs_vcam.dll");
    if dll.is_file() {
        let _ = std::fs::copy(&dll, dir.join("rigrs_vcam.dll"));
    }
    let install = "@echo off\r\n\
net session >nul 2>&1\r\n\
if errorlevel 1 (\r\n\
  echo Please run this file as Administrator: right click - Run as administrator.\r\n\
  pause\r\n\
  exit /b 1\r\n\
)\r\n\
if not exist \"%ProgramData%\\rigrs\" mkdir \"%ProgramData%\\rigrs\"\r\n\
regsvr32 /u /s \"%ProgramData%\\rigrs\\rigrs_vcam.dll\" >nul 2>&1\r\n\
copy /Y \"%~dp0rigrs_vcam.dll\" \"%ProgramData%\\rigrs\\rigrs_vcam.dll\" >nul\r\n\
regsvr32 /s \"%ProgramData%\\rigrs\\rigrs_vcam.dll\"\r\n\
if errorlevel 1 (\r\n\
  echo Registration failed.\r\n\
  pause\r\n\
  exit /b 1\r\n\
)\r\n\
echo rigrsCam installed. Enable the virtual camera in rigrs settings.\r\n\
pause\r\n";
    let uninstall = "@echo off\r\n\
regsvr32 /u /s \"%ProgramData%\\rigrs\\rigrs_vcam.dll\"\r\n\
del /Q \"%ProgramData%\\rigrs\\rigrs_vcam.dll\" >nul 2>&1\r\n\
echo rigrsCam removed.\r\n\
pause\r\n";
    std::fs::write(dir.join("install.bat"), install)?;
    std::fs::write(dir.join("uninstall.bat"), uninstall)?;
    if !dll.is_file() {
        bail!("rigrs_vcam.dll не найден рядом с rigrs.exe — соберите проект (cargo build --release)");
    }
    Ok(())
}
