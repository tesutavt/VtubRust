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
    RemoveItem(u64),
    ClearItems,
    SetBackground(Background),
    Screenshot { transparent: bool },
    SaveScene(String),
    LoadScene(String),
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

pub struct App {
    pub window: Arc<Window>,
    pub gpu: Gpu,
    pub core: Option<&'static Core>,
    pub core_error: Option<String>,
    pub egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_rend: egui_wgpu::Renderer,

    pub settings: Settings,
    pub paths: Paths,
    pub library: Library,

    pub avatar: Option<Live2D>,
    pub items: Vec<Item>,
    next_id: u64,
    pub bg_sprite: Option<(String, Sprite)>,

    pub tracker: Option<Tracker>,
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
    last: Instant,
    pub quit: bool,
    pub occluded: bool,
}

/// Repairs stale paths (moved / reinstalled VTS): Cubism Core next to rigrs.exe wins,
/// then the auto-detected VTube Studio install.
fn fix_paths(s: &mut Settings) {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    let vts = Some(PathBuf::from(&s.vts_dir)).filter(|p| p.is_dir()).or_else(crate::settings::detect_vts_dir);
    if let Some(v) = &vts {
        s.vts_dir = v.to_string_lossy().into_owned();
    }
    if !Path::new(&s.core_dll).is_file() {
        let local = exe_dir.join("Live2DCubismCore.dll");
        if local.is_file() {
            s.core_dll = local.to_string_lossy().into_owned();
        } else if let Some(v) = &vts {
            s.core_dll = v.join(r"VTube Studio_Data\Plugins\x86_64\Live2DCubismCore.dll").to_string_lossy().into_owned();
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

        let mut settings = settings;
        fix_paths(&mut settings);
        // If the configured Core does not load, try the other known locations.
        if load_core_probe(&settings.core_dll).is_err() {
            let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
            let mut cands = vec![exe_dir.join("Live2DCubismCore.dll")];
            if let Some(v) = crate::settings::detect_vts_dir() {
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
        crate::vfx::normalize(&mut settings.vfx);
        let mut app = App {
            window,
            gpu,
            core,
            core_error,
            egui_ctx,
            egui_state,
            egui_rend,
            library: Library::default(),
            settings,
            paths,
            avatar: None,
            items: Vec::new(),
            next_id: 1,
            bg_sprite: None,
            tracker: None,
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
            app.restore_items(&saved);
        }
        let bg = app.settings.background.clone();
        app.set_background(bg);
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
        // Dev hook for screenshots/tests: RIGRS_OPEN_SETTINGS=general|camera|model|hotkeys
        if let Ok(t) = std::env::var("RIGRS_OPEN_SETTINGS") {
            app.ui.settings_open = true;
            app.ui.tab = match t.as_str() {
                "camera" => crate::ui::Tab::Camera,
                "model" => crate::ui::Tab::Model,
                "hotkeys" => crate::ui::Tab::Hotkeys,
                _ => crate::ui::Tab::General,
            };
            if t.starts_with("camera") {
                app.ui.tab = crate::ui::Tab::Camera;
            }
            if let Some(px) = std::env::var("RIGRS_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
                app.ui.pending_scroll = Some(px);
            }
            app.ui.modal = match t.as_str() {
                "camera-cam" => Some(crate::ui::Modal::Camera),
                "camera-quality" => Some(crate::ui::Modal::Quality),
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

    pub fn start_tracker(&mut self) {
        self.tracker = None;
        let cfg = TrackerConfig {
            exe: PathBuf::from(&self.settings.tracker_exe),
            camera: self.settings.camera,
            width: self.settings.cam_size[0],
            height: self.settings.cam_size[1],
            fps: self.settings.cam_fps,
            model: self.settings.track_quality,
            threads: self.settings.track_threads,
            dcap: self.settings.cam_dcap,
            mediapipe: self.settings.track_backend == 1,
            hands: self.settings.track_type >= 1,
            port: self.settings.track_port,
            spawn: true,
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
        if !exe.is_file() {
            return;
        }
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            *s2.lock().unwrap() = Some(tracking::list_cameras(&exe));
        });
        self.cameras_job = Some(slot);
    }

    pub fn load_caps(&mut self, camera: i32) {
        self.caps = None;
        let exe = PathBuf::from(&self.settings.tracker_exe);
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            *s2.lock().unwrap() = Some(tracking::list_caps(&exe, camera));
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
            return;
        };
        t.poll();
        if let Some(e) = t.error.lock().unwrap().clone() {
            self.tracker_error = Some(e);
        }
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
                self.tracking = true;
                self.last_face = Some(f);
            }
            _ => {
                self.tracking = false;
                if self.settings.lost_reset || !face_on {
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

    pub fn load_model(&mut self, e: &ModelEntry) {
        let Some(core) = self.core else {
            self.toast("Cubism Core не загружен — укажите путь к Live2DCubismCore.dll в настройках");
            return;
        };
        self.remember_avatar_xf();
        let key = e.model3.to_string_lossy().into_owned();
        let t0 = Instant::now();
        match Live2D::load(
            core,
            &self.gpu,
            &e.model3,
            e.vtube.as_deref(),
            self.settings.idle_override.get(&key).map(|s| s.as_str()),
            self.settings.rules_override.get(&key),
        ) {
            Ok(mut a) => {
                a.update(0.0, &NEUTRAL, false, false);
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
                self.avatar = Some(a);
                self.settings.last_model = Some(key);
                self.save();
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

    fn make_item(&mut self, source: &Path) -> Result<Item> {
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
            l.fit(vw, vh, 0.3);
            let b = l.local_bounds();
            pivot = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
            let xf = Xform { pos: [0.0, 0.0], ..l.xf };
            (Body::Live2D(Box::new(l)), xf)
        } else {
            let s = Sprite::load(&self.gpu, source)?;
            let scale = (vh * 0.25 / s.size[1]).min(vw * 0.25 / s.size[0]);
            (Body::Sprite(s), Xform { pos: [0.0, 0.0], scale, rot: 0.0, flip: false })
        };
        Ok(Item { id, source: source.to_path_buf(), name, body, xf, front: true, locked: false, pin: None, pivot })
    }

    pub fn add_item(&mut self, source: &Path) -> Option<u64> {
        match self.make_item(source) {
            Ok(it) => {
                let id = it.id;
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
                front: i.front,
                locked: i.locked,
                pin: i.pin.clone(),
            })
            .collect()
    }

    pub fn restore_items(&mut self, saved: &[SavedItem]) {
        self.items.clear();
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
                    it.front = s.front;
                    it.locked = s.locked;
                    it.pin = pin;
                }
            }
        }
    }

    pub fn set_background(&mut self, bg: Background) {
        self.bg_sprite = None;
        if let Background::Image(p) = &bg {
            match Sprite::load(&self.gpu, Path::new(p)) {
                Ok(s) => self.bg_sprite = Some((p.clone(), s)),
                Err(e) => self.toast(format!("Фон: {e:#}")),
            }
        }
        self.settings.background = bg;
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

    pub fn save(&mut self) {
        self.remember_avatar_xf();
        if self.settings.save_scene {
            self.settings.last_items = self.saved_items();
        }
        self.settings.save(&self.paths.settings);
    }

    pub fn shutdown(&mut self) {
        self.save();
        self.tracker = None;
        self.spout = None;
        self.vcam = None;
    }

    // ------------------------------------------------------------------ frame

    pub fn frame(&mut self) {
        let t0 = Instant::now();
        let dt = t0.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = t0;
        self.time += dt;
        self.update_tracking();
        self.update_mouse();
        self.update_tobii();
        self.update_lighting(dt);
        if self.demo.is_some() && !crate::demo::step(self, dt) {
            self.quit = true;
        }
        let (vw, vh) = self.viewport();

        let raw = self.egui_state.take_egui_input(&self.window);
        let ctx = self.egui_ctx.clone();
        let out = ctx.run(raw, |c| crate::ui::run(self, c));
        self.egui_state.handle_platform_output(&self.window, out.platform_output);
        let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);

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
                it.follow(a);
            }
            it.update(dt, &self.inputs, &NEUTRAL, self.tracking, physics);
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
            if let Some(a) = &mut self.avatar {
                a.upload(&self.gpu, vw, vh, masks.as_mut());
            }
            self.stats.masks = masks.as_ref().map(|m| m.sets.len()).unwrap_or(0);
            if let Some((_, s)) = &self.bg_sprite {
                let k = (vw / s.size[0]).max(vh / s.size[1]);
                s.set_affine(&self.gpu, &Xform { pos: [0.0, 0.0], scale: k, rot: 0.0, flip: false }.affine(vw, vh, false));
            }
        }

        // Active post-processing effects (DEFS order).
        let active: Vec<(usize, crate::vfx::Cfg)> = if self.settings.vfx_on {
            self.settings.vfx.iter().enumerate().filter(|(_, c)| c.on).map(|(i, c)| (i, c.clone())).collect()
        } else {
            Vec::new()
        };
        if !active.is_empty() && render {
            let vfx = self.vfx.get_or_insert_with(|| crate::vfx::Renderer::new(&self.gpu));
            if let Some((_, c)) = active.iter().find(|(i, _)| crate::vfx::DEFS[*i].image) {
                vfx.set_aux(&self.gpu, &c.image);
            }
            vfx.ensure(&self.gpu, 0, self.gpu.config.width, self.gpu.config.height);
            for (slot, out) in [(1usize, &self.spout), (2, &self.ndi), (3, &self.vcam)] {
                if let Some(o) = out {
                    let (w, h) = o.size();
                    vfx.ensure(&self.gpu, slot, w, h);
                }
            }
        }
        let refs: Vec<(usize, &crate::vfx::Cfg)> = active.iter().map(|(i, c)| (*i, c)).collect();
        let vfx = self.vfx.as_ref().filter(|_| !refs.is_empty());
        if let Some(v) = vfx {
            v.upload(&self.gpu, &refs, self.time);
        }
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
            if let Some(a) = &self.avatar {
                a.draw_masks(&mut mp, al);
            }
        }
        let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [gpu.config.width, gpu.config.height], pixels_per_point: out.pixels_per_point };
        for (id, delta) in &out.textures_delta.set {
            self.egui_rend.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let mut draw_calls = 0;
        let bg = self.bg_sprite.as_ref().map(|(_, s)| s);
        if let Some(f) = &frame {
            let view = f.texture.create_view(&Default::default());
            {
                let target = vfx.and_then(|v| v.scene_view(0)).unwrap_or(&view);
                let mut p = Painter::new(gpu, &mut enc, target, &gpu.depth, self.clear_color());
                draw_scene(&mut p, bg, &self.items, self.avatar.as_ref());
                draw_calls = p.draw_calls;
            }
            if let Some(v) = vfx {
                v.run(&mut enc, 0, &view, &refs);
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
            let clear = self.clear_color();
            for (slot, out) in [(1usize, &mut self.spout), (2, &mut self.ndi), (3, &mut self.vcam)] {
                let Some(out) = out else { continue };
                {
                    let target = vfx.and_then(|v| v.scene_view(slot)).unwrap_or(&out.offscreen.scene_view);
                    let (bgx, cc) = if out.with_bg { (bg, clear) } else { (None, wgpu::Color::TRANSPARENT) };
                    let mut p = Painter::new(gpu, &mut enc, target, &out.offscreen.depth, cc);
                    draw_scene(&mut p, bgx, &self.items, self.avatar.as_ref());
                }
                if let Some(v) = vfx {
                    v.run(&mut enc, slot, &out.offscreen.scene_view, &refs);
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
                    draw_scene(&mut p, if transparent { None } else { bg }, &self.items, self.avatar.as_ref());
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
            Action::LoadModel(e) => self.load_model(&e),
            Action::UnloadModel => {
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
            Action::AddItemFile(p) => {
                if let Some(id) = self.add_item(&p) {
                    self.ui.selected = Some(id);
                }
            }
            Action::RemoveItem(id) => self.items.retain(|i| i.id != id),
            Action::ClearItems => self.items.clear(),
            Action::SetBackground(b) => {
                self.set_background(b);
                self.save();
            }
            Action::Screenshot { transparent } => self.ui.screenshot = Some(transparent),
            Action::SaveScene(name) => {
                let items = self.saved_items();
                self.settings.item_scenes.retain(|s| s.name != name);
                self.settings.item_scenes.push(ItemScene { name: name.clone(), items });
                self.toast(format!("Сцена «{name}» сохранена"));
                self.save();
            }
            Action::LoadScene(name) => {
                if let Some(s) = self.settings.item_scenes.iter().find(|s| s.name == name).cloned() {
                    self.restore_items(&s.items);
                }
            }
            Action::DeleteScene(name) => {
                self.settings.item_scenes.retain(|s| s.name != name);
                self.save();
            }
            Action::Calibrate => match self.tracker.as_ref().and_then(|t| t.latest()).filter(|f| f.success) {
                Some(f) => {
                    self.settings.calib = Calibration::from_frame(&f);
                    self.toast("Калибровка выполнена");
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
                    let _ = std::process::Command::new(exe).spawn();
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
