//! Downloads the official Live2D Cubism SDK for Native and takes Cubism Core out of it,
//! so the app does not depend on a VTube Studio install.
//!
//! Cubism Core is proprietary (Live2D Proprietary Software License) and is therefore not
//! kept in this repository. The Core DLL is listed in the SDK's `RedistributableFiles.txt`,
//! and the SDK zip is published by Live2D at a fixed URL; the user accepts the license in
//! the UI before the download starts. The license files are saved next to the library.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

pub const LICENSE_URL: &str = "https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html";
pub const SDK_PAGE: &str = "https://www.live2d.com/en/sdk/download/native/";
/// Known SDK releases, newest first (the URL contains the version).
const VERSIONS: [&str; 4] = ["5-r.4.1", "5-r.4", "5-r.3", "5-r.1"];

#[cfg(windows)]
const CORE_IN_ZIP: &str = "Core/dll/windows/x86_64/Live2DCubismCore.dll";
#[cfg(target_os = "linux")]
const CORE_IN_ZIP: &str = "Core/dll/linux/x86_64/libLive2DCubismCore.so";
#[cfg(target_os = "macos")]
const CORE_IN_ZIP: &str = "Core/dll/macos/libLive2DCubismCore.dylib";

pub fn file_name() -> &'static str {
    CORE_IN_ZIP.rsplit('/').next().unwrap()
}

/// Where a downloaded Core lives: `<data>/Core/`.
pub fn installed_path(data: &Path) -> PathBuf {
    data.join("Core").join(file_name())
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    /// Bytes received, total (0 = unknown).
    Downloading(u64, u64),
    Done(PathBuf),
    Failed(String),
}

#[derive(Clone)]
pub struct Downloader {
    state: Arc<Mutex<State>>,
}

impl Default for Downloader {
    fn default() -> Self {
        Downloader { state: Arc::new(Mutex::new(State::Idle)) }
    }
}

impl Downloader {
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn busy(&self) -> bool {
        matches!(self.state(), State::Downloading(..))
    }

    /// Takes a finished result (resets to Idle).
    pub fn take_done(&self) -> Option<PathBuf> {
        let mut g = self.state.lock().unwrap();
        if let State::Done(p) = &*g {
            let p = p.clone();
            *g = State::Idle;
            return Some(p);
        }
        None
    }

    pub fn start(&self, data: &Path) {
        if self.busy() {
            return;
        }
        *self.state.lock().unwrap() = State::Downloading(0, 0);
        let (st, dir) = (self.state.clone(), data.join("Core"));
        std::thread::spawn(move || {
            let r = fetch(&st).and_then(|zip| extract(&zip, &dir));
            *st.lock().unwrap() = match r {
                Ok(p) => State::Done(p),
                Err(e) => State::Failed(format!("{e:#}")),
            };
        });
    }
}

fn fetch(st: &Arc<Mutex<State>>) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(60)).user_agent(concat!("rigrs/", env!("CARGO_PKG_VERSION"))).build();
    let mut last = anyhow!("no SDK version available");
    for v in VERSIONS {
        let url = format!("https://cubism.live2d.com/sdk-native/bin/CubismSdkForNative-{v}.zip");
        let resp = match agent.get(&url).call() {
            Ok(r) => r,
            Err(e) => {
                last = anyhow!("{url}: {e}");
                continue;
            }
        };
        let total: u64 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
        if total > 200 << 20 {
            bail!("unexpected SDK size");
        }
        let mut r = resp.into_reader().take(200 << 20);
        let mut out = Vec::with_capacity(total as usize);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = r.read(&mut buf).context("download interrupted")?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            *st.lock().unwrap() = State::Downloading(out.len() as u64, total);
        }
        if total > 0 && (out.len() as u64) < total {
            bail!("download incomplete");
        }
        return Ok(out);
    }
    Err(last)
}

/// Saves the Core library and the license files from the SDK zip into `dir`.
pub fn extract(zip_bytes: &[u8], dir: &Path) -> Result<PathBuf> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).context("not a zip archive")?;
    std::fs::create_dir_all(dir)?;
    let mut core = None;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let name = f.name().replace('\\', "/");
        // Paths start with the SDK folder name: "CubismSdkForNative-5-r.4.1/Core/...".
        let Some(rel) = name.split_once('/').map(|x| x.1) else { continue };
        let keep = rel == CORE_IN_ZIP || rel == "Core/LICENSE.md" || rel == "Core/RedistributableFiles.txt";
        if !keep {
            continue;
        }
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        let out = dir.join(rel.rsplit('/').next().unwrap());
        std::fs::write(&out, &data).with_context(|| format!("cannot write {}", out.display()))?;
        if rel == CORE_IN_ZIP {
            core = Some(out);
        }
    }
    core.ok_or_else(|| anyhow!("{CORE_IN_ZIP} not found in the SDK archive"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_core_and_license() {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        for (n, d) in [(format!("CubismSdkForNative-X/{CORE_IN_ZIP}"), &b"core"[..]), ("CubismSdkForNative-X/Core/LICENSE.md".into(), b"lic"), ("CubismSdkForNative-X/Samples/a.txt".into(), b"no")] {
            zw.start_file(n, o).unwrap();
            zw.write_all(d).unwrap();
        }
        let zip = zw.finish().unwrap().into_inner();
        let dir = std::env::temp_dir().join(format!("rigrs-core-{}", std::process::id()));
        let p = extract(&zip, &dir).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"core");
        assert!(dir.join("LICENSE.md").is_file());
        assert!(!dir.join("a.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real download from live2d.com (network): `cargo test real_download -- --ignored`.
    #[test]
    #[ignore]
    fn real_download() {
        let dir = std::env::temp_dir().join(format!("rigrs-core-real-{}", std::process::id()));
        let d = Downloader::default();
        d.start(&dir);
        let t0 = std::time::Instant::now();
        while d.busy() && t0.elapsed() < Duration::from_secs(180) {
            std::thread::sleep(Duration::from_millis(200));
        }
        let State::Done(p) = d.state() else { panic!("{:?}", d.state()) };
        assert_eq!(p, installed_path(&dir));
        let core = crate::cubism::core::Core::load(&p).unwrap();
        println!("Core {}", core.version_string());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Extraction + loading from a real SDK zip: `RIGRS_SDK_ZIP=... cargo test real_zip -- --ignored`.
    #[test]
    #[ignore]
    fn real_zip() {
        let zip = std::fs::read(std::env::var("RIGRS_SDK_ZIP").unwrap()).unwrap();
        let dir = std::env::temp_dir().join(format!("rigrs-core-zip-{}", std::process::id()));
        let p = extract(&zip, &dir).unwrap();
        let core = crate::cubism::core::Core::load(&p).unwrap();
        println!("Core {}", core.version_string());
        assert!(dir.join("LICENSE.md").is_file() && dir.join("RedistributableFiles.txt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
