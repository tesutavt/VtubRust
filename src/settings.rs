//! Persistent settings (stored as JSON in %APPDATA%\rigrs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mapping::Rule;
use crate::scene::live2d::Xform;
use crate::scene::Pin;
use crate::tracking::{Calibration, TrackOptions};

/// Finds a VTube Studio install through Steam (registry + library folders).
pub fn detect_vts_dir() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    unsafe {
        use windows::core::w;
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
        let mut buf = [0u16; 520];
        let mut len = (buf.len() * 2) as u32;
        if RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Valve\\Steam"), w!("SteamPath"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)).is_ok() {
            let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
            roots.push(PathBuf::from(String::from_utf16_lossy(&buf[..n])));
        }
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    let mut libs: Vec<PathBuf> = Vec::new();
    for r in &roots {
        libs.push(r.clone());
        if let Ok(vdf) = std::fs::read_to_string(r.join("steamapps").join("libraryfolders.vdf")) {
            for line in vdf.lines() {
                let t = line.trim();
                if let Some(rest) = t.strip_prefix("\"path\"") {
                    let v = rest.trim().trim_matches('"').replace("\\\\", "\\");
                    libs.push(PathBuf::from(v));
                }
            }
        }
    }
    // A leftover empty folder does not count: require the Cubism Core inside.
    libs.into_iter()
        .map(|l| l.join("steamapps").join("common").join("VTube Studio"))
        .find(|p| p.join("VTube Studio.exe").is_file() && p.join("VTube Studio_Data").join("Plugins").join("x86_64").join("Live2DCubismCore.dll").is_file())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum FpsMode {
    Fps30,
    Fps60,
    Fps120,
    Fps144,
    VSync,
    VSyncHalf,
    Unlimited,
}

impl FpsMode {
    pub const ALL: [FpsMode; 7] = [FpsMode::Fps60, FpsMode::Fps30, FpsMode::VSync, FpsMode::VSyncHalf, FpsMode::Fps120, FpsMode::Fps144, FpsMode::Unlimited];
    pub fn vsync(self) -> bool {
        matches!(self, FpsMode::VSync | FpsMode::VSyncHalf)
    }
    pub fn label(self) -> &'static str {
        match self {
            FpsMode::Fps30 => "30 FPS",
            FpsMode::Fps60 => "60 FPS",
            FpsMode::Fps120 => "120 FPS",
            FpsMode::Fps144 => "144 FPS",
            FpsMode::VSync => "VSync",
            FpsMode::VSyncHalf => "VSync (1/2 частоты обновления)",
            FpsMode::Unlimited => "Без ограничения",
        }
    }
    pub fn cap(self) -> Option<f32> {
        match self {
            FpsMode::Fps30 => Some(30.0),
            FpsMode::Fps60 => Some(60.0),
            FpsMode::Fps120 => Some(120.0),
            FpsMode::Fps144 => Some(144.0),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Background {
    Color([f32; 3]),
    Image(String),
    /// Transparent (only meaningful for Spout2 / screenshots; shown as dark grey).
    Transparent,
}

/// Per-model options edited on the "Model" tab.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelCfg {
    pub display_name: String,
    pub lost_idle: String,
    pub save_expressions: bool,
    pub active_expressions: Vec<String>,
    pub move_enabled: bool,
    /// Head-position → model movement multipliers (-20..20).
    pub move_xyz: [f32; 3],
    pub physics_on: bool,
    /// 0 = from the model file, 1 = every app frame, otherwise Hz.
    pub physics_fps: u32,
    /// 0..100, 50 = as authored.
    pub physics_strength: f32,
}

impl Default for ModelCfg {
    fn default() -> Self {
        ModelCfg {
            display_name: String::new(),
            lost_idle: String::new(),
            save_expressions: false,
            active_expressions: Vec::new(),
            move_enabled: false,
            move_xyz: [6.0, 8.0, 11.0],
            physics_on: true,
            physics_fps: 0,
            physics_strength: 50.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedItem {
    pub source: String,
    pub xf: Xform,
    pub front: bool,
    pub locked: bool,
    pub pin: Option<Pin>,
    #[serde(default)]
    pub cfg: Option<crate::scene::ItemCfg>,
    #[serde(default)]
    pub scene: Option<String>,
    /// Model (model3 path) the item is pinned to; pinned items of other models are skipped
    /// when an item scene is loaded (VTS).
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemScene {
    pub name: String,
    pub items: Vec<SavedItem>,
    /// VTS "Item Scene Group": loading a scene unloads items of other scenes in the same group.
    #[serde(default)]
    pub group: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub vts_dir: String,
    pub core_dll: String,
    pub tracker_exe: String,

    pub camera: i32,
    pub cam_size: [u32; 2],
    pub cam_fps: u32,
    pub cam_dcap: i32,
    pub cam_format: String,
    pub track_quality: i32,
    /// 0 = OpenSeeFace, 1 = Mediapipe
    pub track_backend: u8,
    /// 0 = face, 1 = hands, 2 = face + hands
    pub track_type: u8,
    pub track_threads: u32,
    pub track_port: u16,
    pub track_autostart: bool,
    pub lost_reset: bool,
    pub track: TrackOptions,
    pub calib: Calibration,

    pub fps: FpsMode,
    pub show_stats: bool,
    pub low_power_gpu: bool,
    pub physics: bool,
    pub save_scene: bool,
    pub ui_scale: f32,

    pub spout: bool,
    pub spout_name: String,
    pub spout_mirror: bool,
    pub ndi: bool,
    pub ndi_name: String,
    pub ndi_mirror: bool,
    pub ndi_custom_size: bool,
    pub ndi_size: [u32; 2],
    pub log_to_file: bool,
    pub vcam_on: bool,
    pub vcam_mirror: bool,
    pub vfx_on: bool,
    /// User allowed restricted/experimental effects (VTS "restricted effects").
    pub vfx_restricted: bool,
    /// Microphone lipsync (VTS "Advanced Lipsync").
    pub mic: crate::lipsync::MicCfg,
    /// Controller input: 0 = no controller, 1 = all controllers, 2 = the selected one.
    pub controller_mode: u8,
    pub controller_id: String,
    /// VTS effect values by config enum ID (VTS EffectConfigs). Old-format "vfx" data is ignored.
    #[serde(rename = "vfx_vts")]
    pub vfx: crate::vfx::Values,
    pub smooth_masks: bool,
    pub mask_atlas: u32,
    pub mask_max: u32,
    /// Info dialogs the user chose not to see again.
    pub dismissed_info: Vec<u8>,
    pub tobii_enabled: bool,
    pub tobii_replace_head: bool,
    pub tobii_replace_eyes: bool,
    pub tobii_show_gaze: bool,
    pub tobii_calib_rot: [f32; 3],
    pub tobii_calib_gaze: [f32; 2],
    /// 0 = off, 1 = monitor, 2 = window, 3 = fixed colour
    pub light_mode: u8,
    pub light_monitor: usize,
    pub light_window: String,
    pub light_color: [f32; 3],
    pub light_brightness: f32,
    pub light_overlay: f32,
    pub light_smoothing: f32,
    pub light_items: bool,
    pub rec_physics: bool,
    pub rec_unchanged: bool,
    pub rec_seconds: f32,
    pub show_cam_preview: bool,
    /// Screen-space mouse range: [x_min, x_max, y_min, y_max]
    pub mouse_range: [f32; 4],

    pub background: Background,
    pub last_model: Option<String>,
    pub avatar_locked: bool,
    pub model_xf: HashMap<String, Xform>,
    pub extra_model_dirs: Vec<String>,
    pub rules_override: HashMap<String, Vec<Rule>>,
    pub idle_override: HashMap<String, String>,
    pub model_cfg: HashMap<String, ModelCfg>,

    pub last_items: Vec<SavedItem>,
    pub item_scenes: Vec<ItemScene>,
    pub hotkeys_enabled: bool,
    /// Hotkeys per model (key = model3 path), in the VTS JSON format.
    pub model_hotkeys: HashMap<String, Vec<crate::hotkeys::Hotkey>>,
    pub screen_buttons: bool,
    pub send_to_pc: bool,
    pub screen_buttons_opacity: f32,
    pub light_last_mode: u8,
    /// Animated item FPS remembered per item file (VTS remembers it too).
    pub item_fps: HashMap<String, f32>,
    /// Per-model ArtMesh settings (VTS ArtMeshDetails), keyed by model3 path.
    pub model_artmesh: HashMap<String, crate::artmesh::ArtMeshDetails>,
    /// Global item options: drag & drop of files into the window, allowed types.
    pub item_drag_drop: bool,
    /// Live2D Cubism Editor API (VTS "Live2D Cubism Editor API" card).
    pub editor_on: bool,
    pub editor_remote: bool,
    pub editor_ip: String,
    pub editor_port: u16,
    pub editor_send: bool,
    pub editor_physics_only: bool,
    pub editor_token: String,
    pub editor_auto_import: bool,
    pub item_show_pin_notifications: bool,

    pub api_on: bool,
    pub api_port: u16,
    pub api_lan: bool,
    pub api_tokens: Vec<crate::api::ApiToken>,
    pub api_permissions: Vec<String>,

    pub twitch: crate::twitch::TwitchCfg,
    /// UI language: "ru" or "en".
    pub language: String,
}

impl Default for Settings {
    fn default() -> Self {
        let vts = detect_vts_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        Settings {
            core_dll: format!(r"{vts}\VTube Studio_Data\Plugins\x86_64\Live2DCubismCore.dll"),
            tracker_exe: format!(r"{vts}\VTube Studio_Data\StreamingAssets\OpenSeeFaceTracker_v_1_20_2\binary\facetracker.exe"),
            vts_dir: vts,
            camera: 0,
            cam_size: [640, 360],
            cam_fps: 30,
            cam_dcap: -1,
            cam_format: String::new(),
            track_quality: 3,
            track_backend: 0,
            track_type: 0,
            track_threads: 1,
            track_port: 11573,
            track_autostart: true,
            lost_reset: true,
            track: TrackOptions::default(),
            calib: Calibration::default(),
            fps: FpsMode::Fps60,
            show_stats: true,
            low_power_gpu: false,
            physics: true,
            save_scene: true,
            ui_scale: 1.0,
            spout: false,
            spout_name: "rigrs".into(),
            spout_mirror: false,
            ndi: false,
            ndi_name: "rigrs".into(),
            ndi_mirror: false,
            ndi_custom_size: false,
            ndi_size: [1024, 576],
            log_to_file: false,
            vcam_on: false,
            vcam_mirror: false,
            vfx_on: false,
            vfx_restricted: false,
            mic: Default::default(),
            controller_mode: 0,
            controller_id: String::new(),
            vfx: Default::default(),
            smooth_masks: false,
            mask_atlas: 2048,
            mask_max: 256,
            dismissed_info: Vec::new(),
            tobii_enabled: false,
            tobii_replace_head: true,
            tobii_replace_eyes: true,
            tobii_show_gaze: false,
            tobii_calib_rot: [0.0; 3],
            tobii_calib_gaze: [0.5, 0.5],
            light_mode: 0,
            light_monitor: 0,
            light_window: String::new(),
            light_color: [1.0, 0.8, 0.6],
            light_brightness: 37.0,
            light_overlay: 40.0,
            light_smoothing: 3.0,
            light_items: true,
            rec_physics: false,
            rec_unchanged: true,
            rec_seconds: 10.0,
            show_cam_preview: true,
            mouse_range: [0.0, 1920.0, 0.0, 1080.0],
            background: Background::Color([0.0, 0.0, 0.0]),
            last_model: None,
            avatar_locked: false,
            model_xf: HashMap::new(),
            extra_model_dirs: Vec::new(),
            rules_override: HashMap::new(),
            idle_override: HashMap::new(),
            model_cfg: HashMap::new(),
            last_items: Vec::new(),
            item_scenes: Vec::new(),
            hotkeys_enabled: true,
            model_hotkeys: HashMap::new(),
            screen_buttons: false,
            send_to_pc: true,
            screen_buttons_opacity: 75.0,
            light_last_mode: 1,
            item_fps: HashMap::new(),
            model_artmesh: HashMap::new(),
            item_drag_drop: true,
            editor_on: false,
            editor_remote: false,
            editor_ip: "192.168.0.2".into(),
            editor_port: 22033,
            editor_send: true,
            editor_physics_only: false,
            editor_token: String::new(),
            editor_auto_import: false,
            item_show_pin_notifications: false,
            api_on: false,
            api_port: 8001,
            api_lan: false,
            api_tokens: Vec::new(),
            api_permissions: Vec::new(),
            twitch: Default::default(),
            language: "ru".into(),
        }
    }
}

pub struct Paths {
    pub data: PathBuf,
    pub settings: PathBuf,
    pub screenshots: PathBuf,
}

impl Paths {
    pub fn new() -> Paths {
        // RIGRS_DATA overrides the data folder (used for isolated test runs).
        let data = match std::env::var_os("RIGRS_DATA") {
            Some(d) => PathBuf::from(d),
            None => std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("rigrs"),
        };
        for sub in ["Live2DModels", "Items", "Backgrounds", "Screenshots"] {
            let _ = std::fs::create_dir_all(data.join(sub));
        }
        Paths { settings: data.join("settings.json"), screenshots: data.join("Screenshots"), data }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        match crate::formats::read_json::<Settings>(path) {
            Ok(s) => s,
            Err(e) => {
                if path.exists() {
                    eprintln!("settings: {e:#} — using defaults");
                }
                Settings::default()
            }
        }
    }

    pub fn save(&self, path: &Path) {
        if let Ok(s) = serde_json::to_vec_pretty(self) {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    pub fn vts_assets(&self) -> Option<PathBuf> {
        let p = Path::new(&self.vts_dir).join("VTube Studio_Data").join("StreamingAssets");
        p.is_dir().then_some(p)
    }
}
