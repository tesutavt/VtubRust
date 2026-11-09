//! Auto-updater: checks GitHub releases, downloads the Windows zip, swaps the files next to the
//! running exe (a running exe can be renamed but not overwritten) and restarts the app.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context as _, Result};

pub const REPO: &str = "tesutavt/VtubRust";
const ASSET_SUFFIX: &str = "windows-x64.zip";

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub version: (u32, u32, u32),
    pub notes: String,
    pub page: String,
    pub url: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    /// Downloaded bytes / total.
    Downloading(u64, u64),
    Installing,
    /// Files replaced; the app should restart now.
    Ready,
    Error(String),
}

pub struct Updater {
    state: Arc<Mutex<State>>,
}

impl Default for Updater {
    fn default() -> Self {
        Updater { state: Arc::new(Mutex::new(State::Idle)) }
    }
}

/// Current version; `RIGRS_FAKE_VERSION` overrides it for testing the update flow.
pub fn current_version() -> String {
    std::env::var("RIGRS_FAKE_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(60)).user_agent(concat!("rigrs-updater/", env!("CARGO_PKG_VERSION"))).build()
}

fn fetch_latest() -> Result<Release> {
    let v: serde_json::Value = agent()
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("Accept", "application/vnd.github+json")
        .call()
        .context("GitHub недоступен")?
        .into_json()?;
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow!("нет tag_name"))?.to_string();
    let version = parse_version(&tag).ok_or_else(|| anyhow!("непонятная версия {tag}"))?;
    let asset = v["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str().map(|n| n.ends_with(ASSET_SUFFIX)).unwrap_or(false)))
        .ok_or_else(|| anyhow!("в релизе {tag} нет архива для Windows"))?;
    Ok(Release {
        tag,
        version,
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        page: v["html_url"].as_str().unwrap_or_default().to_string(),
        url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
    })
}

impl Updater {
    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or(State::Idle)
    }

    pub fn busy(&self) -> bool {
        matches!(self.state(), State::Checking | State::Downloading(..) | State::Installing)
    }

    /// Checks GitHub in the background.
    pub fn check(&self) {
        if self.busy() {
            return;
        }
        let st = self.state.clone();
        *st.lock().unwrap() = State::Checking;
        std::thread::spawn(move || {
            let r = fetch_latest();
            let cur = parse_version(&current_version()).unwrap_or((0, 0, 0));
            *st.lock().unwrap() = match r {
                Ok(rel) if rel.version > cur => State::Available(rel),
                Ok(_) => State::UpToDate,
                Err(e) => State::Error(format!("{e:#}")),
            };
        });
    }

    /// Downloads and installs `rel` in the background; ends in `Ready` or `Error`.
    pub fn install(&self, rel: Release) {
        if self.busy() {
            return;
        }
        let st = self.state.clone();
        *st.lock().unwrap() = State::Downloading(0, rel.size);
        std::thread::spawn(move || {
            let r = (|| -> Result<()> {
                let zip = download(&rel, &st)?;
                *st.lock().unwrap() = State::Installing;
                let dir = std::env::current_exe()?.parent().ok_or_else(|| anyhow!("нет папки программы"))?.to_path_buf();
                install_zip(&zip, &dir)
            })();
            *st.lock().unwrap() = match r {
                Ok(()) => State::Ready,
                Err(e) => State::Error(format!("{e:#}")),
            };
        });
    }
}

fn download(rel: &Release, st: &Arc<Mutex<State>>) -> Result<Vec<u8>> {
    let resp = agent().get(&rel.url).set("Accept", "application/octet-stream").call().context("не удалось скачать архив")?;
    let total = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(rel.size);
    let mut rd = resp.into_reader();
    let mut buf = Vec::with_capacity(total as usize);
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = rd.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        *st.lock().unwrap() = State::Downloading(buf.len() as u64, total);
        if buf.len() > 512 * 1024 * 1024 {
            bail!("архив слишком большой");
        }
    }
    if rel.size > 0 && buf.len() as u64 != rel.size {
        bail!("архив скачался не полностью ({} из {} байт)", buf.len(), rel.size);
    }
    Ok(buf)
}

fn old_name(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(".old");
    PathBuf::from(s)
}

/// Replaces the files in `dir` with the zip's files; rolls back on failure.
fn install_zip(data: &[u8], dir: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).context("архив повреждён")?;
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        // Only plain names inside the archive root / its single top folder; never leave `dir`.
        let Some(rel) = f.enclosed_name() else { continue };
        let Some(name) = rel.file_name() else { continue };
        let mut bytes = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut bytes)?;
        files.push((PathBuf::from(name), bytes));
    }
    if !files.iter().any(|(n, _)| n.as_os_str().eq_ignore_ascii_case("rigrs.exe")) {
        bail!("в архиве нет rigrs.exe");
    }
    let mut moved: Vec<PathBuf> = Vec::new();
    let mut written: Vec<PathBuf> = Vec::new();
    let res = (|| -> Result<()> {
        for (name, bytes) in &files {
            let target = dir.join(name);
            if target.exists() {
                let old = old_name(&target);
                let _ = std::fs::remove_file(&old);
                std::fs::rename(&target, &old).with_context(|| format!("не удалось заменить {} (нет прав на папку программы?)", target.display()))?;
                moved.push(target.clone());
            }
            std::fs::write(&target, bytes).with_context(|| format!("не удалось записать {}", target.display()))?;
            written.push(target);
        }
        Ok(())
    })();
    if res.is_err() {
        for w in &written {
            let _ = std::fs::remove_file(w);
        }
        for m in &moved {
            let _ = std::fs::rename(old_name(m), m);
        }
    }
    res
}

/// Deletes `*.old` files left by a previous update (the old exe may still be exiting, so retry).
pub fn cleanup_old() {
    let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) else { return };
    std::thread::spawn(move || {
        for _ in 0..20 {
            let mut left = 0;
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().map(|x| x == "old").unwrap_or(false) && std::fs::remove_file(&p).is_err() {
                        left += 1;
                    }
                }
            }
            if left == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Starts the (new) exe and exits this process.
pub fn restart() -> ! {
    if let Ok(exe) = std::env::current_exe() {
        // A GUI-subsystem process has no valid std handles to inherit (spawn fails with
        // ERROR_INVALID_HANDLE), so detach them explicitly.
        use std::process::Stdio;
        let _ = std::process::Command::new(&exe).args(std::env::args_os().skip(1)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.3.1"), Some((0, 3, 1)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v2.0.0-beta"), Some((2, 0, 0)));
        assert!(parse_version("v0.10.0") > parse_version("v0.9.9"));
        assert_eq!(parse_version("abc"), None);
    }

    #[test]
    fn install_and_rollback() {
        let dir = std::env::temp_dir().join(format!("rigrs-upd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rigrs.exe"), b"old").unwrap();
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("rigrs.exe", o).unwrap();
            std::io::Write::write_all(&mut w, b"new").unwrap();
            w.start_file("../evil.txt", o).unwrap();
            std::io::Write::write_all(&mut w, b"x").unwrap();
            w.finish().unwrap();
        }
        install_zip(buf.get_ref(), &dir).unwrap();
        assert_eq!(std::fs::read(dir.join("rigrs.exe")).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("rigrs.exe.old")).unwrap(), b"old");
        assert!(!dir.parent().unwrap().join("evil.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
