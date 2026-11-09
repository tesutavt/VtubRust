//! egui interface: left toolbar, bottom pickers, dialogs, settings panel, scene mouse input.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Align2, Color32, Context, FontId, Pos2, RichText, Sense, Stroke, Vec2};

use crate::app::{Action, App};
use crate::i18n::tr;
use crate::library::ItemEntry;
use crate::mapping::{Rule, INPUT_NAMES};
use crate::scene::live2d::list_motions;
use crate::settings::{Background, FpsMode};

#[derive(PartialEq, Clone, Copy)]
pub enum Strip {
    None,
    Models,
    Items,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Dialog {
    None,
    Backgrounds,
    Scenes,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    General,
    Camera,
    Model,
    Hotkeys,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Avatar { rotate: bool },
    Item { id: u64, rotate: bool, repin: bool },
}

pub struct UiState {
    pub visible: bool,
    pub strip: Strip,
    last_strip: Strip,
    pub dialog: Dialog,
    pub settings_open: bool,
    /// VNet-style collab: setup screen (big button) and session control screen (small button).
    pub collab_open: bool,
    pub collab_control: bool,
    pub tab: Tab,
    pub filter: String,
    pub selected: Option<u64>,
    pub screenshot: Option<bool>,
    thumbs: HashMap<PathBuf, Option<egui::TextureHandle>>,
    thumb_loader: Option<ThumbLoader>,
    /// Item picker rows, built once when the picker opens.
    pub picker_rows: Option<Vec<crate::ui_items::PickRow>>,
    drag: Option<Drag>,
    recording: Option<usize>,
    scene_name: String,
    motions_cache: Option<(PathBuf, Vec<String>)>,
    scroll_req: Option<f32>,
    /// Dev hook: scroll applied once after the settings panel has opened.
    pub pending_scroll: Option<f32>,
    pub modal: Option<Modal>,
    info_next: Option<Modal>,
    info_dont_show: bool,
    pub collapsed: std::collections::HashSet<usize>,
    vfx_save_open: bool,
    vfx_preset_name: String,
    /// Last shown (settings tab, modal) — used to reset scrolling on change.
    shown: (Option<Tab>, Option<Modal>),
    cjk_fonts: bool,
    pub modal_sel: Option<usize>,
    /// Item waiting for its one-time setup (VTS item setup before spawning).
    pub item_setup: Option<crate::ui_items::ItemSetup>,
    /// Item whose config menu is open (long press on the item, like in VTS).
    pub item_menu: Option<u64>,
    pub item_picker: bool,
    pub item_global: bool,
    /// Picker filter: 0 = all, 1 = animated, 2 = Live2D.
    pub item_kind: u8,
    pub item_sub: crate::ui_items::ItemSub,
    /// Press on an item: (item id, time, position) — for the long press.
    item_press: Option<(u64, std::time::Instant, Pos2)>,
    pub typing: bool,
    modal_filter: String,
    /// Choices made so far in the camera wizard: camera, (w, h), fps.
    wiz_cam: i32,
    wiz_res: (u32, u32),
    wiz_fps: u32,
    win_size: [u32; 2],
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            visible: true,
            strip: Strip::None,
            last_strip: Strip::None,
            dialog: Dialog::None,
            settings_open: false,
            collab_open: false,
            collab_control: false,
            tab: Tab::General,
            filter: String::new(),
            selected: None,
            screenshot: None,
            thumbs: HashMap::new(),
            thumb_loader: None,
            picker_rows: None,
            drag: None,
            recording: None,
            scene_name: String::new(),
            motions_cache: None,
            scroll_req: None,
            pending_scroll: None,
            modal: None,
            info_next: None,
            info_dont_show: false,
            collapsed: std::collections::HashSet::new(),
            vfx_save_open: false,
            vfx_preset_name: String::new(),
            shown: (None, None),
            cjk_fonts: false,
            modal_sel: None,
            item_setup: None,
            item_menu: None,
            item_picker: false,
            item_global: false,
            item_kind: 0,
            item_sub: crate::ui_items::ItemSub::Main,
            item_press: None,
            typing: false,
            modal_filter: String::new(),
            wiz_cam: 0,
            wiz_res: (0, 0),
            wiz_fps: 0,
            win_size: [0, 0],
        }
    }
}

const C_MODEL: Color32 = Color32::from_rgb(232, 112, 104);
const C_BG: Color32 = Color32::from_rgb(110, 196, 98);
const C_LOCK: Color32 = Color32::from_rgb(226, 182, 48);
const C_SET: Color32 = Color32::from_rgb(214, 88, 160);
const C_HIDE: Color32 = Color32::from_rgb(120, 126, 150);

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF))
}

/// Adds Windows system fonts as fallbacks: Segoe UI Symbol (●, arrows, …) always,
/// a CJK font only when needed (it costs ~20 MB of RAM).
pub fn install_fonts(ctx: &Context, cjk: bool) {
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, paths: &[&str]| {
        for p in paths {
            if let Ok(b) = std::fs::read(p) {
                fonts.font_data.insert(name.to_string(), egui::FontData::from_owned(b));
                for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(fam).or_default().push(name.to_string());
                }
                return;
            }
        }
    };
    add("symbols", &[r"C:\Windows\Fonts\seguisym.ttf"]);
    if cjk {
        add("cjk", &[r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\msgothic.ttc", r"C:\Windows\Fonts\simsun.ttc"]);
    }
    ctx.set_fonts(fonts);
}

/// Installs the CJK fallback once any visible name needs it.
fn ensure_fonts(app: &mut App, ctx: &Context) {
    if app.ui.cjk_fonts {
        return;
    }
    let need = app.avatar.as_ref().map(|a| has_cjk(&a.name) || has_cjk(&a.model3.to_string_lossy())).unwrap_or(false)
        || app.library.models.iter().any(|m| has_cjk(&m.name))
        || app.library.items.iter().any(|i| has_cjk(i.name()));
    if need {
        install_fonts(ctx, true);
        app.ui.cjk_fonts = true;
    }
}

pub fn setup_style(ctx: &Context, scale: f32) {
    let mut v = egui::Visuals::light();
    v.window_rounding = 12.0.into();
    v.menu_rounding = 8.0.into();
    v.widgets.inactive.rounding = 8.0.into();
    v.widgets.hovered.rounding = 8.0.into();
    v.widgets.active.rounding = 8.0.into();
    v.selection.bg_fill = Color32::from_rgb(40, 140, 230);
    ctx.set_visuals(v);
    ctx.set_zoom_factor(scale.clamp(0.5, 3.0));
}

pub(crate) fn lighten(c: Color32, t: f32) -> Color32 {
    let l = |x: u8| (x as f32 + (255.0 - x as f32) * t) as u8;
    Color32::from_rgb(l(c.r()), l(c.g()), l(c.b()))
}

pub(crate) fn round_button(ui: &mut egui::Ui, icon: &str, color: Color32, radius: f32, active: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(radius * 2.0), Sense::click());
    let c = if resp.hovered() { lighten(color, 0.18) } else { color };
    let p = ui.painter();
    p.circle_filled(rect.center() + Vec2::new(0.0, 2.0), radius, Color32::from_black_alpha(50));
    p.circle_filled(rect.center(), radius, c);
    if active {
        p.circle_stroke(rect.center(), radius - 2.0, Stroke::new(3.0, Color32::WHITE));
    }
    p.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(radius * 0.95), Color32::WHITE);
    resp.on_hover_text(tr(tip))
}

impl UiState {
    /// Forces a rescan of the model's motion files (e.g. after a recording).
    pub fn invalidate_motions(&mut self) {
        self.motions_cache = None;
    }

    pub fn thumb_pub(&mut self, ctx: &Context, path: &Path, budget: &mut u32) -> Option<egui::TextureId> {
        self.thumb(ctx, path, budget)
    }

    /// Thumbnail texture; decoding happens on background threads, `None` until it is ready.
    fn thumb(&mut self, ctx: &Context, path: &Path, _budget: &mut u32) -> Option<egui::TextureId> {
        let loader = self.thumb_loader.get_or_insert_with(|| ThumbLoader::new(ctx.clone()));
        while let Ok((p, img)) = loader.rx.try_recv() {
            loader.pending.remove(&p);
            let tex = img.map(|ci| ctx.load_texture(p.to_string_lossy(), ci, egui::TextureOptions::LINEAR));
            self.thumbs.insert(p, tex);
        }
        if let Some(t) = self.thumbs.get(path) {
            return t.as_ref().map(|t| t.id());
        }
        if loader.pending.insert(path.to_path_buf()) {
            let _ = loader.tx.send(path.to_path_buf());
        }
        None
    }
}

struct ThumbLoader {
    tx: std::sync::mpsc::Sender<PathBuf>,
    rx: std::sync::mpsc::Receiver<(PathBuf, Option<egui::ColorImage>)>,
    pending: std::collections::HashSet<PathBuf>,
}

impl ThumbLoader {
    fn new(ctx: Context) -> Self {
        let (tx, jobs) = std::sync::mpsc::channel::<PathBuf>();
        let (done, rx) = std::sync::mpsc::channel();
        let jobs = std::sync::Arc::new(std::sync::Mutex::new(jobs));
        for k in 0..2 {
            let (jobs, done, ctx) = (jobs.clone(), done.clone(), ctx.clone());
            let _ = std::thread::Builder::new().name(format!("thumbs-{k}")).spawn(move || loop {
                let Ok(p) = jobs.lock().map_err(|_| ()).and_then(|j| j.recv().map_err(|_| ())) else { break };
                let img = image::open(&p).ok().map(|img| {
                    let t = img.thumbnail(128, 128).to_rgba8();
                    egui::ColorImage::from_rgba_unmultiplied([t.width() as usize, t.height() as usize], t.as_raw())
                });
                if done.send((p, img)).is_err() {
                    break;
                }
                ctx.request_repaint();
            });
        }
        ThumbLoader { tx, rx, pending: Default::default() }
    }
}

fn card(ui: &mut egui::Ui, label: &str, tex: Option<egui::TextureId>, placeholder: &str, highlight: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(104.0, 126.0), Sense::click());
    let img = egui::Rect::from_min_size(rect.min + Vec2::new(4.0, 2.0), Vec2::splat(96.0));
    let p = ui.painter();
    let bg = if resp.hovered() { Color32::from_rgb(70, 110, 140) } else { Color32::from_rgb(52, 78, 98) };
    p.rect_filled(img.expand(2.0), 14.0, if highlight { Color32::from_rgb(255, 196, 64) } else { bg });
    match tex {
        Some(t) => {
            p.image(t, img, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            p.rect_filled(img, 12.0, Color32::from_rgb(150, 150, 158));
            p.text(img.center(), Align2::CENTER_CENTER, placeholder, FontId::proportional(18.0), Color32::WHITE);
        }
    }
    let mut text = label.to_string();
    if text.chars().count() > 14 {
        text = text.chars().take(13).collect::<String>() + "…";
    }
    p.text(Pos2::new(rect.center().x, rect.max.y - 11.0), Align2::CENTER_CENTER, text, FontId::proportional(13.0), Color32::WHITE);
    resp.on_hover_text(label)
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub fn run(app: &mut App, ctx: &Context) {
    ensure_fonts(app, ctx);
    hotkeys(app, ctx);
    scene_input(app, ctx);
    let vis = app.ui.visible;
    let e_bar = ease(ctx.animate_bool_with_time(egui::Id::new("anim-bar"), vis && !app.ui.settings_open, 0.3));
    let e_set = ease(ctx.animate_bool_with_time(egui::Id::new("anim-set"), vis && app.ui.settings_open, 0.3));
    let e_strip = ease(ctx.animate_bool_with_time(egui::Id::new("anim-strip"), vis && !app.ui.settings_open && app.ui.strip != Strip::None, 0.25));
    if app.ui.strip != Strip::None {
        app.ui.last_strip = app.ui.strip;
    }
    if e_bar > 0.0 {
        toolbar(app, ctx, e_bar);
    }
    if e_strip > 0.0 {
        match app.ui.last_strip {
            Strip::Models => models_strip(app, ctx, e_strip),
            Strip::Items => items_strip(app, ctx, e_strip),
            Strip::None => {}
        }
    }
    if vis && !app.ui.settings_open {
        match app.ui.dialog {
            Dialog::Backgrounds => backgrounds_dialog(app, ctx),
            Dialog::Scenes => scenes_dialog(app, ctx),
            Dialog::None => {}
        }
        item_panel(app, ctx);
        crate::ui_net::collab_window(app, ctx);
    }
    let now_tab = (vis && app.ui.settings_open).then_some(app.ui.tab);
    if now_tab.is_some() && now_tab != app.ui.shown.0 {
        app.ui.scroll_req = Some(0.0);
    }
    app.ui.shown.0 = now_tab;
    if app.ui.scroll_req.is_none() && app.ui.shown.0.is_some() {
        if let Some(px) = app.ui.pending_scroll.take() {
            app.ui.scroll_req = Some(px);
        }
    }
    if e_set > 0.0 {
        settings_panel(app, ctx, e_set);
    }
    if app.ui.modal.is_some() {
        modal_dialog(app, ctx);
    } else {
        app.ui.shown.1 = None;
    }
    if app.settings.show_stats {
        stats_overlay(app, ctx);
    }
    if app.settings.tobii_enabled && app.settings.tobii_show_gaze {
        if let (Some(g), Some(mon), Ok(wp)) = (app.tobii_state.gaze, app.monitors.first(), app.window.inner_position()) {
            let ppp = ctx.pixels_per_point();
            let sx = mon[0] as f32 + g[0] * (mon[2] - mon[0]) as f32 - wp.x as f32;
            let sy = mon[1] as f32 + g[1] * (mon[3] - mon[1]) as f32 - wp.y as f32;
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("gaze")));
            p.circle_stroke(Pos2::new(sx / ppp, sy / ppp), 14.0, Stroke::new(3.0, Color32::from_rgb(240, 70, 80)));
        }
    }
    crate::ui_hotkeys::screen_buttons(app, ctx);
    crate::ui_hotkeys::reset_notice(app, ctx);
    crate::ui_hotkeys::mesh_select(app, ctx);
    crate::ui_vts::dialogs(app, ctx);
    crate::api_events::draw_debug(app, ctx);
    let dt = app.ui_dt;
    crate::twitch_fx::run(app, ctx, dt);
    crate::ui_hotkeys::api_prompt(app, ctx);
    toasts(app, ctx);
    app.ui.typing = ctx.output(|o| o.ime.is_some());
    if let Some(e) = &app.core_error {
        egui::Area::new(egui::Id::new("core-err")).anchor(Align2::CENTER_TOP, [0.0, 12.0]).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.colored_label(Color32::RED, "Cubism Core не загружен");
                ui.label(e);
                ui.label(tr("Настройки → Общее → путь к Live2DCubismCore.dll"));
            });
        });
    }
}

// ---------------------------------------------------------------------- scene input

fn scene_input(app: &mut App, ctx: &Context) {
    let ppp = ctx.pixels_per_point();
    let (vw, vh) = app.viewport();
    let to_world = |p: Pos2| [p.x * ppp - vw / 2.0, vh / 2.0 - p.y * ppp];
    let (pos, pp, sp, down, released, delta, scroll, dbl) = ctx.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.primary_pressed(),
            i.pointer.secondary_pressed(),
            i.pointer.primary_down() || i.pointer.secondary_down(),
            i.pointer.any_released(),
            i.pointer.delta(),
            i.raw_scroll_delta.y + i.raw_scroll_delta.x,
            i.pointer.button_double_clicked(egui::PointerButton::Primary),
        )
    });
    let over_ui = ctx.is_pointer_over_area();
    let ctrl = ctx.input(|i| i.modifiers.ctrl);

    if let Some(Drag::Item { id, .. }) = app.ui.drag {
        let screen = ctx.screen_rect();
        let trash = egui::Rect::from_center_size(screen.right_bottom() - Vec2::new(70.0, 70.0), Vec2::splat(84.0));
        let over = pos.map(|p| trash.contains(p)).unwrap_or(false);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("trash")));
        painter.circle_filled(trash.center(), 40.0, if over { Color32::from_rgb(236, 80, 96) } else { Color32::from_black_alpha(150) });
        painter.text(trash.center(), Align2::CENTER_CENTER, "🗑", FontId::proportional(38.0), Color32::WHITE);
        if over && (released || !down) {
            app.items.retain(|i| i.id != id);
            app.ui.selected = None;
            app.ui.drag = None;
            app.save();
            return;
        }
    }
    if let (Some((id, t0, p0)), Some(p)) = (app.ui.item_press, pos) {
        if !down {
            app.ui.item_press = None;
        } else if (p - p0).length() > 8.0 {
            app.ui.item_press = None;
        } else if t0.elapsed().as_secs_f32() >= 0.8 {
            app.ui.item_press = None;
            app.ui.item_menu = Some(id);
            app.ui.selected = Some(id);
        }
    }
    if let Some(d) = app.ui.drag {
        if down {
            let dw = [delta.x * ppp, -delta.y * ppp];
            match d {
                Drag::Avatar { rotate } => {
                    if let Some(a) = &mut app.avatar {
                        if rotate {
                            a.xf.rot -= delta.x * 0.4;
                        } else {
                            a.xf.pos[0] += dw[0];
                            a.xf.pos[1] += dw[1];
                        }
                    }
                }
                Drag::Item { id, rotate, .. } => {
                    if let Some(it) = app.items.iter_mut().find(|i| i.id == id) {
                        if rotate {
                            it.xf.rot -= delta.x * 0.4;
                        } else {
                            it.xf.pos[0] += dw[0];
                            it.xf.pos[1] += dw[1];
                        }
                    }
                }
            }
        }
        if released || !down {
            if let Drag::Item { id, .. } = d {
                let (vw, vh) = app.viewport();
                let off_screen = app.items.iter().find(|i| i.id == id).map(|i| i.xf.pos[0].abs() > vw / 2.0 || i.xf.pos[1].abs() > vh / 2.0).unwrap_or(false);
                if off_screen {
                    // VTS: dropping an item off-screen deletes it.
                    app.items.retain(|i| i.id != id);
                    app.ui.selected = None;
                } else if let Some(k) = app.items.iter().position(|i| i.id == id) {
                    let details = app.model_key().and_then(|key| app.settings.model_artmesh.get(&key).cloned());
                    let notify_all = app.settings.item_show_pin_notifications;
                    let mut eaten = false;
                    let mut note = None;
                    if let Some(a) = &app.avatar {
                        let det = details.as_ref().or(a.vts_artmesh.as_ref());
                        let it = &mut app.items[k];
                        // VTS: ArtMeshes that "delete items dropped on them" (e.g. a mouth that eats items).
                        if let Some(pk) = a.pick(it.xf.pos) {
                            if det.map(|d| d.deletes(&a.model.drawables[pk.drawable].id)).unwrap_or(false) {
                                eaten = true;
                            }
                        }
                        if !eaten {
                            // VTS: an item dropped onto the model sticks to the uppermost allowed ArtMesh under its centre.
                            let was = it.pin.is_some();
                            it.pin = None;
                            let mesh = it.pin_to_filtered(a, det);
                            if mesh.is_none() && a.pick(it.xf.pos).is_some() && it.cfg.pin_only_layer != "__never" {
                                note = Some(tr("Не закреплено: прикрепление к этой части модели отключено (Настроить модель → Отключить прикрепление к сеткам ArtMesh)").to_string());
                            } else if it.cfg.pin_notify || notify_all {
                                note = match (&mesh, was) {
                                    (Some(m), _) => Some(format!("{} {m}", tr("Закреплено к слою"))),
                                    (None, true) => Some(tr("Предмет откреплён").to_string()),
                                    _ => None,
                                };
                            }
                        }
                    }
                    if eaten {
                        app.items.remove(k);
                        app.ui.selected = None;
                    } else if dbl {
                        // VTS: double-click an item to lock / unlock it.
                        app.items[k].locked = !app.items[k].locked;
                    }
                    if let Some(n) = note {
                        app.toast(n);
                    }
                }
            }
            app.ui.drag = None;
            app.save();
        }
        return;
    }
    if over_ui {
        return;
    }
    let Some(pos) = pos else { return };
    let w = to_world(pos);

    // Topmost thing under the cursor.
    enum Hit {
        Item(usize),
        Avatar,
        Nothing,
    }
    let front = app.items.iter().enumerate().rev().filter(|(_, i)| i.front()).find(|(_, i)| i.hit(w)).map(|(k, _)| k);
    let hit = if let Some(k) = front {
        Hit::Item(k)
    } else if app.avatar.as_ref().map(|a| a.pick(w).is_some()).unwrap_or(false) {
        Hit::Avatar
    } else if let Some(k) = app.items.iter().enumerate().rev().filter(|(_, i)| !i.front()).find(|(_, i)| i.hit(w)).map(|(k, _)| k) {
        Hit::Item(k)
    } else {
        Hit::Nothing
    };

    // ArtMeshSelectionRequest: a click on the model filters the list to the meshes under it.
    if pp {
        if let (Some(sel), Some(a)) = (app.mesh_select.as_mut(), app.avatar.as_ref()) {
            let under: Vec<String> = a.pick_all(w).into_iter().map(|d| a.model.drawables[d].id.clone()).collect();
            if !under.is_empty() {
                sel.at_click = Some(under);
                return;
            }
        }
    }
    if pp || sp {
        let item_id = if let Hit::Item(k) = hit { Some(app.items[k].id) } else { None };
        crate::api::on_click(app, w, [pos.x * ppp, pos.y * ppp], if pp { 0 } else { 1 }, item_id);
        match hit {
            Hit::Item(k) => {
                let it = &mut app.items[k];
                app.ui.selected = Some(it.id);
                if it.locked && pp {
                    app.ui.item_press = Some((it.id, std::time::Instant::now(), pos));
                }
                if ctrl && pp && !it.locked {
                    // VTS: Ctrl + click flips an item.
                    it.xf.flip = !it.xf.flip;
                } else if !it.locked {
                    app.ui.item_press = Some((it.id, std::time::Instant::now(), pos));
                    let repin = it.pin.take().is_some();
                    app.ui.drag = Some(Drag::Item { id: it.id, rotate: sp, repin });
                }
            }
            Hit::Avatar => {
                app.ui.selected = None;
                if !app.settings.avatar_locked {
                    app.ui.drag = Some(Drag::Avatar { rotate: sp });
                }
            }
            Hit::Nothing => app.ui.selected = None,
        }
    }
    // Locked items aren't dragged, so their double click arrives here: unlock (VTS).
    if dbl {
        if let Hit::Item(k) = hit {
            if app.items[k].locked {
                app.items[k].locked = false;
                app.save();
            }
        }
    }
    // egui reports a double click on the release of the second click.
    if dbl && matches!(hit, Hit::Nothing) {
        app.ui.visible = !app.ui.visible;
    }
    if scroll.abs() > 0.0 {
        let k = 1.1f32.powf(scroll / 50.0);
        match hit {
            Hit::Item(i) => {
                let it = &mut app.items[i];
                if !it.locked {
                    it.xf.scale *= k;
                    if let Some(p) = &mut it.pin {
                        p.scale_ratio *= k;
                    }
                }
            }
            _ => {
                if let (Some(a), false) = (&mut app.avatar, app.settings.avatar_locked) {
                    a.xf.pos = [w[0] + (a.xf.pos[0] - w[0]) * k, w[1] + (a.xf.pos[1] - w[1]) * k];
                    a.xf.scale *= k;
                }
            }
        }
    }
}

fn hotkeys(app: &mut App, ctx: &Context) {
    if !app.ui.visible && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.visible = true;
    }
    // Only a focused text field blocks hotkeys (egui also focuses buttons on Tab).
    let typing = app.window.has_focus() && app.ui.typing;
    crate::hotkeys::update(app, typing);
}

// ---------------------------------------------------------------------- toolbar

fn toolbar(app: &mut App, ctx: &Context, e: f32) {
    egui::Area::new(egui::Id::new("toolbar")).anchor(Align2::LEFT_TOP, [14.0 - (1.0 - e) * 220.0, 14.0]).show(ctx, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(10.0, 14.0);
        let ui_state = &mut app.ui;
        ui.horizontal(|ui| {
            if round_button(ui, "👤", C_MODEL, 30.0, ui_state.strip == Strip::Models, "Модели").clicked() {
                ui_state.strip = if ui_state.strip == Strip::Models { Strip::None } else { Strip::Models };
            }
            let r = round_button(ui, "📷", lighten(C_MODEL, 0.1), 20.0, false, "Скриншот (ПКМ — с прозрачным фоном)");
            if r.clicked() {
                ui_state.screenshot = Some(false);
            }
            if r.secondary_clicked() {
                ui_state.screenshot = Some(true);
            }
        });
        ui.horizontal(|ui| {
            if round_button(ui, "🖼", C_BG, 30.0, ui_state.dialog == Dialog::Backgrounds, "Фон").clicked() {
                ui_state.dialog = if ui_state.dialog == Dialog::Backgrounds { Dialog::None } else { Dialog::Backgrounds };
            }
            if round_button(ui, "★", lighten(C_BG, 0.1), 20.0, ui_state.strip == Strip::Items, "Предметы").clicked() {
                ui_state.strip = Strip::None;
                ui_state.item_picker = true;
            }
            if round_button(ui, "★★", lighten(C_BG, 0.2), 20.0, ui_state.dialog == Dialog::Scenes, "Сцены предметов и хоткеи").clicked() {
                ui_state.dialog = if ui_state.dialog == Dialog::Scenes { Dialog::None } else { Dialog::Scenes };
            }
        });
        ui.horizontal(|ui| {
            let locked = app.settings.avatar_locked;
            if round_button(ui, if locked { "🔒" } else { "🔓" }, C_LOCK, 30.0, locked, "Заблокировать перемещение модели").clicked() {
                app.settings.avatar_locked = !locked;
            }
            if round_button(ui, "↺", lighten(C_LOCK, 0.1), 20.0, false, "Разблокировать всё").clicked() {
                app.settings.avatar_locked = false;
                for it in &mut app.items {
                    it.locked = false;
                }
                app.toast("Модель и все предметы разблокированы");
            }
        });
        ui.horizontal(|ui| {
            if round_button(ui, "⚙", C_SET, 30.0, app.ui.settings_open, "Настройки").clicked() {
                app.ui.settings_open = !app.ui.settings_open;
            }
        });
        ui.horizontal(|ui| {
            crate::ui_net::main_buttons(app, ui);
        });
        ui.horizontal(|ui| {
            if round_button(ui, "👁", C_HIDE, 30.0, false, "Скрыть интерфейс (вернуть: Esc или двойной клик по пустому месту)").clicked() {
                app.ui.visible = false;
            }
        });
    });
}

// ---------------------------------------------------------------------- bottom strips

fn filter_row(ui: &mut egui::Ui, filter: &mut String) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(filter).hint_text(tr("Фильтр")).desired_width(300.0));
        if ui.button("✖").clicked() {
            filter.clear();
        }
    });
}

fn models_strip(app: &mut App, ctx: &Context, e: f32) {
    let frame = egui::Frame::none().fill(Color32::from_rgb(22, 52, 66)).inner_margin(8.0);
    let w = ctx.screen_rect().width();
    egui::Area::new(egui::Id::new("models-strip")).anchor(Align2::LEFT_BOTTOM, [0.0, (1.0 - e) * 200.0]).show(ctx, |ui| frame.show(ui, |ui| {
        ui.set_width(w - 16.0);
        filter_row(ui, &mut app.ui.filter);
        let f = app.ui.filter.to_lowercase();
        let mut budget = 3;
        let current = app.model_key();
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                if card(ui, "Добавить модель", None, "+", false).clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_title("Папка с моделью Live2D").pick_folder() {
                        app.actions.push(Action::AddModelFolder(p));
                    }
                }
                if card(ui, "Убрать модель", None, "✖", false).clicked() {
                    app.actions.push(Action::UnloadModel);
                }
                let models = app.library.models.clone();
                for m in models.iter().filter(|m| f.is_empty() || m.name.to_lowercase().contains(&f)) {
                    let tex = m.icon.as_ref().and_then(|p| app.ui.thumb(ctx, p, &mut budget));
                    let cur = current.as_deref() == Some(&*m.model3.to_string_lossy());
                    if card(ui, &m.name, tex, "No icon", cur).clicked() {
                        app.actions.push(Action::LoadModel(m.clone()));
                    }
                }
            });
        });
        if budget == 0 {
            ctx.request_repaint();
        }
    }));
}

fn items_strip(app: &mut App, ctx: &Context, e: f32) {
    let frame = egui::Frame::none().fill(Color32::from_rgb(28, 60, 44)).inner_margin(8.0);
    let w = ctx.screen_rect().width();
    egui::Area::new(egui::Id::new("items-strip")).anchor(Align2::LEFT_BOTTOM, [0.0, (1.0 - e) * 200.0]).show(ctx, |ui| frame.show(ui, |ui| {
        ui.set_width(w - 16.0);
        filter_row(ui, &mut app.ui.filter);
        let f = app.ui.filter.to_lowercase();
        let mut budget = 3;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                if card(ui, "Добавить файл", None, "+", false).clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Картинки / GIF / Live2D", &["png", "jpg", "jpeg", "gif", "json"])
                        .pick_file()
                    {
                        app.ui.item_setup = Some(crate::ui_items::ItemSetup::new(p, &app.settings));
                    }
                }
                if card(ui, "Убрать все", None, "✖", false).clicked() {
                    app.actions.push(Action::ClearItems);
                }
                let items = app.library.items.clone();
                for it in items.iter().filter(|i| f.is_empty() || i.name().to_lowercase().contains(&f)) {
                    let tex = match it {
                        ItemEntry::Image { path, .. } => app.ui.thumb(ctx, path, &mut budget),
                        ItemEntry::Live2D(m) => m.icon.as_ref().and_then(|p| app.ui.thumb(ctx, p, &mut budget)),
                    };
                    let ph = if matches!(it, ItemEntry::Live2D(_)) { "Live2D" } else { "…" };
                    if card(ui, it.name(), tex, ph, false).clicked() {
                        app.ui.item_setup = Some(crate::ui_items::ItemSetup::new(it.path().to_path_buf(), &app.settings));
                    }
                }
            });
        });
        if budget == 0 {
            ctx.request_repaint();
        }
    }));
}

// ---------------------------------------------------------------------- dialogs

fn backgrounds_dialog(app: &mut App, ctx: &Context) {
    let mut open = true;
    egui::Window::new(tr("Выберите фон"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let cur = app.settings.background.clone();
            let mut pick = None;
            ui.horizontal(|ui| {
                if ui.selectable_label(cur == Background::Color([0.0; 3]), tr("Чёрный")).clicked() {
                    pick = Some(Background::Color([0.0; 3]));
                }
                if ui.selectable_label(cur == Background::Color([0.0, 1.0, 0.0]), "Хромакей").clicked() {
                    pick = Some(Background::Color([0.0, 1.0, 0.0]));
                }
                if ui.selectable_label(cur == Background::Transparent, tr("Прозрачный")).on_hover_text(tr("Для Spout2 и скриншотов")).clicked() {
                    pick = Some(Background::Transparent);
                }
            });
            ui.horizontal(|ui| {
                ui.label(tr("Свой цвет:"));
                let mut c = if let Background::Color(c) = cur { c } else { [0.2, 0.2, 0.25] };
                if ui.color_edit_button_rgb(&mut c).changed() {
                    pick = Some(Background::Color(c));
                }
            });
            ui.separator();
            filter_row(ui, &mut app.ui.filter);
            let f = app.ui.filter.to_lowercase();
            let mut budget = 3;
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for p in app.library.backgrounds.clone() {
                    let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    if !f.is_empty() && !name.to_lowercase().contains(&f) {
                        continue;
                    }
                    let s = p.to_string_lossy().into_owned();
                    ui.horizontal(|ui| {
                        if let Some(t) = app.ui.thumb(ctx, &p, &mut budget) {
                            ui.image((t, Vec2::new(64.0, 36.0)));
                        }
                        if ui.selectable_label(cur == Background::Image(s.clone()), &name).clicked() {
                            pick = Some(Background::Image(s.clone()));
                        }
                    });
                }
            });
            ui.label(RichText::new(tr("Видеофоны (mp4) пока не поддерживаются")).small().weak());
            if let Some(b) = pick {
                app.actions.push(Action::SetBackground(b));
            }
        });
    if !open {
        app.ui.dialog = Dialog::None;
    }
}

fn scenes_dialog(app: &mut App, ctx: &Context) {
    let mut open = true;
    egui::Window::new(tr("Сцены предметов"))
        .open(&mut open)
        .collapsible(false)
        .default_width(420.0)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!("Сейчас на сцене предметов: {}", app.items.len()));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut app.ui.scene_name).hint_text(tr("Название сцены")));
                let name = app.ui.scene_name.trim().to_string();
                if ui.add_enabled(!name.is_empty(), egui::Button::new(tr("Сохранить"))).clicked() {
                    app.actions.push(Action::SaveScene(name));
                    app.ui.scene_name.clear();
                }
            });
            ui.separator();
            for s in app.settings.item_scenes.clone() {
                ui.horizontal(|ui| {
                    ui.label(format!("{} ({} шт.)", s.name, s.items.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🗑").on_hover_text(tr("Удалить")).clicked() {
                            app.actions.push(Action::DeleteScene(s.name.clone()));
                        }
                        if ui.button(tr("⌨ Хоткей")).on_hover_text(tr("Назначить горячую клавишу")).clicked() {
                            if let Some(list) = app.hotkeys_mut() {
                                list.push(crate::hotkeys::Hotkey {
                                    name: format!("Сцена: {}", s.name),
                                    action: "ToggleItemScene".into(),
                                    file: s.name.clone(),
                                    ..Default::default()
                                });
                                let n = list.len() - 1;
                                app.hk.recording = Some((n, Vec::new(), false));
                                app.ui.settings_open = true;
                                app.ui.tab = Tab::Hotkeys;
                                app.ui.scroll_req = Some(f32::MAX);
                            } else {
                                app.toast("Сначала загрузите модель — горячие клавиши хранятся у модели");
                            }
                        }
                        if ui.button(tr("Загрузить")).clicked() {
                            app.actions.push(Action::LoadScene(s.name.clone()));
                        }
                    });
                });
            }
        });
    if !open {
        app.ui.dialog = Dialog::None;
    }
}

fn item_panel(app: &mut App, ctx: &Context) {
    crate::ui_items::item_picker(app, ctx);
    crate::ui_items::global_settings_window(app, ctx);
    crate::ui_items::item_setup_dialog(app, ctx);
    // The full item menu opens on a long press on the item (VTS), not on every click.
    if app.ui.item_menu.is_some() && app.ui.item_menu == app.ui.selected {
        crate::ui_items::item_config(app, ctx);
    } else {
        app.ui.item_menu = None;
    }
}

// ---------------------------------------------------------------------- settings

pub(crate) const BLUE: Color32 = Color32::from_rgb(24, 132, 236);
pub(crate) const TEXT: Color32 = Color32::from_rgb(70, 70, 78);
const PANEL_W: f32 = 580.0;
const BAR_H: f32 = 64.0;

pub(crate) fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Switch with a label (click either).
pub(crate) fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(50.0, 26.0), Sense::click());
        let col = if *on && ui.is_enabled() { TEXT } else { Color32::GRAY };
        let lr = ui.add(egui::Label::new(RichText::new(tr(label)).size(17.0).color(col)).sense(Sense::click()));
        if resp.clicked() || lr.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.15);
        let p = ui.painter();
        let bg = mix(Color32::from_rgb(204, 218, 238), BLUE, t);
        p.rect_filled(rect, 13.0, if ui.is_enabled() { bg } else { Color32::from_rgb(222, 228, 238) });
        let x = egui::lerp(rect.left() + 13.0..=rect.right() - 13.0, t);
        p.circle_filled(Pos2::new(x, rect.center().y), 10.0, Color32::WHITE);
        resp
    })
    .inner
}

pub(crate) fn blue_button(ui: &mut egui::Ui, text: &str, color: Color32, height: f32) -> egui::Response {
    let w = ui.available_width();
    ui.add_sized([w, height], egui::Button::new(RichText::new(tr(text)).size(17.0).color(Color32::WHITE)).fill(color).rounding(9.0))
}

/// Big two-line blue button (title + subtitle).
pub(crate) fn big_button(ui: &mut egui::Ui, title: &str, sub: &str, icon: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 72.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 10.0, if resp.hovered() { lighten(BLUE, 0.12) } else { BLUE });
    p.text(rect.center() - Vec2::new(0.0, 12.0), Align2::CENTER_CENTER, tr(title), FontId::proportional(23.0), Color32::WHITE);
    p.text(rect.center() + Vec2::new(0.0, 16.0), Align2::CENTER_CENTER, tr(sub), FontId::proportional(15.0), Color32::WHITE);
    p.text(rect.right_top() + Vec2::new(-22.0, 18.0), Align2::CENTER_CENTER, icon, FontId::proportional(24.0), Color32::WHITE);
    resp
}

pub(crate) fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(Color32::WHITE)
        .rounding(14.0)
        .inner_margin(egui::Margin::same(16.0))
        .shadow(egui::Shadow { offset: Vec2::new(0.0, 2.0), blur: 10.0, spread: 0.0, color: Color32::from_black_alpha(45) })
}

/// White rounded card with a title.
pub(crate) fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        if !title.is_empty() {
            ui.label(RichText::new(tr(title)).size(25.0).color(TEXT));
            ui.add_space(6.0);
        }
        add(ui);
    });
    ui.add_space(14.0);
}

/// Status dot painted as a circle (the default fonts lack "●").
pub(crate) fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 6.5, color);
}

fn soon_badge(ui: &mut egui::Ui) {
    ui.label(RichText::new(tr(" в разработке ")).small().color(Color32::WHITE).background_color(Color32::from_rgb(236, 96, 128)));
}

/// Card for a feature that is not implemented yet: same layout, disabled controls.
fn stub_card(ui: &mut egui::Ui, title: &str, toggles: &[&str], status: &str) {
    section(ui, title, |ui| {
        soon_badge(ui);
        ui.add_enabled_ui(false, |ui| {
            for t in toggles {
                let mut off = false;
                toggle(ui, &mut off, t);
            }
        });
        if !status.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Статус:")).strong().color(TEXT));
                status_dot(ui, Color32::GRAY);
                ui.label(RichText::new(tr(status)).color(TEXT));
            });
        }
    });
}

fn tab_icon(ui: &mut egui::Ui, icon: &str, selected: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 6.0, Color32::from_rgb(222, 230, 240));
    } else if resp.hovered() {
        p.rect_filled(rect, 6.0, Color32::from_rgb(238, 242, 247));
    }
    p.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(30.0), Color32::from_rgb(40, 40, 46));
    resp.on_hover_text(tr(tip))
}

fn settings_panel(app: &mut App, ctx: &Context, e: f32) {
    let screen = ctx.screen_rect();
    // Top bar slides down from above.
    egui::Area::new(egui::Id::new("set-bar")).fixed_pos(Pos2::new(0.0, -(1.0 - e) * (BAR_H + 10.0))).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).inner_margin(egui::Margin::symmetric(16.0, 6.0)).show(ui, |ui| {
            ui.set_width(screen.width() - 32.0);
            ui.set_height(BAR_H - 12.0);
            ui.horizontal_centered(|ui| {
                for (tab, icon, tip) in [(Tab::General, "⚙", tr("Общие настройки")), (Tab::Camera, "📷", tr("Камера и трекинг")), (Tab::Model, "👤", tr("Настройки модели")), (Tab::Hotkeys, "🎬", tr("Горячие клавиши"))] {
                    if tab_icon(ui, icon, app.ui.tab == tab, tip).clicked() {
                        app.ui.tab = tab;
                        app.ui.scroll_req = Some(0.0);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::click());
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "✖", FontId::proportional(38.0), Color32::from_rgb(240, 96, 96));
                    if resp.on_hover_text(tr("Закрыть настройки")).clicked() {
                        app.ui.settings_open = false;
                        app.save();
                    }
                    if tab_icon(ui, "⛶", false, "На весь экран").clicked() {
                        let fs = app.window.fullscreen().is_some();
                        app.window.set_fullscreen(if fs { None } else { Some(winit::window::Fullscreen::Borderless(None)) });
                    }
                });
            });
        });
    });

    // "Model lock" pill on the left.
    egui::Area::new(egui::Id::new("lock-pill")).fixed_pos(Pos2::new(36.0 - (1.0 - e) * 360.0, BAR_H + 26.0)).show(ctx, |ui| {
        let locked = app.settings.avatar_locked;
        let text = format!("{}  {}", if locked { "🔒" } else { "🔓" }, tr("Блокировка модели"));
        let b = egui::Button::new(RichText::new(text).size(19.0).color(TEXT)).fill(Color32::WHITE).rounding(14.0).min_size(Vec2::new(0.0, 48.0));
        if ui.add(b).clicked() {
            app.settings.avatar_locked = !locked;
        }
    });

    if app.ui.tab == Tab::Camera {
        camera_preview(app, ctx, e);
    }

    // Cards column slides in from the right.
    let x = screen.width() - PANEL_W - 96.0 + (1.0 - e) * (PANEL_W + 140.0);
    let h = screen.height() - BAR_H - 20.0;
    egui::Area::new(egui::Id::new("set-cards")).fixed_pos(Pos2::new(x, BAR_H + 14.0)).constrain(false).show(ctx, |ui| {
        ui.set_min_size(Vec2::new(PANEL_W, h));
        ui.set_max_size(Vec2::new(PANEL_W, h));
        let mut sa = egui::ScrollArea::vertical().id_salt("set-scroll").max_height(h).auto_shrink([false, false]);
        if let Some(o) = app.ui.scroll_req.take() {
            sa = sa.vertical_scroll_offset(o);
        }
        sa.show(ui, |ui| {
            ui.set_width(PANEL_W - 16.0);
            ui.style_mut().visuals.override_text_color = Some(TEXT);
            match app.ui.tab {
                Tab::General => general_tab(app, ui),
                Tab::Camera => camera_tab(app, ui),
                Tab::Model => model_tab(app, ui),
                Tab::Hotkeys => crate::ui_hotkeys::hotkeys_tab(app, ui),
            }
            ui.add_space(40.0);
        });
    });

    // Round scroll buttons on the far right.
    let bx = screen.width() - 70.0 + (1.0 - e) * 140.0;
    for (y, icon, target) in [(BAR_H + 16.0, "⬆", 0.0f32), (screen.height() - 70.0, "⬇", f32::MAX)] {
        egui::Area::new(egui::Id::new(("scroll-btn", icon))).fixed_pos(Pos2::new(bx, y)).show(ctx, |ui| {
            if round_button(ui, icon, Color32::from_rgb(100, 140, 250), 27.0, false, if target == 0.0 { "В начало" } else { "В конец" }).clicked() {
                app.ui.scroll_req = Some(target);
            }
        });
    }
}

fn path_row(ui: &mut egui::Ui, label: &str, value: &mut String, pick: impl FnOnce() -> Option<PathBuf>) -> bool {
    let mut changed = false;
    ui.label(label);
    ui.horizontal(|ui| {
        changed |= ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width() - 40.0)).lost_focus();
        if ui.button("…").clicked() {
            if let Some(p) = pick() {
                *value = p.to_string_lossy().into_owned();
                changed = true;
            }
        }
    });
    changed
}

fn mouse_grid(ui: &mut egui::Ui, x: f32, y: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(170.0, 156.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, Color32::from_rgb(226, 234, 248));
    p.rect_stroke(rect, 6.0, Stroke::new(1.5, Color32::from_rgb(60, 70, 90)));
    for i in 1..10 {
        let fx = rect.left() + rect.width() * i as f32 / 10.0;
        let fy = rect.top() + rect.height() * i as f32 / 10.0;
        p.line_segment([Pos2::new(fx, rect.top()), Pos2::new(fx, rect.bottom())], Stroke::new(1.0, Color32::WHITE));
        p.line_segment([Pos2::new(rect.left(), fy), Pos2::new(rect.right(), fy)], Stroke::new(1.0, Color32::WHITE));
    }
    let dot = Pos2::new(
        egui::lerp(rect.left() + 5.0..=rect.right() - 5.0, (x + 1.0) / 2.0),
        egui::lerp(rect.bottom() - 5.0..=rect.top() + 5.0, (y + 1.0) / 2.0),
    );
    p.circle_filled(dot, 4.0, Color32::from_rgb(240, 80, 100));
}

fn general_tab(app: &mut App, ui: &mut egui::Ui) {
    section(ui, tr("Общие настройки"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("🌐 Язык / Language")).size(17.0));
            for (code, name) in [("ru", "Русский"), ("en", "English")] {
                if ui.selectable_label(app.settings.language == code, RichText::new(name).size(17.0)).clicked() {
                    app.settings.language = code.into();
                    crate::i18n::set_language(code);
                    app.save();
                }
            }
        });
        toggle(ui, &mut app.settings.log_to_file, tr("Вести лог в файл"));
        toggle(ui, &mut app.settings.save_scene, tr("Сохранять сцену"));
        toggle(ui, &mut app.settings.show_stats, tr("Статистика производительности"));
        let mut gpu_prio = !app.settings.low_power_gpu;
        if toggle(ui, &mut gpu_prio, tr("Приоритет видеокарты")).on_hover_text(tr("Дискретная видеокарта; применяется после перезапуска")).changed() {
            app.settings.low_power_gpu = !gpu_prio;
        }
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.set_max_width(340.0);
            ui.vertical(|ui| {
                if blue_button(ui, tr("Сбросить настройки  ↺"), BLUE, 40.0).clicked() {
                    app.actions.push(Action::ResetSettings);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let w = ui.available_width();
                    let fsb = egui::Button::new(RichText::new(tr("На весь экран")).size(17.0).strong().color(Color32::WHITE)).fill(BLUE).rounding(9.0);
                    if ui.add_sized([w * 0.62, 40.0], fsb).clicked() {
                        let fs = app.window.fullscreen().is_some();
                        app.window.set_fullscreen(if fs { None } else { Some(winit::window::Fullscreen::Borderless(None)) });
                    }
                    let q = egui::Button::new(RichText::new(tr("Закрыть\nпрограмму")).size(13.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(236, 96, 128)).rounding(9.0);
                    if ui.add_sized([ui.available_width(), 40.0], q).clicked() {
                        app.actions.push(Action::Quit);
                    }
                });
            });
        });
    });
    crate::ui_vts::update_card(app, ui);
    section(ui, "", |ui| {
        if big_button(ui, tr("Открыть папку данных"), tr("Импорт моделей, фонов, предметов и пр."), "📁").clicked() {
            app.actions.push(Action::OpenDataFolder);
        }
    });
    section(ui, "", |ui| {
        if big_button(ui, tr("Запустить ещё одно окно"), tr("Запуск ещё одного экземпляра rigrs"), "➕").clicked() {
            app.actions.push(Action::NewInstance);
        }
    });
    section(ui, tr("Пути"), |ui| {
        let mut changed = path_row(ui, "Папка VTube Studio (модели, фоны, предметы):", &mut app.settings.vts_dir, || rfd::FileDialog::new().pick_folder());
        changed |= path_row(ui, "Live2DCubismCore.dll:", &mut app.settings.core_dll, || rfd::FileDialog::new().add_filter("DLL", &["dll"]).pick_file());
        changed |= path_row(ui, "OpenSeeFace facetracker.exe:", &mut app.settings.tracker_exe, || rfd::FileDialog::new().add_filter("EXE", &["exe"]).pick_file());
        changed |= path_row(ui, "NVIDIA ExpressionApp.exe:", &mut app.settings.nv_exe, || rfd::FileDialog::new().add_filter("EXE", &["exe"]).pick_file());
        if changed {
            app.actions.push(Action::ReloadCore);
        }
        ui.horizontal(|ui| {
            if ui.button(tr("Пересканировать папки")).clicked() {
                app.actions.push(Action::Rescan);
            }
            if let Some(c) = app.core {
                ui.label(RichText::new(format!("Cubism Core {}", c.version_string())).weak());
            }
        });
        ui.horizontal(|ui| {
            ui.label(tr("Масштаб интерфейса"));
            if ui.add(egui::Slider::new(&mut app.settings.ui_scale, 0.6..=2.0)).changed() {
                ui.ctx().set_zoom_factor(app.settings.ui_scale);
            }
        });
    });
    stub_card(ui, tr("Настройки Wi-Fi-соединения"), &["Запустить сервер"], "Сервер отключен");
    stub_card(ui, tr("Настройки USB-соединения"), &["Подключиться через USB"], "Отключено");
    crate::ui_hotkeys::api_card(app, ui);
    crate::ui_hotkeys::editor_card(app, ui);
    section(ui, tr("Настройка ввода мыши"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("X: {:.0}", app.mouse[0])).size(17.0));
            ui.add_space(80.0);
            ui.label(RichText::new(format!("Y: {:.0}", app.mouse[1])).size(17.0));
        });
        let r = &mut app.settings.mouse_range;
        egui::Grid::new("mouse-range").num_columns(4).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label("");
            ui.label("min");
            ui.label("");
            ui.label("max");
            ui.end_row();
            ui.label(tr("Диапазон по оси X"));
            ui.add(egui::DragValue::new(&mut r[0]).speed(1.0));
            ui.label("—");
            ui.add(egui::DragValue::new(&mut r[1]).speed(1.0));
            ui.end_row();
            ui.label(tr("Диапазон по оси Y"));
            ui.add(egui::DragValue::new(&mut r[2]).speed(1.0));
            ui.label("—");
            ui.add(egui::DragValue::new(&mut r[3]).speed(1.0));
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.label(tr("Параметры на выходе (входы MousePositionX / MousePositionY)"));
        let (mx, my) = (app.inputs[crate::mapping::input::MOUSE_POSITION_X], app.inputs[crate::mapping::input::MOUSE_POSITION_Y]);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(format!("X {mx:+.3}"));
                ui.label(format!("Y {my:+.3}"));
            });
            mouse_grid(ui, mx, my);
        });
    });
    crate::ui_vts::controller_card(app, ui);
    crate::ui_vts::sound_card(app, ui);
    crate::ui_hotkeys::mic_card(app, ui);
    crate::ui_hotkeys::twitch_card(app, ui);
    stub_card(ui, "Discord", &["Войти и активировать"], "Отключено");
    stub_card(ui, "mocopi", &["Запустить", "Показывать параметры трекинга"], "Отключено");
}

#[derive(Clone, Copy, PartialEq)]
pub enum Modal {
    Camera,
    Resolution,
    Fps,
    Format,
    Quality,
    TrackType,
    Idle,
    LostIdle,
    RuleInput(usize),
    RuleOutput(usize),
    LightWindow,
    MaskRes,
    VfxList,
    VfxEdit(usize),
    VfxPresetLoad,
    VfxPresetDelete,
    /// Information dialog (id), followed by `info_next` when dismissed.
    Info(u8),
    HkAction(usize),
    HkFile(usize),
    /// Gesture picker: hotkey, left hand?
    HkGesture(usize, bool),
    HkTwitch(usize),
    HkSound(usize),
    TwitchReward(usize),
    TwitchFeatures,
    TwitchDropper,
    TwitchChat,
    TwitchTriggersCfg,
}

pub const INFO_MASKS: u8 = 0;
pub const INFO_VFX_ALPHA: u8 = 1;
pub const INFO_VFX_ARTISTS: u8 = 2;
pub const INFO_VCAM: u8 = 3;

/// Mask atlas presets: (texture size, max masks).
pub const MASK_PRESETS: [(u32, u32); 6] = [(2048, 64), (2048, 256), (2048, 1024), (4096, 64), (4096, 256), (4096, 1024)];

fn mask_cell(size: u32, max: u32) -> u32 {
    size / ((max / 4) as f32).sqrt().ceil() as u32
}

/// Opens an info dialog unless dismissed; otherwise goes straight to `next`.
pub fn open_info(app: &mut App, id: u8, next: Option<Modal>) {
    if app.settings.dismissed_info.contains(&id) {
        if id == INFO_VFX_ALPHA {
            return open_info(app, INFO_VFX_ARTISTS, next);
        }
        app.ui.modal = next;
    } else {
        app.ui.modal = Some(Modal::Info(id));
        app.ui.info_next = next;
        app.ui.info_dont_show = false;
    }
    app.ui.modal_sel = None;
}

fn info_text(id: u8) -> (&'static str, Vec<&'static str>) {
    match id {
        INFO_MASKS => (
            "",
            vec![
                "Для моделей с большим количеством масок рекомендуется увеличить лимит масок (например, до 256 вместо 64). Учтите, что это уменьшит размер каждой отдельной маски: при разрешении маски меньше 512 пикселей её контуры могут стать размытыми.",
                "Это исправляется увеличением общего размера текстуры масок (например, до 4096 пикселей), но это расходует больше видеопамяти и может снизить производительность на слабых видеокартах.",
                "Подберите оптимальные настройки под свою модель.",
            ],
        ),
        INFO_VFX_ALPHA => (
            "Важно!",
            vec![
                "При использовании визуальных эффектов помните:",
                "• Некоторые эффекты не поддерживают прозрачность: при прозрачном фоне в Spout2/NDI они будут выглядеть плохо. Для них лучше включить фон-картинку.",
                "• Эффекты, которые сдвигают или искажают изображение (например, «рыбий глаз»), сбивают привязку предметов к модели на экране.",
                "• Много эффектов одновременно требуют более мощной видеокарты.",
            ],
        ),
        INFO_VFX_ARTISTS => (
            "Важно!",
            vec![
                "Некоторые художники и риггеры против фильтров, которые искажают их модель.",
                "Перед использованием таких эффектов лучше спросить разрешения у создателей модели.",
            ],
        ),
        _ => (
            "",
            vec![
                "Виртуальная камера rigrsCam устанавливается один раз. Если она не появилась в списке камер (OBS, Discord, Zoom, браузер), нажмите «Открыть папку» и запустите файл install.bat от имени администратора.",
                "После установки камера с названием rigrsCam будет доступна во всех программах. Нужна Windows 11.",
            ],
        ),
    }
}

fn vfx_presets(app: &App) -> Vec<String> {
    crate::vfx::list_presets(&app.paths.data)
}

fn vfx_edit_dialog(app: &mut App, ctx: &Context, idx: usize) {
    use crate::vfx::{Kind, Val};
    let Some(def) = crate::vfx::DEFS.get(idx) else {
        app.ui.modal = None;
        return;
    };
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new("vfx-dim")).fixed_pos(Pos2::ZERO).order(egui::Order::Foreground).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(90));
    });
    let mut done = false;
    let mut back = false;
    let w = 780.0f32.min(screen.width() - 40.0);
    let items: Vec<String> = app.library.items.iter().filter_map(|e| match e {
        crate::library::ItemEntry::Image { path, .. } if path.is_file() => path.file_name().map(|n| n.to_string_lossy().into_owned()),
        _ => None,
    }).collect();
    let mut changed = false;
    egui::Area::new(egui::Id::new("vfx-edit")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        let resp = egui::Frame::none().fill(Color32::WHITE).rounding(6.0).inner_margin(egui::Margin { left: 26.0, right: 26.0, top: 46.0, bottom: 18.0 }).show(ui, |ui| {
            ui.set_width(w - 52.0);
            ui.style_mut().visuals.override_text_color = Some(TEXT);
            ui.label(RichText::new(def.name()).size(26.0));
            if def.distorts {
                ui.label(RichText::new(tr("Эффект искажает картинку: привязка предметов на экране может выглядеть смещённой.")).color(Color32::from_rgb(236, 30, 160)));
            }
            if def.restricted && !app.settings.vfx_restricted {
                ui.label(RichText::new(tr("Экспериментальный эффект: включите «Разрешить экспериментальные эффекты» в настройках эффектов.")).color(Color32::from_rgb(220, 90, 90)));
            }
            ui.add_space(8.0);
            let v = &mut app.settings.vfx;
            egui::ScrollArea::vertical().max_height(screen.height() - 260.0).show(ui, |ui| {
                for c in def.configs {
                    ui.horizontal(|ui| {
                        ui.add_sized([300.0, 22.0], egui::Label::new(RichText::new(c.label()).size(16.0)).truncate());
                        ui.spacing_mut().slider_width = w - 480.0;
                        match c.kind {
                            Kind::Float | Kind::Int => {
                                let mut x = crate::vfx::get_f(v, c);
                                let mut s = egui::Slider::new(&mut x, c.min..=c.max);
                                if c.kind == Kind::Int {
                                    s = s.step_by(1.0);
                                }
                                if ui.add(s).changed() {
                                    v.insert(c.id.into(), Val::Num(x));
                                    changed = true;
                                }
                            }
                            Kind::Bool => {
                                let mut b = crate::vfx::get_f(v, c) > 0.5;
                                if ui.checkbox(&mut b, "").changed() {
                                    v.insert(c.id.into(), Val::Num(if b { 1.0 } else { 0.0 }));
                                    changed = true;
                                }
                            }
                            Kind::Color => {
                                let mut col = crate::vfx::get_color(v, c);
                                let r = if c.alpha { ui.color_edit_button_rgba_unmultiplied(&mut col) } else {
                                    let mut rgb = [col[0], col[1], col[2]];
                                    let r = ui.color_edit_button_rgb(&mut rgb);
                                    col = [rgb[0], rgb[1], rgb[2], 1.0];
                                    r
                                };
                                if r.changed() {
                                    v.insert(c.id.into(), Val::Text(crate::vfx::color_hex(col)));
                                    changed = true;
                                }
                            }
                            Kind::Text | Kind::Item => {
                                let cur = crate::vfx::get_text(v, c);
                                egui::ComboBox::from_id_salt(c.id).selected_text(if cur.is_empty() { tr("картинка не выбрана").to_string() } else { cur.clone() }).width(260.0).height(360.0).show_ui(ui, |ui| {
                                    if ui.selectable_label(cur.is_empty(), tr("(нет)")).clicked() {
                                        v.remove(c.id);
                                        changed = true;
                                    }
                                    for n in &items {
                                        if ui.selectable_label(&cur == n, n).clicked() {
                                            v.insert(c.id.into(), Val::Text(n.clone()));
                                            changed = true;
                                        }
                                    }
                                });
                                if ui.small_button("…").on_hover_text(tr("Выбрать файл")).clicked() {
                                    if let Some(p) = rfd::FileDialog::new().add_filter("PNG/JPG", &["png", "jpg", "jpeg"]).pick_file() {
                                        v.insert(c.id.into(), Val::Text(p.to_string_lossy().into_owned()));
                                        changed = true;
                                    }
                                }
                            }
                        }
                    });
                }
            });
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = egui::Button::new(RichText::new(tr("Готово")).size(17.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240)).rounding(3.0);
                if ui.add_sized([150.0, 40.0], ok).clicked() {
                    done = true;
                }
                let bk = egui::Button::new(RichText::new(tr("Назад")).size(17.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(222, 88, 116)).rounding(3.0);
                if ui.add_sized([150.0, 40.0], bk).clicked() {
                    back = true;
                }
                if ui.button(RichText::new(tr("Сбросить")).size(16.0)).clicked() {
                    crate::vfx::reset_effect(&mut app.settings.vfx, def);
                    changed = true;
                }
            });
        });
        let cpos = Pos2::new(resp.response.rect.center().x, resp.response.rect.top());
        let p = ui.painter();
        p.circle_filled(cpos, 36.0, Color32::WHITE);
        p.circle_filled(cpos, 31.0, Color32::from_rgb(166, 214, 240));
        p.text(cpos, Align2::CENTER_CENTER, "!", FontId::proportional(46.0), Color32::WHITE);
    });
    if changed {
        // Manual changes unload the active preset (VTS).
        app.vfx_preset.clear();
    }
    if done || back {
        app.ui.modal = if back { Some(Modal::VfxList) } else { None };
        app.ui.modal_sel = None;
        app.save();
    }
}

fn info_dialog(app: &mut App, ctx: &Context, id: u8) {
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new("info-dim")).fixed_pos(Pos2::ZERO).order(egui::Order::Foreground).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(130));
    });
    let (title, paras) = info_text(id);
    let w = (screen.width() - 60.0).min(1700.0);
    let mut ok = false;
    let mut open_folder = false;
    egui::Area::new(egui::Id::new("info")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        let resp = egui::Frame::none().fill(Color32::WHITE).inner_margin(egui::Margin { left: 38.0, right: 38.0, top: 46.0, bottom: 18.0 }).show(ui, |ui| {
            ui.set_width(w - 76.0);
            if !title.is_empty() {
                ui.label(RichText::new(tr(title)).size(32.0).color(Color32::from_rgb(236, 30, 160)));
                ui.add_space(6.0);
            }
            for p in &paras {
                ui.add(egui::Label::new(RichText::new(tr(p)).size(20.0).color(Color32::from_rgb(110, 110, 118))).wrap());
                ui.add_space(8.0);
            }
            ui.add_space(20.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut d = app.ui.info_dont_show;
                ui.add_enabled_ui(id != INFO_VCAM, |ui| {
                    if id != INFO_VCAM {
                        toggle(ui, &mut d, tr("Больше не показывать"));
                    }
                });
                app.ui.info_dont_show = d;
            });
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = egui::Button::new(RichText::new("OK").size(18.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240)).rounding(2.0);
                if ui.add_sized([170.0, 42.0], b).clicked() {
                    ok = true;
                }
                if id == INFO_VCAM {
                    let b = egui::Button::new(RichText::new(tr("Открыть папку")).size(18.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240)).rounding(2.0);
                    if ui.add_sized([170.0, 42.0], b).clicked() {
                        open_folder = true;
                    }
                }
            });
        });
        let c = Pos2::new(resp.response.rect.center().x, resp.response.rect.top());
        let p = ui.painter();
        p.circle_filled(c, 36.0, Color32::WHITE);
        p.circle_filled(c, 31.0, Color32::from_rgb(166, 214, 240));
        p.text(c, Align2::CENTER_CENTER, "!", FontId::proportional(46.0), Color32::WHITE);
    });
    if open_folder {
        app.actions.push(Action::OpenVcamFolder);
    }
    if ok {
        if app.ui.info_dont_show && !app.settings.dismissed_info.contains(&id) {
            app.settings.dismissed_info.push(id);
        }
        let next = app.ui.info_next.take();
        if id == INFO_VFX_ALPHA {
            open_info(app, INFO_VFX_ARTISTS, next);
        } else {
            app.ui.modal = next;
            app.ui.modal_sel = None;
        }
    }
}

/// Tracking quality levels as shown to the user → OpenSeeFace `--model` value.
const QUALITY: [(&str, &str, i32); 6] = [
    ("Уровень 5", "Лучшее качество (полная поддержка подмигивания)", 4),
    ("Уровень 4", "Лучшее качество (подмигивание ограничено, моргание стабильно)", 3),
    ("Уровень 3", "Точнее уровня 2", 2),
    ("Уровень 2", "Точнее уровня 1", 1),
    ("Уровень 1", "Точнее уровня 0. С этого уровня появляется отслеживание глаз.", 0),
    ("Уровень 0", "Работает быстро, но неточно и не имеет отслеживания глаз.", -2),
];

fn quality_label(model: i32) -> (&'static str, &'static str) {
    QUALITY.iter().find(|q| q.2 == model).map(|q| (q.0, q.1)).unwrap_or(("Уровень 4", ""))
}

pub(crate) struct Row {
    pub bold: String,
    pub text: String,
    pub enabled: bool,
}

pub(crate) fn row(text: impl Into<String>) -> Row {
    Row { bold: String::new(), text: text.into(), enabled: true }
}

/// Modal chooser: "!" badge, prompt, list of rows, Cancel / Choose, filter field below.
fn modal_dialog(app: &mut App, ctx: &Context) {
    let Some(kind) = app.ui.modal else { return };
    if let Modal::Info(id) = kind {
        info_dialog(app, ctx, id);
        return;
    }
    if let Modal::VfxEdit(i) = kind {
        vfx_edit_dialog(app, ctx, i);
        return;
    }
    if crate::ui_hotkeys::custom_modal(app, ctx, kind) {
        return;
    }
    let hk_rows = crate::ui_hotkeys::modal_rows(app, kind);
    let mut fps_values: Vec<u32> = Vec::new();
    let mut formats: Vec<String> = Vec::new();
    let mut resolutions: Vec<(u32, u32)> = Vec::new();
    let (prompt, rows): (String, Vec<Row>) = if let Some(x) = hk_rows { x } else {
    let (prompt, rows): (&str, Vec<Row>) = match kind {
        Modal::Camera => (
            "Выберите веб-камеру.",
            if app.cameras.is_empty() { vec![Row { bold: String::new(), text: "Поиск камер…".into(), enabled: false }] } else { app.cameras.iter().map(|c| row(c.1.clone())).collect() },
        ),
        Modal::Resolution | Modal::Fps | Modal::Format => {
            let caps = app.caps.clone();
            match caps {
                None => ("Получаю возможности камеры…", vec![Row { bold: String::new(), text: "Загрузка…".into(), enabled: false }]),
                Some(caps) => {
                    for c in &caps {
                        if !resolutions.contains(&(c.width, c.height)) {
                            resolutions.push((c.width, c.height));
                        }
                    }
                    resolutions.sort_by_key(|r| std::cmp::Reverse(r.0 * r.1));
                    let at_res: Vec<_> = caps.iter().filter(|c| (c.width, c.height) == app.ui.wiz_res).collect();
                    for c in &at_res {
                        let top = c.fps_max.floor() as u32;
                        for f in [top, 60, 30, 24, 15] {
                            if f as f32 <= c.fps_max + 0.01 && f as f32 >= c.fps_min - 0.01 && f > 0 && !fps_values.contains(&f) {
                                fps_values.push(f);
                            }
                        }
                        if !formats.contains(&c.format) {
                            formats.push(c.format.clone());
                        }
                    }
                    fps_values.sort_unstable_by(|a, b| b.cmp(a));
                    match kind {
                        Modal::Resolution => (
                            "Выберите разрешение. Чем выше, тем лучше. Для качественного отслеживания глаз, моргания и подмигивания рекомендуется разрешение около 1280x720 или выше.",
                            if resolutions.is_empty() { vec![row(tr("Камера не сообщила режимы — будут использованы настройки по умолчанию"))] } else { resolutions.iter().map(|r| row(format!("{} x {}", r.0, r.1))).collect() },
                        ),
                        Modal::Fps => ("Выберите FPS (частоту кадров) камеры. 15 FPS — хорошо, 30 — лучше.", fps_values.iter().map(|f| row(f.to_string())).collect()),
                        _ => (
                            "Выберите формат цвета. XRGB: наименьшая нагрузка на процессор. YUV/I420: больше нагрузка на процессор. MJPEG: наибольшая нагрузка на процессор, но может обеспечить самую высокую частоту кадров.",
                            formats.iter().map(|f| row(f.clone())).collect(),
                        ),
                    }
                }
            }
        }
        Modal::Idle | Modal::LostIdle => {
            let mut v = vec![row(tr("Нет"))];
            if let Some((_, m)) = &app.ui.motions_cache {
                v.extend(m.iter().map(|f| row(f.clone())));
            }
            (
                if kind == Modal::Idle { "Выберите анимацию ожидания (играет постоянно, трекинг накладывается поверх)." } else { "Выберите анимацию, которая играет, пока лицо не найдено." },
                v,
            )
        }
        Modal::MaskRes => (
            "Настройки масок Live2D. Выше разрешение — чётче маски, но больше расход видеопамяти.",
            MASK_PRESETS
                .iter()
                .map(|&(s, m)| {
                    let c = mask_cell(s, m);
                    let cur = app.settings.mask_atlas == s && app.settings.mask_max == m;
                    Row { bold: format!("{s} x {s}"), text: format!(" пикселей (макс. {m} масок) · {c} x {c} пикселей на маску{}", if cur { "  ✔" } else { "" }), enabled: true }
                })
                .collect(),
        ),
        Modal::Info(_) | Modal::VfxEdit(_) => ("", Vec::new()),
        Modal::VfxList => {
            let mut v = vec![Row { bold: "🗑".into(), text: "Выключить все эффекты".into(), enabled: true }];
            v.extend(crate::vfx::DEFS.iter().enumerate().map(|(i, d)| {
                let on = crate::vfx::effect_active(&app.settings.vfx, d);
                let _ = i;
                Row { bold: if on { "✔".into() } else { String::new() }, text: d.name().into(), enabled: true }
            }));
            (
                if app.settings.vfx_on { "Выберите эффект для настройки." } else { "Выберите эффект для настройки. Внимание!! Визуальные эффекты отключены, поэтому предпросмотр будет отсутствовать." },
                v,
            )
        }
        Modal::VfxPresetLoad | Modal::VfxPresetDelete => (
            if kind == Modal::VfxPresetLoad { "Выберите пресет эффектов для загрузки." } else { "Выберите пресет эффектов для удаления." },
            vfx_presets(app).into_iter().map(row).collect(),
        ),
        Modal::LightWindow => (
            "Выберите окно, цвет которого будет освещать модель.",
            app.light_windows.iter().map(|w| row(w.1.clone())).collect(),
        ),
        Modal::RuleInput(_) => {
            let mut v = vec![Row { bold: String::new(), text: "(нет) — только автоморгание/дыхание".into(), enabled: true }];
            v.extend(INPUT_NAMES.iter().enumerate().map(|(k, n)| Row { bold: (*n).into(), text: format!("  текущее {:+.2}", app.inputs[k]), enabled: true }));
            v.extend(crate::perfect_sync::snapshot().into_iter().map(|(n, x)| Row { bold: n.into(), text: format!("  Perfect Sync / VBridger {x:+.2}"), enabled: true }));
            ("Выберите вход трекинга для этого параметра.", v)
        }
        Modal::RuleOutput(_) => {
            let v = app
                .avatar
                .as_ref()
                .map(|a| {
                    (0..a.model.param_ids.len())
                        .map(|k| Row { bold: a.model.param_ids[k].clone(), text: format!("  [{:.1} … {:.1}]", a.model.param_min[k], a.model.param_max[k]), enabled: true })
                        .collect()
                })
                .unwrap_or_default();
            ("Выберите параметр Live2D, которым будет управлять этот вход.", v)
        }
        Modal::Quality => {
            let mut v = vec![
                Row { bold: "[RTX]".into(), text: tr("NVIDIA Broadcast (нужна видеокарта RTX и DLC VTS «NVIDIA Broadcast Tracker»)").into(), enabled: true },
                Row { bold: "[Телефон]".into(), text: tr("iPhone / Android с приложением VTube Studio (ARKit, лучшее качество)").into(), enabled: true },
                Row { bold: "[Google]".into(), text: "Трекер Mediapipe".into(), enabled: true },
            ];
            v.extend(QUALITY.iter().map(|q| Row { bold: format!("[{}]", q.0), text: q.1.into(), enabled: true }));
            ("Выберите качество отслеживания. Более высокое качество обычно означает лучшее отслеживание, но также задействует больше ресурсов процессора.", v)
        }
        Modal::TrackType => (
            "Выберите тип отслеживания веб-камерой.",
            vec![row(tr("Только лицо")), row(tr("Только руки")), row(tr("Лицо и руки"))],
        ),
        _ => ("", Vec::new()),
    };
    (crate::i18n::tr(prompt).to_string(), rows)
    };

    let screen = ctx.screen_rect();
    // Dim the scene and swallow clicks behind the dialog.
    egui::Area::new(egui::Id::new("modal-dim")).fixed_pos(Pos2::ZERO).order(egui::Order::Foreground).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(110));
    });

    let w = 820.0f32.min(screen.width() - 40.0);
    let mut choose: Option<usize> = None;
    let mut cancel = false;
    egui::Area::new(egui::Id::new("modal")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, -30.0]).show(ctx, |ui| {
        let frame = egui::Frame::none().fill(Color32::WHITE).rounding(6.0).inner_margin(egui::Margin { left: 26.0, right: 26.0, top: 46.0, bottom: 18.0 });
        let resp = frame.show(ui, |ui| {
            ui.set_width(w - 52.0);
            ui.label(RichText::new(&prompt).size(21.0).color(Color32::from_rgb(110, 110, 118)));
            ui.add_space(26.0);
            let f = app.ui.modal_filter.to_lowercase();
            let mut sa = egui::ScrollArea::vertical().id_salt("modal-list").max_height(400.0).min_scrolled_height(400.0).auto_shrink([false, false]);
            if app.ui.shown.1 != Some(kind) {
                sa = sa.vertical_scroll_offset(0.0);
                app.ui.shown.1 = Some(kind);
            }
            sa.show(ui, |ui| {
                for (i, r) in rows.iter().enumerate() {
                    if !f.is_empty() && !format!("{} {}", r.bold, r.text).to_lowercase().contains(&f) {
                        continue;
                    }
                    let sel = app.ui.modal_sel == Some(i);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 58.0), if r.enabled { Sense::click() } else { Sense::hover() });
                    let p = ui.painter();
                    p.rect_filled(rect.translate(Vec2::new(1.5, 2.5)), 8.0, Color32::from_black_alpha(60));
                    let fill = if sel { Color32::from_rgb(190, 222, 252) } else if resp.hovered() && r.enabled { Color32::from_rgb(222, 222, 234) } else { Color32::from_rgb(232, 230, 240) };
                    p.rect_filled(rect, 8.0, fill);
                    let col = if r.enabled { Color32::from_rgb(60, 60, 66) } else { Color32::from_rgb(160, 160, 168) };
                    let mut x = rect.left() + 14.0;
                    if !r.bold.is_empty() {
                        let g = p.layout_no_wrap(r.bold.clone(), FontId::proportional(20.0), col);
                        let gw = g.size().x;
                        p.galley(Pos2::new(x, rect.center().y - g.size().y / 2.0), g.clone(), col);
                        p.galley(Pos2::new(x + 0.6, rect.center().y - g.size().y / 2.0), g, col);
                        x += gw + 8.0;
                    }
                    p.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, tr(&r.text), FontId::proportional(20.0), col);
                    if resp.clicked() {
                        app.ui.modal_sel = Some(i);
                    }
                    if resp.double_clicked() {
                        choose = Some(i);
                    }
                    ui.add_space(10.0);
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can = app.ui.modal_sel.map(|i| rows.get(i).map(|r| r.enabled).unwrap_or(false)).unwrap_or(false);
                let ok = egui::Button::new(RichText::new(tr("Выбрать")).size(17.0).strong().color(Color32::WHITE))
                    .fill(if can { Color32::from_rgb(14, 150, 240) } else { Color32::from_rgb(210, 212, 222) })
                    .rounding(3.0);
                if ui.add_sized([170.0, 40.0], ok).clicked() && can {
                    choose = app.ui.modal_sel;
                }
                ui.add_space(14.0);
                let cancel_label = if kind == Modal::VfxList { "Готово" } else { "Отмена" };
                let no = egui::Button::new(RichText::new(tr(cancel_label)).size(17.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(222, 88, 116)).rounding(3.0);
                if ui.add_sized([170.0, 40.0], no).clicked() {
                    cancel = true;
                }
            });
        });
        // "!" badge on the top edge.
        let c = Pos2::new(resp.response.rect.center().x, resp.response.rect.top());
        let p = ui.painter();
        p.circle_filled(c, 36.0, Color32::WHITE);
        p.circle_filled(c, 31.0, Color32::from_rgb(166, 214, 240));
        p.text(c, Align2::CENTER_CENTER, "!", FontId::proportional(46.0), Color32::WHITE);
        // Filter field under the dialog.
        ui.add_space(14.0);
        egui::Frame::none().fill(Color32::WHITE).rounding(10.0).inner_margin(8.0).show(ui, |ui| {
            ui.set_width(w - 16.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut app.ui.modal_filter).hint_text(tr("Фильтр")).font(FontId::proportional(20.0)).desired_width(w - 110.0));
                if ui.add(egui::Button::new(RichText::new("✖").size(20.0).color(Color32::from_rgb(240, 120, 130))).frame(false)).clicked() {
                    app.ui.modal_filter.clear();
                }
                ui.label(RichText::new("🔍").size(24.0));
            });
        });
    });

    if cancel {
        app.ui.modal = None;
        app.ui.modal_filter.clear();
        return;
    }
    let Some(i) = choose else { return };
    app.ui.modal_sel = None;
    app.ui.modal_filter.clear();
    if crate::ui_hotkeys::modal_choose(app, kind, i) {
        return;
    }
    match kind {
        Modal::Camera => {
            if let Some(c) = app.cameras.get(i) {
                app.ui.wiz_cam = c.0;
                app.load_caps(c.0);
                app.ui.modal = Some(Modal::Resolution);
            }
        }
        Modal::Resolution => match resolutions.get(i) {
            Some(r) => {
                app.ui.wiz_res = *r;
                app.ui.modal = Some(Modal::Fps);
            }
            None => {
                // No capability list: keep the camera, use driver defaults.
                app.settings.camera = app.ui.wiz_cam;
                app.settings.cam_dcap = -1;
                app.settings.cam_format.clear();
                app.ui.modal = None;
                app.save();
            }
        },
        Modal::Fps => {
            if let Some(f) = fps_values.get(i) {
                app.ui.wiz_fps = *f;
                app.ui.modal = Some(Modal::Format);
            }
        }
        Modal::Format => {
            if let Some(fmt) = formats.get(i) {
                let res = app.ui.wiz_res;
                let dcap = app.caps.as_ref().and_then(|caps| {
                    caps.iter().find(|c| (c.width, c.height) == res && &c.format == fmt && c.fps_max + 0.01 >= app.ui.wiz_fps as f32).map(|c| c.index)
                });
                app.settings.camera = app.ui.wiz_cam;
                app.settings.cam_size = [res.0, res.1];
                app.settings.cam_fps = app.ui.wiz_fps;
                app.settings.cam_dcap = dcap.unwrap_or(-1);
                app.settings.cam_format = fmt.clone();
                app.ui.modal = None;
                app.save();
            }
        }
        Modal::Idle | Modal::LostIdle => {
            let file = if i == 0 { String::new() } else { app.ui.motions_cache.as_ref().and_then(|(_, m)| m.get(i - 1).cloned()).unwrap_or_default() };
            if kind == Modal::Idle {
                app.actions.push(Action::SetIdle(if file.is_empty() { None } else { Some(file) }));
            } else if let Some(a) = &mut app.avatar {
                a.set_lost_idle(&file);
            }
            app.ui.modal = None;
        }
        Modal::MaskRes => {
            if let Some(&(s, m)) = MASK_PRESETS.get(i) {
                app.actions.push(Action::SetMaskRes(s, m));
            }
            app.ui.modal = None;
        }
        Modal::Info(_) | Modal::VfxEdit(_) | Modal::HkAction(_) | Modal::HkFile(_) | Modal::HkGesture(..) | Modal::HkTwitch(_) | Modal::HkSound(_) | Modal::TwitchReward(_) | Modal::TwitchFeatures | Modal::TwitchDropper | Modal::TwitchChat | Modal::TwitchTriggersCfg => app.ui.modal = None,
        Modal::VfxList => {
            if i == 0 {
                app.settings.vfx.clear();
                app.vfx_preset.clear();
                app.save();
            } else {
                app.ui.modal = Some(Modal::VfxEdit(i - 1));
            }
        }
        Modal::VfxPresetLoad | Modal::VfxPresetDelete => {
            if let Some(name) = vfx_presets(app).get(i).cloned() {
                let path = app.paths.data.join("vfx_presets").join(format!("{name}.json"));
                if kind == Modal::VfxPresetLoad {
                    match crate::vfx::load_preset(&app.paths.data, &name) {
                        Some(v) => {
                            app.vfx_fade_to(v, 0.5);
                            app.vfx_preset = name.clone();
                            app.toast(format!("Пресет «{name}» загружен"));
                        }
                        None => app.toast(format!("Пресет «{name}» не удалось прочитать")),
                    }
                } else {
                    let _ = std::fs::remove_file(&path);
                    app.toast(format!("Пресет «{name}» удалён"));
                }
                app.save();
            }
            app.ui.modal = None;
        }
        Modal::LightWindow => {
            if let Some(w) = app.light_windows.get(i) {
                app.actions.push(Action::SetLightWindow(w.1.clone()));
            }
            app.ui.modal = None;
        }
        Modal::RuleInput(r) | Modal::RuleOutput(r) => {
            let key = app.model_key();
            if let (Some(a), Some(key)) = (&mut app.avatar, key) {
                if let Some(rule) = a.mapping.rules.get_mut(r) {
                    if let Modal::RuleInput(_) = kind {
                        let ps = crate::perfect_sync::input_names();
                        rule.input = if i == 0 {
                            String::new()
                        } else if let Some(n) = INPUT_NAMES.get(i - 1) {
                            n.to_string()
                        } else {
                            ps.get(i - 1 - INPUT_NAMES.len()).map(|n| n.to_string()).unwrap_or_default()
                        };
                    } else if let Some(pid) = a.model.param_ids.get(i) {
                        rule.output = pid.clone();
                    }
                }
                a.mapping.recompile(&a.model);
                app.settings.rules_override.insert(key, a.mapping.rules.clone());
            }
            app.ui.modal = None;
        }
        Modal::Quality => {
            let kind = app.settings.track_type;
            if i == 0 {
                app.set_tracking_mode(crate::tracking::backend::NVIDIA, 0);
            } else if i == 1 {
                app.set_tracking_mode(crate::tracking::backend::PHONE, 0);
            } else if i == 2 {
                app.set_tracking_mode(1, kind);
            } else if let Some(q) = QUALITY.get(i - 3) {
                app.settings.track_quality = q.2;
                app.set_tracking_mode(0, kind);
            }
            app.ui.modal = None;
        }
        Modal::TrackType => {
            let backend = app.settings.track_backend;
            app.set_tracking_mode(backend, i.min(2) as u8);
            app.ui.modal = None;
        }
    }
}

/// Light-blue "selector" button followed by "↳ value" lines.
pub(crate) fn picker(ui: &mut egui::Ui, label: &str, values: &[String], enabled: bool) -> egui::Response {
    let fill = if enabled { Color32::from_rgb(120, 190, 250) } else { Color32::from_rgb(200, 226, 250) };
    let r = ui.add_enabled(enabled, egui::Button::new(RichText::new(tr(label)).size(17.0).color(Color32::WHITE)).fill(fill).rounding(14.0).min_size(Vec2::new(320.0, 34.0)));
    for (i, v) in values.iter().enumerate() {
        let prefix = if i == 0 { "   » " } else { "        " };
        ui.label(RichText::new(format!("{prefix}{}", tr(v))).size(18.0).color(Color32::from_rgb(150, 150, 160)));
    }
    r
}

/// Pill slider with the value shown on a blue knob and Min/Max captions.
pub(crate) fn pill_slider(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, left: &str, right: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let knob_w = 96.0;
    if let Some(p) = resp.interact_pointer_pos() {
        let t = ((p.x - rect.left() - knob_w / 2.0) / (rect.width() - knob_w)).clamp(0.0, 1.0);
        let nv = (lo + t * (hi - lo)).round();
        if nv != *v {
            *v = nv;
            resp.mark_changed();
        }
    }
    let t = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
    let p = ui.painter();
    p.rect_filled(rect, 16.0, Color32::from_rgb(236, 242, 250));
    p.text(rect.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, left, FontId::proportional(13.0), Color32::GRAY);
    p.text(rect.right_center() - Vec2::new(12.0, 0.0), Align2::RIGHT_CENTER, right, FontId::proportional(13.0), Color32::GRAY);
    let kx = rect.left() + t * (rect.width() - knob_w);
    let knob = egui::Rect::from_min_size(Pos2::new(kx, rect.top()), Vec2::new(knob_w, rect.height()));
    p.rect_filled(knob, 16.0, BLUE);
    p.text(knob.center(), Align2::CENTER_CENTER, format!("{:.0}", v), FontId::proportional(19.0), Color32::WHITE);
    resp
}

const FACE_LINES: &[(usize, usize, bool)] = &[(0, 16, false), (17, 21, false), (22, 26, false), (27, 30, false), (31, 35, false), (36, 41, true), (42, 47, true), (48, 59, true), (60, 65, true)];

/// Floating preview card with the tracked landmarks (centre of the screen).
fn camera_preview(app: &mut App, ctx: &Context, e: f32) {
    if !app.settings.show_cam_preview {
        return;
    }
    let screen = ctx.screen_rect();
    let x = screen.width() - PANEL_W - 96.0 - 490.0 + (1.0 - e) * (PANEL_W + 640.0);
    egui::Area::new(egui::Id::new("cam-preview")).fixed_pos(Pos2::new(x, BAR_H + 26.0)).show(ctx, |ui| {
        ui.set_width(470.0);
        card_frame().show(ui, |ui| {
            ui.set_width(440.0);
            let face = app.last_face.filter(|_| app.tracking);
            ui.horizontal(|ui| {
                let dot = if face.is_some() { Color32::from_rgb(90, 200, 110) } else { Color32::from_rgb(240, 120, 120) };
                status_dot(ui, dot);
                let fps = app.tracker.as_ref().map(|t| t.rate).unwrap_or(0.0);
                let conf = face.map(|f| f.confidence * 100.0).unwrap_or(0.0);
                ui.label(RichText::new(format!("{fps:.0} FPS, Confidence:   {conf:.0}%")).size(19.0).color(Color32::GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "✖", FontId::proportional(28.0), Color32::from_rgb(240, 96, 96));
                    if resp.on_hover_text(tr("Скрыть превью")).clicked() {
                        app.settings.show_cam_preview = false;
                    }
                });
            });
            let (rect, _) = ui.allocate_exact_size(Vec2::new(440.0, 270.0), Sense::hover());
            let mirror = app.settings.track.mirror;
            // Normalised-image → preview mapping (Mediapipe face and hands use 0..1 coordinates).
            let aspect = face.map(|f| f.size[0].max(1.0) / f.size[1].max(1.0)).unwrap_or(4.0 / 3.0);
            let bw = rect.width().min(rect.height() * aspect);
            let bh = bw / aspect;
            let norm = |x: f32, y: f32| {
                let x = if mirror { 1.0 - x } else { x };
                Pos2::new(rect.center().x - bw / 2.0 + x * bw, rect.center().y - bh / 2.0 + y * bh)
            };
            let hands = app.last_hands;
            if let Some(h) = hands {
                const BONES: [(usize, usize); 20] = [(0, 1), (1, 2), (2, 3), (3, 4), (0, 5), (5, 6), (6, 7), (7, 8), (5, 9), (9, 10), (10, 11), (11, 12), (9, 13), (13, 14), (14, 15), (15, 16), (13, 17), (0, 17), (17, 18), (18, 19)];
                let p = ui.painter();
                for hand in h.hands.iter().flatten() {
                    let pt = |i: usize| norm(hand.pts[i][0], hand.pts[i][1]);
                    for &(a, b) in BONES.iter() {
                        p.line_segment([pt(a), pt(b)], Stroke::new(2.0, Color32::from_rgb(90, 190, 120)));
                    }
                    p.line_segment([pt(19), pt(20)], Stroke::new(2.0, Color32::from_rgb(90, 190, 120)));
                    for i in 0..21 {
                        p.circle_filled(pt(i), 3.0, Color32::from_rgb(40, 160, 90));
                    }
                }
            }
            if let Some(f) = face.filter(|f| f.n_mp > 0) {
                let p = ui.painter();
                for q in f.mp[..f.n_mp as usize].iter() {
                    p.circle_filled(norm(q[0], q[1]), 1.1, Color32::from_rgb(40, 140, 230));
                }
            } else if let Some(f) = face {
                let (fw, fh) = (f.size[0].max(1.0), f.size[1].max(1.0));
                let s = (rect.width() / fw).min(rect.height() / fh);
                let off = rect.center() - Vec2::new(fw * s, fh * s) / 2.0;
                let pt = |i: usize| {
                    let l = f.lms[i];
                    let x = if mirror { fw - l[0] } else { l[0] };
                    Pos2::new(off.x + x * s, off.y + l[1] * s)
                };
                let p = ui.painter();
                let line = Stroke::new(1.5, Color32::from_rgb(60, 150, 230));
                for &(a, b, closed) in FACE_LINES {
                    for i in a..b {
                        p.line_segment([pt(i), pt(i + 1)], line);
                    }
                    if closed {
                        p.line_segment([pt(b), pt(a)], line);
                    }
                }
                for i in 0..66 {
                    p.circle_filled(pt(i), 2.6, Color32::from_rgb(40, 140, 230));
                }
                for i in [66, 67] {
                    p.circle_filled(pt(i), 4.0, Color32::from_rgb(240, 80, 100));
                }
            } else if hands.map(|h| h.hands.iter().all(|x| x.is_none())).unwrap_or(true) {
                let msg = if app.tracker.is_some() { "Лицо не найдено" } else { "Камера выключена" };
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(20.0), Color32::GRAY);
            }
        });
        ui.add_space(10.0);
        card_frame().inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
            ui.set_width(454.0);
            ui.horizontal(|ui| {
                let b = egui::Button::new(RichText::new(tr("Калибровать")).size(19.0).color(Color32::WHITE)).fill(BLUE).rounding(12.0);
                if ui.add_sized([350.0, 46.0], b).clicked() {
                    app.actions.push(Action::Calibrate);
                }
                let name = match app.settings.track_backend {
                    1 => "Media\npipe",
                    2 => "NVIDIA\nRTX",
                    3 => "Phone\nARKit",
                    _ => "OpenSee\nFace",
                };
                ui.label(RichText::new(name).size(13.0).strong().color(Color32::from_rgb(220, 70, 90)));
            });
        });
    });
}

fn camera_tab(app: &mut App, ui: &mut egui::Ui) {
    section(ui, tr("Отслеживание камерой"), |ui| {
        let running = app.tracker.is_some();
        // Camera
        let cam_name = app.cameras.iter().find(|c| c.0 == app.settings.camera).map(|c| c.1.clone()).unwrap_or_else(|| format!("Камера #{}", app.settings.camera));
        let mut vals = vec![cam_name, format!("- {} x {} ({} FPS)", app.settings.cam_size[0], app.settings.cam_size[1], app.settings.cam_fps)];
        if !app.settings.cam_format.is_empty() {
            vals.push(format!("- {}", app.settings.cam_format));
        }
        if picker(ui, tr("Выбрать камеру"), &vals, !running).clicked() {
            app.actions.push(Action::RefreshCameras);
            app.ui.modal = Some(Modal::Camera);
            app.ui.modal_sel = None;
        }
        ui.add_space(4.0);
        let qlabel = match app.settings.track_backend {
            1 => "[Google] Трекер Mediapipe".to_string(),
            2 => "[RTX] NVIDIA Broadcast".to_string(),
            3 => tr("[Телефон] iPhone / Android").to_string(),
            _ => quality_label(app.settings.track_quality).0.to_string(),
        };
        if picker(ui, tr("Качество отслеживания"), &[qlabel], !running).clicked() {
            app.ui.modal = Some(Modal::Quality);
            app.ui.modal_sel = None;
        }
        ui.add_space(4.0);
        crate::ui_net::tracker_extra(app, ui, running);
        let face_only = app.settings.track_backend >= crate::tracking::backend::NVIDIA;
        let tlabel = ["Только лицо", "Только руки", "Лицо и руки"][if face_only { 0 } else { app.settings.track_type.min(2) as usize }].to_string();
        if picker(ui, tr("Тип отслеживания"), &[tlabel], !running && !face_only).clicked() {
            app.ui.modal = Some(Modal::TrackType);
            app.ui.modal_sel = None;
        }
        ui.add_space(8.0);
        ui.label(RichText::new(tr("Ограничение FPS веб-камеры")).size(20.0).color(if running { Color32::GRAY } else { TEXT }));
        let mut fps = app.settings.cam_fps as f32;
        ui.add_enabled_ui(!running, |ui| {
            if pill_slider(ui, &mut fps, 10.0..=100.0, "10 FPS", "").changed() {
                app.settings.cam_fps = fps as u32;
            }
        });
        ui.add_space(16.0);
        toggle(ui, &mut app.settings.track_autostart, tr("Автозапуск камеры при старте"));
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 12.0) / 2.0;
            let on = egui::Button::new(RichText::new(tr("Включить камеру")).size(17.0).color(Color32::WHITE)).fill(if running { Color32::from_rgb(200, 226, 250) } else { BLUE }).rounding(14.0);
            if ui.add_sized([w, 36.0], on).clicked() {
                app.actions.push(Action::StartTracker);
            }
            let off = egui::Button::new(RichText::new(tr("Выключить камеру")).size(17.0).color(Color32::WHITE)).fill(if running { BLUE } else { Color32::from_rgb(200, 226, 250) }).rounding(14.0);
            if ui.add_sized([w, 36.0], off).clicked() {
                app.actions.push(Action::StopTracker);
            }
        });
        let status = match (&app.tracker, &app.tracker_error) {
            (_, Some(e)) => format!("ошибка — {e}"),
            (Some(_), None) if app.tracking => "отслеживание включено".into(),
            (Some(_), None) => "ищу лицо…".into(),
            (None, None) => "камера выключена".into(),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Статус:")).size(18.0).strong());
            ui.label(RichText::new(status).size(18.0));
        });
        if !app.settings.show_cam_preview && ui.button(tr("Показать превью трекинга")).clicked() {
            app.settings.show_cam_preview = true;
        }
    });

    section(ui, tr("Настройки отслеживания"), |ui| {
        ui.label(RichText::new(tr("Модель при потере отслеживания")).size(20.0));
        ui.radio_value(&mut app.settings.lost_reset, false, RichText::new(tr("Остаётся в текущей позе")).size(17.0));
        ui.radio_value(&mut app.settings.lost_reset, true, RichText::new(tr("Возвращается в исходную позу")).size(17.0));
        ui.add_space(8.0);
        let t = &mut app.settings.track;
        ui.label(RichText::new(tr("Синхронизация моргания")).size(20.0));
        ui.radio_value(&mut t.blink_sync, 0, RichText::new(tr("Никогда")).size(17.0));
        ui.radio_value(&mut t.blink_sync, 1, RichText::new(tr("Всегда")).size(17.0));
        ui.radio_value(&mut t.blink_sync, 2, RichText::new(tr("При повороте головы")).size(17.0));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Чувствительность моргания")).size(17.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new(tr("   Сброс   ")).size(17.0).color(Color32::WHITE)).fill(BLUE).rounding(14.0)).clicked() {
                    t.sens = [50.0; 5];
                }
            });
        });
        pill_slider(ui, &mut t.sens[0], 0.0..=100.0, "Min", "Max");
        for (i, name) in [(1, "Чувствительность открытия глаз"), (2, "Чувствительность открытия рта"), (3, "Чувствительность улыбки"), (4, "Чувствительность движения бровей")] {
            ui.add_space(8.0);
            ui.label(RichText::new(name).size(17.0));
            pill_slider(ui, &mut t.sens[i], 0.0..=100.0, "Min", "Max");
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Направления осей")).size(20.0));
        toggle(ui, &mut t.mirror, tr("Зеркальный режим (как в зеркале)"));
        toggle(ui, &mut t.invert[0], tr("Инвертировать поворот головы"));
        toggle(ui, &mut t.invert[1], tr("Инвертировать наклон вверх/вниз"));
        toggle(ui, &mut t.invert[2], tr("Инвертировать наклон вбок"));
        toggle(ui, &mut t.invert_gaze[0], tr("Инвертировать взгляд по X"));
        toggle(ui, &mut t.invert_gaze[1], tr("Инвертировать взгляд по Y"));
        egui::CollapsingHeader::new(tr("Текущие значения трекинга")).show(ui, |ui| {
            for (i, n) in INPUT_NAMES.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_sized([150.0, 16.0], egui::Label::new(*n));
                    ui.label(format!("{:+.2}", app.inputs[i]));
                });
            }
        });
        egui::CollapsingHeader::new(tr("Perfect Sync / VBridger (ARKit)")).show(ui, |ui| {
            ui.label(RichText::new(tr("Доступны с трекерами NVIDIA, телефоном и Mediapipe. Назначаются на параметры Live2D по имени, как кастомные параметры VBridger.")).size(14.0).color(Color32::GRAY));
            for (n, x) in crate::perfect_sync::snapshot() {
                ui.horizontal(|ui| {
                    ui.add_sized([170.0, 16.0], egui::Label::new(n));
                    ui.label(format!("{x:+.2}"));
                });
            }
        });
    });

    section(ui, tr("Трекер глаз Tobii"), |ui| {
        let mut on = app.settings.tobii_enabled;
        if toggle(ui, &mut on, tr("Активировать Tobii Eye Tracker")).changed() {
            app.actions.push(Action::SetTobii(on));
        }
        ui.add_enabled_ui(on, |ui| {
            toggle(ui, &mut app.settings.tobii_replace_head, tr("Заменить на отслеживание головы Tobii")).on_hover_text(tr("Углы и положение головы берутся из Tobii вместо камеры"));
            toggle(ui, &mut app.settings.tobii_replace_eyes, tr("Заменить на отслеживание глаз Tobii")).on_hover_text(tr("Направление взгляда берётся из точки взгляда на экране"));
            toggle(ui, &mut app.settings.tobii_show_gaze, tr("Отображать точку взгляда"));
            ui.add_space(4.0);
            if ui.add_sized([320.0, 34.0], egui::Button::new(RichText::new(tr("Калибровать")).size(17.0).color(Color32::WHITE)).fill(BLUE).rounding(6.0)).on_hover_text(tr("Смотрите в центр экрана прямо и нажмите")).clicked() {
                app.actions.push(Action::CalibrateTobii);
            }
        });
        ui.add_space(8.0);
        let st = &app.tobii_state;
        ui.horizontal(|ui| {
            status_dot(ui, if st.connected { Color32::from_rgb(90, 200, 110) } else { Color32::GRAY });
            ui.label(RichText::new(tr("Статус:")).size(18.0).strong());
        });
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(96.0, 60.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, 2.0, Color32::from_rgb(220, 230, 245));
            p.rect_stroke(r, 2.0, Stroke::new(1.0, Color32::from_rgb(60, 60, 70)));
            p.line_segment([Pos2::new(r.center().x, r.top()), Pos2::new(r.center().x, r.bottom())], Stroke::new(0.5, Color32::GRAY));
            p.line_segment([Pos2::new(r.left(), r.center().y), Pos2::new(r.right(), r.center().y)], Stroke::new(0.5, Color32::GRAY));
            if let Some(g) = st.gaze {
                p.circle_filled(Pos2::new(r.left() + g[0].clamp(0.0, 1.0) * r.width(), r.top() + g[1].clamp(0.0, 1.0) * r.height()), 3.0, Color32::from_rgb(240, 70, 80));
            }
            let status = if !on { "Трекер выключен".to_string() } else { st.status.clone() };
            ui.add(egui::Label::new(RichText::new(status).size(17.0)).wrap());
        });
    });

    section(ui, tr("Качество и эффекты"), |ui| {
        ui.label(RichText::new(tr("Частота кадров приложения")).size(20.0));
        let before = app.settings.fps;
        for m in FpsMode::ALL {
            ui.radio_value(&mut app.settings.fps, m, RichText::new(m.label()).size(17.0));
        }
        if before != app.settings.fps {
            app.gpu.set_vsync(app.settings.fps.vsync());
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let ws = &mut app.ui.win_size;
            if ws[0] == 0 {
                *ws = [app.gpu.config.width, app.gpu.config.height];
            }
            ui.add(egui::DragValue::new(&mut ws[0]).range(320..=7680));
            ui.label("X");
            ui.add(egui::DragValue::new(&mut ws[1]).range(240..=4320));
            if ui.add(egui::Button::new(RichText::new(tr("Установить разрешение")).size(16.0).color(Color32::WHITE)).fill(BLUE).rounding(14.0)).clicked() {
                let _ = app.window.request_inner_size(winit::dpi::PhysicalSize::new(ws[0], ws[1]));
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Настройки маски Live2D")).size(22.0));
        toggle(ui, &mut app.settings.smooth_masks, tr("Сглаженные маски Live2D")).on_hover_text(tr("Выкл.: жёсткие маски через stencil (0 МБ видеопамяти). Вкл.: мягкие края через текстуру масок."));
        if blue_button(ui, tr("Настроить"), BLUE, 38.0).clicked() {
            open_info(app, INFO_MASKS, Some(Modal::MaskRes));
        }
        let (s, m) = (app.settings.mask_atlas, app.settings.mask_max);
        let c = mask_cell(s, m);
        let pink = Color32::from_rgb(236, 30, 160);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(RichText::new(format!("{s} x {s}")).color(pink));
            ui.label(tr(" пикселей (макс. "));
            ui.label(RichText::new(format!("{m} масок")).color(pink));
            ui.label(")");
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(RichText::new(format!("{c} x {c}")).color(pink));
            ui.label(tr(" пикселей на маску"));
        });
        if app.settings.smooth_masks {
            ui.label(RichText::new(format!("Сейчас используется масок: {} · видеопамять атласа {} МБ", app.stats.masks, (s as u64 * s as u64 * 4) >> 20)).small().weak());
        }
    });

    section(ui, tr("Визуальные эффекты (VFX)"), |ui| {
        let n = crate::vfx::active_effects(&app.settings.vfx).len();
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("?").size(18.0).strong().color(Color32::WHITE)).fill(Color32::from_rgb(60, 60, 66)).rounding(14.0)).clicked() {
                app.settings.dismissed_info.retain(|d| *d != INFO_VFX_ALPHA && *d != INFO_VFX_ARTISTS);
                open_info(app, INFO_VFX_ALPHA, None);
            }
            if toggle(ui, &mut app.settings.vfx_on, tr("Активировать")).changed() {
                let ev = serde_json::json!({ "currentOnState": app.settings.vfx_on, "currentPreset": app.vfx_preset });
                app.api_events.push(("PostProcessingEvent".into(), ev));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let st = if app.settings.vfx_on && n > 0 { "Вкл." } else { "Выкл." };
                ui.label(RichText::new(st).size(18.0).color(Color32::GRAY));
            });
        });
        ui.add_space(8.0);
        if blue_button(ui, &format!("Число выбранных эффектов: {n}"), BLUE, 40.0).clicked() {
            open_info(app, INFO_VFX_ALPHA, Some(Modal::VfxList));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Пресеты")).size(19.0));
            ui.add_space(10.0);
            let btn = |ui: &mut egui::Ui, icon: &str, tip: &str| {
                ui.add_sized([62.0, 40.0], egui::Button::new(RichText::new(icon).size(20.0).color(Color32::WHITE)).fill(BLUE).rounding(10.0)).on_hover_text(tip).clicked()
            };
            if btn(ui, "💾", "Сохранить текущие эффекты как пресет") {
                app.ui.vfx_save_open = !app.ui.vfx_save_open;
            }
            if btn(ui, "📂", "Загрузить пресет") {
                app.ui.modal = Some(Modal::VfxPresetLoad);
                app.ui.modal_sel = None;
            }
            if btn(ui, "🗑", "Удалить пресет") {
                app.ui.modal = Some(Modal::VfxPresetDelete);
                app.ui.modal_sel = None;
            }
            if btn(ui, "⚙", "Открыть папку пресетов") {
                let d = app.paths.data.join("vfx_presets");
                let _ = std::fs::create_dir_all(&d);
                let _ = std::process::Command::new("explorer").arg(&d).spawn();
            }
        });
        toggle(ui, &mut app.settings.vfx_restricted, tr("Разрешить экспериментальные эффекты"));
        if !app.vfx_preset.is_empty() {
            ui.label(RichText::new(format!("{} {}", tr("Активный пресет:"), app.vfx_preset)).weak());
        }
        if app.ui.vfx_save_open {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut app.ui.vfx_preset_name).hint_text(tr("Название пресета")).desired_width(300.0));
                let name = app.ui.vfx_preset_name.trim().replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
                if ui.add_enabled(!name.is_empty(), egui::Button::new(tr("Сохранить"))).clicked() {
                    let d = app.paths.data.join("vfx_presets");
                    let _ = std::fs::create_dir_all(&d);
                    let _ = d;
                    if crate::vfx::save_preset(&app.paths.data, &name, &app.settings.vfx).is_ok() {
                        app.vfx_preset = name.clone();
                        app.toast(format!("Пресет «{name}» сохранён"));
                    }
                    app.ui.vfx_save_open = false;
                    app.ui.vfx_preset_name.clear();
                }
            });
        }
    });
    section(ui, tr("Наложение освещения"), |ui| {
        ui.label(RichText::new(tr("Экран")).size(19.0));
        let mode = app.settings.light_mode;
        ui.horizontal(|ui| {
            let off = egui::Button::new(RichText::new(tr("Выкл.")).size(17.0).color(Color32::WHITE)).fill(if mode == 0 { BLUE } else { Color32::from_rgb(150, 200, 245) }).rounding(14.0);
            if ui.add_sized([96.0, 34.0], off).clicked() {
                app.actions.push(Action::SetLight(0, 0));
            }
            ui.add_space(14.0);
            for i in 0..5usize {
                let exists = i < app.monitors.len();
                let sel = mode == 1 && app.settings.light_monitor == i;
                let col = if sel { BLUE } else if exists { Color32::from_rgb(150, 200, 245) } else { Color32::from_rgb(214, 228, 245) };
                if ui.add_enabled(exists, egui::Button::new(RichText::new(format!("{}", i + 1)).size(17.0).color(Color32::WHITE)).fill(col).rounding(14.0).min_size(Vec2::new(40.0, 34.0))).on_hover_text(format!("Монитор {}", i + 1)).clicked() {
                    app.actions.push(Action::SetLight(1, i));
                }
            }
            let mut c = app.settings.light_color;
            let r = ui.color_edit_button_rgb(&mut c).on_hover_text(tr("Свой цвет освещения"));
            if r.changed() || (r.clicked() && mode != 3) {
                app.settings.light_color = c;
                if mode != 3 {
                    app.actions.push(Action::SetLight(3, 0));
                }
            }
        });
        let wlabel = if mode == 2 { format!("Окно: {}", app.settings.light_window) } else { "Окно".to_string() };
        if blue_button(ui, &wlabel, if mode == 2 { BLUE } else { Color32::from_rgb(90, 165, 240) }, 36.0).clicked() {
            app.light_windows = crate::lighting::windows();
            app.ui.modal = Some(Modal::LightWindow);
            app.ui.modal_sel = None;
        }
        ui.add_space(8.0);
        ui.columns(2, |c| {
            c[0].label(RichText::new(tr("Настройка цвета")).size(19.0));
            c[0].label(RichText::new(tr("Яркость")).size(17.0));
            pill_knob(&mut c[0], &mut app.settings.light_brightness, 0.0..=100.0, "Min", "Max", "");
            c[0].label(RichText::new(tr("Наложение")).size(17.0));
            pill_knob(&mut c[0], &mut app.settings.light_overlay, 0.0..=100.0, "Min", "Max", "");
            c[0].label(RichText::new(tr("Сглаживание")).size(17.0));
            pill_knob(&mut c[0], &mut app.settings.light_smoothing, 0.0..=100.0, "", "Max", "");
            // Live capture preview.
            let ui = &mut c[1];
            ui.add_space(30.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 120.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, 4.0, Color32::from_rgb(230, 205, 210));
            p.rect_stroke(r, 4.0, Stroke::new(1.5, Color32::from_rgb(60, 60, 70)));
            let s = &app.light_sample;
            if mode == 3 {
                let c = app.settings.light_color;
                p.rect_filled(r.shrink(6.0), 2.0, Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8));
            } else if mode != 0 && s.ok && s.thumb.len() == crate::lighting::THUMB_W * crate::lighting::THUMB_H * 4 {
                let cw = r.width() / crate::lighting::THUMB_W as f32;
                let ch = r.height() / crate::lighting::THUMB_H as f32;
                for y in 0..crate::lighting::THUMB_H {
                    for x in 0..crate::lighting::THUMB_W {
                        let k = (y * crate::lighting::THUMB_W + x) * 4;
                        let cell = egui::Rect::from_min_size(Pos2::new(r.left() + x as f32 * cw, r.top() + y as f32 * ch), Vec2::new(cw + 0.5, ch + 0.5));
                        p.rect_filled(cell, 0.0, Color32::from_rgb(s.thumb[k], s.thumb[k + 1], s.thumb[k + 2]));
                    }
                }
            } else {
                p.text(r.center(), Align2::CENTER_CENTER, "NO SIGNAL", FontId::proportional(20.0), Color32::from_rgb(150, 40, 60));
            }
            let t = app.light_tint;
            ui.horizontal(|ui| {
                ui.label(tr("Итоговый цвет:"));
                let (sw, _) = ui.allocate_exact_size(Vec2::new(40.0, 18.0), Sense::hover());
                ui.painter().rect_filled(sw, 3.0, Color32::from_rgb((t[0].min(1.0) * 255.0) as u8, (t[1].min(1.0) * 255.0) as u8, (t[2].min(1.0) * 255.0) as u8));
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            toggle(ui, &mut app.settings.light_items, tr("Наложение\nна элементы"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new(tr("   Сброс   ")).size(17.0).color(Color32::WHITE)).fill(BLUE).rounding(14.0)).clicked() {
                    app.settings.light_brightness = 37.0;
                    app.settings.light_overlay = 40.0;
                    app.settings.light_smoothing = 3.0;
                }
            });
        });
        if mode != 0 {
            ui.ctx().request_repaint();
        }
    });

    section(ui, tr("Настройка виртуальной камеры"), |ui| {
        if blue_button(ui, tr("Помощь с установкой"), BLUE, 36.0).clicked() {
            open_info(app, INFO_VCAM, None);
        }
        let installed = crate::vcam::installed();
        let mut on = app.settings.vcam_on;
        if toggle(ui, &mut on, tr("Включить виртуальную камеру")).changed() {
            app.actions.push(Action::SetVcam(on));
        }
        if toggle(ui, &mut app.settings.vcam_mirror, tr("Отзеркалить видео  ")).changed() {
            if let Some(v) = &app.vcam {
                v.set_mirror(&app.gpu, app.settings.vcam_mirror);
            }
        }
        let (txt, col) = match (&app.vcam, &app.vcam_error, installed) {
            (_, Some(e), _) => (e.clone(), Color32::RED),
            (Some(v), None, _) if v.vcam_connected() => ("rigrsCam работает: изображение передаётся".to_string(), Color32::from_rgb(30, 150, 60)),
            (Some(_), None, _) => ("rigrsCam включена — выберите её в OBS / Discord / Zoom".to_string(), Color32::from_rgb(30, 120, 200)),
            (None, None, false) => ("Камера не установлена — нажмите «Помощь с установкой»".to_string(), Color32::from_rgb(220, 120, 0)),
            (None, None, true) => ("Установлена, выключена".to_string(), Color32::GRAY),
        };
        ui.add(egui::Label::new(RichText::new(txt).color(col)).wrap());
        ui.label(RichText::new(tr("1280×720, 30 FPS, с фоном и эффектами")).small().weak());
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Настройки Spout2")).size(22.0));
        if blue_button(ui, tr("Установить Spout2"), BLUE, 36.0).on_hover_text(tr("Spout2 нужен программе-получателю (например, плагин OBS Spout2)")).clicked() {
            app.actions.push(Action::OpenUrl("https://spout.zeal.co/".into()));
        }
        let mut on = app.settings.spout;
        if toggle(ui, &mut on, tr("Активировать Spout2")).changed() {
            app.actions.push(Action::SetSpout(on));
        }
        if toggle(ui, &mut app.settings.spout_mirror, tr("Отзеркалить видео")).changed() {
            if let Some(s) = &app.spout {
                s.set_mirror(&app.gpu, app.settings.spout_mirror);
            }
        }
        ui.horizontal(|ui| {
            ui.label(tr("Имя отправителя"));
            if ui.text_edit_singleline(&mut app.settings.spout_name).lost_focus() && app.settings.spout {
                app.actions.push(Action::SetSpout(true));
            }
        });
        if let Some(s) = &app.spout {
            let (w, h) = s.size();
            ui.label(RichText::new(format!("Отправляется {w}×{h}, кадров: {}", s.frames_sent)).color(Color32::from_rgb(30, 150, 60)));
        }
        if let Some(e) = &app.spout_error {
            ui.colored_label(Color32::RED, e);
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Протокол NDI")).size(22.0));
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new(tr("   Подробнее   ")).size(17.0).color(Color32::WHITE)).fill(BLUE).rounding(14.0)).clicked() {
                app.actions.push(Action::OpenUrl("https://ndi.video/tools/".into()));
            }
            if !crate::ndi::runtime_installed() {
                ui.label(RichText::new(tr("NDI Runtime не найден — установите NDI Tools")).color(Color32::from_rgb(220, 120, 0)));
            }
        });
        let mut on = app.settings.ndi;
        if toggle(ui, &mut on, tr("Активировать NDI")).changed() {
            app.actions.push(Action::SetNdi(on));
        }
        if toggle(ui, &mut app.settings.ndi_mirror, tr("Отзеркалить видео ")).changed() {
            if let Some(s) = &app.ndi {
                s.set_mirror(&app.gpu, app.settings.ndi_mirror);
            }
        }
        ui.horizontal(|ui| {
            let ch = toggle(ui, &mut app.settings.ndi_custom_size, tr("Разрешение NDI")).changed();
            let a = ui.add(egui::DragValue::new(&mut app.settings.ndi_size[0]).range(16..=7680));
            ui.label("X");
            let b = ui.add(egui::DragValue::new(&mut app.settings.ndi_size[1]).range(16..=4320));
            if (ch || a.lost_focus() || b.lost_focus() || a.drag_stopped() || b.drag_stopped()) && app.settings.ndi {
                app.actions.push(Action::SetNdi(true));
            }
        });
        ui.horizontal(|ui| {
            ui.label(tr("Имя источника"));
            if ui.text_edit_singleline(&mut app.settings.ndi_name).lost_focus() && app.settings.ndi {
                app.actions.push(Action::SetNdi(true));
            }
        });
        if let Some(s) = &app.ndi {
            let (w, h) = s.size();
            ui.label(RichText::new(format!("Отправляется {w}×{h}, кадров: {}", s.frames_sent)).color(Color32::from_rgb(30, 150, 60)));
        }
        if let Some(e) = &app.ndi_error {
            ui.colored_label(Color32::RED, e);
        }
    });

    section(ui, tr("Запись анимации Live2D"), |ui| {
        let rec = app.recorder.is_some();
        {
            let c = Pos2::new(ui.max_rect().right() - 18.0, ui.min_rect().top() + 16.0);
            let col = if rec && (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0 { Color32::from_rgb(235, 60, 70) } else { Color32::from_rgb(222, 222, 228) };
            ui.painter().circle_filled(c, 16.0, col);
            ui.painter().circle_stroke(c, 16.0, Stroke::new(1.0, Color32::from_rgb(190, 190, 196)));
        }
        ui.add_enabled_ui(!rec, |ui| {
            toggle(ui, &mut app.settings.rec_physics, tr("Запись параметров физики"));
            toggle(ui, &mut app.settings.rec_unchanged, tr("Запись неизменённых параметров"));
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Длительность записи")).size(18.0));
            pill_knob(ui, &mut app.settings.rec_seconds, 2.0..=60.0, "2s", "60s", "");
        });
        ui.add_space(8.0);
        let label = if let Some(r) = &app.recorder { format!("Остановить запись ({:.1} / {:.0} с)", r.elapsed(), r.duration) } else { "Начать запись Live2D".to_string() };
        if blue_button(ui, &label, if rec { Color32::from_rgb(222, 88, 116) } else { BLUE }, 40.0).clicked() {
            app.actions.push(Action::ToggleRecording);
        }
        ui.add_space(6.0);
        let status = if rec { "Идёт запись… Двигайтесь перед камерой.".to_string() } else { app.rec_status.clone() };
        ui.label(RichText::new(status).size(17.0));
        if let (false, Some(p)) = (rec, &app.rec_last) {
            if ui.button(tr("📁 Открыть папку с записями")).clicked() {
                if let Some(d) = p.parent() {
                    let _ = std::process::Command::new("explorer").arg(d).spawn();
                }
            }
        }
        if rec {
            ui.ctx().request_repaint();
        }
    });
}

/// Pill slider whose knob shows a caption (e.g. "X" or "Сглаживание") and the value above it.
fn pill_knob(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, left: &str, right: &str, knob_text: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, 50.0), Sense::click_and_drag());
    let track = egui::Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 18.0), rect.max);
    let (lo, hi) = (*range.start(), *range.end());
    let knob_w = 104.0;
    if let Some(p) = resp.interact_pointer_pos() {
        let t = ((p.x - track.left() - knob_w / 2.0) / (track.width() - knob_w)).clamp(0.0, 1.0);
        let nv = (lo + t * (hi - lo)).round();
        if nv != *v {
            *v = nv;
            resp.mark_changed();
        }
    }
    let t = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
    let enabled = ui.is_enabled();
    let p = ui.painter();
    p.rect_filled(track, 16.0, Color32::from_rgb(236, 242, 250));
    p.text(track.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, left, FontId::proportional(13.0), Color32::GRAY);
    p.text(track.right_center() - Vec2::new(12.0, 0.0), Align2::RIGHT_CENTER, right, FontId::proportional(13.0), Color32::GRAY);
    let kx = track.left() + t * (track.width() - knob_w);
    let knob = egui::Rect::from_min_size(Pos2::new(kx, track.top()), Vec2::new(knob_w, track.height()));
    p.rect_filled(knob, 16.0, if enabled { BLUE } else { Color32::from_rgb(170, 210, 245) });
    p.text(knob.center(), Align2::CENTER_CENTER, knob_text, FontId::proportional(15.0), Color32::WHITE);
    p.text(Pos2::new(knob.center().x, rect.top() + 8.0), Align2::CENTER_CENTER, format!("{:.0}", v), FontId::proportional(14.0), Color32::GRAY);
    resp
}

/// Small solid blue button with "» value" under it (used for Вход / Выход).
fn io_button(ui: &mut egui::Ui, label: &str, value: &str) -> egui::Response {
    let r = ui.add(egui::Button::new(RichText::new(label).size(16.0).color(Color32::WHITE)).fill(BLUE).rounding(2.0).min_size(Vec2::new(160.0, 32.0)));
    ui.label(RichText::new(format!("  » {value}")).size(22.0).color(TEXT));
    r
}

/// IN → OUT diagram with editable range fields and live value markers.
fn io_diagram(ui: &mut egui::Ui, r: &mut Rule, in_val: f32, out_val: Option<f32>, prange: (f32, f32)) -> bool {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 200.0), Sense::hover());
    let lx = rect.left() + 70.0;
    let rx = rect.right() - 90.0;
    let (top, bot) = (rect.top() + 34.0, rect.bottom() - 34.0);
    let mid = (top + bot) / 2.0;
    let dash = Color32::from_rgb(170, 170, 178);
    let p = ui.painter().clone();
    for x in [lx, rx] {
        let mut y = rect.top() + 8.0;
        while y < rect.bottom() - 8.0 {
            p.circle_filled(Pos2::new(x, y), 1.2, dash);
            y += 6.0;
        }
    }
    let line = Stroke::new(1.6, Color32::from_rgb(50, 50, 56));
    let in_hi_y = top + 14.0;
    let in_lo_y = bot - 14.0;
    let (out_hi_y, out_lo_y) = (top - 4.0, bot + 4.0);
    let step_x = rx - 130.0;
    p.line_segment([Pos2::new(lx + 10.0, in_hi_y - 14.0), Pos2::new(step_x, in_hi_y - 14.0)], line);
    p.line_segment([Pos2::new(step_x, in_hi_y - 14.0), Pos2::new(step_x, out_hi_y - 14.0)], line);
    p.line_segment([Pos2::new(step_x, out_hi_y - 14.0), Pos2::new(rx - 4.0, out_hi_y - 14.0)], line);
    p.line_segment([Pos2::new(lx + 10.0, in_lo_y + 14.0), Pos2::new(step_x, in_lo_y + 14.0)], line);
    p.line_segment([Pos2::new(step_x, in_lo_y + 14.0), Pos2::new(step_x, out_lo_y + 14.0)], line);
    p.line_segment([Pos2::new(step_x, out_lo_y + 14.0), Pos2::new(rx - 4.0, out_lo_y + 14.0)], line);
    p.text(Pos2::new(lx + 12.0, mid), Align2::LEFT_CENTER, "IN", FontId::proportional(18.0), TEXT);
    p.text(Pos2::new(rx - 12.0, mid), Align2::RIGHT_CENTER, "OUT", FontId::proportional(18.0), TEXT);
    p.text(Pos2::new(lx - 10.0, mid), Align2::RIGHT_CENTER, format!("{in_val:.2}"), FontId::proportional(17.0), TEXT);
    let ov = out_val.unwrap_or(0.0);
    p.text(Pos2::new(rx + 10.0, mid), Align2::LEFT_CENTER, format!("{ov:.2}"), FontId::proportional(17.0), TEXT);
    p.text(Pos2::new(rx - 4.0, rect.top() + 8.0), Align2::RIGHT_CENTER, format!("{:.0} Max", prange.1), FontId::proportional(12.0), Color32::GRAY);
    p.text(Pos2::new(rx - 4.0, rect.bottom() - 6.0), Align2::RIGHT_CENTER, format!("{:.0} Min", prange.0), FontId::proportional(12.0), Color32::GRAY);
    // Live markers.
    let red = Color32::from_rgb(240, 70, 80);
    let span = r.in_range[1] - r.in_range[0];
    let tin = if span.abs() > 1e-6 { (in_val - r.in_range[0]) / span } else { 0.5 };
    p.circle_filled(Pos2::new(lx, egui::lerp(in_lo_y..=in_hi_y, tin.clamp(-0.3, 1.3))), 3.5, red);
    let pspan = prange.1 - prange.0;
    let tout = if pspan.abs() > 1e-6 { (ov - prange.0) / pspan } else { 0.5 };
    p.circle_filled(Pos2::new(rx, egui::lerp(bot..=top, tout.clamp(0.0, 1.0))), 3.5, red);
    // Editable fields.
    let mut ch = false;
    let field = |ui: &mut egui::Ui, at: Pos2, v: &mut f32| {
        let fr = egui::Rect::from_min_size(at, Vec2::new(96.0, 26.0));
        ui.put(fr, egui::DragValue::new(v).speed(0.05).max_decimals(2)).changed()
    };
    ch |= field(ui, Pos2::new(lx + 14.0, in_hi_y - 8.0), &mut r.in_range[1]);
    ch |= field(ui, Pos2::new(lx + 14.0, in_lo_y - 18.0), &mut r.in_range[0]);
    ch |= field(ui, Pos2::new(rx - 104.0, out_hi_y - 8.0), &mut r.out_range[1]);
    ch |= field(ui, Pos2::new(rx - 104.0, out_lo_y - 18.0), &mut r.out_range[0]);
    ch
}

fn model_tab(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let Some(key) = app.model_key() else {
        section(ui, tr("Модель не загружена"), |ui| {
            ui.label(tr("Откройте ленту моделей кнопкой 👤 на левой панели и выберите модель."));
        });
        return;
    };
    let entry = app.library.models.iter().find(|m| m.model3.to_string_lossy() == key).cloned();
    let mut budget = 1;
    let icon = entry.as_ref().and_then(|e| e.icon.clone()).and_then(|p| app.ui.thumb(&ctx, &p, &mut budget));
    let motions = {
        let av = app.avatar.as_ref().unwrap();
        if app.ui.motions_cache.as_ref().map(|(d, _)| d != &av.dir).unwrap_or(true) {
            app.ui.motions_cache = Some((av.dir.clone(), list_motions(&av.dir)));
        }
        app.ui.motions_cache.as_ref().map(|(_, m)| m.clone()).unwrap_or_default()
    };
    let _ = motions;
    let mut open: Option<Modal> = None;
    let mut autosetup = false;

    let mut cust = false;
    {
        let av = app.avatar.as_mut().unwrap();
        section(ui, "", |ui| {
            ui.horizontal(|ui| {
                let mut name = if av.cfg.display_name.is_empty() { av.name.clone() } else { av.cfg.display_name.clone() };
                let te = egui::TextEdit::singleline(&mut name).font(FontId::proportional(24.0)).desired_width(ui.available_width() - 96.0);
                if ui.add(te).changed() {
                    av.cfg.display_name = name.clone();
                    av.name = name;
                }
                let (r, _) = ui.allocate_exact_size(Vec2::splat(84.0), Sense::hover());
                match icon {
                    Some(t) => {
                        ui.painter().image(t, r, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().rect_filled(r, 10.0, Color32::from_rgb(150, 150, 158));
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, "No\nicon", FontId::proportional(18.0), Color32::WHITE);
                    }
                }
            });
            ui.label(RichText::new(tr("Модель Live2D:")).size(21.0).strong());
            ui.label(RichText::new(format!("    {}", av.model3.file_name().unwrap_or_default().to_string_lossy())).size(19.0).color(Color32::GRAY));
            ui.label(RichText::new(tr("Конфиг модели (.vtube.json):")).size(21.0).strong());
            let vt = entry.as_ref().and_then(|e| e.vtube.as_ref()).map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_else(|| "нет — используются стандартные настройки".into());
            ui.label(RichText::new(format!("    {vt}")).size(19.0).color(Color32::GRAY));
            ui.add_space(8.0);
            let idle = av.idle.clone().unwrap_or_else(|| "нет".into());
            if io_button(ui, "Анимация ожидания  ⚡", &idle).clicked() {
                open = Some(Modal::Idle);
            }
            let lost = if av.cfg.lost_idle.is_empty() { "нет".to_string() } else { av.cfg.lost_idle.clone() };
            if io_button(ui, "Анимация при потере отслеживания  ⚡", &lost).clicked() {
                open = Some(Modal::LostIdle);
            }
            ui.add_space(6.0);
            ui.add_enabled_ui(false, |ui| {
                let mut on = true;
                toggle(ui, &mut on, tr("Не публиковать в Мастерской"));
            });
            toggle(ui, &mut av.cfg.save_expressions, tr("Сохранять активные выражения")).on_hover_text(tr("Включённые выражения восстановятся при следующей загрузке модели"));
            ui.add_space(10.0);
            if blue_button(ui, tr("АВТОНАСТРОЙКА  🚀"), BLUE, 56.0).on_hover_text(tr("Пересобрать маппинг по стандартным параметрам модели")).clicked() {
                autosetup = true;
            }
        });
        section(ui, "", |ui| {
            ui.add_enabled_ui(false, |ui| {
                blue_button(ui, tr("ИСПРАВИТЬ БЕЛЫЕ КОНТУРЫ"), BLUE, 44.0);
            });
            ui.label(RichText::new(tr("Не требуется: текстуры премультиплицируются при загрузке, белых краёв нет.")).small().weak());
        });
        section(ui, "", |ui| {
            soon_badge(ui);
            ui.add_enabled_ui(false, |ui| {
                blue_button(ui, tr("ОТПРАВИТЬ МОДЕЛЬ НА ТЕЛЕФОН"), BLUE, 44.0);
            });
        });
        section(ui, "", |ui| {
            if blue_button(ui, tr("НАСТРОИТЬ МОДЕЛЬ  📌"), BLUE, 44.0).clicked() {
                cust = true;
            }
        });
        crate::ui_vts::model_volume(&mut app.settings.model_sound, &key, ui);
        section(ui, tr("Настройка движения"), |ui| {
            toggle(ui, &mut av.cfg.move_enabled, tr("Активировать")).on_hover_text(tr("Модель смещается и приближается вслед за положением головы перед камерой"));
            ui.add_enabled_ui(av.cfg.move_enabled, |ui| {
                for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                    pill_knob(ui, &mut av.cfg.move_xyz[i], -20.0..=20.0, "-20", "20", n);
                }
            });
        });
        section(ui, tr("Настройки физики"), |ui| {
            if !av.has_physics() {
                ui.label(RichText::new(tr("У модели нет файла физики (physics3.json)")).weak());
            }
            toggle(ui, &mut av.cfg.physics_on, tr("Использовать физику"));
            toggle(ui, &mut app.settings.physics, tr("Физика для всех моделей и предметов"));
            ui.add_enabled_ui(false, |ui| {
                let mut off = false;
                toggle(ui, &mut off, tr("Старая система физики"));
            });
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Частота кадров физики Live2D")).size(20.0));
            for (v, l) in [(0, "Как в файле модели"), (30, "30 FPS"), (60, "60 FPS"), (120, "120 FPS"), (1, "Соответствует FPS приложения")] {
                ui.radio_value(&mut av.cfg.physics_fps, v, RichText::new(l).size(17.0));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Сила физики")).size(20.0));
            pill_knob(ui, &mut av.cfg.physics_strength, 0.0..=100.0, "Min", "Max", "");
            ui.label(RichText::new(tr("Физика перетаскивания")).size(20.0).color(Color32::GRAY));
            ui.add_enabled_ui(false, |ui| {
                let mut v = 0.0;
                pill_knob(ui, &mut v, 0.0..=100.0, "", "Max", "");
            });
        });
        if !av.expressions.is_empty() {
            section(ui, tr("Выражения (exp3)"), |ui| {
                for e in &mut av.expressions {
                    toggle(ui, &mut e.active, &e.name);
                }
            });
        }
    }

    // Parameter rules, one card each.
    let mut changed = false;
    let mut remove = None;
    {
        let av = app.avatar.as_mut().unwrap();
        let values = av.model.params().to_vec();
        for i in 0..av.mapping.rules.len() {
            let in_val = av.mapping.last_in.get(i).copied().unwrap_or(0.0);
            let pidx = av.model.param(&av.mapping.rules[i].output);
            let prange = pidx.map(|k| (av.model.param_min[k], av.model.param_max[k])).unwrap_or((0.0, 1.0));
            let out_val = pidx.map(|k| values[k]);
            let collapsed = app.ui.collapsed.contains(&i);
            let r = &mut av.mapping.rules[i];
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut r.name).font(FontId::proportional(20.0)).desired_width(ui.available_width() - 110.0));
                    if ui.add(egui::Button::new(RichText::new("🗑").size(22.0).color(Color32::from_rgb(230, 70, 70))).frame(false)).on_hover_text(tr("Удалить")).clicked() {
                        remove = Some(i);
                    }
                    let (cr, cresp) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
                    ui.painter().circle_filled(cr.center(), 14.0, Color32::from_rgb(150, 150, 156));
                    ui.painter().rect_filled(egui::Rect::from_center_size(cr.center(), Vec2::new(16.0, 3.0)), 1.0, Color32::WHITE);
                    if cresp.on_hover_text(if collapsed { "Развернуть" } else { "Свернуть" }).clicked() {
                        if collapsed {
                            app.ui.collapsed.remove(&i);
                        } else {
                            app.ui.collapsed.insert(i);
                        }
                    }
                });
                if collapsed {
                    ui.label(RichText::new(format!("{} → {}", r.input, r.output)).weak());
                    return;
                }
                ui.columns(2, |c| {
                    if io_button(&mut c[0], "Вход", if r.input.is_empty() { "(нет)" } else { &r.input }).clicked() {
                        open = Some(Modal::RuleInput(i));
                    }
                    if io_button(&mut c[0], "Выход", &r.output).clicked() {
                        open = Some(Modal::RuleOutput(i));
                    }
                    changed |= toggle(&mut c[1], &mut r.clamp_input, "Диапазон\nпредела").changed();
                    c[1].add_space(30.0);
                    changed |= toggle(&mut c[1], &mut r.clamp_output, "Диапазон\nпредела").changed();
                });
                ui.add_space(6.0);
                changed |= pill_knob(ui, &mut r.smoothing, 0.0..=100.0, "Min", "Max", "Сглаживание").changed();
                changed |= toggle(ui, &mut r.auto_blink, tr("Автоматическое моргание")).changed();
                changed |= toggle(ui, &mut r.auto_breath, tr("Автоматическое дыхание")).changed();
                changed |= io_diagram(ui, r, in_val, out_val, prange);
            });
            ui.add_space(14.0);
        }
        ui.vertical_centered(|ui| {
            if round_button(ui, "+", Color32::from_rgb(160, 190, 240), 30.0, false, "Добавить параметр").clicked() {
                let first = av.model.param_ids.first().cloned().unwrap_or_default();
                av.mapping.rules.push(Rule {
                    name: "Новый параметр".into(),
                    input: "FaceAngleX".into(),
                    in_range: [-30.0, 30.0],
                    out_range: [-30.0, 30.0],
                    clamp_input: false,
                    clamp_output: false,
                    auto_blink: false,
                    auto_breath: false,
                    output: first,
                    smoothing: 15.0,
                });
                changed = true;
            }
        });
        if let Some(i) = remove {
            av.mapping.rules.remove(i);
            app.ui.collapsed.clear();
            changed = true;
        }
        if changed {
            av.mapping.recompile(&av.model);
            app.settings.rules_override.insert(key.clone(), av.mapping.rules.clone());
        }
    }
    if autosetup {
        app.ui_vts.dlg = Some(crate::ui_vts::Dlg::AutoSetup);
    }
    if cust {
        app.ui_vts.dlg = Some(crate::ui_vts::Dlg::Customize);
    }
    if let Some(m) = open {
        app.ui.modal = Some(m);
        app.ui.modal_sel = None;
    }


}

// ---------------------------------------------------------------------- overlays

fn stats_overlay(app: &App, ctx: &Context) {
    let s = &app.stats;
    let (anchor, off) = if app.ui.visible && app.ui.settings_open { (Align2::LEFT_BOTTOM, [12.0, -12.0]) } else { (Align2::RIGHT_TOP, [-12.0, 12.0]) };
    egui::Area::new(egui::Id::new("stats")).anchor(anchor, off).interactable(false).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::from_black_alpha(170)).rounding(8.0).inner_margin(8.0).show(ui, |ui| {
            let t = |ui: &mut egui::Ui, s: String| {
                ui.label(RichText::new(s).monospace().color(Color32::from_rgb(220, 240, 220)));
            };
            let en = crate::i18n::is_en();
            let (frame, ms, mb, trk, fps, texl) = if en { ("frame CPU", "ms", "MB", "tracker ", "fps", "textures") } else { ("кадр CPU", "мс", "МБ", "трекер  ", "к/с", "текстуры") };
            t(ui, format!("FPS {:6.1}   {frame} {:5.2} {ms}   draw calls {}", s.fps, s.cpu_ms, s.draw_calls));
            t(ui, format!("rigrs    CPU {:5.1}%   RAM {:6.1} {mb} (private {:6.1})", s.app.sample.cpu_pct, s.app.sample.working_set_mb, s.app.sample.private_mb));
            if app.tracker.is_some() {
                t(ui, format!("{trk} CPU {:5.1}%   RAM {:6.1} {mb}   {:4.0} {fps}", s.tracker.sample.cpu_pct, s.tracker.sample.working_set_mb, app.tracker.as_ref().map(|t| t.rate).unwrap_or(0.0)));
            }
            let tex: u64 = app.avatar.as_ref().map(|a| a.texture_bytes).unwrap_or(0) + app.items.iter().map(|i| i.texture_bytes()).sum::<u64>();
            t(ui, format!("GPU {}   {texl} {:.0} {mb}", app.gpu.adapter_name, tex as f64 / 1048576.0));
        });
    });
}

fn toasts(app: &App, ctx: &Context) {
    if app.toasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("toasts")).anchor(Align2::CENTER_TOP, [0.0, 16.0]).interactable(false).show(ctx, |ui| {
        for (t, _) in app.toasts.iter().rev().take(4) {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(t);
            });
        }
    });
}
