//! Files the built-in tracker needs and where they come from.
//!
//! * ONNX Runtime (`onnxruntime.dll`, MIT): next to the exe, `<data>/Tracker/`, or the
//!   copy inside a VTube Studio install; otherwise downloaded from Microsoft's GitHub
//!   release.
//! * OpenSeeFace models (BSD-2-Clause): `models/` next to the exe, `<data>/Tracker/models`,
//!   or VTube Studio's copy; otherwise downloaded from the OpenSeeFace repository.
//!
//! Both licences allow redistribution, so a release zip may simply ship these files.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use super::tracker::{landmark_model, DETECTION_MODEL, GAZE_MODEL};

const ORT_VERSION: &str = "1.17.3";
const MODELS_URL: &str = "https://raw.githubusercontent.com/emilianavt/OpenSeeFace/master/models/";
const OSF_LICENSE_URL: &str = "https://raw.githubusercontent.com/emilianavt/OpenSeeFace/master/LICENSE";

#[cfg(windows)]
pub const ORT_FILE: &str = "onnxruntime.dll";
#[cfg(target_os = "linux")]
pub const ORT_FILE: &str = "libonnxruntime.so";
#[cfg(target_os = "macos")]
pub const ORT_FILE: &str = "libonnxruntime.dylib";

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default()
}

pub fn tracker_dir(data: &Path) -> PathBuf {
    data.join("Tracker")
}

/// `osf_exe`: the configured `facetracker.exe` (its folder holds VTube Studio's copies).
pub fn find_runtime(data: &Path, osf_exe: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RIGRS_ORT").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let mut dirs = vec![exe_dir(), exe_dir().join("OpenSeeFace"), tracker_dir(data)];
    if let Some(d) = osf_exe.parent() {
        dirs.push(d.to_path_buf());
    }
    dirs.into_iter().map(|d| d.join(ORT_FILE)).find(|p| p.is_file())
}

fn model_files(model_type: i32) -> [&'static str; 3] {
    [landmark_model(model_type), GAZE_MODEL, DETECTION_MODEL]
}

pub fn find_models(data: &Path, osf_exe: &Path, model_type: i32) -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(p) = std::env::var_os("RIGRS_OSF_MODELS") {
        dirs.push(PathBuf::from(p));
    }
    dirs.extend([exe_dir().join("models"), exe_dir().join("OpenSeeFace").join("models"), tracker_dir(data).join("models")]);
    if let Some(d) = osf_exe.parent() {
        dirs.push(d.join("models"));
    }
    dirs.into_iter().find(|d| model_files(model_type).iter().all(|f| d.join(f).is_file()))
}

pub fn ready(data: &Path, osf_exe: &Path, model_type: i32) -> bool {
    find_runtime(data, osf_exe).is_some() && find_models(data, osf_exe, model_type).is_some()
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    /// What is being downloaded, bytes received, total (0 = unknown).
    Downloading(String, u64, u64),
    Done,
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

fn get(agent: &ureq::Agent, url: &str, what: &str, st: &Arc<Mutex<State>>, limit: u64) -> Result<Vec<u8>> {
    let resp = agent.get(url).call().map_err(|e| anyhow!("{url}: {e}"))?;
    let total: u64 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut r = resp.into_reader().take(limit);
    let mut out = Vec::with_capacity(total as usize);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).context("download interrupted")?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        *st.lock().unwrap() = State::Downloading(what.to_string(), out.len() as u64, total);
    }
    if total > 0 && (out.len() as u64) < total {
        bail!("{what}: download incomplete");
    }
    Ok(out)
}

/// Takes `onnxruntime.dll` and its licence files out of the official release zip.
pub fn extract_runtime(zip_bytes: &[u8], dir: &Path) -> Result<PathBuf> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).context("not a zip archive")?;
    std::fs::create_dir_all(dir)?;
    let mut found = None;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let name = f.name().replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or("").to_string();
        let out = match base.as_str() {
            "onnxruntime.dll" if name.contains("/lib/") => dir.join(ORT_FILE),
            "LICENSE" => dir.join("onnxruntime-LICENSE.txt"),
            "ThirdPartyNotices.txt" => dir.join("onnxruntime-ThirdPartyNotices.txt"),
            _ => continue,
        };
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        std::fs::write(&out, &data).with_context(|| format!("cannot write {}", out.display()))?;
        if base == "onnxruntime.dll" {
            found = Some(out);
        }
    }
    found.ok_or_else(|| anyhow!("onnxruntime.dll not found in the archive"))
}

impl Downloader {
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn busy(&self) -> bool {
        matches!(self.state(), State::Downloading(..))
    }

    pub fn take_done(&self) -> bool {
        let mut g = self.state.lock().unwrap();
        if *g == State::Done {
            *g = State::Idle;
            return true;
        }
        false
    }

    /// Downloads whatever is missing for `model_type` into `<data>/Tracker`.
    pub fn start(&self, data: &Path, osf_exe: &Path, model_type: i32) {
        if self.busy() {
            return;
        }
        *self.state.lock().unwrap() = State::Downloading(String::new(), 0, 0);
        let (st, data, osf_exe) = (self.state.clone(), data.to_path_buf(), osf_exe.to_path_buf());
        std::thread::spawn(move || {
            let r = (|| -> Result<()> {
                let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(60)).user_agent(concat!("rigrs/", env!("CARGO_PKG_VERSION"))).build();
                let dir = tracker_dir(&data);
                if find_models(&data, &osf_exe, model_type).is_none() {
                    let mdir = dir.join("models");
                    std::fs::create_dir_all(&mdir)?;
                    for f in model_files(model_type) {
                        if !mdir.join(f).is_file() {
                            let bytes = get(&agent, &format!("{MODELS_URL}{f}"), f, &st, 64 << 20)?;
                            // ONNX files are protobuf; a tiny or HTML answer means a wrong URL.
                            if bytes.len() < 100_000 {
                                bail!("{f}: unexpected content");
                            }
                            std::fs::write(mdir.join(f), bytes)?;
                        }
                    }
                    if let Ok(lic) = get(&agent, OSF_LICENSE_URL, "LICENSE", &st, 1 << 20) {
                        let _ = std::fs::write(dir.join("OpenSeeFace-LICENSE.txt"), lic);
                    }
                }
                if find_runtime(&data, &osf_exe).is_none() {
                    if !cfg!(windows) {
                        bail!("install ONNX Runtime ({ORT_FILE}) or set RIGRS_ORT to its path");
                    }
                    let url = format!("https://github.com/microsoft/onnxruntime/releases/download/v{ORT_VERSION}/onnxruntime-win-x64-{ORT_VERSION}.zip");
                    let zip = get(&agent, &url, "ONNX Runtime", &st, 200 << 20)?;
                    extract_runtime(&zip, &dir)?;
                }
                Ok(())
            })();
            *st.lock().unwrap() = match r {
                Ok(()) => State::Done,
                Err(e) => State::Failed(format!("{e:#}")),
            };
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `RIGRS_ORT_ZIP=onnxruntime-win-x64-1.17.3.zip cargo test extract_real_runtime -- --ignored`
    #[test]
    #[ignore]
    fn extract_real_runtime() {
        let zip = std::fs::read(std::env::var("RIGRS_ORT_ZIP").unwrap()).unwrap();
        let dir = std::env::temp_dir().join(format!("rigrs-ort-{}", std::process::id()));
        let p = extract_runtime(&zip, &dir).unwrap();
        assert!(std::fs::metadata(&p).unwrap().len() > 5_000_000);
        assert!(dir.join("onnxruntime-LICENSE.txt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
