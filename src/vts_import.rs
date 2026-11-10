//! Moving off a VTube Studio install: copies the model / item / background library from
//! VTS's `StreamingAssets` into the rigrs data folder and rewrites the saved settings so that
//! everything keyed by a file path (model positions, hotkeys, ArtMesh settings, pinned items,
//! the background, ...) points at the copies. After that the VTS folder is not used any more.
//!
//! Files are copied, never moved or deleted: the VTube Studio install stays as it was.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde_json::Value;

/// Library folders that exist with the same names on both sides.
pub const FOLDERS: [&str; 3] = ["Live2DModels", "Items", "Backgrounds"];

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    /// Bytes copied, total bytes, current file.
    Copying(u64, u64, String),
    /// Files copied, files that were already there.
    Done(u64, u64),
    Failed(String),
}

#[derive(Clone)]
pub struct Importer {
    state: Arc<Mutex<State>>,
}

impl Default for Importer {
    fn default() -> Self {
        Importer { state: Arc::new(Mutex::new(State::Idle)) }
    }
}

fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        match e.metadata() {
            Ok(m) if m.is_dir() => walk(&p, out),
            Ok(m) => out.push((p, m.len())),
            Err(_) => {}
        }
    }
}

/// Size of the library that `copy` would look at (for the UI).
pub fn library_size(assets: &Path) -> (u64, u64) {
    let mut files = Vec::new();
    for f in FOLDERS {
        walk(&assets.join(f), &mut files);
    }
    (files.len() as u64, files.iter().map(|f| f.1).sum())
}

/// Copies the library folders. Existing files of the same size are kept (so the import can be
/// repeated or resumed); nothing is ever overwritten with different content or deleted.
pub fn copy(assets: &Path, data: &Path, progress: &dyn Fn(u64, u64, &str)) -> Result<(u64, u64)> {
    let mut files = Vec::new();
    for f in FOLDERS {
        walk(&assets.join(f), &mut files);
    }
    let total: u64 = files.iter().map(|f| f.1).sum();
    let (mut done, mut copied, mut kept) = (0u64, 0u64, 0u64);
    for (src, size) in files {
        let rel = src.strip_prefix(assets).context("path outside the library")?;
        let dst = data.join(rel);
        progress(done, total, &rel.to_string_lossy());
        let same = std::fs::metadata(&dst).map(|m| m.is_file() && m.len() == size).unwrap_or(false);
        if same {
            kept += 1;
        } else if dst.exists() {
            // A different file with this name already lives in the rigrs library: keep it.
            kept += 1;
        } else {
            if let Some(p) = dst.parent() {
                std::fs::create_dir_all(p).with_context(|| format!("cannot create {}", p.display()))?;
            }
            std::fs::copy(&src, &dst).with_context(|| format!("cannot copy {}", src.display()))?;
            copied += 1;
        }
        done += size;
    }
    progress(total, total, "");
    Ok((copied, kept))
}

impl Importer {
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn busy(&self) -> bool {
        matches!(self.state(), State::Copying(..))
    }

    /// Takes a finished copy (resets to Idle).
    pub fn take_done(&self) -> Option<(u64, u64)> {
        let mut g = self.state.lock().unwrap();
        if let State::Done(c, k) = &*g {
            let r = (*c, *k);
            *g = State::Idle;
            return Some(r);
        }
        None
    }

    pub fn start(&self, assets: &Path, data: &Path) {
        if self.busy() {
            return;
        }
        *self.state.lock().unwrap() = State::Copying(0, 0, String::new());
        let (st, assets, data) = (self.state.clone(), assets.to_path_buf(), data.to_path_buf());
        std::thread::spawn(move || {
            let r = copy(&assets, &data, &|done, total, file| {
                *st.lock().unwrap() = State::Copying(done, total, file.to_string());
            });
            *st.lock().unwrap() = match r {
                Ok((c, k)) => State::Done(c, k),
                Err(e) => State::Failed(format!("{e:#}")),
            };
        });
    }
}

fn remap_str(s: &str, old: &str, new: &str) -> Option<String> {
    // Windows paths: compare case-insensitively, accept either slash.
    let norm = |x: &str| x.replace('/', "\\").to_lowercase();
    let (sn, on) = (norm(s), norm(old));
    if !sn.starts_with(&on) {
        return None;
    }
    let rest = s.get(old.len()..)?;
    if !(rest.is_empty() || rest.starts_with('\\') || rest.starts_with('/')) {
        return None;
    }
    Some(format!("{new}{rest}"))
}

/// Rewrites every string and every object key that starts with `old` to start with `new`.
pub fn remap_value(v: &mut Value, old: &str, new: &str) -> usize {
    match v {
        Value::String(s) => match remap_str(s, old, new) {
            Some(n) => {
                *s = n;
                1
            }
            None => 0,
        },
        Value::Array(a) => a.iter_mut().map(|x| remap_value(x, old, new)).sum(),
        Value::Object(m) => {
            let mut n = 0;
            let keys: Vec<String> = m.keys().cloned().collect();
            for k in keys {
                if let Some(nk) = remap_str(&k, old, new) {
                    if let Some(val) = m.remove(&k) {
                        // Never clobber an entry that already exists under the new path.
                        m.entry(nk).or_insert(val);
                        n += 1;
                    }
                }
            }
            n + m.values_mut().map(|x| remap_value(x, old, new)).sum::<usize>()
        }
        _ => 0,
    }
}

/// Points the saved settings at the copied library and switches the VTube Studio folder off.
/// `core`: our own Cubism Core, when there is one. Returns the number of rewritten paths.
pub fn detach_settings(file: &Path, assets: &Path, data: &Path, core: Option<&Path>) -> Result<usize> {
    let text = std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut v: Value = serde_json::from_str(&text).context("settings are not valid JSON")?;
    // The library folders first, so that non-library VTS paths (tracker, Core) stay untouched.
    let mut n = 0;
    for f in FOLDERS {
        n += remap_value(&mut v, &assets.join(f).to_string_lossy(), &data.join(f).to_string_lossy());
    }
    v["use_vts"] = Value::Bool(false);
    v["vts_dir"] = Value::String(String::new());
    if let Some(c) = core {
        v["core_dll"] = Value::String(c.to_string_lossy().into_owned());
    }
    let tmp = file.with_extension("json.detach");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&v)?)?;
    std::fs::rename(&tmp, file)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remaps_keys_and_values_only_under_the_prefix() {
        let old = r"H:\Steam\VTube Studio\VTube Studio_Data\StreamingAssets\Live2DModels";
        let new = r"C:\Users\me\AppData\Roaming\rigrs\Live2DModels";
        let mut v = json!({
            "last_model": format!(r"{old}\tesuta\tesuta.model3.json"),
            "model_xf": { format!(r"{}\TESUTA\tesuta.model3.json", old.to_lowercase()): { "pos": [1, 2] } },
            "other": r"H:\Steam\VTube Studio\VTube Studio_Data\StreamingAssets\Live2DModelsExtra\x.json",
            "tracker_exe": r"H:\Steam\VTube Studio\VTube Studio_Data\StreamingAssets\OpenSeeFace\facetracker.exe",
            "list": [format!("{old}/a/b.model3.json"), 5],
        });
        assert_eq!(remap_value(&mut v, old, new), 3);
        assert_eq!(v["last_model"], format!(r"{new}\tesuta\tesuta.model3.json"));
        assert!(v["model_xf"].as_object().unwrap().keys().next().unwrap().starts_with(new));
        assert!(v["other"].as_str().unwrap().contains("Live2DModelsExtra"), "a longer folder name is not the prefix");
        assert!(v["tracker_exe"].as_str().unwrap().starts_with(r"H:\Steam"));
        assert_eq!(v["list"][0], format!("{new}/a/b.model3.json"));
    }

    #[test]
    fn copy_keeps_existing_and_detaches_settings() {
        let base = std::env::temp_dir().join(format!("rigrs-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (assets, data) = (base.join("vts").join("StreamingAssets"), base.join("data"));
        std::fs::create_dir_all(assets.join("Live2DModels/m/tex")).unwrap();
        std::fs::create_dir_all(assets.join("Items")).unwrap();
        std::fs::create_dir_all(assets.join("Config")).unwrap();
        std::fs::write(assets.join("Live2DModels/m/m.model3.json"), b"{}").unwrap();
        std::fs::write(assets.join("Live2DModels/m/tex/t.png"), vec![1u8; 5000]).unwrap();
        std::fs::write(assets.join("Items/hat.png"), b"vts hat").unwrap();
        std::fs::write(assets.join("Config/secret.json"), b"x").unwrap();
        // The user already has a different hat in rigrs: it must survive.
        std::fs::create_dir_all(data.join("Items")).unwrap();
        std::fs::write(data.join("Items/hat.png"), b"my own hat!").unwrap();

        assert_eq!(library_size(&assets).0, 3);
        let (copied, kept) = copy(&assets, &data, &|_, _, _| {}).unwrap();
        assert_eq!((copied, kept), (2, 1));
        assert_eq!(std::fs::read(data.join("Items/hat.png")).unwrap(), b"my own hat!");
        assert_eq!(std::fs::read(data.join("Live2DModels/m/tex/t.png")).unwrap().len(), 5000);
        assert!(!data.join("Config").exists(), "only library folders are copied");
        assert!(assets.join("Live2DModels/m/m.model3.json").is_file(), "the source is left in place");
        // Running it again copies nothing.
        assert_eq!(copy(&assets, &data, &|_, _, _| {}).unwrap(), (0, 3));

        let file = base.join("settings.json");
        let model = assets.join("Live2DModels").join("m").join("m.model3.json").to_string_lossy().into_owned();
        std::fs::write(&file, serde_json::to_vec(&json!({ "last_model": model, "vts_dir": "x", "model_xf": { model.clone(): 1 }, "core_dll": "old.dll" })).unwrap()).unwrap();
        let n = detach_settings(&file, &assets, &data, Some(Path::new("new.dll"))).unwrap();
        assert_eq!(n, 2);
        let v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let want = data.join("Live2DModels").join("m").join("m.model3.json").to_string_lossy().into_owned();
        assert_eq!(v["last_model"], want);
        assert_eq!(v["model_xf"][&want], 1);
        assert_eq!(v["use_vts"], false);
        assert_eq!(v["vts_dir"], "");
        assert_eq!(v["core_dll"], "new.dll");
        let _ = std::fs::remove_dir_all(&base);
    }
}
