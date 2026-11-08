//! Hotkeys in the VTube Studio format (the same JSON objects as in `.vtube.json`, so a
//! model's existing hotkeys — keys, gestures, Twitch triggers — are imported as they are).
//! Hotkeys belong to a model, like in VTS.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::app::{Action, App};

// ------------------------------------------------------------------ data (VTS JSON layout)

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct HkPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct Gestures {
    pub gesture_left: String,
    pub gesture_right: String,
    pub gesture_combinator: String,
    pub allow_mirrored_gesture: bool,
    pub deactivate_exp_when_gesture_not_detected: bool,
    pub seconds_until_detection: f32,
}

impl Default for Gestures {
    fn default() -> Self {
        Gestures {
            gesture_left: String::new(),
            gesture_right: String::new(),
            gesture_combinator: "AND".into(),
            allow_mirrored_gesture: false,
            deactivate_exp_when_gesture_not_detected: false,
            seconds_until_detection: 0.5,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct LoadModelSettings {
    pub load_at_current_position: bool,
}

impl Default for LoadModelSettings {
    fn default() -> Self {
        LoadModelSettings { load_at_current_position: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Restrict {
    #[serde(rename = "Allowed_You")]
    pub you: bool,
    #[serde(rename = "Allowed_Subs")]
    pub subs: bool,
    #[serde(rename = "Allowed_Mods")]
    pub mods: bool,
    #[serde(rename = "Allowed_VIPs")]
    pub vips: bool,
    #[serde(rename = "Allowed_Artists")]
    pub artists: bool,
    #[serde(rename = "Allowed_UserNames")]
    pub by_name: bool,
    #[serde(rename = "UserNames")]
    pub user_names: Vec<String>,
    #[serde(rename = "Allowed_UserIDs")]
    pub by_id: bool,
    #[serde(rename = "UserIDs")]
    pub user_ids: Vec<String>,
}

impl Default for Restrict {
    fn default() -> Self {
        Restrict { you: true, subs: false, mods: false, vips: false, artists: false, by_name: false, user_names: Vec::new(), by_id: false, user_ids: Vec::new() }
    }
}

/// `TwitchTriggers` block of a VTS hotkey.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TwitchTriggers {
    #[serde(rename = "Active")]
    pub active: bool,
    #[serde(rename = "CooldownActive")]
    pub cooldown_active: bool,
    #[serde(rename = "CooldownSeconds")]
    pub cooldown_seconds: f32,
    #[serde(rename = "ResetActive")]
    pub reset_active: bool,
    #[serde(rename = "HideModelResetDialog")]
    pub hide_model_reset_dialog: bool,
    #[serde(rename = "Active_GiftSubTrigger")]
    pub gift_sub: bool,
    #[serde(rename = "Active_BitTrigger")]
    pub bits: bool,
    #[serde(rename = "Active_RedeemTrigger")]
    pub redeem: bool,
    #[serde(rename = "Active_PowerUpTrigger")]
    pub power_up: bool,
    #[serde(rename = "Active_TextCommandTrigger")]
    pub text: bool,
    #[serde(rename = "Active_SubTrigger")]
    pub sub: bool,
    #[serde(rename = "Active_FollowTrigger")]
    pub follow: bool,
    #[serde(rename = "Active_RaidTrigger")]
    pub raid: bool,
    #[serde(rename = "Active_ShoutoutTrigger")]
    pub shoutout: bool,
    #[serde(rename = "Active_AdbreakTrigger")]
    pub ad_break: bool,
    #[serde(rename = "Trigger_TextCommandTrigger")]
    pub text_words: String,
    #[serde(rename = "Trigger_Redeem_Name")]
    pub redeem_name: String,
    #[serde(rename = "Trigger_Redeem_ID")]
    pub redeem_id: String,
    #[serde(rename = "Trigger_PowerUp_Name")]
    pub power_up_name: String,
    #[serde(rename = "Trigger_PowerUp_ID")]
    pub power_up_id: String,
    #[serde(rename = "UseTextTriggerUserTypeRestriction")]
    pub restrict_text: bool,
    #[serde(rename = "RestrictTextTriggerTo")]
    pub restrict: Restrict,
    #[serde(rename = "AllowRepeatedFollow")]
    pub allow_repeated_follow: bool,
    #[serde(rename = "Trigger_GiftSubTrigger")]
    pub gift_range: Range,
    #[serde(rename = "Trigger_BitTrigger")]
    pub bit_range: Range,
}

impl Default for TwitchTriggers {
    fn default() -> Self {
        TwitchTriggers {
            active: false,
            cooldown_active: false,
            cooldown_seconds: 30.0,
            reset_active: false,
            hide_model_reset_dialog: false,
            gift_sub: false,
            bits: false,
            redeem: false,
            power_up: false,
            text: false,
            sub: false,
            follow: false,
            raid: false,
            shoutout: false,
            ad_break: false,
            text_words: String::new(),
            redeem_name: String::new(),
            redeem_id: String::new(),
            power_up_name: String::new(),
            power_up_id: String::new(),
            restrict_text: false,
            restrict: Restrict::default(),
            allow_repeated_follow: false,
            gift_range: Range { min: 1.0, max: 100.0 },
            bit_range: Range { min: 100.0, max: 3000.0 },
        }
    }
}

impl TwitchTriggers {
    /// Number of enabled trigger kinds.
    pub fn count(&self) -> usize {
        [self.gift_sub, self.bits, self.redeem, self.power_up, self.text, self.sub, self.follow, self.raid, self.shoutout, self.ad_break].iter().filter(|b| **b).count()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct Keys {
    pub trigger1: String,
    pub trigger2: String,
    pub trigger3: String,
    pub screen_button: i32,
}

impl Default for Keys {
    fn default() -> Self {
        Keys { trigger1: String::new(), trigger2: String::new(), trigger3: String::new(), screen_button: -1 }
    }
}

impl Keys {
    pub fn list(&self) -> Vec<&str> {
        [self.trigger1.as_str(), self.trigger2.as_str(), self.trigger3.as_str()].into_iter().filter(|k| !k.is_empty()).collect()
    }
    pub fn set(&mut self, keys: &[String]) {
        self.trigger1 = keys.first().cloned().unwrap_or_default();
        self.trigger2 = keys.get(1).cloned().unwrap_or_default();
        self.trigger3 = keys.get(2).cloned().unwrap_or_default();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct SoundSlot {
    pub file: String,
    pub volume: f32,
}

impl Default for SoundSlot {
    fn default() -> Self {
        SoundSlot { file: String::new(), volume: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct SoundConfig {
    pub sound_active: bool,
    pub stop_when_inactive: bool,
    pub stop_fade_out_seconds: f32,
    pub sound_slots: Vec<SoundSlot>,
}

impl Default for SoundConfig {
    fn default() -> Self {
        SoundConfig { sound_active: false, stop_when_inactive: false, stop_fade_out_seconds: 2.0, sound_slots: Vec::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct Hotkey {
    #[serde(rename = "HotkeyID")]
    pub id: String,
    pub name: String,
    pub action: String,
    pub file: String,
    pub folder: String,
    pub position: HkPosition,
    pub hand_gesture_settings: Gestures,
    pub load_model_settings: LoadModelSettings,
    pub twitch_triggers: TwitchTriggers,
    pub triggers: Keys,
    pub sound_config: SoundConfig,
    pub is_global: bool,
    pub is_active: bool,
    pub minimized: bool,
    pub stops_on_last_frame: bool,
    pub deactivate_after_key_up: bool,
    pub only_load_one_random_item: bool,
    pub deactivate_after_seconds: bool,
    pub deactivate_after_seconds_amount: f32,
    pub fade_seconds_amount: f32,
    pub on_screen_hotkey_color: Rgba,
    /// Everything else VTS stores (kept so the data round-trips).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Hotkey {
    fn default() -> Self {
        Hotkey {
            id: new_id(),
            name: String::new(),
            action: "ToggleExpression".into(),
            file: String::new(),
            folder: String::new(),
            position: HkPosition::default(),
            hand_gesture_settings: Gestures::default(),
            load_model_settings: LoadModelSettings::default(),
            twitch_triggers: TwitchTriggers::default(),
            triggers: Keys::default(),
            sound_config: SoundConfig::default(),
            is_global: true,
            is_active: true,
            minimized: false,
            stops_on_last_frame: false,
            deactivate_after_key_up: false,
            only_load_one_random_item: false,
            deactivate_after_seconds: false,
            deactivate_after_seconds_amount: 10.0,
            fade_seconds_amount: 0.25,
            on_screen_hotkey_color: Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            extra: Map::new(),
        }
    }
}

pub fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut s = String::new();
    for i in 0..2u8 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u8(i);
        s.push_str(&format!("{:016x}", h.finish()));
    }
    s
}

// ------------------------------------------------------------------ actions

/// (VTS action id, label, supported by rigrs, needs a file/target)
pub const ACTIONS: &[(&str, &str, bool, bool)] = &[
    ("TriggerAnimation", "Проиграть анимацию (motion3)", true, true),
    ("ChangeIdleAnimation", "Сменить анимацию ожидания", true, true),
    ("ToggleExpression", "Вкл./выкл. выражение (exp3)", true, true),
    ("RemoveAllExpressions", "Удалить все выражения", true, false),
    ("MoveModel", "Переместить модель", true, false),
    ("ChangeBackground", "Сменить фон", true, true),
    ("ReloadMicrophone", "Перезагрузить микрофон", false, false),
    ("ReloadTextures", "Перезагрузить текстуру модели", true, false),
    ("CalibrateCam", "Откалибровать камеру", true, false),
    ("ChangeVTSModel", "Сменить модель VTS", true, true),
    ("TakeScreenshot", "Сделать скриншот", true, false),
    ("ScreenColorOverlay", "Наложение на экран", true, false),
    ("RemoveAllItems", "Удалить все элементы со сцены", true, false),
    ("ToggleItemScene", "Переключить сцену элементов", true, true),
    ("DownloadRandomWorkshopItem", "Случайный предмет Мастерской", true, false),
    ("ExecuteItemAction", "Действие элемента Live2D", true, true),
    ("ArtMeshColorPreset", "Пресет цветов сетки ArtMesh", true, false),
    ("ToggleTracker", "Вкл./выкл. трекер", true, false),
    ("ToggleTwitchFeature", "Вкл./выкл. функцию Twitch", true, true),
    ("LoadEffectPreset", "Загрузить пресет эффектов", true, true),
    ("ToggleLive2DEditorAPI", "Синхронизация параметров с Live2D Editor API", false, false),
    ("WebItemAction", "Запустить действие веб-элемента", false, false),
    ("ToggleModelSound", "Toggle Model Volume", false, false),
    ("ToggleUI", "Показать/скрыть интерфейс (rigrs)", true, false),
];

pub fn action_label(id: &str) -> String {
    ACTIONS.iter().find(|a| a.0 == id).map(|a| crate::i18n::tr(a.1).to_string()).unwrap_or_else(|| id.to_string())
}

pub fn action_needs_file(id: &str) -> bool {
    ACTIONS.iter().find(|a| a.0 == id).map(|a| a.3).unwrap_or(false)
}

/// Caption of the file picker for an action.
pub fn file_caption(id: &str) -> &'static str {
    match id {
        "TriggerAnimation" | "ChangeIdleAnimation" => "Анимация",
        "ToggleExpression" => "Выражение",
        "ChangeBackground" => "Фон",
        "ChangeVTSModel" => "Модель",
        "ToggleItemScene" => "Сцена элементов",
        "ToggleTwitchFeature" => "Функция Twitch",
        "LoadEffectPreset" => "Пресет эффектов",
        _ => "Файл",
    }
}

pub const TWITCH_FEATURES: [(&str, &str); 3] = [("EmoteDropper", "Twitch Emote Dropper"), ("FlyingChat", "Пролетающий чат Twitch"), ("HotkeyTriggers", "Триггеры горячей клавиши Twitch")];

/// Hand gestures recognised from the 21 hand landmarks.
pub const GESTURES: [(&str, &str); 10] = [
    ("", "Нет"),
    ("OpenHand", "Открытая ладонь ✋"),
    ("Fist", "Кулак ✊"),
    ("Victory", "Победа ✌"),
    ("ThumbsUp", "Палец вверх 👍"),
    ("Pointing", "Указательный палец ☝"),
    ("Rock", "Коза 🤘"),
    ("Call", "«Позвони мне» 🤙"),
    ("DualTriangle", "Треугольник 🔺 (две руки)"),
    ("DualIsForMe", "«Это мне?» 👉👈 (две руки)"),
];

/// Two-hand gestures (VTS "dual-hand gesture": triangle, "is-for-me").
pub fn detect_dual(a: &crate::tracking::Hand, b: &crate::tracking::Hand) -> &'static str {
    let d = |p: [f32; 3], q: [f32; 3]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
    let palm = (d(a.pts[0], a.pts[9]) + d(b.pts[0], b.pts[9])) * 0.5;
    if palm < 1e-4 {
        return "";
    }
    let (fa, fb) = (fingers(a), fingers(b));
    // Index tips touching and thumb tips touching, index above thumbs: a triangle.
    if d(a.pts[8], b.pts[8]) < palm * 0.6 && d(a.pts[4], b.pts[4]) < palm * 0.6 && a.pts[8][1] < a.pts[4][1] && fa[1] && fb[1] {
        return "DualTriangle";
    }
    // Both hands pointing with the index fingers, tips close together: "is it for me?".
    let pointing = |f: [bool; 5]| f[1] && !f[2] && !f[3] && !f[4];
    if pointing(fa) && pointing(fb) && d(a.pts[8], b.pts[8]) < palm * 1.3 {
        return "DualIsForMe";
    }
    ""
}

pub fn gesture_label(id: &str) -> &'static str {
    GESTURES.iter().find(|g| g.0 == id).map(|g| crate::i18n::tr(g.1)).unwrap_or("")
}

/// Which fingers are extended (thumb, index, middle, ring, pinky).
fn fingers(h: &crate::tracking::Hand) -> [bool; 5] {
    let p = &h.pts;
    let d = |a: usize, b: usize| ((p[a][0] - p[b][0]).powi(2) + (p[a][1] - p[b][1]).powi(2)).sqrt();
    let palm = d(0, 9).max(1e-4);
    let ext = |tip: usize, pip: usize| d(tip, 0) > d(pip, 0) * 1.15 && d(tip, 0) > palm * 1.1;
    let thumb = d(4, 17) > d(3, 17) * 1.08 && d(4, 5) > palm * 0.45;
    [thumb, ext(8, 6), ext(12, 10), ext(16, 14), ext(20, 18)]
}

pub fn detect_gesture(h: &crate::tracking::Hand) -> &'static str {
    match fingers(h) {
        [_, true, true, true, true] => "OpenHand",
        [false, false, false, false, false] => "Fist",
        [_, true, true, false, false] => "Victory",
        [true, false, false, false, false] => {
            // Thumb tip above the wrist (image y grows downwards).
            if h.pts[4][1] < h.pts[0][1] { "ThumbsUp" } else { "Fist" }
        }
        [false, true, false, false, false] => "Pointing",
        [_, true, false, false, true] => "Rock",
        [true, false, false, false, true] => "Call",
        _ => "",
    }
}

// ------------------------------------------------------------------ key names (VTS / Unity style)

/// (name used in the JSON, Windows virtual-key code).
/// The first part is exactly VTube Studio's key list (Files/RestrictedRawKey.cs), so recorded
/// combinations are written with VTS names; the rest are extra keys rigrs also accepts.
pub const KEYS: &[(&str, u16)] = &[
    ("LeftMouseButton", 0x01), ("RightMouseButton", 0x02), ("MiddleMouseButton", 0x04), ("Tab", 0x09), ("CapsLock", 0x14),
    ("Escape", 0x1B), ("Space", 0x20), ("Left", 0x25), ("Up", 0x26), ("Right", 0x27), ("Down", 0x28), ("Print", 0x2A),
    ("Delete", 0x2E), ("PageUp", 0x21), ("PageDown", 0x22),
    ("N0", 0x30), ("N1", 0x31), ("N2", 0x32), ("N3", 0x33), ("N4", 0x34), ("N5", 0x35), ("N6", 0x36), ("N7", 0x37), ("N8", 0x38), ("N9", 0x39),
    ("A", 0x41), ("B", 0x42), ("C", 0x43), ("D", 0x44), ("E", 0x45), ("F", 0x46), ("G", 0x47), ("H", 0x48), ("I", 0x49),
    ("J", 0x4A), ("K", 0x4B), ("L", 0x4C), ("M", 0x4D), ("N", 0x4E), ("O", 0x4F), ("P", 0x50), ("Q", 0x51), ("R", 0x52),
    ("S", 0x53), ("T", 0x54), ("U", 0x55), ("V", 0x56), ("W", 0x57), ("X", 0x58), ("Y", 0x59), ("Z", 0x5A),
    ("LeftWindows", 0x5B), ("RightWindows", 0x5C),
    ("Numpad0", 0x60), ("Numpad1", 0x61), ("Numpad2", 0x62), ("Numpad3", 0x63), ("Numpad4", 0x64), ("Numpad5", 0x65), ("Numpad6", 0x66),
    ("Numpad7", 0x67), ("Numpad8", 0x68), ("Numpad9", 0x69), ("Multiply", 0x6A), ("Add", 0x6B), ("Subtract", 0x6D), ("Decimal", 0x6E), ("Divide", 0x6F),
    ("F1", 0x70), ("F2", 0x71), ("F3", 0x72), ("F4", 0x73), ("F5", 0x74), ("F6", 0x75), ("F7", 0x76), ("F8", 0x77), ("F9", 0x78), ("F10", 0x79),
    ("F11", 0x7A), ("F12", 0x7B), ("F13", 0x7C), ("F14", 0x7D), ("F15", 0x7E), ("F16", 0x7F), ("F17", 0x80), ("F18", 0x81), ("F19", 0x82), ("F20", 0x83),
    ("F21", 0x84), ("F22", 0x85), ("F23", 0x86), ("F24", 0x87), ("NumLock", 0x90), ("ScrollLock", 0x91),
    ("LeftShift", 0xA0), ("RightShift", 0xA1), ("LeftControl", 0xA2), ("RightControl", 0xA3), ("Alt", 0xA4),
    // ---- extra keys (not in VTS)
    ("RightAlt", 0xA5), ("Return", 0x0D), ("Backspace", 0x08), ("Insert", 0x2D), ("Home", 0x24), ("End", 0x23), ("Pause", 0x13),
    ("Semicolon", 0xBA), ("Equals", 0xBB), ("Comma", 0xBC), ("Minus", 0xBD), ("Period", 0xBE), ("Slash", 0xBF), ("BackQuote", 0xC0),
    ("LeftBracket", 0xDB), ("Backslash", 0xDC), ("RightBracket", 0xDD), ("Quote", 0xDE), ("Mouse4", 0x05), ("Mouse5", 0x06),
];

fn vk_of(name: &str) -> Option<u16> {
    // Aliases written by older VTS versions / earlier rigrs builds.
    let name = match name {
        "Alpha0" => "N0", "Alpha1" => "N1", "Alpha2" => "N2", "Alpha3" => "N3", "Alpha4" => "N4",
        "Alpha5" => "N5", "Alpha6" => "N6", "Alpha7" => "N7", "Alpha8" => "N8", "Alpha9" => "N9",
        "Keypad0" => "Numpad0", "Keypad1" => "Numpad1", "Keypad2" => "Numpad2", "Keypad3" => "Numpad3", "Keypad4" => "Numpad4",
        "Keypad5" => "Numpad5", "Keypad6" => "Numpad6", "Keypad7" => "Numpad7", "Keypad8" => "Numpad8", "Keypad9" => "Numpad9",
        "Enter" => "Return", "UpArrow" => "Up", "DownArrow" => "Down", "LeftArrow" => "Left", "RightArrow" => "Right",
        "LeftAlt" => "Alt", "Mouse3" => "MiddleMouseButton", "Esc" => "Escape",
        n => n,
    };
    KEYS.iter().find(|k| k.0.eq_ignore_ascii_case(name)).map(|k| k.1)
}

#[cfg(windows)]
fn vk_down(vk: u16) -> bool {
    unsafe { (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

#[cfg(not(windows))]
fn vk_down(_vk: u16) -> bool {
    false
}

// ------------------------------------------------------------------ runtime

/// What an auto-reset restores (VTS wiki "Twitch Hotkey Triggers → Auto-reset").
#[derive(Clone, Debug)]
pub enum Undo {
    /// Turn the expression off again.
    Expression(String),
    /// Move the model back to where it was.
    Move(crate::scene::live2d::Xform),
    /// Unload the item scene.
    Scene(String),
    /// Restore the previous effect values.
    Effects(Vec<crate::vfx::Cfg>, bool),
    /// Turn the Twitch feature off again.
    Feature(String),
    /// Load the previous model again.
    Model(std::path::PathBuf, String),
}

#[derive(Clone, Debug)]
pub struct Reset {
    pub hotkey: String,
    pub due: Instant,
    pub undo: Undo,
    pub hide_dialog: bool,
}

/// Undo record for a hotkey action, captured before the action runs.
fn capture_undo(app: &App, h: &Hotkey) -> Option<Undo> {
    Some(match h.action.as_str() {
        "ToggleExpression" => Undo::Expression(h.file.clone()),
        "MoveModel" => Undo::Move(app.avatar.as_ref()?.xf),
        "ToggleItemScene" => Undo::Scene(h.file.trim_end_matches(".json").to_string()),
        "LoadEffectPreset" => Undo::Effects(app.settings.vfx.clone(), app.settings.vfx_on),
        "ToggleTwitchFeature" => Undo::Feature(h.file.clone()),
        "ChangeVTSModel" => {
            let a = app.avatar.as_ref()?;
            Undo::Model(a.model3.clone(), a.name.clone())
        }
        _ => return None,
    })
}

/// Executes an auto-reset.
pub fn run_reset(app: &mut App, r: &Reset) {
    match &r.undo {
        Undo::Expression(file) => {
            if let Some(a) = app.avatar.as_mut() {
                if let Some(e) = a.expressions.iter_mut().find(|e| e.file.ends_with(file.as_str())) {
                    e.active = false;
                }
            }
        }
        Undo::Move(xf) => {
            if let Some(a) = app.avatar.as_ref() {
                app.api_model_move = Some((a.xf, *xf, 0.0, 0.5));
            }
        }
        Undo::Scene(name) => app.unload_scene(name, 0.3),
        Undo::Effects(cfg, on) => {
            app.settings.vfx = cfg.clone();
            app.settings.vfx_on = *on;
        }
        Undo::Feature(f) => {
            let t = &mut app.settings.twitch;
            match f.as_str() {
                "EmoteDropper" => t.dropper.enabled = false,
                "FlyingChat" => t.chat.enabled = false,
                _ => t.triggers_on = false,
            }
        }
        Undo::Model(path, _) => {
            if let Some(m) = app.library.models.iter().find(|m| &m.model3 == path).cloned() {
                app.hk.hotkey_model_load = true;
                app.actions.push(Action::LoadModel(m));
            }
        }
    }
}

/// Cancels cooldowns / resets of a hotkey (manual trigger or config change, as in VTS).
pub fn cancel(app: &mut App, hotkey: &str) {
    app.hk.cooldown.remove(hotkey);
    app.hk.resets.retain(|r| r.hotkey != hotkey);
}

/// Called before a model is replaced: pending resets of the old model run immediately
/// (except "load previous model" ones), and all cooldowns are cancelled.
pub fn before_model_change(app: &mut App) {
    let manual = !std::mem::take(&mut app.hk.hotkey_model_load);
    let pending = std::mem::take(&mut app.hk.resets);
    for r in &pending {
        if !matches!(r.undo, Undo::Model(..)) {
            run_reset(app, r);
        }
    }
    if !manual {
        app.hk.resets = pending.into_iter().filter(|r| matches!(r.undo, Undo::Model(..))).collect();
    }
    app.hk.cooldown.clear();
}

#[derive(Default)]
pub struct Runtime {
    /// Keys held during the previous frame.
    held: HashSet<u16>,
    /// Key recording for hotkey index: keys collected so far.
    pub recording: Option<(usize, Vec<String>, bool)>,
    /// Hotkey id → time when it auto-deactivates.
    deactivate_at: HashMap<String, Instant>,
    /// Hotkeys activated by a key that should turn off on key-up: id → keys.
    key_up: HashMap<String, Vec<u16>>,
    /// Gesture state per hotkey id: since when it is detected, active.
    gesture: HashMap<String, (Option<Instant>, bool)>,
    /// Twitch cooldowns: hotkey id → last trigger.
    pub cooldown: HashMap<String, Instant>,
    /// Pending auto-resets after a Twitch cooldown (VTS "Reset after cooldown").
    pub resets: Vec<Reset>,
    /// Set while a ChangeVTSModel hotkey / reset loads a model (manual loads cancel model resets).
    pub hotkey_model_load: bool,
    /// Loaded ArtMesh color preset: (hotkey id, entries).
    pub color_preset: Option<(String, Vec<crate::artmesh::IdValue>)>,
    /// Followers seen this session (repeated follow filter).
    pub followers: HashSet<String>,
    /// Sounds currently playing (MCI aliases).
    sounds: Vec<(String, String)>,
    sound_seq: u32,
}

impl App {
    /// The current model's hotkeys (created from its .vtube.json on first use).
    pub fn hotkeys(&self) -> &[Hotkey] {
        self.model_key().and_then(|k| self.settings.model_hotkeys.get(&k)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn hotkeys_mut(&mut self) -> Option<&mut Vec<Hotkey>> {
        let k = self.model_key()?;
        Some(self.settings.model_hotkeys.entry(k).or_default())
    }
}

/// Seeds the model's hotkeys from its .vtube.json (once).
pub fn on_model_loaded(app: &mut App) {
    let Some(key) = app.model_key() else { return };
    if app.settings.model_hotkeys.contains_key(&key) {
        return;
    }
    let list = app.avatar.as_ref().map(|a| a.vts_hotkeys.clone()).unwrap_or_default();
    app.settings.model_hotkeys.insert(key, list);
}

/// Per-frame: keyboard (global or focused), gestures, auto-deactivation, sounds.
pub fn update(app: &mut App, typing: bool) {
    let focused = app.window.has_focus();
    // ---- key recording
    if let Some((idx, keys, mut any)) = app.hk.recording.take() {
        let mut keys = keys;
        let mut now_down = Vec::new();
        for &(name, vk) in KEYS {
            if vk == 0x01 || vk == 0x1B {
                continue;
            }
            if vk_down(vk) {
                now_down.push(vk);
                if !keys.iter().any(|k| k == name) && keys.len() < 2 {
                    keys.push(name.to_string());
                    any = true;
                }
            }
        }
        if vk_down(0x1B) {
            // Esc clears the key combination.
            if let Some(h) = app.hotkeys_mut().and_then(|v| v.get_mut(idx)) {
                h.triggers.set(&[]);
            }
            app.hk.held = [0x1Bu16].into_iter().collect();
            app.save();
            return;
        }
        if any && now_down.is_empty() {
            if let Some(h) = app.hotkeys_mut().and_then(|v| v.get_mut(idx)) {
                h.triggers.set(&keys);
            }
            app.hk.held.clear();
            app.save();
        } else {
            app.hk.recording = Some((idx, keys, any));
        }
        return;
    }

    // ---- keyboard
    let list: Vec<Hotkey> = app.hotkeys().to_vec();
    let mut held = HashSet::new();
    let mut fire = Vec::new();
    if app.settings.hotkeys_enabled && !typing {
        for h in list.iter().filter(|h| h.is_active) {
            let keys: Vec<u16> = h.triggers.list().iter().filter_map(|k| vk_of(k)).collect();
            if keys.is_empty() || keys.len() != h.triggers.list().len() {
                continue;
            }
            if !focused && !(h.is_global && app.settings.send_to_pc) {
                continue;
            }
            let all = keys.iter().all(|&k| vk_down(k));
            for &k in &keys {
                if vk_down(k) {
                    held.insert(k);
                }
            }
            // Fires when the combination becomes complete.
            let was = keys.iter().all(|k| app.hk.held.contains(k));
            if all && !was {
                fire.push((h.id.clone(), keys.clone()));
            }
        }
    }
    // Live2D items with "Use normal hotkey triggers": their own keyboard hotkeys.
    let mut item_fire: Vec<(u64, Hotkey)> = Vec::new();
    if app.settings.hotkeys_enabled && !typing {
        for it in &app.items {
            let crate::scene::Body::Live2D(l) = &it.body else { continue };
            if !it.cfg.normal_hotkeys {
                continue;
            }
            for h in l.vts_hotkeys.iter().filter(|h| h.is_active) {
                let keys: Vec<u16> = h.triggers.list().iter().filter_map(|k| vk_of(k)).collect();
                if keys.is_empty() || keys.len() != h.triggers.list().len() || (!focused && !(h.is_global && app.settings.send_to_pc)) {
                    continue;
                }
                let all = keys.iter().all(|&k| vk_down(k));
                for &k in &keys {
                    if vk_down(k) {
                        held.insert(k);
                    }
                }
                if all && !keys.iter().all(|k| app.hk.held.contains(k)) {
                    item_fire.push((it.id, h.clone()));
                }
            }
        }
    }
    for (iid, h) in item_fire {
        if let Some(crate::scene::Body::Live2D(l)) = app.items.iter_mut().find(|i| i.id == iid).map(|i| &mut i.body) {
            run_on_item(l, &h);
        }
    }
    app.hk.held = held;
    for (id, keys) in fire {
        trigger(app, &id, Source::Key(keys));
    }

    // ---- key-up deactivation
    let up: Vec<String> = app.hk.key_up.iter().filter(|(_, ks)| !ks.iter().all(|&k| vk_down(k))).map(|(id, _)| id.clone()).collect();
    for id in up {
        app.hk.key_up.remove(&id);
        deactivate(app, &id);
    }

    // ---- timed deactivation
    let now = Instant::now();
    let due: Vec<String> = app.hk.deactivate_at.iter().filter(|(_, t)| **t <= now).map(|(id, _)| id.clone()).collect();
    for id in due {
        app.hk.deactivate_at.remove(&id);
        deactivate(app, &id);
    }

    // ---- reset after Twitch cooldown
    let due: Vec<Reset> = app.hk.resets.iter().filter(|r| r.due <= now).cloned().collect();
    app.hk.resets.retain(|r| r.due > now);
    for r in due {
        run_reset(app, &r);
    }

    // ---- hand gestures
    let hands = app.last_hands.filter(|h| h.received.elapsed() < Duration::from_millis(500));
    let detected: [&str; 2] = match hands {
        // OpenSeeFace reports hands in image order; mirror so "left" is the user's left hand.
        Some(h) => [h.hands[1].as_ref().map(detect_gesture).unwrap_or(""), h.hands[0].as_ref().map(detect_gesture).unwrap_or("")],
        None => ["", ""],
    };
    let dual = match hands {
        Some(hf) => match (&hf.hands[0], &hf.hands[1]) {
            (Some(a), Some(b)) => detect_dual(a, b),
            _ => "",
        },
        None => "",
    };
    for h in list.iter().filter(|h| h.is_active) {
        let g = &h.hand_gesture_settings;
        if g.gesture_left.is_empty() && g.gesture_right.is_empty() {
            continue;
        }
        let is_dual = g.gesture_left.starts_with("Dual") || g.gesture_right.starts_with("Dual");
        let side = |want: &str, have: &str| want.is_empty() || want == have;
        let both = g.gesture_combinator != "OR";
        let ok_l = side(&g.gesture_left, detected[0]);
        let ok_r = side(&g.gesture_right, detected[1]);
        let mut ok = if both { ok_l && ok_r } else { (!g.gesture_left.is_empty() && g.gesture_left == detected[0]) || (!g.gesture_right.is_empty() && g.gesture_right == detected[1]) };
        if !ok && g.allow_mirrored_gesture {
            let ml = side(&g.gesture_left, detected[1]);
            let mr = side(&g.gesture_right, detected[0]);
            ok = if both { ml && mr } else { (!g.gesture_left.is_empty() && g.gesture_left == detected[1]) || (!g.gesture_right.is_empty() && g.gesture_right == detected[0]) };
        }
        if is_dual {
            let want = if g.gesture_left.starts_with("Dual") { &g.gesture_left } else { &g.gesture_right };
            ok = !dual.is_empty() && dual == want.as_str();
        }
        let st = app.hk.gesture.entry(h.id.clone()).or_insert((None, false));
        if ok {
            let since = *st.0.get_or_insert(now);
            if !st.1 && now.duration_since(since).as_secs_f32() >= g.seconds_until_detection {
                st.1 = true;
                let id = h.id.clone();
                trigger(app, &id, Source::Gesture);
            }
        } else {
            let was = st.1;
            *st = (None, false);
            if was && g.deactivate_exp_when_gesture_not_detected {
                let id = h.id.clone();
                deactivate(app, &id);
            }
        }
    }
}

pub enum Source {
    Key(Vec<u16>),
    Gesture,
    Screen,
    Api,
    Twitch,
    Test,
}

/// Runs a hotkey of the current model by id (or name).
pub fn trigger(app: &mut App, id_or_name: &str, src: Source) -> bool {
    let Some(h) = app.hotkeys().iter().find(|h| h.id == id_or_name || (!h.name.is_empty() && h.name.eq_ignore_ascii_case(id_or_name))).cloned() else {
        return false;
    };
    let by_api = matches!(src, Source::Api);
    let mut activated_expression = false;
    let undo = if matches!(src, Source::Twitch) && h.twitch_triggers.reset_active && h.twitch_triggers.cooldown_active { capture_undo(app, &h) } else { None };
    if !matches!(src, Source::Twitch) {
        // VTS: triggering the hotkey manually cancels its cooldown and pending auto-reset.
        cancel(app, &h.id);
    }
    if h.action == "ChangeVTSModel" {
        app.hk.hotkey_model_load = true;
    }
    match h.action.as_str() {
        "TriggerAnimation" => {
            if let Some(a) = app.avatar.as_mut() {
                if !a.play_once(&h.file) {
                    app.toast(format!("Анимация «{}» не найдена", h.file));
                }
            }
        }
        // VTS: changes the looping idle animation without saving it into the model config.
        "ChangeIdleAnimation" => {
            if let Some(a) = app.avatar.as_mut() {
                a.set_idle((!h.file.is_empty()).then_some(h.file.as_str()));
            }
        }
        "ToggleExpression" => {
            if let Some(a) = app.avatar.as_mut() {
                if let Some(e) = a.expressions.iter_mut().find(|e| e.file.ends_with(&h.file) || (!h.file.is_empty() && e.name == h.file)) {
                    e.fade = h.fade_seconds_amount.max(0.0);
                    e.active = !e.active;
                    activated_expression = e.active;
                    let ev = serde_json::json!({ "modelID": a.model_id, "modelName": a.name, "isLive2DItem": false, "itemInstanceID": "", "justLoaded": false, "expressionFile": e.file, "expressionName": e.name, "active": e.active });
                    app.api_events.push(("ExpressionToggledEvent".into(), ev));
                } else {
                    app.toast(format!("Выражение «{}» не найдено в модели", h.file));
                }
            }
        }
        "RemoveAllExpressions" => {
            if let Some(a) = app.avatar.as_mut() {
                for e in &mut a.expressions {
                    e.fade = h.fade_seconds_amount.max(0.0);
                    e.active = false;
                }
            }
        }
        "MoveModel" => {
            let (vw, vh) = app.viewport();
            if let Some(a) = app.avatar.as_ref() {
                let from = a.xf;
                let mut to = from;
                to.pos = [h.position.x * vw / 2.0, h.position.y * vh / 2.0];
                if h.position.z > 0.0 {
                    to.scale = h.position.z;
                }
                to.rot = h.position.rotation;
                let t = h.fade_seconds_amount.clamp(0.0, 10.0);
                if t <= 0.0 {
                    app.avatar.as_mut().unwrap().xf = to;
                } else {
                    app.api_model_move = Some((from, to, 0.0, t));
                }
            }
        }
        "ChangeBackground" => {
            if let Some(p) = app.library.backgrounds.iter().find(|p| p.file_name().map(|n| n.to_string_lossy() == h.file).unwrap_or(false)).cloned() {
                app.actions.push(Action::SetBackground(crate::settings::Background::Image(p.to_string_lossy().into_owned())));
            } else {
                app.toast(format!("Фон «{}» не найден", h.file));
            }
        }
        "ReloadTextures" => {
            if let Some(m) = app.avatar.as_ref().and_then(|a| app.library.models.iter().find(|m| m.model3 == a.model3)).cloned() {
                app.actions.push(Action::LoadModel(m));
            }
        }
        "CalibrateCam" => app.actions.push(Action::Calibrate),
        "ChangeVTSModel" => {
            match app.library.models.iter().find(|m| m.name == h.file || m.model3.to_string_lossy() == h.file || m.vtube.as_ref().map(|v| v.file_name().map(|n| n.to_string_lossy() == h.file).unwrap_or(false)).unwrap_or(false)).cloned() {
                Some(m) => {
                    let keep = h.load_model_settings.load_at_current_position.then(|| app.avatar.as_ref().map(|a| a.xf)).flatten();
                    app.actions.push(Action::LoadModel(m));
                    app.pending_model_xf = keep;
                }
                None => app.toast(format!("Модель «{}» не найдена", h.file)),
            }
        }
        "TakeScreenshot" => app.ui.screenshot = Some(false),
        "ScreenColorOverlay" => {
            // VTS: applies the display light overlay settings recorded in the hotkey.
            let o = h.extra.get("ColorOverlay").cloned().unwrap_or_default();
            let st = &mut app.settings;
            let num = |k: &str, d: f64| o[k].as_f64().unwrap_or(d) as f32;
            if o["On"].as_bool().unwrap_or(false) {
                st.light_brightness = num("BaseValue", st.light_brightness as f64);
                st.light_overlay = num("OverlayValue", st.light_overlay as f64);
                st.light_smoothing = num("Smoothing", st.light_smoothing as f64);
                st.light_items = o["IncludeItems"].as_bool().unwrap_or(st.light_items);
                let mode = if o["IsStaticColor"].as_bool().unwrap_or(false) {
                    let c = &o["StaticColor"];
                    st.light_color = [c["r"].as_f64().unwrap_or(1.0) as f32, c["g"].as_f64().unwrap_or(1.0) as f32, c["b"].as_f64().unwrap_or(1.0) as f32];
                    3
                } else if !o["WindowName"].as_str().unwrap_or("").is_empty() {
                    st.light_window = o["WindowName"].as_str().unwrap_or("").to_string();
                    2
                } else {
                    1
                };
                let mon = o["Display"].as_i64().unwrap_or(0).max(0) as usize;
                app.actions.push(Action::SetLight(mode, mon));
            } else {
                let mon = app.settings.light_monitor;
                app.actions.push(Action::SetLight(0, mon));
            }
        }
        "RemoveAllItems" => app.actions.push(Action::ClearItems),
        "ToggleItemScene" => {
            let name = h.file.trim_end_matches(".json").to_string();
            let loaded = app.toggle_scene(&name, h.fade_seconds_amount.max(0.0));
            if loaded && h.deactivate_after_seconds {
                app.hk.deactivate_at.insert(h.id.clone(), Instant::now() + Duration::from_secs_f32(h.deactivate_after_seconds_amount.max(0.1)));
            } else if !loaded {
                app.hk.deactivate_at.remove(&h.id);
            }
        }
        "DownloadRandomWorkshopItem" => {
            let n = app.library.items.len();
            if n > 0 {
                let k = (Instant::now().elapsed().as_nanos() as usize ^ std::process::id() as usize ^ app.items.len().wrapping_mul(7919) ^ crate::hotkeys::new_id().len()) % n;
                let k = (k + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as usize)) % n;
                let e = app.library.items[k].clone();
                app.actions.push(Action::AddItem(e));
            }
        }
        "ArtMeshColorPreset" => {
            // VTS: loads the recorded multiply/screen preset; triggering it again (or an empty
            // preset) removes the loaded preset.
            let entries: Vec<crate::artmesh::IdValue> = h
                .extra
                .get("ColorScreenMultiplyPreset")
                .and_then(|p| p.get("ArtMeshMultiplyAndScreenColors"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let same = app.hk.color_preset.as_ref().map(|p| p.0 == h.id).unwrap_or(false);
            app.hk.color_preset = if same || entries.is_empty() { None } else { Some((h.id.clone(), entries)) };
            app.apply_artmesh();
        }
        "ToggleTracker" => app.actions.push(if app.tracker.is_some() { Action::StopTracker } else { Action::StartTracker }),
        "ToggleTwitchFeature" => {
            let t = &mut app.settings.twitch;
            match h.file.as_str() {
                "EmoteDropper" => t.dropper.enabled = !t.dropper.enabled,
                "FlyingChat" => t.chat.enabled = !t.chat.enabled,
                _ => t.triggers_on = !t.triggers_on,
            }
        }
        "LoadEffectPreset" => {
            if h.file.is_empty() {
                app.settings.vfx_on = !app.settings.vfx_on;
            } else {
                let path = app.paths.data.join("vfx_presets").join(format!("{}.json", h.file));
                if let Ok(mut v) = crate::formats::read_json::<Vec<crate::vfx::Cfg>>(&path) {
                    crate::vfx::normalize(&mut v);
                    app.settings.vfx = v;
                    app.settings.vfx_on = true;
                }
            }
        }
        "ExecuteItemAction" => {
            // File = "<item folder name>::<hotkey name>" (VTS triggers it in every loaded instance).
            let (item, name) = h.file.split_once("::").unwrap_or(("", h.file.as_str()));
            let mut hit = 0;
            for it in app.items.iter_mut() {
                let folder = item_folder(&it.source);
                if let crate::scene::Body::Live2D(l) = &mut it.body {
                    if item.is_empty() || folder == item {
                        if let Some(ih) = l.vts_hotkeys.iter().find(|x| x.name == name).cloned() {
                            run_on_item(l, &ih);
                            hit += 1;
                        }
                    }
                }
            }
            if hit == 0 {
                app.toast(format!("Live2D-предмет для действия «{}» не загружен", h.file));
            }
        }
        "ToggleUI" => app.ui.visible = !app.ui.visible,
        other => {
            app.toast(format!("Действие «{}» пока не поддерживается в rigrs", action_label(other)));
            return false;
        }
    }

    // Deactivation settings for expressions.
    if h.action == "ToggleExpression" && activated_expression {
        if h.deactivate_after_seconds {
            app.hk.deactivate_at.insert(h.id.clone(), Instant::now() + Duration::from_secs_f32(h.deactivate_after_seconds_amount.max(0.1)));
        }
        if let (true, Source::Key(keys)) = (h.deactivate_after_key_up, &src) {
            app.hk.key_up.insert(h.id.clone(), keys.clone());
        }
    } else if h.action == "ToggleExpression" {
        app.hk.deactivate_at.remove(&h.id);
        app.hk.key_up.remove(&h.id);
    }
    if let Some(undo) = undo {
        let due = Instant::now() + Duration::from_secs_f32(h.twitch_triggers.cooldown_seconds.max(1.0));
        app.hk.resets.retain(|r| r.hotkey != h.id);
        app.hk.resets.push(Reset { hotkey: h.id.clone(), due, undo, hide_dialog: h.twitch_triggers.hide_model_reset_dialog });
    }

    play_sounds(app, &h);

    let a = app.avatar.as_ref();
    let ev = serde_json::json!({
        "hotkeyID": h.id, "hotkeyName": h.name, "hotkeyAction": h.action, "hotkeyFile": h.file, "hotkeyTriggeredByAPI": by_api,
        "modelID": a.map(|a| a.model_id.clone()).unwrap_or_default(), "modelName": a.map(|a| a.name.clone()).unwrap_or_default(), "isLive2DItem": false,
    });
    app.api_events.push(("HotkeyTriggeredEvent".into(), ev));
    true
}

/// Folder name of a Live2D item (how VTS lists item hotkeys).
pub fn item_folder(source: &std::path::Path) -> String {
    source.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Item hotkeys VTS allows to trigger: expressions, remove expressions, animations.
pub fn run_on_item(l: &mut crate::scene::live2d::Live2D, h: &Hotkey) {
    match h.action.as_str() {
        "ToggleExpression" => {
            if let Some(e) = l.expressions.iter_mut().find(|e| e.file.ends_with(&h.file)) {
                e.fade = h.fade_seconds_amount.max(0.0);
                e.active = !e.active;
            }
        }
        "RemoveAllExpressions" => l.expressions.iter_mut().for_each(|e| e.active = false),
        "TriggerAnimation" => {
            l.play_once(&h.file);
        }
        "ChangeIdleAnimation" => l.set_idle((!h.file.is_empty()).then_some(h.file.as_str())),
        _ => {}
    }
}

/// Undoes a toggled hotkey (expression off, …) — used by key-up, timers and gestures.
fn deactivate(app: &mut App, id: &str) {
    let Some(h) = app.hotkeys().iter().find(|h| h.id == id).cloned() else { return };
    if h.action == "ToggleExpression" {
        if let Some(a) = app.avatar.as_mut() {
            if let Some(e) = a.expressions.iter_mut().find(|e| e.file.ends_with(&h.file)) {
                e.fade = h.fade_seconds_amount.max(0.0);
                e.active = false;
            }
        }
    }
    if h.action == "ToggleItemScene" {
        let name = h.file.trim_end_matches(".json").to_string();
        app.unload_scene(&name, h.fade_seconds_amount.max(0.0));
    }
    if h.sound_config.stop_when_inactive {
        stop_sounds(app, &h.id);
    }
}

// ------------------------------------------------------------------ sounds (MCI: wav/mp3)

#[cfg(windows)]
fn mci(cmd: &str) {
    let w: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = windows::Win32::Media::Multimedia::mciSendStringW(windows::core::PCWSTR(w.as_ptr()), None, None);
    }
}

#[cfg(not(windows))]
fn mci(_cmd: &str) {}

pub fn sounds_dir(app: &App) -> std::path::PathBuf {
    app.paths.data.join("Sounds")
}

fn play_sounds(app: &mut App, h: &Hotkey) {
    if !h.sound_config.sound_active || h.sound_config.sound_slots.is_empty() {
        return;
    }
    let dir = sounds_dir(app);
    for s in &h.sound_config.sound_slots {
        let p = if std::path::Path::new(&s.file).is_absolute() { std::path::PathBuf::from(&s.file) } else { dir.join(&s.file) };
        if !p.exists() {
            continue;
        }
        app.hk.sound_seq += 1;
        let alias = format!("rigrs{}", app.hk.sound_seq);
        mci(&format!("open \"{}\" type mpegvideo alias {alias}", p.display()));
        mci(&format!("setaudio {alias} volume to {}", (s.volume.clamp(0.0, 1.0) * 1000.0) as i32));
        mci(&format!("play {alias}"));
        app.hk.sounds.push((h.id.clone(), alias));
    }
    // Close finished ones lazily (keep the list short).
    if app.hk.sounds.len() > 32 {
        let old: Vec<_> = app.hk.sounds.drain(..16).collect();
        for (_, a) in old {
            mci(&format!("close {a}"));
        }
    }
}

fn stop_sounds(app: &mut App, id: &str) {
    let (stop, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut app.hk.sounds).into_iter().partition(|s| s.0 == id);
    app.hk.sounds = keep;
    for (_, a) in stop {
        mci(&format!("stop {a}"));
        mci(&format!("close {a}"));
    }
}

// ------------------------------------------------------------------ Twitch → hotkeys

/// A Twitch event that can trigger hotkeys.
#[derive(Clone, Debug)]
pub enum TwitchEvent {
    Chat { user: String, user_id: String, text: String, broadcaster: bool, sub: bool, mod_: bool, vip: bool, artist: bool },
    Bits { user: String, amount: u32 },
    GiftSub { user: String, count: u32 },
    Sub { user: String },
    Follow { user: String, user_id: String },
    Redeem { user: String, reward_id: String, reward_title: String },
    PowerUp { user: String, kind: String },
    Raid { from: String, viewers: u32 },
    Shoutout { to: String },
    AdBreak { seconds: u32 },
}

impl TwitchEvent {
    pub fn describe(&self) -> String {
        match self {
            TwitchEvent::Chat { user, text, .. } => format!("{user}: {text}"),
            TwitchEvent::Bits { user, amount } => format!("{user} — {amount} битс"),
            TwitchEvent::GiftSub { user, count } => format!("{user} подарил {count} подписок"),
            TwitchEvent::Sub { user } => format!("{user} оформил подписку"),
            TwitchEvent::Follow { user, .. } => format!("{user} отслеживает канал"),
            TwitchEvent::Redeem { user, reward_title, .. } => format!("{user} активировал награду «{reward_title}»"),
            TwitchEvent::PowerUp { user, kind } => format!("{user}: Power-Up {kind}"),
            TwitchEvent::Raid { from, viewers } => format!("Рейд от {from} ({viewers} зрителей)"),
            TwitchEvent::Shoutout { to } => format!("Отметка канала {to}"),
            TwitchEvent::AdBreak { seconds } => format!("Рекламная пауза {seconds} с"),
        }
    }
}

/// Does this trigger block react to the event?
pub fn matches(t: &TwitchTriggers, ev: &TwitchEvent, repeated_follow: bool) -> bool {
    let in_range = |r: &Range, v: f32| v >= r.min && (r.max >= 1e6 || v <= r.max);
    match ev {
        TwitchEvent::Chat { user, user_id, text, broadcaster, sub, mod_, vip, artist } => {
            if !t.text || t.text_words.trim().is_empty() {
                return false;
            }
            let lower = text.to_lowercase();
            let words: Vec<String> = t.text_words.split([',', ';', '\n']).map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
            let hit = words.iter().any(|w| lower.split_whitespace().any(|x| x == w) || (w.contains(' ') && lower.contains(w.as_str())));
            if !hit {
                return false;
            }
            if !t.restrict_text {
                return true;
            }
            let r = &t.restrict;
            (r.you && *broadcaster)
                || (r.subs && *sub)
                || (r.mods && *mod_)
                || (r.vips && *vip)
                || (r.artists && *artist)
                || (r.by_name && r.user_names.iter().any(|n| n.eq_ignore_ascii_case(user)))
                || (r.by_id && r.user_ids.iter().any(|n| n == user_id))
        }
        TwitchEvent::Bits { amount, .. } => t.bits && in_range(&t.bit_range, *amount as f32),
        TwitchEvent::GiftSub { count, .. } => t.gift_sub && in_range(&t.gift_range, *count as f32),
        TwitchEvent::Sub { .. } => t.sub,
        TwitchEvent::Follow { .. } => t.follow && (!repeated_follow || t.allow_repeated_follow),
        TwitchEvent::Redeem { reward_id, reward_title, .. } => {
            t.redeem && ((!t.redeem_id.is_empty() && t.redeem_id == *reward_id) || (t.redeem_id.is_empty() && !t.redeem_name.is_empty() && t.redeem_name.eq_ignore_ascii_case(reward_title)))
        }
        TwitchEvent::PowerUp { kind, .. } => t.power_up && (t.power_up_id.is_empty() || t.power_up_id == *kind),
        TwitchEvent::Raid { .. } => t.raid,
        TwitchEvent::Shoutout { .. } => t.shoutout,
        TwitchEvent::AdBreak { .. } => t.ad_break,
    }
}

/// Fires every hotkey whose Twitch triggers match the event (respecting cooldowns).
pub fn on_twitch(app: &mut App, ev: &TwitchEvent) {
    if !app.settings.twitch.triggers_on {
        return;
    }
    let repeated = match ev {
        TwitchEvent::Follow { user_id, .. } => !app.hk.followers.insert(user_id.clone()),
        _ => false,
    };
    let list: Vec<Hotkey> = app.hotkeys().to_vec();
    for h in list.iter().filter(|h| h.is_active && h.twitch_triggers.active) {
        let t = &h.twitch_triggers;
        if !matches(t, ev, repeated) {
            continue;
        }
        if t.cooldown_active {
            if let Some(last) = app.hk.cooldown.get(&h.id) {
                if last.elapsed().as_secs_f32() < t.cooldown_seconds {
                    continue;
                }
            }
        }
        app.hk.cooldown.insert(h.id.clone(), Instant::now());
        if app.settings.twitch.trigger_log {
            app.log(&format!("twitch → hotkey «{}»: {}", h.name, ev.describe()));
        }
        if app.settings.twitch.trigger_notify {
            let name = if h.name.is_empty() { action_label(&h.action) } else { h.name.clone() };
            app.toast(format!("Twitch: {} → «{name}»", ev.describe()));
        }
        trigger(app, &h.id, Source::Twitch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(text: &str, mod_: bool) -> TwitchEvent {
        TwitchEvent::Chat { user: "u".into(), user_id: "1".into(), text: text.into(), broadcaster: false, sub: false, mod_, vip: false, artist: false }
    }

    #[test]
    fn text_trigger_and_restriction() {
        let mut t = TwitchTriggers { text: true, text_words: "!dance, hello world".into(), ..Default::default() };
        assert!(matches(&t, &chat("let's !dance now", false), false));
        assert!(matches(&t, &chat("Hello World everyone", false), false));
        assert!(!matches(&t, &chat("!dancing", false), false));
        t.restrict_text = true;
        t.restrict.you = false;
        t.restrict.mods = true;
        assert!(!matches(&t, &chat("!dance", false), false));
        assert!(matches(&t, &chat("!dance", true), false));
    }

    #[test]
    fn ranges_and_follows() {
        let t = TwitchTriggers { bits: true, gift_sub: true, follow: true, ..Default::default() };
        assert!(matches(&t, &TwitchEvent::Bits { user: "a".into(), amount: 100 }, false));
        assert!(!matches(&t, &TwitchEvent::Bits { user: "a".into(), amount: 99 }, false));
        assert!(matches(&t, &TwitchEvent::GiftSub { user: "a".into(), count: 5 }, false));
        assert!(matches(&t, &TwitchEvent::Follow { user: "a".into(), user_id: "1".into() }, false));
        assert!(!matches(&t, &TwitchEvent::Follow { user: "a".into(), user_id: "1".into() }, true));
    }

    #[test]
    fn vts_hotkey_json_roundtrip() {
        let j = r#"{"HotkeyID":"dc085967d13543208aa97aa0b4e8d2e9","Name":"Dance F1","Action":"TriggerAnimation","File":"dancedance.motion3.json","Triggers":{"Trigger1":"F1","Trigger2":"","Trigger3":"","ScreenButton":-1},"TwitchTriggers":{"Active":true,"Active_BitTrigger":true,"Trigger_BitTrigger":{"Min":100.0,"Max":3000.0}},"IsGlobal":true,"IsActive":true,"SomethingNew":{"X":1}}"#;
        let h: Hotkey = serde_json::from_str(j).unwrap();
        assert_eq!(h.triggers.list(), vec!["F1"]);
        assert!(h.twitch_triggers.active && h.twitch_triggers.bits);
        assert_eq!(h.twitch_triggers.bit_range.max, 3000.0);
        let back = serde_json::to_value(&h).unwrap();
        assert_eq!(back["SomethingNew"]["X"], 1);
        assert_eq!(back["HotkeyID"], "dc085967d13543208aa97aa0b4e8d2e9");
    }
}
