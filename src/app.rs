//! Application state, per-frame loop and deferred actions.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use winit::window::Window;

use crate::cubism::core::Core;
use crate::gpu::{copy_to_buffer, padded_row, Gpu, MaskAlloc, Painter};
use crate::library::{self, ItemEntry, Library, ModelEntry};
use crate::mapping::{Inputs, NEUTRAL};
use crate::scene::live2d::{Live2D, Xform};
use crate::scene::sprite::Sprite;
use crate::scene::{draw_scene, Body, Item};
use crate::settings::*;
use crate::spout::VideoOut;
use crate::stats::FrameStats;
use crate::tracking::{self, Calibration, FaceFrame, HandsFrame, Tracker, TrackerConfig};
use crate::ui::UiState;

pub enum Action {
    LoadModel(ModelEntry),
    UnloadModel,
    AddModelFolder(PathBuf),
    AddItem(ItemEntry),
    AddItemFile(PathBuf),
    /// Spawns an item with the one-time setup from the item dialog.
    SpawnItem(crate::ui_items::ItemSetup),
    /// "Create Live2D item from model": copies the current model into the Items folder.
    ModelToItem,
    RemoveItem(u64),
    ClearItems,
    SetBackground(Background),
    Screenshot { transparent: bool },
    SaveScene(String),
    LoadScene(String),
    ToggleScene(String),
    SetApi(bool),
    SetTwitch(bool),
    DeleteScene(String),
    Calibrate,
    StartTracker,
    StopTracker,
    RefreshCameras,
    Rescan,
    ReloadCore,
    ToggleExpression(String),
    ResetExpressions,
    SetSpout(bool),
    SetNdi(bool),
    SetVcam(bool),
    ToggleRecording,
    SetTobii(bool),
    SetMaskRes(u32, u32),
    OpenVcamFolder,
    CalibrateTobii,
    SetLight(u8, usize),
    SetLightWindow(String),
    OpenUrl(String),
    SetIdle(Option<String>),
    ResetModelPosition,
    OpenDataFolder,
    NewInstance,
    ResetSettings,
    Quit,
}

type BgSlot = Arc<Mutex<Option<Result<crate::scene::sprite::SpritePrepared>>>>;
type ModelSlot = Arc<Mutex<Option<Result<crate::scene::live2d::Prepared>>>>;

struct ModelJob {
    entry: ModelEntry,
    slot: ModelSlot,
    t0: Instant,
}

pub struct App {
    pub window: Arc<Window>,
    pub gpu: Gpu,
    pub core: Option<&'static Core>,
    pub core_error: Option<String>,
    pub egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_rend: egui_wgpu::Renderer,
    /// One egui renderer per video output (Spout2, NDI, virtual camera) for the Twitch overlay.
    /// Separate instances because each keeps its own vertex buffers for the frame.
    fx_rend: Vec<egui_wgpu::Renderer>,

    pub settings: Settings,
    pub paths: Paths,
    pub library: Library,

    pub avatar: Option<Live2D>,
    pub items: Vec<Item>,
    next_id: u64,
    pub bg_sprite: Option<(String, Sprite)>,

    pub tracker: Option<Tracker>,
    /// VNet-style collab session.
    pub net: Option<crate::net::Session>,
    pub core_dl: crate::core_dl::Downloader,
    /// "Move library to rigrs" copy job.
    pub vts_import: crate::vts_import::Importer,
    /// Library copied: on exit rewrite the saved paths (VTS assets folder) and restart.
    detach_on_exit: Option<PathBuf>,
    pub tracker_dl: crate::osf::assets::Downloader,
    pub mocopi: Option<crate::mocopi::Receiver>,
    pub mocopi_error: Option<String>,
    pub discord: Option<crate::discord::Presence>,
    pub started_unix: u64,
    /// Phone discovery running in the background (Wi-Fi / USB cards).
    pub phone_scan: Option<(bool, Arc<Mutex<Option<Vec<(std::net::IpAddr, bool)>>>>)>,
    pub phone_found: Vec<(std::net::IpAddr, bool)>,
    pub tracker_error: Option<String>,
    pub tracking: bool,
    pub inputs: Inputs,
    pub last_face: Option<FaceFrame>,
    pub last_hands: Option<HandsFrame>,
    good_frames: u32,
    pub cameras: Vec<(i32, String)>,
    cameras_job: Option<Arc<Mutex<Option<Vec<(i32, String)>>>>>,
    /// Capabilities of the camera being configured (None = still loading).
    pub caps: Option<Vec<tracking::CamCap>>,
    caps_job: Option<Arc<Mutex<Option<Vec<tracking::CamCap>>>>>,

    pub ui: UiState,
    pub stats: FrameStats,
    pub spout: Option<VideoOut>,
    pub spout_error: Option<String>,
    pub ndi: Option<VideoOut>,
    pub recorder: Option<crate::record::Recorder>,
    pub demo: Option<crate::demo::Demo>,
    pub vfx: Option<crate::vfx::Renderer>,
    time: f32,
    pub tobii: Option<crate::tobii::Tobii>,
    pub tobii_state: crate::tobii::TobiiState,
    pub light: Option<crate::lighting::Sampler>,
    pub light_sample: crate::lighting::Sample,
    pub light_tint: [f32; 3],
    pub light_windows: Vec<(isize, String)>,
    pub monitors: Vec<[i32; 4]>,
    pub rec_status: String,
    pub rec_last: Option<PathBuf>,
    pub ndi_error: Option<String>,
    pub vcam: Option<VideoOut>,
    pub vcam_error: Option<String>,
    pub actions: Vec<Action>,
    pub toasts: Vec<(String, Instant)>,

    /// Global cursor position (screen pixels).
    pub mouse: [f32; 2],

    // Hotkeys
    pub hk: crate::hotkeys::Runtime,
    pub pending_model_xf: Option<Xform>,
    /// Item scene currently shown by a ToggleItemScene hotkey.
    pub active_scene: Option<String>,

    // Plugin API
    pub api: Option<crate::api::ApiServer>,
    pub api_error: Option<String>,
    pub api_prompts: crate::api::PromptQueue,
    pub api_events: Vec<(String, serde_json::Value)>,
    pub api_item_owner: std::collections::HashMap<u64, (u64, bool)>,
    pub api_custom: std::collections::BTreeMap<String, crate::api::CustomParam>,
    pub api_inject: crate::api::InjectMap,
    pub api_face_found: Option<(bool, Instant)>,
    pub api_physics: Option<(f32, Instant, String)>,
    pub api_model_move: Option<(Xform, Xform, f32, f32)>,
    pub api_item_moves: Vec<(u64, Xform, Xform, f32, f32, String)>,

    // Twitch
    pub twitch: Option<crate::twitch::Twitch>,
    pub twitch_auth: Option<String>,
    pub twitch_login: Option<std::sync::mpsc::Receiver<crate::twitch::Msg>>,
    pub fx: crate::twitch_fx::Fx,
    pub ui_dt: f32,
    /// Microphone volume 0..1 (lipsync), also used by effects.
    pub mic_level: f32,
    pub mic: Option<crate::lipsync::Mic>,
    pub mic_devices: Vec<(String, String)>,
    pub pads: crate::controller::Controllers,
    pub audio: Option<crate::audio::Audio>,
    pub sfx_rt: crate::sfx::Runtime,
    /// Name of the loaded effect preset ("" = none).
    pub vfx_preset: String,
    /// Effect fade: (from, to, elapsed, duration).
    vfx_fade: Option<(crate::vfx::Values, crate::vfx::Values, f32, f32)>,
    pub editor: Option<crate::cubism_editor::Editor>,
    pub mesh_select: Option<crate::api_events::MeshSelect>,
    /// Snapshot of the current model's config (hotkeys, other) to detect ModelConfigChangedEvent.
    cfg_snapshot: (u64, u64),
    pub ui_extra: crate::ui_model_extra::ModelExtraState,
    pub ui_vts: crate::ui_vts::VtsState,
    /// Model being prepared on a worker thread (file reads + texture decoding).
    model_job: Option<ModelJob>,
    /// Background image being decoded on a worker thread.
    bg_job: Option<(String, BgSlot)>,
    /// Saved items to restore once the startup model is loaded (pins need the model).
    pending_restore: Option<Vec<crate::settings::SavedItem>>,
    pub updater: crate::updater::Updater,
    /// Set when an update was installed: quit, then start the new exe.
    pub restart_after_quit: bool,
    /// The "new version" banner was closed for this session / the toast was shown.
    pub update_banner_closed: bool,
    update_toasted: bool,
    last: Instant,
    pub quit: bool,
    pub occluded: bool,
}

/// Repairs stale paths (moved / reinstalled VTS): Cubism Core next to rigrs.exe wins,
/// then the auto-detected VTube Studio install.
fn fix_paths(s: &mut Settings) {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    // "Work without VTube Studio": the install is neither used nor looked for.
    let vts = if s.use_vts { Some(PathBuf::from(&s.vts_dir)).filter(|p| !s.vts_dir.trim().is_empty() && p.is_dir()).or_else(crate::settings::detect_vts_dir) } else { None };
    if let Some(v) = &vts {
        s.vts_dir = v.to_string_lossy().into_owned();
    }
    if !Path::new(&s.core_dll).is_file() {
        // Next to the exe, then the copy downloaded from live2d.com, then a VTube Studio install.
        let local = exe_dir.join(crate::core_dl::file_name());
        let downloaded = crate::core_dl::installed_path(&Paths::new().data);
        if local.is_file() {
            s.core_dll = local.to_string_lossy().into_owned();
        } else if downloaded.is_file() {
            s.core_dll = downloaded.to_string_lossy().into_owned();
        } else if let Some(v) = &vts {
            s.core_dll = v.join(r"VTube Studio_Data\Plugins\x86_64\Live2DCubismCore.dll").to_string_lossy().into_owned();
        }
    }
    if !Path::new(&s.nv_exe).is_file() {
        let local = exe_dir.join("MXTracker").join("ExpressionApp.exe");
        if local.is_file() {
            s.nv_exe = local.to_string_lossy().into_owned();
        } else if let Some(v) = &vts {
            s.nv_exe = v.join(r"VTube Studio_Data\StreamingAssets\MXTracker\ExpressionApp.exe").to_string_lossy().into_owned();
        }
    }
    if !Path::new(&s.tracker_exe).is_file() {
        let local = exe_dir.join("OpenSeeFace").join("facetracker.exe");
        if local.is_file() {
            s.tracker_exe = local.to_string_lossy().into_owned();
        } else if let Some(v) = &vts {
            s.tracker_exe = v.join(r"VTube Studio_Data\StreamingAssets\OpenSeeFaceTracker_v_1_20_2\binary\facetracker.exe").to_string_lossy().into_owned();
        }
    }
}

/// Checks that a Cubism Core DLL loads (the library is unloaded again).
fn load_core_probe(path: &str) -> Result<()> {
    if !Path::new(path).is_file() {
        anyhow::bail!("not found");
    }
    Core::load(Path::new(path)).map(|_| ())
}

fn load_core(path: &str) -> Result<&'static Core> {
    Ok(Box::leak(Box::new(Core::load(Path::new(path))?)))
}

impl App {
    pub fn new(window: Arc<Window>) -> Result<App> {
        let paths = Paths::new();
        let settings = Settings::load(&paths.settings);
        let gpu = pollster::block_on(Gpu::new(window.clone(), settings.fps.vsync(), settings.low_power_gpu, settings.mask_atlas))?;

        let egui_ctx = egui::Context::default();
        crate::ui::setup_style(&egui_ctx, settings.ui_scale);
        crate::ui::install_fonts(&egui_ctx, false);
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let egui_rend = egui_wgpu::Renderer::new(&gpu.device, gpu.format, None, 1, false);
        let fx_rend = (0..3).map(|_| egui_wgpu::Renderer::new(&gpu.device, gpu.format, None, 1, false)).collect();

        let mut settings = settings;
        fix_paths(&mut settings);
        // If the configured Core does not load, try the other known locations.
        if load_core_probe(&settings.core_dll).is_err() {
            let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
            let mut cands = vec![exe_dir.join("Live2DCubismCore.dll")];
            cands.push(crate::core_dl::installed_path(&paths.data));
            if let Some(v) = crate::settings::detect_vts_dir().filter(|_| settings.use_vts) {
                cands.push(v.join(r"VTube Studio_Data\Plugins\x86_64\Live2DCubismCore.dll"));
                settings.vts_dir = v.to_string_lossy().into_owned();
                settings.tracker_exe = v.join(r"VTube Studio_Data\StreamingAssets\OpenSeeFaceTracker_v_1_20_2\binary\facetracker.exe").to_string_lossy().into_owned();
            }
            if let Some(c) = cands.into_iter().find(|c| load_core_probe(&c.to_string_lossy()).is_ok()) {
                settings.core_dll = c.to_string_lossy().into_owned();
            }
        }
        let (core, core_error) = match load_core(&settings.core_dll) {
            Ok(c) => {
                println!("Cubism Core {}", c.version_string());
                (Some(c), None)
            }
            Err(e) => (None, Some(format!("{e:#}"))),
        };

        let mut settings = settings;
        crate::i18n::set_language(&settings.language);
        let mut app = App {
            window,
            gpu,
            core,
            core_error,
            egui_ctx,
            egui_state,
            egui_rend,
            fx_rend,
            library: Library::default(),
            settings,
            paths,
            avatar: None,
            items: Vec::new(),
            next_id: 1,
            bg_sprite: None,
            tracker: None,
            net: None,
            core_dl: Default::default(),
            vts_import: Default::default(),
            detach_on_exit: None,
            tracker_dl: Default::default(),
            mocopi: None,
            mocopi_error: None,
            discord: None,
            started_unix: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            phone_scan: None,
            phone_found: Vec::new(),
            tracker_error: None,
            tracking: false,
            inputs: NEUTRAL,
            last_face: None,
            last_hands: None,
            good_frames: 0,
            cameras: Vec::new(),
            cameras_job: None,
            caps: None,
            caps_job: None,
            ui: UiState::default(),
            stats: FrameStats::default(),
            spout: None,
            spout_error: None,
            ndi: None,
            recorder: None,
            demo: std::env::var_os("RIGRS_DEMO").map(|_| crate::demo::Demo::new()),
            vfx: None,
            time: 0.0,
            tobii: None,
            tobii_state: Default::default(),
            light: None,
            light_sample: Default::default(),
            light_tint: [1.0; 3],
            light_windows: Vec::new(),
            monitors: crate::lighting::monitors(),
            rec_status: "Готов к записи анимации Live2D.".into(),
            rec_last: None,
            ndi_error: None,
            vcam: None,
            vcam_error: None,
            actions: Vec::new(),
            toasts: Vec::new(),
            mouse: [0.0, 0.0],
            hk: Default::default(),
            pending_model_xf: None,
            active_scene: None,
            api: None,
            api_error: None,
            api_prompts: Default::default(),
            api_events: Vec::new(),
            api_item_owner: Default::default(),
            api_custom: Default::default(),
            api_inject: Default::default(),
            api_face_found: None,
            api_physics: None,
            api_model_move: None,
            api_item_moves: Vec::new(),
            twitch: None,
            twitch_auth: None,
            twitch_login: None,
            fx: Default::default(),
            ui_dt: 0.0,
            mic_level: 0.0,
            mic: None,
            mic_devices: Vec::new(),
            pads: Default::default(),
            audio: None,
            sfx_rt: Default::default(),
            vfx_preset: String::new(),
            vfx_fade: None,
            editor: None,
            mesh_select: None,
            cfg_snapshot: (0, 0),
            ui_extra: Default::default(),
            ui_vts: Default::default(),
            model_job: None,
            bg_job: None,
            pending_restore: None,
            updater: Default::default(),
            restart_after_quit: false,
            update_banner_closed: false,
            update_toasted: false,
            last: Instant::now(),
            quit: false,
            occluded: false,
        };
        app.rescan();
        if let Some(key) = app.settings.last_model.clone() {
            if let Some(e) = app.library.models.iter().find(|m| m.model3.to_string_lossy() == key).cloned() {
                app.load_model(&e);
            }
        }
        if app.settings.save_scene {
            let saved = app.settings.last_items.clone();
            if app.model_job.is_some() {
                app.pending_restore = Some(saved);
            } else {
                app.restore_items(&saved);
            }
        }
        let bg = app.settings.background.clone();
        app.set_background(bg);
        // The window is still hidden at startup: wait for the model and background so the
        // first frame shown is complete (both are decoded in parallel on worker threads).
        let wait = Instant::now();
        while (app.model_job.is_some() || app.bg_job.is_some()) && wait.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(2));
            app.poll_model_job();
            app.poll_bg_job();
        }
        if app.settings.track_autostart {
            app.start_tracker();
        }
        if app.settings.spout {
            app.set_spout(true);
        }
        if app.settings.ndi {
            app.set_ndi(true);
        }
        if app.settings.vcam_on {
            app.set_vcam(true);
        }
        if app.settings.tobii_enabled {
            app.tobii = Some(crate::tobii::Tobii::start());
        }
        app.apply_light_source();
        app.refresh_cameras();
        if app.settings.api_on {
            app.set_api(true);
        }
        if app.settings.twitch.enabled {
            app.set_twitch(true);
        }
        if app.settings.editor_on {
            app.set_editor(true);
        }
        if app.settings.mic.on {
            app.set_mic(true);
        }
        // Dev hook for multiplayer tests: RIGRS_NET=host | join:<address>.
        if let Ok(v) = std::env::var("RIGRS_NET") {
            app.settings.net.host = v == "host";
            if let Some(addr) = v.strip_prefix("join:") {
                app.settings.net.address = addr.to_string();
            }
            if let Ok(n) = std::env::var("RIGRS_NET_NICK") {
                app.settings.net.nickname = n;
            }
            crate::net::start(&mut app);
        }
        crate::updater::cleanup_old();
        if app.settings.update_check && std::env::var_os("RIGRS_DATA").is_none() || std::env::var_os("RIGRS_FAKE_VERSION").is_some() {
            app.updater.check();
        }
        // Dev hook for screenshots/tests: RIGRS_OPEN_SETTINGS=general|camera|model|hotkeys
        if let Ok(t) = std::env::var("RIGRS_OPEN_SETTINGS") {
            app.ui.settings_open = true;
            app.ui.tab = match t.as_str() {
                "camera" => crate::ui::Tab::Camera,
                "model" => crate::ui::Tab::Model,
                "hotkeys" | "hk-twitch" | "hk-action" => crate::ui::Tab::Hotkeys,
                _ => crate::ui::Tab::General,
            };
            if t.starts_with("camera") {
                app.ui.tab = crate::ui::Tab::Camera;
            }
            if let Some(px) = std::env::var("RIGRS_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
                app.ui.pending_scroll = Some(px);
            }
            use crate::ui_vts::Dlg;
            app.ui_vts.dlg = match std::env::var("RIGRS_DLG").unwrap_or_default().as_str() {
                "sound" => Some(Dlg::SoundSettings),
                "triggers" => Some(Dlg::SoundTriggers),
                "customize" => Some(Dlg::Customize),
                "autosetup" => Some(Dlg::AutoSetup),
                "autoconfirm" => Some(Dlg::AutoConfirm),
                "copy" => Some(Dlg::CopyFrom),
                "mesh1" => Some(Dlg::Meshes(1)),
                _ => None,
            };
            app.ui_vts.pad_widget = std::env::var("RIGRS_DLG").map(|v| v == "pad").unwrap_or(false);
            // Dev hook: start "Move library to rigrs" right away (end-to-end test).
            if std::env::var("RIGRS_DLG").map(|v| v == "detach").unwrap_or(false) {
                if let Some(assets) = app.settings.vts_assets() {
                    app.vts_import.start(&assets, &app.paths.data);
                }
            }
            if std::env::var("RIGRS_DLG").map(|v| v == "items").unwrap_or(false) {
                app.ui.settings_open = false;
                app.ui.item_picker = true;
            }
            app.ui.modal = match t.as_str() {
                "camera-cam" => Some(crate::ui::Modal::Camera),
                "camera-quality" => Some(crate::ui::Modal::Quality),
                "hk-twitch" => Some(crate::ui::Modal::HkTwitch(0)),
                "hk-action" => Some(crate::ui::Modal::HkAction(0)),
                "twitch-features" => Some(crate::ui::Modal::TwitchFeatures),
                "twitch-dropper" => Some(crate::ui::Modal::TwitchDropper),
                "twitch-chat" => Some(crate::ui::Modal::TwitchChat),
                _ => None,
            };
        }
        Ok(app)
    }

    pub fn log(&self, s: &str) {
        if self.settings.log_to_file {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.paths.data.join("rigrs.log")) {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let _ = writeln!(f, "[{secs}] {s}");
            }
        }
    }

    fn update_mouse(&mut self) {
        #[cfg(windows)]
        unsafe {
            let mut p = windows::Win32::Foundation::POINT::default();
            if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p).is_ok() {
                self.mouse = [p.x as f32, p.y as f32];
            }
        }
        let r = self.settings.mouse_range;
        let nx = (self.mouse[0] - r[0]) / (r[1] - r[0]).max(1.0);
        let ny = (self.mouse[1] - r[2]) / (r[3] - r[2]).max(1.0);
        self.inputs[crate::mapping::input::MOUSE_POSITION_X] = (nx * 2.0 - 1.0).clamp(-1.0, 1.0);
        self.inputs[crate::mapping::input::MOUSE_POSITION_Y] = (1.0 - ny * 2.0).clamp(-1.0, 1.0);
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        let s = s.into();
        self.log(&s);
        println!("{s}");
        self.toasts.push((s, Instant::now()));
    }

    pub fn on_window_event(&mut self, e: &winit::event::WindowEvent) -> bool {
        self.egui_state.on_window_event(&self.window, e).consumed
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.gpu.resize(w, h);
        if self.spout.is_some() && w > 0 && h > 0 {
            self.set_spout(true);
        }
        if self.ndi.is_some() && !self.settings.ndi_custom_size && w > 0 && h > 0 {
            self.set_ndi(true);
        }
    }

    pub fn viewport(&self) -> (f32, f32) {
        (self.gpu.config.width as f32, self.gpu.config.height as f32)
    }

    pub fn rescan(&mut self) {
        let extra: Vec<PathBuf> = self.settings.extra_model_dirs.iter().map(PathBuf::from).collect();
        self.library = library::scan(self.settings.vts_assets().as_deref(), &self.paths.data, &extra);
    }

    // ------------------------------------------------------------------ tracking

    /// Built-in tracker files when the current mode can use it (OpenSeeFace levels, face only).
    /// Returns (ONNX Runtime, models folder, quality level to run). The level is the selected
    /// one, or the nearest installed one.
    pub fn builtin_tracker(&self) -> Option<(PathBuf, PathBuf, i32)> {
        let s = &self.settings;
        if !s.track_builtin || s.track_backend != tracking::backend::OPENSEEFACE || s.track_type != 0 {
            return None;
        }
        let exe = PathBuf::from(&s.tracker_exe);
        let rt = crate::osf::assets::find_runtime(&self.paths.data, &exe)?;
        let (models, level) = crate::osf::assets::find_models_any(&self.paths.data, &exe, s.track_quality)?;
        Some((rt, models, level))
    }

    /// Camera lists come from Media Foundation (true) or from `facetracker.exe` (false).
    pub fn cameras_builtin(&self) -> bool {
        cfg!(windows) && self.settings.track_builtin && self.settings.track_backend == tracking::backend::OPENSEEFACE && self.settings.track_type == 0
    }

    pub fn start_tracker(&mut self) {
        self.tracker = None;
        let builtin = self.builtin_tracker();
        let s = &self.settings;
        if builtin.is_none() && s.track_builtin && s.track_backend == tracking::backend::OPENSEEFACE && s.track_type == 0 && !Path::new(&s.tracker_exe).is_file() {
            self.tracker_error = Some(crate::i18n::tr("Нужны файлы встроенного трекера — нажмите «Скачать трекер» на вкладке камеры").to_string());
            return;
        }
        let cfg = TrackerConfig {
            exe: PathBuf::from(&self.settings.tracker_exe),
            camera: self.settings.camera,
            width: self.settings.cam_size[0],
            height: self.settings.cam_size[1],
            fps: self.settings.cam_fps,
            model: builtin.as_ref().map(|b| b.2).unwrap_or(self.settings.track_quality),
            threads: self.settings.track_threads,
            dcap: self.settings.cam_dcap,
            mediapipe: self.settings.track_backend == 1,
            hands: self.settings.track_type >= 1,
            port: if self.settings.track_backend == tracking::backend::PHONE { self.settings.phone_local_port } else { self.settings.track_port },
            spawn: true,
            backend: self.settings.track_backend,
            nv_exe: PathBuf::from(&self.settings.nv_exe),
            nv_cal: self.settings.nv_cal.clone(),
            phone_ip: self.settings.phone_ip.clone(),
            phone_port: self.settings.phone_port,
            builtin: builtin.map(|b| (b.0, b.1)),
        };
        match Tracker::start(&cfg) {
            Ok(t) => {
                self.tracker = Some(t);
                self.tracker_error = None;
                self.good_frames = 0;
            }
            Err(e) => self.tracker_error = Some(format!("{e:#}")),
        }
    }

    pub fn refresh_cameras(&mut self) {
        let exe = PathBuf::from(&self.settings.tracker_exe);
        let builtin = self.cameras_builtin();
        if !builtin && !exe.is_file() {
            return;
        }
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            *s2.lock().unwrap() = Some(if builtin { crate::osf::camera::list() } else { tracking::list_cameras(&exe) });
        });
        self.cameras_job = Some(slot);
    }

    pub fn load_caps(&mut self, camera: i32) {
        self.caps = None;
        let exe = PathBuf::from(&self.settings.tracker_exe);
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        let builtin = self.cameras_builtin();
        std::thread::spawn(move || {
            let caps = if builtin {
                crate::osf::camera::modes(camera).into_iter().map(|m| tracking::CamCap { index: m.index as i32, width: m.width, height: m.height, fps_min: m.fps, fps_max: m.fps, format: m.format }).collect()
            } else {
                tracking::list_caps(&exe, camera)
            };
            *s2.lock().unwrap() = Some(caps);
        });
        self.caps_job = Some(slot);
    }

    fn update_tracking(&mut self) {
        let caps = self.caps_job.as_ref().and_then(|j| j.lock().unwrap().take());
        if let Some(c) = caps {
            self.caps = Some(c);
            self.caps_job = None;
        }
        let done = self.cameras_job.as_ref().and_then(|j| j.lock().unwrap().take());
        if let Some(list) = done {
            self.cameras = list;
            self.cameras_job = None;
        }
        let Some(t) = &mut self.tracker else {
            self.tracking = false;
            self.inputs = NEUTRAL;
            crate::perfect_sync::clear();
            return;
        };
        t.poll();
        let (err, was_builtin) = (t.error.lock().unwrap().clone(), t.is_builtin());
        if let Some(e) = err {
            // The built-in tracker failed (camera, runtime): go back to facetracker.exe if there is one.
            if was_builtin && Path::new(&self.settings.tracker_exe).is_file() {
                self.settings.track_builtin = false;
                self.settings.cam_dcap = -1;
                self.toast(format!("{}: {e}", crate::i18n::tr("Встроенный трекер не запустился, включаю facetracker.exe")));
                self.start_tracker();
                self.save();
                return;
            }
            self.tracker_error = Some(e);
        }
        let Some(t) = &mut self.tracker else { return };
        let nv_cal = t.nv_cal.lock().unwrap().take();
        let phone_hotkey = t.phone_hotkey.lock().unwrap().take();
        if let Some(c) = nv_cal {
            self.settings.nv_cal = c;
            self.toast("NVIDIA: калибровка сохранена");
            self.save();
        }
        if let Some(k) = phone_hotkey {
            // Phone on-screen hotkeys 1..8 → hotkeys bound to that screen button (VTS `ScreenButton`).
            let id = self.hotkeys().iter().find(|h| h.triggers.screen_button == k).map(|h| h.id.clone());
            if let Some(id) = id {
                let _ = crate::api::trigger_hotkey(self, &id);
            }
        }
        let Some(t) = &mut self.tracker else { return };
        let hands = t.latest_hands().filter(|h| h.received.elapsed() < Duration::from_millis(400));
        let face_on = self.settings.track_type != 1;
        match t.latest().filter(|_| face_on) {
            Some(f) if f.success && f.received.elapsed() < Duration::from_millis(400) => {
                self.good_frames += 1;
                if !self.settings.calib.valid && self.good_frames > 20 {
                    self.settings.calib = Calibration::from_frame(&f);
                    self.toast("Автокалибровка выполнена — смотрите прямо в камеру и нажмите «Калибровать», если нужно");
                }
                self.inputs = tracking::to_inputs(&f, &self.settings.calib, &self.settings.track);
                crate::perfect_sync::update(&f, self.settings.track.mirror);
                self.tracking = true;
                self.last_face = Some(f);
            }
            _ => {
                self.tracking = false;
                if self.settings.lost_reset || !face_on {
                    crate::perfect_sync::clear();
                    let keep = self.inputs;
                    self.inputs = NEUTRAL;
                    // Mouse inputs are not tracking-dependent.
                    for i in [crate::mapping::input::MOUSE_POSITION_X, crate::mapping::input::MOUSE_POSITION_Y] {
                        self.inputs[i] = keep[i];
                    }
                }
            }
        }
        if self.settings.track_type >= 1 {
            let h = hands.unwrap_or(HandsFrame { hands: [None, None], received: std::time::Instant::now() });
            tracking::hand_inputs(&mut self.inputs, &h, &self.settings.track);
            if self.settings.track_type == 1 && hands.map(|h| h.hands.iter().any(|x| x.is_some())).unwrap_or(false) {
                self.tracking = true;
            }
        }
        self.last_hands = hands;
    }

    /// Switches tracker backend / type and restarts the tracker if it runs.
    pub fn set_tracking_mode(&mut self, backend: u8, kind: u8) {
        if backend != self.settings.track_backend {
            // OpenSeeFace and Mediapipe have different neutral poses.
            self.settings.calib.valid = false;
        }
        self.settings.track_backend = backend;
        self.settings.track_type = kind;
        if self.tracker.is_some() {
            self.start_tracker();
        }
        self.save();
    }

    // ------------------------------------------------------------------ models & items

    pub fn model_key(&self) -> Option<String> {
        self.avatar.as_ref().map(|a| a.model3.to_string_lossy().into_owned())
    }

    /// Starts loading a model: files and textures are prepared on a worker thread, the app
    /// keeps running (the current model stays on screen) and `poll_model_job` finishes the
    /// load on the main thread once the data is ready.
    pub fn load_model(&mut self, e: &ModelEntry) {
        if self.core.is_none() {
            self.toast("Cubism Core не загружен — укажите путь к Live2DCubismCore.dll в настройках");
            return;
        }
        let slot: ModelSlot = Arc::new(Mutex::new(None));
        let (s2, p) = (slot.clone(), e.model3.clone());
        let spawned = std::thread::Builder::new().name("model-load".into()).spawn(move || {
            let r = Live2D::prepare(&p);
            *s2.lock().unwrap() = Some(r);
        });
        if spawned.is_err() {
            // No thread: load in place.
            let r = Live2D::prepare(&e.model3);
            self.finish_model_load(e, r, Instant::now());
            return;
        }
        self.model_job = Some(ModelJob { entry: e.clone(), slot, t0: Instant::now() });
    }

    /// True while a model is being loaded in the background.
    pub fn model_loading(&self) -> Option<&str> {
        self.model_job.as_ref().map(|j| j.entry.name.as_str())
    }

    fn poll_model_job(&mut self) {
        let Some(job) = &self.model_job else { return };
        let Some(r) = job.slot.lock().unwrap().take() else { return };
        let job = self.model_job.take().unwrap();
        self.finish_model_load(&job.entry, r, job.t0);
        if let Some(saved) = self.pending_restore.take() {
            self.restore_items(&saved);
        }
    }

    fn finish_model_load(&mut self, e: &ModelEntry, pre: Result<crate::scene::live2d::Prepared>, t0: Instant) {
        let Some(core) = self.core else { return };
        let tm = Instant::now();
        let dbg = std::env::var_os("RIGRS_HITCH").is_some();
        let mark = |what: &str| if dbg { eprintln!("LOAD {what}: {:.1} ms", tm.elapsed().as_secs_f32() * 1000.0) };
        self.remember_avatar_xf();
        let key = e.model3.to_string_lossy().into_owned();
        match pre.and_then(|pre| {
            Live2D::load_prepared(
                core,
                &self.gpu,
                pre,
                e.vtube.as_deref(),
                self.settings.idle_override.get(&key).map(|s| s.as_str()),
                self.settings.rules_override.get(&key),
            )
        }) {
            Ok(mut a) => {
                mark("load_prepared");
                a.update(0.0, &NEUTRAL, false, false);
                mark("first update");
                let (vw, vh) = self.viewport();
                match self.settings.model_xf.get(&key) {
                    Some(xf) => a.xf = *xf,
                    None => a.fit(vw, vh, 0.9),
                }
                if let Some(cfg) = self.settings.model_cfg.get(&key).cloned() {
                    a.apply_cfg(cfg);
                }
                for it in &mut self.items {
                    it.pin = None;
                }
                self.toast(format!(
                    "Загружена модель «{}» за {} мс: {} мешей, {} параметров, {} цепочек физики, текстуры {:.0} МБ",
                    a.name,
                    t0.elapsed().as_millis(),
                    a.model.drawables.len(),
                    a.model.param_ids.len(),
                    a.physics_chains(),
                    a.texture_bytes as f64 / 1048576.0
                ));
                a.load_ms = t0.elapsed().as_millis() as u64;
                if let Some(xf) = self.pending_model_xf.take() {
                    a.xf = xf;
                }
                let ev = serde_json::json!({ "modelLoaded": true, "modelName": a.name, "modelID": a.model_id });
                self.api_events.push(("ModelLoadedEvent".into(), ev));
                self.avatar = Some(a);
                self.settings.last_model = Some(key);
                self.active_scene = None;
                mark("before hotkeys");
                crate::hotkeys::on_model_loaded(self);
                mark("hotkeys");
                if let Some(key) = self.model_key() {
                    if !self.settings.model_sound.contains_key(&key) {
                        let vol = e.vtube.as_ref().and_then(|p| crate::formats::read_json::<crate::formats::VtubeJson>(p).ok()).and_then(|v| v.general_settings).and_then(|g| g["ModelVolume"].as_f64()).unwrap_or(90.0);
                        self.settings.model_sound.insert(key, crate::sfx::ModelSound { volume: vol as f32, ..Default::default() });
                    }
                }
                self.apply_artmesh();
                mark("artmesh");
                self.save();
                mark("save");
            }
            Err(err) => self.toast(format!("Не удалось загрузить модель: {err:#}")),
        }
    }

    fn remember_avatar_xf(&mut self) {
        if let Some(a) = &self.avatar {
            let key = a.model3.to_string_lossy().into_owned();
            self.settings.model_xf.insert(key.clone(), a.xf);
            let mut cfg = a.cfg.clone();
            cfg.active_expressions = if cfg.save_expressions { a.expressions.iter().filter(|e| e.active).map(|e| e.file.clone()).collect() } else { Vec::new() };
            self.settings.model_cfg.insert(key, cfg);
        }
    }

    pub(crate) fn make_item(&mut self, source: &Path) -> Result<Item> {
        let (vw, vh) = self.viewport();
        let name = source.file_name().unwrap_or_default().to_string_lossy().trim_end_matches(".model3.json").to_string();
        let id = self.next_id;
        self.next_id += 1;
        let mut pivot = [0.0, 0.0];
        let (body, xf) = if source.to_string_lossy().ends_with(".model3.json") {
            let core = self.core.ok_or_else(|| anyhow::anyhow!("Cubism Core не загружен"))?;
            let dir = source.parent().unwrap_or(Path::new("."));
            let vtube = std::fs::read_dir(dir)?
                .flatten()
                .map(|e| e.path())
                .find(|p| p.to_string_lossy().ends_with(".vtube.json"));
            let mut l = Live2D::load(core, &self.gpu, source, vtube.as_deref(), None, None)?;
            l.update(0.0, &NEUTRAL, false, false);
            l.fit(vw, vh, 0.55);
            let b = l.local_bounds();
            pivot = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
            let xf = Xform { pos: [0.0, 0.0], ..l.xf };
            (Body::Live2D(Box::new(l)), xf)
        } else {
            let s = Sprite::load(&self.gpu, source)?;
            // Spawn at a comfortable size (about half the window height); the mouse wheel
            // scales items without limits, like in VTS.
            let scale = (vh * 0.45 / s.size[1]).min(vw * 0.6 / s.size[0]);
            (Body::Sprite(s), Xform { pos: [0.0, 0.0], scale, rot: 0.0, flip: false })
        };
        let mut it = Item::new(id, source.to_path_buf(), name, body, xf, pivot);
        // New items go in front of everything else (next free order, as VTS does).
        it.cfg.order = self.items.iter().map(|i| i.cfg.order).max().unwrap_or(0).max(0) + 1;
        if let Some(fps) = self.settings.item_fps.get(&source.to_string_lossy().to_string()) {
            it.cfg.fps = *fps;
        }
        Ok(it)
    }

    /// Dev hooks for the two-instance multiplayer test: `RIGRS_NET_ITEM=<path>` spawns an item
    /// after 3 s, `RIGRS_NET_SWAP=<secs>` switches to another library model at that time.
    fn dev_net_test(&mut self) {
        use std::sync::atomic::{AtomicU8, Ordering};
        static DONE: AtomicU8 = AtomicU8::new(0);
        let done = DONE.load(Ordering::Relaxed);
        if done & 1 == 0 && self.time > 3.0 {
            DONE.fetch_or(1, Ordering::Relaxed);
            if let Ok(p) = std::env::var("RIGRS_NET_ITEM") {
                if let Some(id) = self.add_item(Path::new(&p)) {
                    if let Some(it) = self.items.iter_mut().find(|i| i.id == id) {
                        it.xf.pos = [200.0, 100.0];
                    }
                }
            }
        }
        if done & 2 == 0 {
            if let Some(t) = std::env::var("RIGRS_NET_SWAP").ok().and_then(|v| v.parse::<f32>().ok()) {
                if self.time > t {
                    DONE.fetch_or(2, Ordering::Relaxed);
                    let cur = self.avatar.as_ref().map(|a| a.model3.clone());
                    if let Some(m) = self.library.models.iter().find(|m| Some(&m.model3) != cur.as_ref()).cloned() {
                        eprintln!("NETTEST swap to {}", m.name);
                        self.actions.push(Action::LoadModel(m));
                    }
                }
            }
        }
    }

    pub fn add_item(&mut self, source: &Path) -> Option<u64> {
        match self.make_item(source) {
            Ok(it) => {
                let id = it.id;
                if source.to_string_lossy().ends_with(".model3.json") {
                    self.item_sound(source, true);
                }
                self.items.push(it);
                Some(id)
            }
            Err(e) => {
                self.toast(format!("Не удалось добавить предмет: {e:#}"));
                None
            }
        }
    }

    pub fn saved_items(&self) -> Vec<SavedItem> {
        self.items
            .iter()
            .map(|i| SavedItem {
                source: i.source.to_string_lossy().into_owned(),
                xf: i.xf,
                front: i.front(),
                locked: i.locked,
                pin: i.pin.clone(),
                cfg: Some(i.cfg.clone()),
                scene: i.scene.clone(),
                model: if i.pin.is_some() { self.model_key() } else { None },
            })
            .collect()
    }

    /// Items of a scene currently in the scene (not fading out).
    pub fn scene_loaded(&self, name: &str) -> bool {
        self.items.iter().any(|i| i.scene.as_deref() == Some(name) && i.alpha_target > 0.0)
    }

    /// VTS item scene toggle: loads the scene (unloading other scenes of the same group),
    /// or unloads its remaining items if any are still loaded. `fade` in seconds.
    pub fn toggle_scene(&mut self, name: &str, fade: f32) -> bool {
        if self.scene_loaded(name) {
            self.unload_scene(name, fade);
            return false;
        }
        let Some(sc) = self.settings.item_scenes.iter().find(|s| s.name == name).cloned() else {
            self.toast(format!("Сцена элементов «{name}» не найдена"));
            return false;
        };
        if !sc.group.is_empty() {
            let same_group: Vec<String> = self.settings.item_scenes.iter().filter(|s| s.group == sc.group && s.name != sc.name).map(|s| s.name.clone()).collect();
            for g in same_group {
                self.unload_scene(&g, fade);
            }
        }
        let model = self.model_key();
        let saved: Vec<SavedItem> = sc.items.iter().filter(|s| s.pin.is_none() || s.model.is_none() || s.model == model).cloned().collect();
        let before: Vec<u64> = self.items.iter().map(|i| i.id).collect();
        self.restore_items_into(&saved, false);
        for it in self.items.iter_mut().filter(|i| !before.contains(&i.id)) {
            it.scene = Some(name.to_string());
            if fade > 0.0 {
                it.alpha = 0.0;
                it.alpha_target = 1.0;
                it.fade_speed = 1.0 / fade;
            }
        }
        self.active_scene = Some(name.to_string());
        true
    }

    pub fn unload_scene(&mut self, name: &str, fade: f32) {
        for it in self.items.iter_mut().filter(|i| i.scene.as_deref() == Some(name)) {
            it.alpha_target = 0.0;
            it.fade_speed = if fade > 0.0 { 1.0 / fade } else { 0.0 };
        }
        if self.active_scene.as_deref() == Some(name) {
            self.active_scene = None;
        }
    }

    pub fn restore_items(&mut self, saved: &[SavedItem]) {
        self.restore_items_into(saved, true);
    }

    pub fn restore_items_into(&mut self, saved: &[SavedItem], clear: bool) {
        if clear {
            self.items.clear();
        }
        for s in saved {
            if let Some(id) = self.add_item(Path::new(&s.source)) {
                let av_ok = |p: &crate::scene::Pin, a: &Live2D| {
                    a.model.drawables.get(p.drawable).map(|d| p.tri.iter().all(|&t| (t as usize) < d.vertex_count)).unwrap_or(false)
                };
                let pin = match (&s.pin, &self.avatar) {
                    (Some(p), Some(a)) if av_ok(p, a) => Some(p.clone()),
                    _ => None,
                };
                if let Some(it) = self.items.iter_mut().find(|i| i.id == id) {
                    it.xf = s.xf;
                    match &s.cfg {
                        Some(c) => it.cfg = c.clone(),
                        None => it.cfg.order = if s.front { it.cfg.order } else { -it.cfg.order },
                    }
                    it.locked = s.locked;
                    it.pin = pin;
                    it.scene = s.scene.clone();
                }
            }
        }
    }

    /// Switches the background. Images are decoded on a worker thread; the old background
    /// stays until the new one is ready (no freeze on big 4K images).
    pub fn set_background(&mut self, bg: Background) {
        self.bg_job = None;
        match &bg {
            Background::Image(p) => {
                let slot: BgSlot = Arc::new(Mutex::new(None));
                let (s2, path) = (slot.clone(), PathBuf::from(p));
                let r = std::thread::Builder::new().name("bg-load".into()).spawn(move || {
                    *s2.lock().unwrap() = Some(Sprite::prepare(&path));
                });
                if r.is_ok() {
                    self.bg_job = Some((p.clone(), slot));
                } else {
                    match Sprite::load(&self.gpu, Path::new(p)) {
                        Ok(s) => self.bg_sprite = Some((p.clone(), s)),
                        Err(e) => self.toast(format!("Фон: {e:#}")),
                    }
                }
            }
            _ => self.bg_sprite = None,
        }
        self.settings.background = bg;
    }

    fn poll_bg_job(&mut self) {
        let Some((_, slot)) = &self.bg_job else { return };
        let Some(r) = slot.lock().unwrap().take() else { return };
        let (p, _) = self.bg_job.take().unwrap();
        match r {
            Ok(pre) => self.bg_sprite = Some((p, Sprite::load_prepared(&self.gpu, pre))),
            Err(e) => {
                self.bg_sprite = None;
                self.toast(format!("Фон: {e:#}"));
            }
        }
    }

    fn clear_color(&self) -> wgpu::Color {
        match &self.settings.background {
            Background::Color(c) => wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 },
            Background::Image(_) => wgpu::Color::BLACK,
            Background::Transparent => wgpu::Color { r: 0.12, g: 0.12, b: 0.14, a: 1.0 },
        }
    }

    pub fn set_spout(&mut self, on: bool) {
        self.spout = None;
        self.spout_error = None;
        self.settings.spout = on;
        if !on {
            return;
        }
        let (w, h) = (self.gpu.config.width, self.gpu.config.height);
        match VideoOut::spout(&self.gpu, &self.settings.spout_name, w, h, self.settings.spout_mirror) {
            Ok(s) => self.spout = Some(s),
            Err(e) => self.spout_error = Some(format!("{e:#}")),
        }
    }

    pub fn set_tobii(&mut self, on: bool) {
        self.settings.tobii_enabled = on;
        self.tobii = if on { Some(crate::tobii::Tobii::start()) } else { None };
        if !on {
            self.tobii_state = Default::default();
        }
    }

    pub fn calibrate_tobii(&mut self) {
        let s = &self.tobii_state;
        if let Some(r) = s.head_rot {
            self.settings.tobii_calib_rot = r;
        }
        if let Some(g) = s.gaze {
            self.settings.tobii_calib_gaze = g;
        }
        self.toast("Tobii: текущий взгляд и поза головы приняты за центр");
    }

    fn update_tobii(&mut self) {
        let Some(t) = &self.tobii else { return };
        self.tobii_state = t.snapshot();
        let s = &self.tobii_state;
        if !s.updated.map(|u| u.elapsed() < Duration::from_millis(300)).unwrap_or(false) {
            return;
        }
        use crate::mapping::input::*;
        let o = &self.settings.track;
        let m = if o.mirror { -1.0 } else { 1.0 };
        let inv = |b: bool| if b { -1.0 } else { 1.0 };
        if self.settings.tobii_replace_eyes {
            if let Some(g) = s.gaze {
                let c = self.settings.tobii_calib_gaze;
                let gx = ((g[0] - c[0]) * 2.5 * m * inv(o.invert_gaze[0])).clamp(-1.0, 1.0);
                let gy = (-(g[1] - c[1]) * 2.5 * inv(o.invert_gaze[1])).clamp(-1.0, 1.0);
                self.inputs[EYE_LEFT_X] = gx;
                self.inputs[EYE_RIGHT_X] = gx;
                self.inputs[EYE_LEFT_Y] = gy;
                self.inputs[EYE_RIGHT_Y] = gy;
            }
        }
        if self.settings.tobii_replace_head {
            if let Some(r) = s.head_rot {
                let c = self.settings.tobii_calib_rot;
                let d = |i: usize| (r[i] - c[i]).to_degrees();
                self.inputs[FACE_ANGLE_X] = d(1) * m * inv(o.invert[0]);
                self.inputs[FACE_ANGLE_Y] = d(0) * inv(o.invert[1]);
                self.inputs[FACE_ANGLE_Z] = d(2) * m * inv(o.invert[2]);
                self.tracking = true;
            }
            if let Some(p) = s.head_pos {
                self.inputs[FACE_POSITION_X] = p[0] / 10.0 * m;
                self.inputs[FACE_POSITION_Y] = p[1] / 10.0;
                self.inputs[FACE_POSITION_Z] = -(p[2] - 600.0) / 10.0;
            }
        }
    }

    /// (Re)starts the screen sampler for the current lighting mode.
    pub fn apply_light_source(&mut self) {
        use crate::lighting::{Sampler, Source};
        let src = match self.settings.light_mode {
            1 => Some(Source::Monitor(self.settings.light_monitor)),
            2 => {
                if self.light_windows.is_empty() {
                    self.light_windows = crate::lighting::windows();
                }
                self.light_windows.iter().find(|w| w.1 == self.settings.light_window).map(|w| Source::Window(w.0))
            }
            _ => None,
        };
        if self.light.as_ref().map(|l| Some(l.source)) != Some(src) || src.is_none() {
            self.light = src.map(Sampler::start);
        }
    }

    fn update_lighting(&mut self, dt: f32) {
        let st = &self.settings;
        let base = match st.light_mode {
            0 => None,
            3 => Some(st.light_color),
            _ => self.light.as_ref().map(|l| l.sample()).filter(|s| s.ok).map(|s| {
                let c = s.color;
                self.light_sample = s;
                c
            }),
        };
        let target = match base {
            None => [1.0; 3],
            Some(c) => {
                let o = st.light_overlay / 100.0;
                let b = 0.5 + st.light_brightness / 50.0;
                [0, 1, 2].map(|i| ((1.0 - o) + o * (c[i] * b).clamp(0.0, 1.6)).clamp(0.0, 1.6))
            }
        };
        let tau = st.light_smoothing * 0.03;
        let a = if tau <= 0.0 { 1.0 } else { 1.0 - (-dt / tau).exp() };
        for i in 0..3 {
            self.light_tint[i] += (target[i] - self.light_tint[i]) * a;
        }
        let t = self.light_tint;
        if let Some(av) = &mut self.avatar {
            av.tint = t;
        }
        let it = if st.light_items { t } else { [1.0; 3] };
        for item in &mut self.items {
            item.set_tint(&self.gpu, it);
        }
    }

    pub fn start_recording(&mut self) {
        let Some(a) = &self.avatar else {
            self.rec_status = "Сначала загрузите модель.".into();
            return;
        };
        let dir = a.dir.join("rigrs_recordings");
        self.recorder = Some(crate::record::Recorder::new(
            a.model.param_ids.clone(),
            &a.physics_outputs(),
            self.settings.rec_physics,
            self.settings.rec_unchanged,
            self.settings.rec_seconds,
            dir,
        ));
    }

    pub fn stop_recording(&mut self) {
        if let Some(r) = self.recorder.take() {
            let path = r.finish();
            self.rec_status = format!("Сохранено: {}", path.display());
            self.toast(format!("Анимация записана: {}", path.file_name().unwrap_or_default().to_string_lossy()));
            self.ui_motions_dirty();
            self.rec_last = Some(path);
        }
    }

    fn ui_motions_dirty(&mut self) {
        self.ui.invalidate_motions();
    }

    pub fn set_vcam(&mut self, on: bool) {
        self.vcam = None;
        self.vcam_error = None;
        self.settings.vcam_on = on;
        if !on {
            return;
        }
        match VideoOut::vcam(&self.gpu, self.settings.vcam_mirror) {
            Ok(v) => self.vcam = Some(v),
            Err(e) => self.vcam_error = Some(format!("{e:#}")),
        }
    }

    pub fn set_ndi(&mut self, on: bool) {
        self.ndi = None;
        self.ndi_error = None;
        self.settings.ndi = on;
        if !on {
            return;
        }
        let (w, h) = if self.settings.ndi_custom_size {
            (self.settings.ndi_size[0].max(16), self.settings.ndi_size[1].max(16))
        } else {
            (self.gpu.config.width, self.gpu.config.height)
        };
        match VideoOut::ndi(&self.gpu, &self.settings.ndi_name, w, h, self.settings.ndi_mirror) {
            Ok(s) => self.ndi = Some(s),
            Err(e) => self.ndi_error = Some(format!("{e:#}")),
        }
    }

    /// VTS item drag & drop: PNG/JPG/GIF files, PNG-frame folders and Live2D model folders
    /// dropped on the window spawn at the centre of the scene.
    pub fn on_dropped_file(&mut self, p: PathBuf) {
        if !self.settings.item_drag_drop {
            return;
        }
        let ok_img = ["png", "jpg", "jpeg", "gif"].iter().any(|e| p.extension().map(|x| x.eq_ignore_ascii_case(e)).unwrap_or(false));
        let target = if p.is_dir() {
            match library::model_in_dir(&p) {
                Some(m) => Some(m.model3),
                None if library::png_frames_dir(&p) => Some(p.clone()),
                None => None,
            }
        } else if ok_img || p.to_string_lossy().ends_with(".model3.json") {
            Some(p.clone())
        } else {
            None
        };
        match target {
            Some(t) => {
                if self.items.len() >= 120 {
                    self.toast("Достигнут лимит предметов в сцене");
                } else if let Some(id) = self.add_item(&t) {
                    self.ui.selected = Some(id);
                }
            }
            None => self.toast(format!("Этот файл нельзя добавить как предмет: {}", p.display())),
        }
    }

    /// ArtMesh settings of the current model: user edits, else the model's .vtube.json.
    pub fn artmesh_details(&self) -> Option<crate::artmesh::ArtMeshDetails> {
        let key = self.model_key()?;
        self.settings.model_artmesh.get(&key).cloned().or_else(|| self.avatar.as_ref().and_then(|a| a.vts_artmesh.clone()))
    }

    /// Re-applies ArtMesh colors / lighting multipliers to the current model.
    pub fn apply_artmesh(&mut self) {
        let d = self.artmesh_details().unwrap_or_default();
        let preset = self.hk.color_preset.clone();
        if let Some(a) = self.avatar.as_mut() {
            a.apply_artmesh(&d);
            if let Some((_, entries)) = preset {
                for e in entries {
                    if let (Some(i), Some((m, s))) = (a.drawable_index(&e.id), crate::artmesh::parse_colors(&e.value)) {
                        a.art_tint[i] = m;
                        a.art_screen[i] = s;
                    }
                }
            }
        }
    }

    /// Changes the effect values, fading over `secs` (0 = instantly).
    pub fn vfx_fade_to(&mut self, to: crate::vfx::Values, secs: f32) {
        if secs <= 0.0 {
            self.settings.vfx = to;
            self.vfx_fade = None;
        } else {
            self.vfx_fade = Some((self.settings.vfx.clone(), to, 0.0, secs));
        }
    }

    fn update_vfx_fade(&mut self, dt: f32) {
        if let Some((from, to, t, dur)) = self.vfx_fade.as_mut() {
            *t += dt;
            let k = (*t / *dur).min(1.0);
            self.settings.vfx = if k >= 1.0 { to.clone() } else { crate::vfx::blend(from, to, k) };
            if k >= 1.0 {
                self.vfx_fade = None;
            }
        }
    }

    /// Plays a sound (starts the engine on first use).
    pub fn play_sound(&mut self, p: crate::audio::Play) {
        if self.audio.is_none() {
            self.audio = Some(crate::audio::Audio::start(&self.settings.sound));
        }
        if let Some(a) = &self.audio {
            a.play(p);
        }
    }

    /// Model volume × spatial volume, and spatial pan, for the current model.
    pub fn model_sound_gain(&self) -> (f32, f32) {
        let Some(key) = self.model_key() else { return (1.0, 0.0) };
        let ms = self.settings.model_sound.get(&key).cloned().unwrap_or_default();
        let (size, x) = crate::api::model_size_x(self).unwrap_or((0.0, 0.0));
        let (v, pan) = ms.spatial(size, x);
        (ms.gain() * v, pan)
    }

    /// Parameter sound triggers of the model and Live2D items (VTS parameter sound triggers).
    fn update_sfx(&mut self, dt: f32) {
        let data = self.paths.data.join("Sounds");
        let mut plays = Vec::new();
        // (key, model3, dir, ids, values, mins, maxs, gain, pan)
        let mut models = Vec::new();
        if let Some(a) = &self.avatar {
            let key = a.model3.to_string_lossy().into_owned();
            let (g, pan) = self.model_sound_gain();
            models.push((key, a.dir.clone(), a.model.param_ids.clone(), a.model.params().to_vec(), a.model.param_min.clone(), a.model.param_max.clone(), g, pan));
        }
        for it in &self.items {
            if let crate::scene::Body::Live2D(l) = &it.body {
                let key = l.model3.to_string_lossy().into_owned();
                let g = self.settings.model_sound.get(&key).map(|m| m.gain()).unwrap_or(0.9) * it.alpha;
                let (vw, _) = self.viewport();
                models.push((key, l.dir.clone(), l.model.param_ids.clone(), l.model.params().to_vec(), l.model.param_min.clone(), l.model.param_max.clone(), g, (it.xf.pos[0] / (vw / 2.0)).clamp(-1.0, 1.0) * 0.5));
            }
        }
        for (key, dir, ids, vals, mins, maxs, gain, pan) in models {
            let Some(ms) = self.settings.model_sound.get(&key) else { continue };
            if ms.triggers.is_empty() {
                continue;
            }
            let fired = crate::sfx::evaluate(&mut self.sfx_rt, &key, ms, &ids, &vals, &mins, &maxs, dt);
            for (ti, speed) in fired {
                let t = &ms.triggers[ti];
                let lerp = |r: [f32; 2]| r[0] + (r[1] - r[0]) * speed;
                let v = if t.speed_to_volume { lerp(t.speed_to_volume_range) } else { 1.0 };
                let p = if t.speed_to_pitch { lerp(t.speed_to_pitch_range) } else { 1.0 };
                let dirs = vec![dir.clone()];
                if let Some(req) = crate::audio::request(&t.sound, |n| crate::hotkeys::resolve_sound(&dirs, &data, n), &key, gain * v, p, pan) {
                    plays.push(req);
                }
            }
        }
        for p in plays {
            self.play_sound(p);
        }
        if let Some(a) = &self.audio {
            a.set_cfg(&self.settings.sound);
        }
    }

    /// Item load / delete sounds of a Live2D item (VTS "Item load sound" / "Item delete sound").
    pub fn item_sound(&mut self, model3: &Path, load: bool) {
        let key = model3.to_string_lossy().into_owned();
        let Some(ms) = self.settings.model_sound.get(&key).cloned() else { return };
        let dirs = vec![model3.parent().map(|p| p.to_path_buf()).unwrap_or_default()];
        let data = self.paths.data.join("Sounds");
        for t in ms.triggers.iter().filter(|t| t.active && if load { t.item_load_sound } else { t.item_delete_sound }) {
            if let Some(req) = crate::audio::request(&t.sound, |n| crate::hotkeys::resolve_sound(&dirs, &data, n), &key, ms.gain(), 1.0, 0.0) {
                self.play_sound(req);
            }
        }
    }

    pub fn set_mic(&mut self, on: bool) {
        self.settings.mic.on = on;
        self.mic = None;
        if on {
            self.mic = Some(crate::lipsync::Mic::start(&self.settings.mic));
        }
    }

    /// Voice parameters from the microphone (VTS VoiceA/I/U/E/O, VoiceVolume, …).
    fn update_voice(&mut self) {
        use crate::mapping::input::*;
        let mut finished = None;
        let v = match &self.mic {
            Some(m) => {
                m.set_cfg(&self.settings.mic);
                if let Some((key, _, Some(profile))) = m.calib_state() {
                    finished = Some((key, profile));
                }
                m.voice()
            }
            None => crate::lipsync::Voice { silence: 1.0, ..Default::default() },
        };
        if let Some((key, profile)) = finished {
            self.settings.mic.profiles.insert(key.clone(), profile);
            self.toast(format!("Калибровка «{key}» завершена"));
            self.save();
        }
        self.mic_level = v.volume;
        let i = &mut self.inputs;
        i[VOICE_VOLUME] = v.volume;
        i[VOICE_FREQUENCY] = v.frequency;
        i[VOICE_A] = v.vowels[0];
        i[VOICE_I] = v.vowels[1];
        i[VOICE_U] = v.vowels[2];
        i[VOICE_E] = v.vowels[3];
        i[VOICE_O] = v.vowels[4];
        i[VOICE_SILENCE] = v.silence;
        i[VOICE_VOLUME_PLUS_MOUTH_OPEN] = (i[MOUTH_OPEN] + v.volume).min(1.0);
        i[VOICE_FREQUENCY_PLUS_MOUTH_SMILE] = (i[MOUTH_SMILE] + v.frequency).min(1.0);
    }

    pub fn set_editor(&mut self, on: bool) {
        self.settings.editor_on = on;
        self.editor = None;
        if on {
            let st = &self.settings;
            let host = if st.editor_remote { st.editor_ip.trim().to_string() } else { "127.0.0.1".into() };
            self.editor = Some(crate::cubism_editor::Editor::start(host, st.editor_port, st.editor_token.clone()));
        }
        self.push_editor_event();
    }

    /// Live2DCubismEditorConnectedEvent payload.
    pub fn editor_state(&self) -> serde_json::Value {
        serde_json::json!({
            "tryingToConnect": self.settings.editor_on,
            "connected": self.editor.as_ref().map(|e| e.connected && e.approved).unwrap_or(false),
            "shouldSendParameters": self.settings.editor_send,
        })
    }

    pub fn push_editor_event(&mut self) {
        let st = self.editor_state();
        self.api_events.push(("Live2DCubismEditorConnectedEvent".into(), st));
    }

    fn update_editor(&mut self) {
        use crate::cubism_editor::Msg;
        let Some(ed) = self.editor.as_mut() else { return };
        let msgs = ed.drain();
        let sending = ed.approved && self.settings.editor_send && (!self.settings.editor_physics_only || ed.edit_mode == "Physics");
        if sending {
            if let Some(a) = &self.avatar {
                let vals = a.model.params();
                ed.send_params(a.model.param_ids.iter().cloned().zip(vals.iter().copied()).collect());
            }
        }
        for m in msgs {
            match m {
                Msg::Token(t) => {
                    self.settings.editor_token = t;
                    self.save();
                }
                Msg::State { .. } => self.push_editor_event(),
                Msg::Exported(p) => {
                    if self.settings.editor_auto_import {
                        match crate::cubism_editor::import_export(Path::new(&p), &self.paths.data.join("Live2DModels")) {
                            Ok(d) => {
                                self.rescan();
                                self.toast(format!("Модель из Cubism Editor скопирована: {}", d.display()));
                            }
                            Err(e) => self.toast(format!("Не удалось скопировать модель из Cubism Editor: {e:#}")),
                        }
                    } else {
                        self.toast(format!("Cubism Editor: экспортирована модель {p}"));
                    }
                }
                _ => {}
            }
        }
    }

    pub fn set_api(&mut self, on: bool) {
        self.settings.api_on = on;
        self.api = None;
        self.api_error = None;
        if on {
            match crate::api::ApiServer::start(self.settings.api_port, self.settings.api_lan) {
                Ok(s) => self.api = Some(s),
                Err(e) => {
                    self.api_error = Some(format!("{e:#}"));
                    self.toast(format!("API: {e:#}"));
                }
            }
        }
    }

    pub fn set_twitch(&mut self, on: bool) {
        self.settings.twitch.enabled = on;
        self.twitch = None;
        self.fx.clear();
        if on {
            let t = &self.settings.twitch;
            if t.channel.trim().is_empty() && t.login.is_empty() {
                self.toast("Twitch: укажите имя канала");
                self.settings.twitch.enabled = false;
                return;
            }
            self.twitch = Some(crate::twitch::Twitch::start(t));
        }
    }

    pub fn twitch_login(&mut self) {
        let cid = self.settings.twitch.client_id.trim().to_string();
        if cid.is_empty() {
            self.toast("Twitch: укажите Client ID своего приложения (dev.twitch.tv/console)");
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        crate::twitch::login(cid, tx);
        self.twitch_login = Some(rx);
        self.twitch_auth = Some("Запрос кода у Twitch…".into());
    }

    fn update_twitch(&mut self) {
        use crate::twitch::Msg;
        // Dev hook: RIGRS_FX_TEST=chat|drop keeps spawning test content.
        if let Ok(k) = std::env::var("RIGRS_FX_TEST") {
            if self.time > 2.0 && (self.time / 1.5).floor() != ((self.time - self.ui_dt) / 1.5).floor() {
                crate::twitch_fx::test(self, k == "drop");
            }
        }
        let mut msgs = Vec::new();
        if let Some(rx) = &self.twitch_login {
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
        }
        if let Some(t) = self.twitch.as_mut() {
            msgs.extend(t.drain());
        }
        for m in msgs {
            match m {
                Msg::Event(ev) => {
                    if !matches!(ev, crate::hotkeys::TwitchEvent::Chat { .. }) {
                        if let Some(t) = self.twitch.as_mut() {
                            t.recent.push_front(ev.describe());
                            t.recent.truncate(20);
                        }
                    }
                    crate::hotkeys::on_twitch(self, &ev);
                }
                Msg::Chat(c) => {
                    if self.settings.twitch.dropper.enabled || self.settings.twitch.chat.enabled {
                        self.fx.on_chat(c);
                    }
                }
                Msg::DeviceCode { user_code, uri } => {
                    self.twitch_auth = Some(format!("Откройте {uri} и введите код: {user_code}"));
                    self.actions.push(Action::OpenUrl(uri));
                }
                Msg::Authorized { access, refresh, login, user_id } => {
                    let t = &mut self.settings.twitch;
                    t.access_token = access;
                    t.refresh_token = refresh;
                    t.login = login.clone();
                    t.user_id = user_id;
                    if t.channel.trim().is_empty() {
                        t.channel = login.clone();
                    }
                    self.twitch_auth = None;
                    self.twitch_login = None;
                    self.toast(format!("Twitch: вход выполнен как {login}"));
                    self.set_twitch(true);
                    self.save();
                }
                Msg::AuthFailed(e) => {
                    self.twitch_auth = None;
                    self.twitch_login = None;
                    self.toast(e);
                }
                _ => {}
            }
        }
    }

    pub fn save(&mut self) {
        self.remember_avatar_xf();
        // ModelConfigChangedEvent when the user changed the loaded model's config.
        if let Some(key) = self.model_key() {
            use std::hash::{Hash, Hasher};
            let h = |v: String| {
                let mut s = std::collections::hash_map::DefaultHasher::new();
                v.hash(&mut s);
                s.finish()
            };
            let hk = h(serde_json::to_string(&self.settings.model_hotkeys.get(&key)).unwrap_or_default());
            let other = h(format!(
                "{}{}{}{}",
                serde_json::to_string(&self.settings.model_cfg.get(&key)).unwrap_or_default(),
                serde_json::to_string(&self.settings.rules_override.get(&key)).unwrap_or_default(),
                serde_json::to_string(&self.settings.model_artmesh.get(&key)).unwrap_or_default(),
                self.settings.idle_override.get(&key).cloned().unwrap_or_default()
            ));
            let (ohk, oother) = self.cfg_snapshot;
            if (ohk != 0 || oother != 0) && (hk != ohk || other != oother) {
                let mut ev = crate::api::model_config_event(self);
                ev["hotkeyConfigChanged"] = serde_json::json!(hk != ohk);
                self.api_events.push(("ModelConfigChangedEvent".into(), ev));
            }
            self.cfg_snapshot = (hk, other);
        }
        if self.settings.save_scene {
            self.settings.last_items = self.saved_items();
        }
        self.settings.save(&self.paths.settings);
    }

    /// Our own Cubism Core (downloaded or next to the exe), if there is one.
    pub fn own_core(&self) -> Option<PathBuf> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
        [crate::core_dl::installed_path(&self.paths.data), exe_dir.join(crate::core_dl::file_name())].into_iter().find(|p| p.is_file())
    }

    pub fn shutdown(&mut self) {
        self.save();
        if let Some(assets) = self.detach_on_exit.take() {
            let core = self.own_core();
            if let Err(e) = crate::vts_import::detach_settings(&self.paths.settings, &assets, &self.paths.data, core.as_deref()) {
                eprintln!("detach from VTube Studio failed: {e:#}");
            }
        }
        self.tracker = None;
        self.spout = None;
        self.vcam = None;
    }

    // ------------------------------------------------------------------ frame

    pub fn frame(&mut self) {
        let t0 = Instant::now();
        let raw_dt = t0.duration_since(self.last).as_secs_f32();
        if raw_dt > 0.04 && std::env::var_os("RIGRS_HITCH").is_some() {
            eprintln!("HITCH {:.0} ms at {:.1} s", raw_dt * 1000.0, self.time);
        }
        let dt = raw_dt.min(0.1);
        self.last = t0;
        self.time += dt;
        if let crate::updater::State::Ready = self.updater.state() {
            if !self.restart_after_quit {
                self.restart_after_quit = true;
                self.quit = true;
            }
        }
        // Dev hook for the end-to-end update test: install as soon as an update is found.
        if std::env::var_os("RIGRS_AUTO_UPDATE").is_some() {
            if let crate::updater::State::Available(r) = self.updater.state() {
                self.updater.install(r);
            }
        }
        if !self.update_toasted {
            if let crate::updater::State::Available(r) = self.updater.state() {
                self.update_toasted = true;
                self.toast(format!("{} {}", crate::i18n::tr("Доступна новая версия"), r.tag));
            }
        }
        self.poll_model_job();
        self.poll_bg_job();
        if std::env::var_os("RIGRS_NET").is_some() {
            self.dev_net_test();
        }
        self.update_tracking();
        self.update_voice();
        let pad_ui = self.ui.settings_open && self.ui.tab == crate::ui::Tab::General;
        self.pads.poll(self.settings.controller_mode, &self.settings.controller_id, pad_ui);
        self.pads.write_inputs(&mut self.inputs);
        crate::api::apply_injections(self);
        self.update_mouse();
        self.update_tobii();
        self.update_lighting(dt);
        if self.demo.is_some() && !crate::demo::step(self, dt) {
            self.quit = true;
        }
        crate::api::pump(self, dt);
        self.update_twitch();
        self.update_editor();
        self.update_vfx_fade(dt);
        self.update_sfx(dt);
        if let Some(a) = self.avatar.as_mut() {
            a.glitch = if self.settings.vfx_on { crate::vfx::model_glitch(&self.settings.vfx) } else { [0.0; 3] };
        }
        let (vw, vh) = self.viewport();
        self.ui_dt = dt;

        let raw = self.egui_state.take_egui_input(&self.window);
        let ctx = self.egui_ctx.clone();
        let out = ctx.run(raw, |c| crate::ui::run(self, c));
        self.egui_state.handle_platform_output(&self.window, out.platform_output);
        let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
        let has_outputs = self.spout.is_some() || self.ndi.is_some() || self.vcam.is_some();
        let fx_prims = if has_outputs && !self.fx.shapes.is_empty() {
            let shapes = std::mem::take(&mut self.fx.shapes);
            ctx.tessellate(shapes.into_iter().map(|shape| egui::epaint::ClippedShape { clip_rect: egui::Rect::EVERYTHING, shape }).collect(), 1.0)
        } else {
            Vec::new()
        };
        let fx_screen = self.fx.screen;

        crate::net::pump(self, dt);
        crate::ui_net::pump_extras(self, dt);
        if self.tracker_dl.take_done() {
            self.toast("Встроенный трекер установлен");
            self.actions.push(Action::StartTracker);
        }
        if let Some(p) = self.core_dl.take_done() {
            self.settings.core_dll = p.to_string_lossy().into_owned();
            if self.core.is_some() {
                self.toast("Cubism Core скачан: своя копия будет использоваться со следующего запуска");
            }
            self.actions.push(Action::ReloadCore);
        }
        if let Some((copied, kept)) = self.vts_import.take_done() {
            // The scene still refers to the VTS files: finish through a restart, with the
            // paths rewritten after the final save (see `shutdown`).
            if let Some(a) = self.settings.vts_assets() {
                eprintln!("library moved: {copied} files copied, {kept} already there");
                self.detach_on_exit = Some(a);
                self.restart_after_quit = true;
                self.quit = true;
            }
        }

        // Simulation
        let physics = self.settings.physics;
        if let Some(a) = &mut self.avatar {
            a.update(dt, &self.inputs, self.tracking, physics);
            if let Some(r) = &mut self.recorder {
                r.push(a.model.params());
            }
        }
        if self.recorder.as_ref().map(|r| r.done()).unwrap_or(false) || (self.recorder.is_some() && self.avatar.is_none()) {
            self.stop_recording();
        }
        for it in &mut self.items {
            if let Some(a) = &self.avatar {
                it.follow(a, dt);
            }
            it.update(dt, &self.inputs, &NEUTRAL, self.tracking, physics);
        }
        self.items.retain(|i| !(i.alpha_target <= 0.0 && i.alpha <= 0.001));
        if self.api.is_some() {
            let mut evs = Vec::new();
            if let Some(a) = &self.avatar {
                for e in &a.anim_events {
                    evs.push((e.clone(), a.model_id.clone(), a.name.clone(), false));
                }
            }
            for it in &self.items {
                if let Body::Live2D(l) = &it.body {
                    for e in &l.anim_events {
                        evs.push((e.clone(), l.model_id.clone(), l.name.clone(), true));
                    }
                }
            }
            for ((k, t, d, name, len, idle), mid, mname, item) in evs {
                self.api_events.push((
                    "ModelAnimationEvent".into(),
                    serde_json::json!({
                        "animationEventType": k, "animationEventTime": t, "animationEventData": d, "animationName": name,
                        "animationLength": len, "isIdleAnimation": idle, "modelID": mid, "modelName": mname, "isLive2DItem": item,
                    }),
                ));
            }
        }
        if let Some((_, s)) = &mut self.bg_sprite {
            s.update(dt);
        }

        // Upload
        let render = vw >= 1.0 && vh >= 1.0;
        let mut masks = self.settings.smooth_masks.then(|| MaskAlloc::new(self.gpu.atlas_size, self.settings.mask_max));
        if render {
            for it in &mut self.items {
                it.upload(&self.gpu, vw, vh, masks.as_mut());
            }
            if let Some(n) = &mut self.net {
                for it in n.scene_items_mut() {
                    it.upload(&self.gpu, vw, vh, masks.as_mut());
                }
            }
            if let Some(a) = &mut self.avatar {
                a.upload(&self.gpu, vw, vh, masks.as_mut());
            }
            self.stats.masks = masks.as_ref().map(|m| m.sets.len()).unwrap_or(0);
            if let Some((_, s)) = &self.bg_sprite {
                let k = (vw / s.size[0]).max(vh / s.size[1]);
                s.set_affine(&self.gpu, &Xform { pos: [0.0, 0.0], scale: k, rot: 0.0, flip: false }.affine(vw, vh, false));
            }
        }

        // Active post-processing effects (VTS order). BackgroundShift runs in the compose pass,
        // ModelGlitch deforms the model vertices.
        let vfx_vals = if self.settings.vfx_on { self.settings.vfx.clone() } else { Default::default() };
        let active = crate::vfx::active_effects(&vfx_vals);
        let passes: Vec<usize> = active.iter().copied().filter(|&i| !matches!(crate::vfx::DEFS[i].id, "BackgroundShift" | "ModelGlitch")).collect();
        let use_vfx = render && active.iter().any(|&i| crate::vfx::DEFS[i].id != "ModelGlitch");
        if use_vfx {
            let dirs: Vec<PathBuf> = self.settings.vts_assets().into_iter().map(|a| a.join("Items")).chain(std::iter::once(self.paths.data.join("Items"))).collect();
            let vfx = self.vfx.get_or_insert_with(|| crate::vfx::Renderer::new(&self.gpu));
            for &e in &passes {
                for t in crate::vfx::textures(&vfx_vals, &crate::vfx::DEFS[e]) {
                    vfx.load_image(&self.gpu, &dirs, &t);
                }
            }
            vfx.ensure(&self.gpu, 0, self.gpu.config.width, self.gpu.config.height);
            for (slot, out) in [(1usize, &self.spout), (2, &self.ndi), (3, &self.vcam)] {
                if let Some(o) = out {
                    let (w, h) = o.size();
                    vfx.ensure(&self.gpu, slot, w, h);
                }
            }
        }
        let vfx_inputs = {
            let (wx, wy) = self.window.inner_position().map(|p| (p.x as f32, p.y as f32)).unwrap_or((0.0, 0.0));
            let ws = self.window.inner_size();
            let mu = [((self.mouse[0] - wx) / ws.width.max(1) as f32).clamp(0.0, 1.0), ((self.mouse[1] - wy) / ws.height.max(1) as f32).clamp(0.0, 1.0)];
            crate::vfx::FrameInputs {
                time: self.time,
                dt,
                mic: self.mic_level,
                head: [self.inputs[crate::mapping::input::FACE_ANGLE_X] / 30.0, self.inputs[crate::mapping::input::FACE_ANGLE_Y] / 30.0],
                mouse: [mu[0], mu[1], if crate::hotkeys::vk_down(0x01) && self.window.has_focus() { 1.0 } else { 0.0 }],
            }
        };
        let clear_col = self.clear_color();
        let mut vfx = if use_vfx { self.vfx.as_mut() } else { None };
        let gpu = &self.gpu;
        let frame = if render && !self.occluded {
            match gpu.surface.get_current_texture() {
                Ok(f) => Some(f),
                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    None
                }
                Err(e) => {
                    eprintln!("surface: {e}");
                    None
                }
            }
        } else {
            None
        };

        let mut enc = gpu.device.create_command_encoder(&Default::default());
        if let Some(al) = masks.as_ref().filter(|m| !m.sets.is_empty() && render) {
            gpu.write_mask_sets(&al.sets);
            let mut mp = gpu.mask_painter(&mut enc);
            for it in &self.items {
                it.draw_masks(&mut mp, al);
            }
            for it in self.net.as_ref().map(|n| n.scene_items()).unwrap_or_default() {
                it.draw_masks(&mut mp, al);
            }
            if let Some(a) = &self.avatar {
                a.draw_masks(&mut mp, al);
            }
        }
        let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [gpu.config.width, gpu.config.height], pixels_per_point: out.pixels_per_point };
        for (id, delta) in &out.textures_delta.set {
            self.egui_rend.update_texture(&gpu.device, &gpu.queue, *id, delta);
            for r in &mut self.fx_rend {
                r.update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        let mut draw_calls = 0;
        let bg = self.bg_sprite.as_ref().map(|(_, s)| s);
        let net_items: Vec<&Item> = self.net.as_ref().map(|n| n.scene_items()).unwrap_or_default();
        if let Some(f) = &frame {
            let view = f.texture.create_view(&Default::default());
            match vfx.as_deref_mut() {
                Some(v) => {
                    // Model layer (transparent) and background layer, combined by the effects.
                    {
                        let mut p = Painter::new(gpu, &mut enc, v.scene_view(0).unwrap(), &gpu.depth, wgpu::Color::TRANSPARENT);
                        draw_scene(&mut p, None, &self.items, &net_items, self.avatar.as_ref());
                        draw_calls = p.draw_calls;
                    }
                    {
                        let mut p = Painter::new(gpu, &mut enc, v.bg_view(0).unwrap(), &gpu.depth, clear_col);
                        draw_scene(&mut p, bg, &[], &[], None);
                    }
                    v.run(gpu, &mut enc, 0, &view, &vfx_vals, &passes, vfx_inputs);
                }
                None => {
                    let mut p = Painter::new(gpu, &mut enc, &view, &gpu.depth, clear_col);
                    draw_scene(&mut p, bg, &self.items, &net_items, self.avatar.as_ref());
                    draw_calls = p.draw_calls;
                }
            }
            self.egui_rend.update_buffers(&gpu.device, &gpu.queue, &mut enc, &jobs, &sd);
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.egui_rend.render(&mut pass.forget_lifetime(), &jobs, &sd);
        }

        // Video outputs (Spout2 / NDI): avatar + items on a transparent background, no UI.
        if render {
            let clear = clear_col;
            for (slot, out) in [(1usize, &mut self.spout), (2, &mut self.ndi), (3, &mut self.vcam)] {
                let Some(out) = out else { continue };
                let (bgx, cc) = if out.with_bg { (bg, clear) } else { (None, wgpu::Color::TRANSPARENT) };
                match vfx.as_deref_mut() {
                    Some(v) if v.scene_view(slot).is_some() => {
                        {
                            let mut p = Painter::new(gpu, &mut enc, v.scene_view(slot).unwrap(), &out.offscreen.depth, wgpu::Color::TRANSPARENT);
                            draw_scene(&mut p, None, &self.items, &net_items, self.avatar.as_ref());
                        }
                        {
                            let mut p = Painter::new(gpu, &mut enc, v.bg_view(slot).unwrap(), &out.offscreen.depth, cc);
                            draw_scene(&mut p, bgx, &[], &[], None);
                        }
                        v.run(gpu, &mut enc, slot, &out.offscreen.scene_view, &vfx_vals, &passes, vfx_inputs);
                    }
                    _ => {
                        let mut p = Painter::new(gpu, &mut enc, &out.offscreen.scene_view, &out.offscreen.depth, cc);
                        draw_scene(&mut p, bgx, &self.items, &net_items, self.avatar.as_ref());
                    }
                }
                // Twitch Emote Dropper / flying chat on top of the scene, mapped like the model
                // (window points → output pixels, same stretch as the scene itself).
                if !fx_prims.is_empty() && fx_screen.x > 0.0 && fx_screen.y > 0.0 {
                    let (w, h) = out.size();
                    let (sx, sy) = (w as f32 / fx_screen.x, h as f32 / fx_screen.y);
                    let prims: Vec<egui::ClippedPrimitive> = fx_prims
                        .iter()
                        .map(|p| {
                            let mut p = p.clone();
                            let r = p.clip_rect;
                            p.clip_rect = if r == egui::Rect::EVERYTHING {
                                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))
                            } else {
                                egui::Rect::from_min_max(egui::pos2(r.min.x * sx, r.min.y * sy), egui::pos2(r.max.x * sx, r.max.y * sy))
                            };
                            if let egui::epaint::Primitive::Mesh(m) = &mut p.primitive {
                                for v in &mut m.vertices {
                                    v.pos.x *= sx;
                                    v.pos.y *= sy;
                                }
                            }
                            p
                        })
                        .collect();
                    let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: 1.0 };
                    let rend = &mut self.fx_rend[slot - 1];
                    rend.update_buffers(&gpu.device, &gpu.queue, &mut enc, &prims, &sd);
                    let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("twitch-fx-out"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &out.offscreen.scene_view,
                            resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    rend.render(&mut pass.forget_lifetime(), &prims, &sd);
                }
                gpu.unpremultiply(&mut enc, &out.offscreen);
                out.record_copy(&mut enc);
            }
        }

        let shot = match self.ui.screenshot.take() {
            Some(transparent) if render => {
                let off = gpu.offscreen(gpu.config.width, gpu.config.height);
                {
                    let clear = if transparent { wgpu::Color::TRANSPARENT } else { self.clear_color() };
                    let mut p = Painter::new(gpu, &mut enc, &off.scene_view, &off.depth, clear);
                    draw_scene(&mut p, if transparent { None } else { bg }, &self.items, &net_items, self.avatar.as_ref());
                }
                off.set_mirror(&gpu.queue, false);
                gpu.unpremultiply(&mut enc, &off);
                let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("screenshot"),
                    size: padded_row(off.w) as u64 * off.h as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                copy_to_buffer(&mut enc, &off.out, &buf, off.w, off.h);
                Some((buf, off.w, off.h))
            }
            _ => None,
        };

        gpu.queue.submit(Some(enc.finish()));
        if let Some(f) = frame {
            self.window.pre_present_notify();
            f.present();
        }
        for out in [&mut self.spout, &mut self.ndi, &mut self.vcam].into_iter().flatten() {
            out.after_submit();
        }
        gpu.device.poll(wgpu::Maintain::Poll);
        for out in [&mut self.spout, &mut self.ndi, &mut self.vcam].into_iter().flatten() {
            out.pump();
        }
        for id in &out.textures_delta.free {
            self.egui_rend.free_texture(id);
            for r in &mut self.fx_rend {
                r.free_texture(id);
            }
        }
        if let Some((buf, w, h)) = shot {
            self.finish_screenshot(buf, w, h);
        }

        let actions = std::mem::take(&mut self.actions);
        for a in actions {
            self.apply(a);
        }
        self.toasts.retain(|(_, t)| t.elapsed() < Duration::from_secs(5));
        self.stats.draw_calls = draw_calls;
        #[cfg(windows)]
        let th = self.tracker.as_ref().and_then(|t| t.process_handle());
        #[cfg(not(windows))]
        let th = None;
        self.stats.frame(t0.elapsed().as_secs_f32() * 1000.0, th);
    }

    fn finish_screenshot(&mut self, buf: wgpu::Buffer, w: u32, h: u32) {
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.gpu.device.poll(wgpu::Maintain::Wait);
        let row = padded_row(w) as usize;
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        {
            let data = slice.get_mapped_range();
            for y in 0..h as usize {
                px.extend_from_slice(&data[y * row..y * row + w as usize * 4]);
            }
        }
        buf.unmap();
        if self.gpu.is_bgra() {
            for p in px.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
        }
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let path = self.paths.screenshots.join(format!("screenshot_{secs}.png"));
        let shown = path.display().to_string();
        std::thread::spawn(move || {
            if let Some(img) = image::RgbaImage::from_raw(w, h, px) {
                if let Err(e) = img.save(&path) {
                    eprintln!("screenshot: {e}");
                }
            }
        });
        self.toast(format!("Скриншот: {shown}"));
    }

    // ------------------------------------------------------------------ actions

    fn apply(&mut self, a: Action) {
        match a {
            Action::LoadModel(e) => {
                crate::hotkeys::before_model_change(self);
                self.load_model(&e);
            }
            Action::UnloadModel => {
                if let Some(a) = &self.avatar {
                    self.api_events.push(("ModelLoadedEvent".into(), serde_json::json!({ "modelLoaded": false, "modelName": a.name, "modelID": a.model_id })));
                }
                self.remember_avatar_xf();
                self.avatar = None;
                self.settings.last_model = None;
                for it in &mut self.items {
                    it.pin = None;
                }
            }
            Action::AddModelFolder(p) => {
                self.settings.extra_model_dirs.push(p.to_string_lossy().into_owned());
                self.rescan();
                if let Some(m) = library::model_in_dir(&p) {
                    self.load_model(&m);
                }
                self.save();
            }
            Action::AddItem(e) => {
                if let Some(id) = self.add_item(e.path()) {
                    self.ui.selected = Some(id);
                }
            }
            Action::SpawnItem(s) => {
                if let Some(id) = self.add_item(&s.path) {
                    let order = crate::ui_items::free_order(self, s.order, Some(id));
                    let details = self.artmesh_details();
                    let item_details = self.settings.model_artmesh.get(&s.path.to_string_lossy().to_string()).cloned();
                    if let Some(it) = self.items.iter_mut().find(|i| i.id == id) {
                        it.cfg.order = order;
                        it.xf.flip = s.flip;
                        it.cfg.fps = s.fps;
                        if let (Body::Live2D(l), Some(d)) = (&mut it.body, &item_details) {
                            l.apply_artmesh(d);
                        }
                        // Spawned over the model → pinned right away.
                        if s.pin {
                            if let Some(a) = &self.avatar {
                                it.pin_to_filtered(a, details.as_ref().or(a.vts_artmesh.as_ref()));
                            }
                        } else {
                            it.cfg.pin_only_layer = "__never".into();
                        }
                    }
                    if s.fps > 0.0 {
                        self.settings.item_fps.insert(s.path.to_string_lossy().into_owned(), s.fps);
                    }
                    self.ui.selected = Some(id);
                    if s.finetune {
                        self.ui.item_menu = Some(id);
                    }
                    self.save();
                }
            }
            Action::ModelToItem => {
                if let Some(a) = &self.avatar {
                    let dst = self.paths.data.join("Items").join(&a.name);
                    match crate::cubism_editor::import_export(&a.model3, &self.paths.data.join("Items")) {
                        Ok(d) => {
                            let _ = dst;
                            self.rescan();
                            self.toast(format!("Модель скопирована в предметы: {}", d.display()));
                        }
                        Err(e) => self.toast(format!("Не удалось создать предмет: {e:#}")),
                    }
                } else {
                    self.toast("Сначала загрузите модель");
                }
            }
            Action::AddItemFile(p) => {
                if let Some(id) = self.add_item(&p) {
                    self.ui.selected = Some(id);
                }
            }
            Action::RemoveItem(id) => {
                if let Some(src) = self.items.iter().find(|i| i.id == id).map(|i| i.source.clone()) {
                    if src.to_string_lossy().ends_with(".model3.json") {
                        self.item_sound(&src, false);
                    }
                }
                self.items.retain(|i| i.id != id);
            }
            Action::ClearItems => self.items.clear(),
            Action::SetBackground(b) => {
                let name = match &b {
                    Background::Image(p) => std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    other => format!("{other:?}"),
                };
                self.api_events.push(("BackgroundChangedEvent".into(), serde_json::json!({ "backgroundName": name })));
                self.set_background(b);
                self.save();
            }
            Action::Screenshot { transparent } => self.ui.screenshot = Some(transparent),
            Action::SaveScene(name) => {
                let items = self.saved_items().into_iter().map(|mut s| {
                    s.scene = None;
                    s
                }).collect();
                let group = self.settings.item_scenes.iter().find(|s| s.name == name).map(|s| s.group.clone()).unwrap_or_default();
                self.settings.item_scenes.retain(|s| s.name != name);
                self.settings.item_scenes.push(ItemScene { name: name.clone(), items, group });
                self.toast(format!("Сцена «{name}» сохранена"));
                self.save();
            }
            Action::LoadScene(name) => {
                if !self.scene_loaded(&name) {
                    self.toggle_scene(&name, 0.0);
                }
            }
            Action::ToggleScene(name) => {
                let name = name.trim_end_matches(".json").to_string();
                self.toggle_scene(&name, 0.0);
            }
            Action::SetApi(on) => {
                self.set_api(on);
                self.save();
            }
            Action::SetTwitch(on) => {
                self.set_twitch(on);
                self.save();
            }
            Action::DeleteScene(name) => {
                self.settings.item_scenes.retain(|s| s.name != name);
                self.save();
            }
            Action::Calibrate => match self.tracker.as_ref().and_then(|t| t.latest()).filter(|f| f.success) {
                Some(f) => {
                    self.settings.calib = Calibration::from_frame(&f);
                    if self.tracker.as_ref().map(|t| t.nvidia_calibrate()).unwrap_or(false) {
                        self.toast("Калибровка выполнена (NVIDIA: нейтральное лицо сохраняется…)");
                    } else {
                        self.toast("Калибровка выполнена");
                    }
                    self.save();
                }
                None => self.toast("Калибровка: лицо не найдено"),
            },
            Action::StartTracker => self.start_tracker(),
            Action::StopTracker => {
                self.tracker = None;
                self.tracker_error = None;
            }
            Action::RefreshCameras => self.refresh_cameras(),
            Action::Rescan => self.rescan(),
            Action::ReloadCore => {
                if self.core.is_none() {
                    match load_core(&self.settings.core_dll) {
                        Ok(c) => {
                            self.core = Some(c);
                            self.core_error = None;
                            self.toast(format!("Cubism Core {} загружен", c.version_string()));
                        }
                        Err(e) => self.core_error = Some(format!("{e:#}")),
                    }
                }
                self.rescan();
                self.save();
            }
            Action::ToggleExpression(file) => {
                if let Some(a) = &mut self.avatar {
                    if let Some(e) = a.expressions.iter_mut().find(|e| e.file == file || e.name == file) {
                        e.active = !e.active;
                    }
                }
            }
            Action::ResetExpressions => {
                if let Some(a) = &mut self.avatar {
                    for e in &mut a.expressions {
                        e.active = false;
                    }
                }
            }
            Action::SetSpout(on) => {
                self.set_spout(on);
                self.save();
            }
            Action::OpenVcamFolder => {
                let d = self.paths.data.join("vcam");
                if let Err(e) = crate::vcam::prepare_folder(&d) {
                    self.toast(format!("{e:#}"));
                }
                let _ = std::process::Command::new("explorer").arg(&d).spawn();
            }
            Action::SetMaskRes(size, max) => {
                self.settings.mask_atlas = size;
                self.settings.mask_max = max;
                self.gpu.set_atlas_size(size);
                if let Some(a) = &mut self.avatar {
                    a.rebind(&self.gpu);
                }
                for it in &mut self.items {
                    it.rebind(&self.gpu);
                }
                if let Some((_, s)) = &mut self.bg_sprite {
                    s.rebind(&self.gpu);
                }
                self.save();
            }
            Action::SetTobii(on) => {
                self.set_tobii(on);
                self.save();
            }
            Action::CalibrateTobii => {
                self.calibrate_tobii();
                self.save();
            }
            Action::SetLight(mode, monitor) => {
                if mode != 0 {
                    self.settings.light_last_mode = mode;
                }
                self.settings.light_mode = mode;
                self.settings.light_monitor = monitor;
                self.apply_light_source();
                self.save();
            }
            Action::SetLightWindow(title) => {
                self.settings.light_mode = 2;
                self.settings.light_window = title;
                self.apply_light_source();
                self.save();
            }
            Action::ToggleRecording => {
                if self.recorder.is_some() {
                    self.stop_recording();
                } else {
                    self.start_recording();
                }
            }
            Action::SetVcam(on) => {
                self.set_vcam(on);
                self.save();
            }
            Action::SetNdi(on) => {
                self.set_ndi(on);
                self.save();
            }
            Action::OpenUrl(u) => {
                let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &u]).spawn();
            }
            Action::SetIdle(f) => {
                if let Some(a) = &mut self.avatar {
                    a.set_idle(f.as_deref());
                    let key = a.model3.to_string_lossy().into_owned();
                    self.settings.idle_override.insert(key, f.unwrap_or_default());
                    self.save();
                }
            }
            Action::ResetModelPosition => {
                let (vw, vh) = self.viewport();
                if let Some(a) = &mut self.avatar {
                    a.fit(vw, vh, 0.9);
                }
            }
            Action::OpenDataFolder => {
                let _ = std::process::Command::new("explorer").arg(&self.paths.data).spawn();
            }
            Action::NewInstance => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
                }
            }
            Action::ResetSettings => {
                self.settings = Settings::default();
                self.settings.save(&self.paths.settings);
                self.toast("Настройки сброшены");
            }
            Action::Quit => self.quit = true,
        }
    }
}
