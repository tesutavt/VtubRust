//! Discovers models, items and backgrounds on disk (the user's VTS data folder and our own).

use std::path::{Path, PathBuf};

use crate::formats::{read_json, VtubeJson};

#[derive(Clone, Debug)]
pub struct ModelEntry {
    pub name: String,
    pub model3: PathBuf,
    pub vtube: Option<PathBuf>,
    pub icon: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum ItemEntry {
    Live2D(ModelEntry),
    Image { name: String, path: PathBuf },
}

impl ItemEntry {
    pub fn name(&self) -> &str {
        match self {
            ItemEntry::Live2D(m) => &m.name,
            ItemEntry::Image { name, .. } => name,
        }
    }
    pub fn path(&self) -> &Path {
        match self {
            ItemEntry::Live2D(m) => &m.model3,
            ItemEntry::Image { path, .. } => path,
        }
    }
}

#[derive(Default)]
pub struct Library {
    pub models: Vec<ModelEntry>,
    pub items: Vec<ItemEntry>,
    pub backgrounds: Vec<PathBuf>,
}

fn ends(p: &Path, suffix: &str) -> bool {
    p.file_name().map(|n| n.to_string_lossy().to_lowercase().ends_with(suffix)).unwrap_or(false)
}

fn is_image(p: &Path) -> bool {
    ["png", "jpg", "jpeg", "gif"].iter().any(|e| p.extension().map(|x| x.eq_ignore_ascii_case(e)).unwrap_or(false))
}

/// Looks for a model in `dir` (and one level of subfolders).
pub fn model_in_dir(dir: &Path) -> Option<ModelEntry> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    if let Some(vt_path) = files.iter().find(|p| ends(p, ".vtube.json")) {
        if let Ok(vt) = read_json::<VtubeJson>(vt_path) {
            let m3 = dir.join(&vt.file_references.model);
            if m3.is_file() {
                let icon = Some(dir.join(&vt.file_references.icon)).filter(|p| !vt.file_references.icon.is_empty() && p.is_file());
                let name = if vt.name.is_empty() { dir.file_name()?.to_string_lossy().into_owned() } else { vt.name };
                return Some(ModelEntry { name, model3: m3, vtube: Some(vt_path.clone()), icon });
            }
        }
    }
    if let Some(m3) = files.iter().find(|p| ends(p, ".model3.json")) {
        let icon = files.iter().find(|p| p.file_name().map(|n| n.to_string_lossy().starts_with("icon")).unwrap_or(false) && is_image(p)).cloned();
        return Some(ModelEntry { name: dir.file_name()?.to_string_lossy().into_owned(), model3: m3.clone(), vtube: None, icon });
    }
    files.iter().filter(|p| p.is_dir()).find_map(|d| {
        let sub = model_in_dir_shallow(d)?;
        Some(ModelEntry { name: dir.file_name()?.to_string_lossy().into_owned(), ..sub })
    })
}

fn model_in_dir_shallow(dir: &Path) -> Option<ModelEntry> {
    let m3 = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| ends(p, ".model3.json"))?;
    Some(ModelEntry { name: String::new(), model3: m3, vtube: None, icon: None })
}

fn scan_models(root: &Path, out: &mut Vec<ModelEntry>) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for d in dirs {
        if let Some(m) = model_in_dir(&d) {
            out.push(m);
        }
    }
}

pub fn png_frames_dir(p: &Path) -> bool {
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().any(|e| e.path().extension().map(|x| x.eq_ignore_ascii_case("png")).unwrap_or(false)))
        .unwrap_or(false)
}

fn scan_items(root: &Path, out: &mut Vec<ItemEntry>) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            if let Some(m) = model_in_dir(&p) {
                out.push(ItemEntry::Live2D(m));
            } else if png_frames_dir(&p) {
                // VTS: a folder of PNGs is an animated item.
                let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                out.push(ItemEntry::Image { name, path: p });
            }
        } else if is_image(&p) {
            let name = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            out.push(ItemEntry::Image { name, path: p });
        }
    }
}

fn scan_images(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_image(p)).collect();
    v.sort();
    out.extend(v);
}

/// `vts_assets` = `<VTS>/VTube Studio_Data/StreamingAssets`; `data` = our own data folder.
pub fn scan(vts_assets: Option<&Path>, data: &Path, extra_models: &[PathBuf]) -> Library {
    let mut lib = Library::default();
    for root in vts_assets.into_iter().chain(std::iter::once(data)) {
        scan_models(&root.join("Live2DModels"), &mut lib.models);
        scan_items(&root.join("Items"), &mut lib.items);
        scan_images(&root.join("Backgrounds"), &mut lib.backgrounds);
    }
    for d in extra_models {
        if let Some(m) = model_in_dir(d) {
            if !lib.models.iter().any(|x| x.model3 == m.model3) {
                lib.models.push(m);
            }
        } else {
            scan_models(d, &mut lib.models);
        }
    }
    lib
}
