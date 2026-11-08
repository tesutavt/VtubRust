//! Items UI in the VTube Studio layout:
//! * item picker ("Выберите элемент из списка.") with the special rows, thumbnails,
//!   Live2D / FPS badges, filter and All / Animated / Live2D switch;
//! * one-time item setup before spawning ("Настройки элемента": order slider 30…-30,
//!   animation FPS, pin to the Live2D model, mirror, Cancel / OK, Configure/Finetune Item);
//! * item settings on a long press ("Item Settings for Item"): between-layer placement,
//!   pinning controls, expressions, item controls, Live2D item settings, colors, sounds,
//!   volume, delete, visualization.

use egui::{Align2, Color32, Context, FontId, Pos2, RichText, Sense, Stroke, Vec2};

use crate::app::{Action, App};
use crate::i18n::tr;
use crate::library::ItemEntry;
use crate::scene::{Body, LAYER_BACK, LAYER_FRONT};
use crate::ui_hotkeys::{dim, group_title, pink_row};

const BLUE_BTN: Color32 = Color32::from_rgb(14, 150, 240);
const RED_BTN: Color32 = Color32::from_rgb(222, 88, 116);
const PINK: Color32 = Color32::from_rgb(236, 30, 160);

/// One-time setup shown before an item is spawned.
#[derive(Clone, Debug)]
pub struct ItemSetup {
    pub path: std::path::PathBuf,
    pub order: i32,
    pub pin: bool,
    pub flip: bool,
    pub fps: f32,
    /// Open the full item settings right after spawning ("Configure/Finetune Item").
    pub finetune: bool,
}

impl ItemSetup {
    pub fn new(path: std::path::PathBuf, st: &crate::settings::Settings) -> ItemSetup {
        let fps = st.item_fps.get(&path.to_string_lossy().to_string()).copied().unwrap_or(0.0);
        ItemSetup { path, order: if st.item_order == 0 { 1 } else { st.item_order }, pin: st.item_pin, flip: false, fps, finetune: false }
    }
}

/// Sub-window of the item settings.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ItemSub {
    Main,
    Global,
    Live2D,
    Colors,
    Sounds,
    Expressions,
    Layer(u8),
}

fn is_animated(p: &std::path::Path) -> bool {
    p.is_dir() || p.extension().map(|e| e.eq_ignore_ascii_case("gif")).unwrap_or(false)
}

fn item_name(p: &std::path::Path) -> String {
    if p.to_string_lossy().ends_with(".model3.json") {
        p.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

fn badge(painter: &egui::Painter, rect: egui::Rect, text: &str) {
    let g = painter.layout_no_wrap(text.to_string(), FontId::proportional(13.0), Color32::WHITE);
    let r = egui::Rect::from_min_size(Pos2::new(rect.right() - g.size().x - 22.0, rect.top() + 3.0), Vec2::new(g.size().x + 16.0, 17.0));
    painter.rect_filled(r, 8.0, Color32::from_rgb(110, 112, 140));
    painter.galley(Pos2::new(r.left() + 8.0, r.top() + 1.0), g, Color32::WHITE);
}

fn row_button(ui: &mut egui::Ui, w: f32, fill: Color32, text: RichText, icon: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 58.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect.translate(Vec2::new(1.5, 2.5)), 8.0, Color32::from_black_alpha(60));
    p.rect_filled(rect, 8.0, if resp.hovered() { crate::ui::lighten(fill, 0.08) } else { fill });
    let g = p.layout_no_wrap(text.text().to_string(), FontId::proportional(20.0), Color32::from_rgb(60, 60, 66));
    p.galley(Pos2::new(rect.left() + 14.0, rect.center().y - g.size().y / 2.0), g, Color32::from_rgb(60, 60, 66));
    p.text(rect.right_center() - Vec2::new(30.0, 0.0), Align2::CENTER_CENTER, icon, FontId::proportional(26.0), PINK);
    ui.add_space(8.0);
    resp
}

/// One picker row; computed once per opening (no file-system calls while drawing).
#[derive(Clone)]
pub struct PickRow {
    name: String,
    name_lc: String,
    path: std::path::PathBuf,
    thumb: Option<std::path::PathBuf>,
    l2d: bool,
    anim: bool,
}

fn pick_rows(items: &[ItemEntry]) -> Vec<PickRow> {
    items
        .iter()
        .map(|it| {
            let (l2d, anim, thumb) = match it {
                ItemEntry::Live2D(m) => (true, false, m.icon.clone()),
                ItemEntry::Image { path, .. } if path.is_dir() => {
                    let first = std::fs::read_dir(path).ok().and_then(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("png")).unwrap_or(false)).min());
                    (false, true, first)
                }
                ItemEntry::Image { path, .. } => (false, is_animated(path), Some(path.clone())),
            };
            let name = it.name().to_string();
            PickRow { name_lc: name.to_lowercase(), name, path: it.path().to_path_buf(), thumb, l2d, anim }
        })
        .collect()
}

/// Item picker modal (opened by the ★ button).
pub fn item_picker(app: &mut App, ctx: &Context) {
    if !app.ui.item_picker {
        app.ui.picker_rows = None;
        return;
    }
    if app.ui.picker_rows.is_none() {
        app.ui.picker_rows = Some(pick_rows(&app.library.items));
    }
    dim(ctx, "picker-dim");
    let screen = ctx.screen_rect();
    let w = 820.0f32.min(screen.width() - 40.0);
    let mut close = false;
    let mut choose: Option<std::path::PathBuf> = None;
    let mut budget = 3u32;
    // Header, Cancel, filter and radio take ~360 px; the list gets the rest.
    let list_h = (screen.height() - 400.0).clamp(150.0, 900.0);
    // Fixed position (an anchored Area uses last frame's size and drifts while the size settles).
    let total_h = 46.0 + 30.0 + 20.0 + list_h + 10.0 + 44.0 + 18.0 + 14.0 + 52.0 + 8.0 + 110.0;
    let top = ((screen.height() - total_h) / 2.0).max(40.0);
    egui::Area::new(egui::Id::new("item-picker")).order(egui::Order::Tooltip).constrain(false).fixed_pos(Pos2::new(screen.center().x - w / 2.0, top)).show(ctx, |ui| {
        let resp = egui::Frame::none().fill(Color32::WHITE).rounding(6.0).inner_margin(egui::Margin { left: 26.0, right: 26.0, top: 46.0, bottom: 18.0 }).show(ui, |ui| {
            ui.set_width(w - 52.0);
            ui.label(RichText::new(tr("Выберите элемент из списка.")).size(21.0).color(Color32::from_rgb(110, 110, 118)));
            ui.add_space(20.0);
            let f = app.ui.filter.to_lowercase();
            let kind = app.ui.item_kind;
            let all = app.ui.picker_rows.take().unwrap_or_default();
            let (al, aa, ai) = (app.settings.item_allow_live2d, app.settings.item_allow_animated, app.settings.item_allow_images);
            let shown: Vec<&PickRow> = all
                .iter()
                .filter(|r| f.is_empty() || r.name_lc.contains(&f))
                .filter(|r| !((kind == 1 && !r.anim) || (kind == 2 && !r.l2d)))
                .filter(|r| !(!al && r.l2d || !aa && r.anim || !ai && !r.l2d && !r.anim))
                .collect();
            const SPECIALS: usize = 4;
            egui::ScrollArea::vertical().id_salt("picker-list").max_height(list_h).min_scrolled_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let bw = ui.available_width();
                let beige = Color32::from_rgb(240, 226, 214);
                for idx in 0..SPECIALS + shown.len() {
                    if idx < SPECIALS {
                        let (text, icon) = match idx {
                            0 => (format!("{} ({})", tr("Удалить все элементы со сцены"), app.items.len()), "🗑"),
                            1 => (tr("Создать элемент Live2D из модели").to_string(), "👤"),
                            2 => (tr("Общие настройки элементов").to_string(), "⚙"),
                            _ => (tr("Добавить файл…").to_string(), "+"),
                        };
                        if row_button(ui, bw, beige, RichText::new(text), icon).clicked() {
                            match idx {
                                0 => app.actions.push(Action::ClearItems),
                                1 => {
                                    app.actions.push(Action::ModelToItem);
                                    close = true;
                                }
                                2 => app.ui.item_global = true,
                                _ => {
                                    if let Some(p) = rfd::FileDialog::new().add_filter("PNG / JPG / GIF / Live2D", &["png", "jpg", "jpeg", "gif", "json"]).pick_file() {
                                        choose = Some(p);
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    let it = shown[idx - SPECIALS];
                    let (l2d, anim) = (it.l2d, it.anim);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(bw, 58.0), Sense::click());
                    ui.add_space(8.0);
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let tex = it.thumb.as_ref().and_then(|p| app.ui.thumb_pub(ctx, p, &mut budget));
                    let p = ui.painter();
                    p.rect_filled(rect.translate(Vec2::new(1.5, 2.5)), 8.0, Color32::from_black_alpha(60));
                    p.rect_filled(rect, 8.0, if resp.hovered() { Color32::from_rgb(222, 222, 234) } else { Color32::from_rgb(232, 230, 240) });
                    let ir = egui::Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 5.0), Vec2::splat(48.0));
                    p.rect_filled(ir, 4.0, Color32::WHITE);
                    if let Some(t) = tex {
                        p.image(t, ir.shrink(2.0), egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    p.text(Pos2::new(ir.right() + 10.0, rect.center().y), Align2::LEFT_CENTER, &it.name, FontId::proportional(20.0), Color32::from_rgb(60, 60, 66));
                    if l2d {
                        badge(p, rect, "Live2D");
                    } else if anim {
                        let fps = app.settings.item_fps.get(&it.path.to_string_lossy().to_string()).copied().unwrap_or(0.0);
                        badge(p, rect, &if fps > 0.0 { format!("{fps:.0} FPS") } else { "GIF".into() });
                    }
                    if resp.clicked() {
                        choose = Some(it.path.clone());
                    }
                }
            });
            drop(shown);
            app.ui.picker_rows = Some(all);
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 44.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([170.0, 40.0], egui::Button::new(RichText::new(tr("Отмена")).size(17.0).strong().color(Color32::WHITE)).fill(RED_BTN).rounding(3.0)).clicked() {
                    close = true;
                }
            });
        });
        let c = Pos2::new(resp.response.rect.center().x, resp.response.rect.top());
        let p = ui.painter();
        p.circle_filled(c, 36.0, Color32::WHITE);
        p.circle_filled(c, 31.0, Color32::from_rgb(166, 214, 240));
        p.text(c, Align2::CENTER_CENTER, "!", FontId::proportional(46.0), Color32::WHITE);
        ui.add_space(14.0);
        egui::Frame::none().fill(Color32::WHITE).rounding(10.0).inner_margin(8.0).show(ui, |ui| {
            ui.set_width(w - 16.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut app.ui.filter).hint_text(tr("Фильтр")).font(FontId::proportional(20.0)).desired_width(w - 110.0));
                if ui.add(egui::Button::new(RichText::new("✖").size(20.0).color(Color32::from_rgb(240, 120, 130))).frame(false)).clicked() {
                    app.ui.filter.clear();
                }
                ui.label(RichText::new("🔍").size(24.0));
            });
        });
        ui.add_space(8.0);
        egui::Frame::none().fill(Color32::WHITE).rounding(10.0).inner_margin(10.0).show(ui, |ui| {
            for (k, label) in ["Все", "Анимированные", "Live2D"].iter().enumerate() {
                if ui.radio(app.ui.item_kind == k as u8, RichText::new(tr(label)).size(19.0)).clicked() {
                    app.ui.item_kind = k as u8;
                }
            }
        });
    });
    if let Some(p) = choose {
        app.ui.item_setup = Some(ItemSetup::new(p, &app.settings));
        close = true;
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.item_picker = false;
    }
}

/// Free VTS order next to `want` (orders are unique; the model is at 0).
pub fn free_order(app: &App, want: i32, except: Option<u64>) -> i32 {
    let taken = |o: i32| o == 0 || app.items.iter().any(|i| Some(i.id) != except && i.cfg.order == o);
    if !taken(want) {
        return want;
    }
    let dir = if want >= 0 { 1 } else { -1 };
    let mut o = want;
    while taken(o) && o.abs() < 1000 {
        o += dir;
    }
    o
}

/// Order slider 30 … -30 (left = in front), VTS style.
fn order_slider(ui: &mut egui::Ui, v: &mut i32) -> egui::Response {
    let w = ui.available_width();
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::click_and_drag());
    let knob_w = 100.0;
    if let Some(p) = resp.interact_pointer_pos() {
        let t = ((p.x - rect.left() - knob_w / 2.0) / (rect.width() - knob_w)).clamp(0.0, 1.0);
        let mut nv = (30.0 - t * 60.0).round() as i32;
        if nv == 0 {
            nv = if *v > 0 { -1 } else { 1 };
        }
        if nv != *v {
            *v = nv;
            resp.mark_changed();
        }
    }
    let t = (30.0 - *v as f32) / 60.0;
    let p = ui.painter();
    p.rect_filled(rect, 17.0, Color32::from_rgb(236, 242, 250));
    p.text(rect.left_center() + Vec2::new(14.0, 0.0), Align2::LEFT_CENTER, "30", FontId::proportional(14.0), Color32::GRAY);
    p.text(rect.right_center() - Vec2::new(14.0, 0.0), Align2::RIGHT_CENTER, "-30", FontId::proportional(14.0), Color32::GRAY);
    let knob = egui::Rect::from_min_size(Pos2::new(rect.left() + t * (rect.width() - knob_w), rect.top()), Vec2::new(knob_w, rect.height()));
    p.rect_filled(knob, 17.0, crate::ui::BLUE);
    p.text(knob.center(), Align2::CENTER_CENTER, v.to_string(), FontId::proportional(20.0), Color32::WHITE);
    resp
}

/// Camera → model → background scheme with the item position.
fn order_scheme(ui: &mut egui::Ui, order: i32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), Sense::hover());
    let p = ui.painter();
    let cam = rect.left_bottom() + Vec2::new(rect.width() * 0.25, -20.0);
    let bg = rect.right_top() + Vec2::new(-rect.width() * 0.2, 30.0);
    p.line_segment([bg, cam], Stroke::new(3.0f32, Color32::from_rgb(80, 60, 60)));
    p.text(cam - Vec2::new(10.0, -2.0), Align2::CENTER_CENTER, "📷", FontId::proportional(30.0), Color32::from_rgb(160, 100, 60));
    p.text(bg, Align2::CENTER_CENTER, "🏞", FontId::proportional(48.0), Color32::from_rgb(90, 160, 90));
    let mid = cam + (bg - cam) * 0.55;
    p.text(mid, Align2::CENTER_CENTER, "🐱", FontId::proportional(56.0), Color32::from_rgb(90, 90, 90));
    // Front (positive) → toward the camera, back (negative) → toward the background.
    let k = 0.55 - (order.clamp(-30, 30) as f32 / 30.0) * 0.4;
    let dot = cam + (bg - cam) * k;
    p.circle_filled(dot, 7.0, crate::ui::BLUE);
}

/// "Настройки элемента" before spawning.
pub fn item_setup_dialog(app: &mut App, ctx: &Context) {
    let Some(mut s) = app.ui.item_setup.clone() else { return };
    let name = item_name(&s.path);
    let animated = is_animated(&s.path);
    let free = free_order(app, s.order, None);
    let mut done: Option<bool> = None;
    dim(ctx, "setup-dim");
    egui::Area::new(egui::Id::new("item-setup")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, -20.0]).show(ctx, |ui| {
        let resp = egui::Frame::none().fill(Color32::WHITE).rounding(6.0).inner_margin(egui::Margin { left: 26.0, right: 26.0, top: 46.0, bottom: 18.0 }).show(ui, |ui| {
            ui.set_width(600.0);
            ui.style_mut().visuals.override_text_color = Some(crate::ui::TEXT);
            ui.label(RichText::new(tr("Настройки элемента")).size(26.0));
            ui.label(RichText::new(format!("   » {name}")).size(19.0).color(Color32::GRAY));
            order_scheme(ui, s.order);
            ui.label(RichText::new(tr("Порядок элементов")).size(22.0));
            order_slider(ui, &mut s.order);
            ui.vertical_centered(|ui| {
                if free == s.order {
                    ui.label(RichText::new("OK").size(16.0));
                } else {
                    ui.label(RichText::new(format!("{} {free}", tr("занято, будет"))).size(16.0).color(PINK));
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(animated, |ui| {
                        ui.label(RichText::new(tr("Анимация")).size(18.0));
                        ui.add(egui::DragValue::new(&mut s.fps).speed(0.1).range(0.0..=60.0).fixed_decimals(2));
                        ui.label(RichText::new("FPS").size(18.0));
                    });
                });
                ui.horizontal(|ui| crate::ui::toggle(ui, &mut s.pin, tr("Прикреплять к модели Live2D")));
                ui.horizontal(|ui| crate::ui::toggle(ui, &mut s.flip, tr("Отзеркалить")));
            });
            ui.add_space(16.0);
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 44.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).strong().color(Color32::WHITE)).fill(BLUE_BTN).rounding(3.0)).clicked() {
                    done = Some(true);
                }
                ui.add_space(14.0);
                if ui.add_sized([120.0, 40.0], egui::Button::new(RichText::new(tr("Отмена")).size(17.0).strong().color(Color32::WHITE)).fill(RED_BTN).rounding(3.0)).clicked() {
                    done = Some(false);
                }
            });
        });
        let c = Pos2::new(resp.response.rect.center().x, resp.response.rect.top());
        let p = ui.painter();
        p.circle_filled(c, 36.0, Color32::WHITE);
        p.circle_filled(c, 31.0, Color32::from_rgb(166, 214, 240));
        p.text(c, Align2::CENTER_CENTER, "!", FontId::proportional(46.0), Color32::WHITE);
        ui.add_space(12.0);
        egui::Frame::none().fill(Color32::WHITE).inner_margin(10.0).show(ui, |ui| {
            if ui.add_sized([630.0, 36.0], egui::Button::new(RichText::new(format!("⚙  {}", tr("Настроить элемент подробнее"))).size(17.0).color(Color32::WHITE)).fill(BLUE_BTN)).clicked() {
                s.finetune = true;
                done = Some(true);
            }
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        done = Some(false);
    }
    match done {
        Some(true) => {
            s.order = free;
            app.settings.item_order = s.order;
            app.settings.item_pin = s.pin;
            app.ui.item_setup = None;
            app.actions.push(Action::SpawnItem(s));
        }
        Some(false) => app.ui.item_setup = None,
        None => app.ui.item_setup = Some(s),
    }
}

fn layer_label(v: &str) -> String {
    match v {
        "" => tr("не выбран").to_string(),
        LAYER_FRONT => tr("Всегда перед моделью").to_string(),
        LAYER_BACK => tr("Всегда за моделью").to_string(),
        id => id.to_string(),
    }
}

fn blue_card(ui: &mut egui::Ui, title: &str, sub: &str, button: &str) -> bool {
    let w = ui.available_width();
    let mut clicked = false;
    egui::Frame::none().fill(Color32::from_rgb(214, 234, 252)).stroke(Stroke::new(1.5f32, Color32::from_rgb(120, 180, 240))).rounding(8.0).inner_margin(egui::Margin::symmetric(10.0, 6.0)).show(ui, |ui| {
        ui.set_width(w - 22.0);
        ui.label(RichText::new(tr(title)).size(19.0).color(Color32::from_rgb(70, 70, 80)));
        if !sub.is_empty() {
            ui.label(RichText::new(sub).size(13.0).color(PINK));
        }
        let bw = ui.available_width() - 40.0;
        clicked = ui.add_sized([bw, 28.0], egui::Button::new(RichText::new(tr(button)).color(Color32::WHITE)).fill(BLUE_BTN)).clicked();
    });
    ui.add_space(4.0);
    clicked
}

/// "Item Settings for Item" (long press on an item).
pub fn item_config(app: &mut App, ctx: &Context) {
    let Some(id) = app.ui.selected else { return };
    let Some(idx) = app.items.iter().position(|i| i.id == id) else {
        app.ui.selected = None;
        app.ui.item_menu = None;
        return;
    };
    if app.ui.item_sub != ItemSub::Main {
        item_sub(app, ctx, idx);
        return;
    }
    let meshes: Vec<String> = app.avatar.as_ref().map(|a| (0..a.order_len()).filter_map(|p| a.order_at(p)).map(|d| a.model.drawables[d].id.clone()).collect()).unwrap_or_default();
    let mut ok = false;
    let mut remove = false;
    let mut sub = None;
    dim(ctx, "itemcfg-dim");
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new("item-cfg")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).rounding(4.0).inner_margin(14.0).show(ui, |ui| {
            ui.set_width(600.0);
            ui.style_mut().visuals.override_text_color = Some(crate::ui::TEXT);
            let it = &mut app.items[idx];
            let (kind, layers) = match &it.body {
                Body::Live2D(l) => ("Live2D", l.model.drawables.len()),
                Body::Sprite(s) if s.frame_count() > 1 => ("Animated", 1),
                _ => ("Image", 1),
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Настройки предмета:")).size(17.0));
                ui.label(RichText::new(format!("\"{}\"", it.name)).size(17.0).color(PINK));
            });
            ui.label(RichText::new(format!("({kind}, {}: {layers}, {}: {})", tr("слоёв"), tr("порядок"), it.cfg.order)).size(14.0).weak());
            egui::ScrollArea::vertical().max_height(screen.height() - 220.0).show(ui, |ui| {
                group_title(ui, "Поместить предмет между слоями");
                let is_l2d = matches!(it.body, Body::Live2D(_));
                let mut front_on = !it.cfg.layer.is_empty();
                if pink_row(ui, &mut front_on, if is_l2d { "Передняя часть здесь" } else { "Предмет здесь" }, &format!("{}: {}", tr("Слой"), layer_label(if it.cfg.layer.is_empty() { LAYER_FRONT } else { &it.cfg.layer }))) {
                    it.cfg.layer = if front_on { LAYER_FRONT.to_string() } else { String::new() };
                }
                if front_on && ui.button(tr("Выбрать слой…")).clicked() {
                    sub = Some(ItemSub::Layer(0));
                }
                if is_l2d {
                    let mut back_on = !it.cfg.split_back.is_empty();
                    if pink_row(ui, &mut back_on, "Задняя часть здесь", &format!("{}: {}", tr("Слой"), layer_label(if it.cfg.split_back.is_empty() { LAYER_BACK } else { &it.cfg.split_back }))) {
                        it.cfg.split_back = if back_on { LAYER_BACK.to_string() } else { String::new() };
                    }
                    if back_on && ui.button(tr("Выбрать слой для задней части…")).clicked() {
                        sub = Some(ItemSub::Layer(1));
                    }
                    if blue_card(ui, "Разделить Live2D-предмет по слою", &format!("{}: {}", tr("Слой"), if it.cfg.split.is_empty() { tr("не выбран").to_string() } else { it.cfg.split.clone() }), "SPLIT") {
                        sub = Some(ItemSub::Layer(2));
                    }
                }
                it.cfg.split_front = it.cfg.layer.clone();
                group_title(ui, "Закрепление");
                pink_row(ui, &mut it.cfg.pin_only_below, "Закреплять только к слоям ниже предмета", "");
                let mut only = !it.cfg.pin_only_layer.is_empty() && it.cfg.pin_only_layer != "__never";
                let only_sub = format!("{}: {}", tr("Слой"), if only { it.cfg.pin_only_layer.clone() } else { tr("не выбран").to_string() });
                if pink_row(ui, &mut only, "Закреплять только к определённому слою", &only_sub) {
                    if only {
                        sub = Some(ItemSub::Layer(3));
                    } else {
                        it.cfg.pin_only_layer.clear();
                    }
                }
                let mut pinning = it.cfg.pin_only_layer != "__never";
                if pink_row(ui, &mut pinning, "Прикреплять к модели Live2D", "") {
                    it.cfg.pin_only_layer = if pinning { String::new() } else { "__never".into() };
                    if !pinning {
                        it.pin = None;
                    }
                }
                if is_l2d {
                    group_title(ui, "Выражения");
                    if blue_card(ui, "Выражения предмета", "", "Настроить") {
                        sub = Some(ItemSub::Expressions);
                    }
                    pink_row(ui, &mut it.cfg.remember_expressions, "Запоминать выражения", "Сохранять активные выражения");
                }
                group_title(ui, "Управление предметом");
                pink_row(ui, &mut it.xf.flip, "Отзеркален", "");
                pink_row(ui, &mut it.locked, "Заблокирован", "");
                pink_row(ui, &mut it.cfg.censor, "Цензура", "");
                ui.horizontal(|ui| {
                    ui.label(tr("Порядок"));
                    let mut o = it.cfg.order;
                    if ui.add(egui::DragValue::new(&mut o).range(-100..=100)).changed() && o != 0 {
                        it.cfg.order = o;
                    }
                    ui.label(tr("Сглаживание"));
                    ui.add(egui::Slider::new(&mut it.cfg.smoothing, 0.0..=1.0));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Размер"));
                    let mut s = it.xf.scale;
                    if ui.add(egui::DragValue::new(&mut s).speed(0.01).range(0.001..=f32::MAX)).changed() {
                        if let Some(p) = &mut it.pin {
                            p.scale_ratio *= s / it.xf.scale;
                        }
                        it.xf.scale = s;
                    }
                    ui.label(tr("Поворот"));
                    let mut r = it.xf.rot;
                    if ui.add(egui::DragValue::new(&mut r).speed(1.0).suffix("°")).changed() {
                        if let Some(p) = &mut it.pin {
                            p.rot0 += r - it.xf.rot;
                        }
                        it.xf.rot = r;
                    }
                });
                if let Body::Sprite(s) = &it.body {
                    if s.frame_count() > 1 {
                        ui.horizontal(|ui| {
                            ui.label(tr("FPS анимации"));
                            ui.add(egui::Slider::new(&mut it.cfg.fps, 0.0..=60.0));
                        });
                    }
                }
                if blue_card(ui, "Общие настройки элементов", "", "Открыть") {
                    sub = Some(ItemSub::Global);
                }
                if is_l2d {
                    if blue_card(ui, "Настройки Live2D-предмета", "", "Открыть") {
                        sub = Some(ItemSub::Live2D);
                    }
                    if blue_card(ui, "Цвета частей (ArtMesh)", "", "Настроить цвета") {
                        sub = Some(ItemSub::Colors);
                    }
                    if blue_card(ui, "Звуковые триггеры параметров", "", "Настроить") {
                        sub = Some(ItemSub::Sounds);
                    }
                }
                if blue_card(ui, "Удалить этот предмет", "", "Удалить") {
                    remove = true;
                }
                group_title(ui, "Визуализация");
                pink_row(ui, &mut it.cfg.colorize, "Подсвечивать передние/задние слои", "Слои модели перед предметом и за ним");
                pink_row(ui, &mut it.cfg.pin_notify, "Показывать уведомления о закреплении", "");
            });
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 44.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).strong().color(Color32::WHITE)).fill(BLUE_BTN).rounding(3.0)).clicked() {
                    ok = true;
                }
            });
        });
    });
    let _ = meshes;
    if let Some(s) = sub {
        app.ui.item_sub = s;
    }
    if remove {
        app.actions.push(Action::RemoveItem(id));
        ok = true;
    }
    if ok || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.item_menu = None;
        app.save();
    }
}

fn sub_frame(app: &mut App, ctx: &Context, title: &str, add: impl FnOnce(&mut App, &mut egui::Ui)) {
    dim(ctx, "itemsub-dim");
    let mut back = false;
    let h = ctx.screen_rect().height() - 220.0;
    egui::Area::new(egui::Id::new("item-sub")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).rounding(4.0).inner_margin(14.0).show(ui, |ui| {
            ui.set_width(600.0);
            ui.style_mut().visuals.override_text_color = Some(crate::ui::TEXT);
            ui.label(RichText::new(tr(title)).size(22.0));
            egui::ScrollArea::vertical().max_height(h).show(ui, |ui| add(app, ui));
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 44.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).color(Color32::WHITE)).fill(BLUE_BTN)).clicked() {
                    back = true;
                }
            });
        });
    });
    if back || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.item_sub = ItemSub::Main;
        app.save();
    }
}

fn item_sub(app: &mut App, ctx: &Context, idx: usize) {
    let sub = app.ui.item_sub;
    match sub {
        ItemSub::Main => {}
        ItemSub::Global => global_settings(app, ctx),
        ItemSub::Layer(which) => {
            let meshes: Vec<String> = match which {
                2 => match &app.items[idx].body {
                    Body::Live2D(l) => (0..l.order_len()).filter_map(|p| l.order_at(p)).map(|d| l.model.drawables[d].id.clone()).collect(),
                    _ => Vec::new(),
                },
                _ => app.avatar.as_ref().map(|a| (0..a.order_len()).filter_map(|p| a.order_at(p)).map(|d| a.model.drawables[d].id.clone()).collect()).unwrap_or_default(),
            };
            let title = ["Слой для передней части", "Слой для задней части", "Слой разделения предмета", "Единственный слой для закрепления"][which as usize];
            sub_frame(app, ctx, title, |app, ui| {
                ui.add(egui::TextEdit::singleline(&mut app.ui.filter).hint_text(tr("Фильтр")).desired_width(f32::INFINITY));
                let f = app.ui.filter.to_lowercase();
                let it = &mut app.items[idx];
                let target = match which {
                    0 => &mut it.cfg.layer,
                    1 => &mut it.cfg.split_back,
                    2 => &mut it.cfg.split,
                    _ => &mut it.cfg.pin_only_layer,
                };
                if which < 2 {
                    for special in [LAYER_FRONT, LAYER_BACK] {
                        if ui.selectable_label(*target == special, layer_label(special)).clicked() {
                            *target = special.to_string();
                        }
                    }
                }
                for m in meshes.iter().rev().filter(|m| f.is_empty() || m.to_lowercase().contains(&f)) {
                    if ui.selectable_label(target == m, m).clicked() {
                        *target = m.clone();
                    }
                }
            });
        }
        ItemSub::Live2D => sub_frame(app, ctx, "Настройки Live2D-предмета", |app, ui| {
            let it = &mut app.items[idx];
            ui.label(RichText::new(tr("Множители углов головы, которые получает предмет (ParamAngleX/Y/Z).")).size(13.0).weak());
            for (k, n) in ["X", "Y", "Z"].iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{} {n}", tr("Множитель")));
                    ui.add(egui::Slider::new(&mut it.cfg.angle_mult[k], -3.0..=3.0));
                });
            }
            pink_row(ui, &mut it.cfg.default_unpinned, "Возвращать к умолчанию, когда не закреплён", "Предмет не следует за трекингом, пока не закреплён");
            pink_row(ui, &mut it.cfg.normal_hotkeys, "Использовать обычные горячие клавиши предмета", "Клавиши из модели предмета (выражения и анимации)");
        }),
        ItemSub::Expressions => sub_frame(app, ctx, "Выражения предмета", |app, ui| {
            if let Body::Live2D(l) = &mut app.items[idx].body {
                if l.expressions.is_empty() {
                    ui.label(tr("У предмета нет выражений."));
                }
                for e in l.expressions.iter_mut() {
                    let mut on = e.active;
                    if pink_row(ui, &mut on, &e.name.clone(), &e.file.clone()) {
                        e.active = on;
                    }
                }
            }
        }),
        ItemSub::Colors => {
            let (key, ids) = match &app.items[idx].body {
                Body::Live2D(l) => (l.model3.to_string_lossy().into_owned(), l.model.drawables.iter().map(|d| d.id.clone()).collect::<Vec<_>>()),
                _ => (String::new(), Vec::new()),
            };
            sub_frame(app, ctx, "Цвета частей предмета", |app, ui| {
                let mut d = app.settings.model_artmesh.get(&key).cloned().unwrap_or_default();
                let before = d.clone();
                ui.add(egui::TextEdit::singleline(&mut app.ui.filter).hint_text(tr("Фильтр")).desired_width(f32::INFINITY));
                let f = app.ui.filter.to_lowercase();
                egui::Grid::new("item-colors").striped(true).num_columns(3).show(ui, |ui| {
                    for id in ids.iter().filter(|i| f.is_empty() || i.to_lowercase().contains(&f)).take(400) {
                        ui.label(id);
                        let (m, s) = d.colors(id).unwrap_or(([1.0; 4], [0.0, 0.0, 0.0, 1.0]));
                        let mut mc = egui::Rgba::from_rgba_unmultiplied(m[0], m[1], m[2], m[3]);
                        let mut sc = egui::Rgba::from_rgba_unmultiplied(s[0], s[1], s[2], 1.0);
                        let a = egui::widgets::color_picker::color_edit_button_rgba(ui, &mut mc, egui::widgets::color_picker::Alpha::OnlyBlend).changed();
                        let b = egui::widgets::color_picker::color_edit_button_rgba(ui, &mut sc, egui::widgets::color_picker::Alpha::Opaque).changed();
                        if a || b {
                            let v = sc.to_rgba_unmultiplied();
                            d.set_colors(id, mc.to_rgba_unmultiplied(), [v[0], v[1], v[2], 1.0]);
                        }
                        ui.end_row();
                    }
                });
                if d != before {
                    app.settings.model_artmesh.insert(key.clone(), d.clone());
                    if let Body::Live2D(l) = &mut app.items[idx].body {
                        l.apply_artmesh(&d);
                    }
                }
            });
        }
        ItemSub::Sounds => {
            let (key, params, dir) = match &app.items[idx].body {
                Body::Live2D(l) => (l.model3.to_string_lossy().into_owned(), l.model.param_ids.clone(), l.dir.clone()),
                _ => (String::new(), Vec::new(), std::path::PathBuf::new()),
            };
            sub_frame(app, ctx, "Звук предмета", |app, ui| {
                crate::ui_hotkeys::model_sound_for(app, ui, "Громкость и звуковые триггеры", key, params, dir);
            });
        }
    }
}

/// Global item settings (drag & drop, allowed item types, notifications).
pub fn global_settings(app: &mut App, ctx: &Context) {
    sub_frame(app, ctx, "Общие настройки элементов", |app, ui| {
        let st = &mut app.settings;
        pink_row(ui, &mut st.item_drag_drop, "Перетаскивание файлов в окно", "PNG, JPG, GIF, папки с кадрами и модели Live2D");
        pink_row(ui, &mut st.item_allow_images, "Показывать картинки (PNG/JPG)", "");
        pink_row(ui, &mut st.item_allow_animated, "Показывать анимированные (GIF, папки с кадрами)", "");
        pink_row(ui, &mut st.item_allow_live2d, "Показывать Live2D-предметы", "");
        pink_row(ui, &mut st.item_show_pin_notifications, "Уведомления о закреплении для всех предметов", "");
    });
}

/// Global item settings opened from the picker (not tied to an item).
pub fn global_settings_window(app: &mut App, ctx: &Context) {
    if !app.ui.item_global {
        return;
    }
    let before = app.ui.item_sub;
    app.ui.item_sub = ItemSub::Global;
    global_settings(app, ctx);
    if app.ui.item_sub == ItemSub::Main {
        app.ui.item_global = false;
    }
    app.ui.item_sub = if app.ui.item_global { before } else { ItemSub::Main };
}
