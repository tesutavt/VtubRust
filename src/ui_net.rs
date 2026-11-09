//! UI pieces for the extra trackers (NVIDIA / phone) and the VNet-style collab window.

use egui::{Color32, RichText};

use crate::app::App;
use crate::i18n::tr;
use crate::tracking::backend;

/// Backend-specific settings shown under "Tracking quality" in the camera tab.
pub fn tracker_extra(app: &mut App, ui: &mut egui::Ui, running: bool) {
    match app.settings.track_backend {
        backend::PHONE => {
            ui.add_space(4.0);
            ui.label(RichText::new(tr("IP телефона (видно в приложении VTube Studio, включите «3rd Party PC Clients»)")).size(15.0));
            ui.add_enabled_ui(!running, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut app.settings.phone_ip).hint_text("192.168.1.23").desired_width(170.0));
                    ui.label(tr("порт"));
                    ui.add(egui::DragValue::new(&mut app.settings.phone_port).range(1..=65535));
                    ui.label(tr("локальный"));
                    ui.add(egui::DragValue::new(&mut app.settings.phone_local_port).range(1024..=65535));
                });
            });
            ui.label(
                RichText::new(tr("Телефон может одновременно отправлять данные и в VBridger. Кнопки 1-8 на экране телефона запускают горячие клавиши с ScreenButton 1-8."))
                    .size(13.0)
                    .color(Color32::GRAY),
            );
        }
        backend::NVIDIA => {
            ui.add_space(4.0);
            let found = std::path::Path::new(&app.settings.nv_exe).is_file();
            let (txt, col) = if found {
                (tr("ExpressionApp.exe найден"), Color32::from_rgb(60, 160, 90))
            } else {
                (tr("ExpressionApp.exe не найден: установите бесплатное DLC VTube Studio «NVIDIA Broadcast Tracker» в Steam или укажите путь в настройках"), Color32::from_rgb(220, 80, 80))
            };
            ui.label(RichText::new(txt).size(14.0).color(col));
            ui.label(
                RichText::new(tr("Только Windows и видеокарты RTX 20xx и новее. Нажмите «Калибровать» с нейтральным лицом — коэффициенты сохранятся для следующих запусков."))
                    .size(13.0)
                    .color(Color32::GRAY),
            );
            if !app.settings.nv_cal.is_empty() && ui.add_enabled(!running, egui::Button::new(RichText::new(tr("Сбросить калибровку NVIDIA")).color(Color32::WHITE)).fill(crate::vts_style::BLUE).rounding(3.0)).clicked() {
                app.settings.nv_cal.clear();
                app.save();
            }
        }
        _ => {}
    }
}

// ====================================================================== VNet-style collab UI
//
// Layout follows VTube Studio's VNet screens (wiki "Multiplayer"): a big purple "antenna"
// button opens Collab Setup, a small "people" button next to it opens Session Control.
// Icons are drawn with shapes so they don't depend on emoji fonts.

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::net::{Role, MAX_PARTICIPANTS};

const PURPLE: Color32 = Color32::from_rgb(138, 122, 186);
const PURPLE_ROW: Color32 = Color32::from_rgb(128, 112, 178);
const LAVENDER: Color32 = Color32::from_rgb(242, 238, 252);
const SKY: Color32 = Color32::from_rgb(120, 190, 240);
const VBLUE: Color32 = Color32::from_rgb(10, 132, 240);
const PINK: Color32 = Color32::from_rgb(222, 92, 120);
const PINK_SOFT: Color32 = Color32::from_rgb(240, 182, 196);
const FIELD: Color32 = Color32::from_rgb(238, 243, 252);
const INK: Color32 = Color32::from_rgb(50, 50, 60);
const ICON: Color32 = Color32::from_rgb(70, 70, 80);
const PERSON: Color32 = Color32::from_rgb(110, 160, 220);
const GREEN: Color32 = Color32::from_rgb(110, 200, 120);
const RED: Color32 = Color32::from_rgb(226, 90, 100);
const W: f32 = 330.0;

// ---------------------------------------------------------------- icons

fn person(p: &egui::Painter, c: Pos2, s: f32, col: Color32) {
    p.circle_filled(c + Vec2::new(0.0, -s * 0.28), s * 0.24, col);
    let body = Rect::from_center_size(c + Vec2::new(0.0, s * 0.3), Vec2::new(s * 0.78, s * 0.42));
    p.rect_filled(body, egui::Rounding { nw: s * 0.36, ne: s * 0.36, sw: s * 0.08, se: s * 0.08 }, col);
}

fn people(p: &egui::Painter, c: Pos2, s: f32, col: Color32) {
    person(p, c + Vec2::new(-s * 0.3, s * 0.04), s * 0.62, col);
    person(p, c + Vec2::new(s * 0.3, s * 0.04), s * 0.62, col);
    person(p, c + Vec2::new(0.0, s * 0.1), s * 0.8, col);
}

fn lock(p: &egui::Painter, c: Pos2, s: f32, col: Color32) {
    let body = Rect::from_center_size(c + Vec2::new(0.0, s * 0.18), Vec2::new(s * 0.62, s * 0.5));
    p.rect_filled(body, s * 0.06, col);
    let r = s * 0.2;
    let top = c + Vec2::new(0.0, -s * 0.08);
    let pts: Vec<Pos2> = (0..=16).map(|i| {
        let a = std::f32::consts::PI * (i as f32 / 16.0);
        Pos2::new(top.x - r * a.cos(), top.y - r * a.sin())
    }).collect();
    p.add(egui::Shape::line(pts, Stroke::new(s * 0.09, col)));
    p.line_segment([top + Vec2::new(-r, 0.0), top + Vec2::new(-r, s * 0.08)], Stroke::new(s * 0.09, col));
    p.line_segment([top + Vec2::new(r, 0.0), top + Vec2::new(r, s * 0.08)], Stroke::new(s * 0.09, col));
    p.circle_filled(body.center(), s * 0.06, Color32::WHITE);
}

fn crown(p: &egui::Painter, c: Pos2, s: f32) {
    let y0 = c.y + s * 0.3;
    let pts = vec![
        Pos2::new(c.x - s * 0.45, y0),
        Pos2::new(c.x - s * 0.5, c.y - s * 0.3),
        Pos2::new(c.x - s * 0.2, c.y),
        Pos2::new(c.x, c.y - s * 0.38),
        Pos2::new(c.x + s * 0.2, c.y),
        Pos2::new(c.x + s * 0.5, c.y - s * 0.3),
        Pos2::new(c.x + s * 0.45, y0),
    ];
    p.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgb(255, 214, 60), Stroke::new(1.5, Color32::from_rgb(214, 160, 30))));
    let _ = pts;
}

fn cross(p: &egui::Painter, c: Pos2, s: f32, col: Color32) {
    let st = Stroke::new(s * 0.22, col);
    p.line_segment([c + Vec2::splat(-s * 0.35), c + Vec2::splat(s * 0.35)], st);
    p.line_segment([c + Vec2::new(-s * 0.35, s * 0.35), c + Vec2::new(s * 0.35, -s * 0.35)], st);
}

fn antenna(p: &egui::Painter, c: Pos2, s: f32, col: Color32) {
    let st = Stroke::new(s * 0.07, col);
    let top = c + Vec2::new(0.0, -s * 0.12);
    p.circle_filled(top, s * 0.07, col);
    p.line_segment([top, c + Vec2::new(-s * 0.2, s * 0.42)], st);
    p.line_segment([top, c + Vec2::new(s * 0.2, s * 0.42)], st);
    p.line_segment([c + Vec2::new(-s * 0.1, s * 0.15), c + Vec2::new(s * 0.1, s * 0.15)], st);
    p.line_segment([c + Vec2::new(-s * 0.15, s * 0.3), c + Vec2::new(s * 0.15, s * 0.3)], st);
    for (r, a0) in [(s * 0.2, 0.6f32), (s * 0.32, 0.6)] {
        for side in [-1.0f32, 1.0] {
            let pts: Vec<Pos2> = (0..=10)
                .map(|i| {
                    let a = -a0 + 2.0 * a0 * i as f32 / 10.0;
                    let ang = if side < 0.0 { std::f32::consts::PI + a } else { a };
                    Pos2::new(top.x + r * ang.cos(), top.y + r * ang.sin())
                })
                .collect();
            p.add(egui::Shape::line(pts, st));
        }
    }
}

fn arrow(p: &egui::Painter, c: Pos2, s: f32, up: bool, col: Color32) {
    let h = if up { -s * 0.45 } else { s * 0.45 };
    p.add(egui::Shape::convex_polygon(vec![c + Vec2::new(0.0, h), c + Vec2::new(-s * 0.45, -h), c + Vec2::new(s * 0.45, -h)], col, Stroke::NONE));
}

fn plus_button(ui: &mut egui::Ui, c: Pos2, r: f32, id: egui::Id) -> egui::Response {
    let rect = Rect::from_center_size(c, Vec2::splat(r * 2.0));
    let resp = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    p.circle_filled(c, r, if resp.hovered() { lighten(PURPLE, 0.15) } else { PURPLE });
    let st = Stroke::new(r * 0.22, Color32::from_rgb(255, 224, 140));
    p.line_segment([c + Vec2::new(-r * 0.5, 0.0), c + Vec2::new(r * 0.5, 0.0)], st);
    p.line_segment([c + Vec2::new(0.0, -r * 0.5), c + Vec2::new(0.0, r * 0.5)], st);
    resp
}

fn lighten(c: Color32, t: f32) -> Color32 {
    let l = |x: u8| (x as f32 + (255.0 - x as f32) * t) as u8;
    Color32::from_rgb(l(c.r()), l(c.g()), l(c.b()))
}

fn icon_button(ui: &mut egui::Ui, radius: f32, active: bool, tip: &str, draw: impl Fn(&egui::Painter, Pos2, f32)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(radius * 2.0), Sense::click());
    let p = ui.painter();
    p.circle_filled(rect.center() + Vec2::new(0.0, 2.0), radius, Color32::from_black_alpha(50));
    p.circle_filled(rect.center(), radius, if resp.hovered() { lighten(PURPLE, 0.15) } else { PURPLE });
    if active {
        p.circle_stroke(rect.center(), radius - 2.0, Stroke::new(3.0, Color32::WHITE));
    }
    draw(p, rect.center(), radius * 1.3);
    resp.on_hover_text(tr(tip))
}

/// The two collab buttons of the main menu (left column, above "hide UI").
pub fn main_buttons(app: &mut App, ui: &mut egui::Ui) {
    let live = app.net.is_some();
    if icon_button(ui, 30.0, app.ui.collab_open, "Настройка коллаба (VNet)", |p, c, s| antenna(p, c, s, Color32::WHITE)).clicked() {
        app.ui.collab_open = !app.ui.collab_open;
        app.ui.collab_control = false;
    }
    if icon_button(ui, 20.0, app.ui.collab_control || live, "Управление коллабом", |p, c, s| people(p, c, s * 0.75, Color32::WHITE)).clicked() {
        app.ui.collab_control = !app.ui.collab_control;
        app.ui.collab_open = false;
    }
}

// ---------------------------------------------------------------- widgets

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(Color32::WHITE)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .shadow(egui::Shadow { offset: Vec2::new(0.0, 2.0), blur: 8.0, spread: 0.0, color: Color32::from_black_alpha(40) })
        .show(ui, |ui| {
            ui.set_width(W - 24.0);
            ui.set_max_width(W - 24.0);
            add(ui)
        });
}

fn switch(ui: &mut egui::Ui, on: bool, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id, on, 0.15);
    let p = ui.painter();
    p.rect_filled(rect, size.y * 0.5, Color32::from_rgb(214, 234, 252));
    p.rect_stroke(rect, size.y * 0.5, Stroke::new(1.5, SKY));
    let x = egui::lerp(rect.left() + size.y * 0.5..=rect.right() - size.y * 0.5, t);
    p.circle_filled(Pos2::new(x, rect.center().y), size.y * 0.4, if on { VBLUE } else { SKY });
    resp
}

fn vts_toggle(ui: &mut egui::Ui, on: &mut bool, label: &str) {
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(40.0, 20.0), Sense::click());
        let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.15);
        let p = ui.painter();
        p.rect_filled(rect, 10.0, if *on { VBLUE } else { Color32::from_rgb(214, 228, 244) });
        let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
        p.circle_filled(Pos2::new(x, rect.center().y), 8.0, Color32::WHITE);
        let l = ui.add(egui::Label::new(RichText::new(tr(label)).size(15.0).color(if *on { INK } else { Color32::GRAY })).sense(Sense::click()));
        if resp.clicked() || l.clicked() {
            *on = !*on;
        }
    });
}

fn help(ui: &mut egui::Ui, title: &str, tip: &str) {
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 8.5, ICON);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "?", FontId::proportional(13.0), Color32::WHITE);
        resp.on_hover_text(tr(tip));
        ui.label(RichText::new(tr(title)).size(17.0).color(INK));
    });
}

fn wide_button(ui: &mut egui::Ui, text: &str, fill: Color32, w: f32, enabled: bool) -> egui::Response {
    let fill = if enabled { fill } else { lighten(fill, 0.45) };
    ui.add_enabled(enabled, egui::Button::new(RichText::new(tr(text)).size(16.0).strong().color(Color32::WHITE)).fill(fill).rounding(3.0).min_size(Vec2::new(w, 30.0)))
}

/// VTS pill slider: light track, value range labels, dark-purple knob with the value.
fn fps_slider(ui: &mut egui::Ui, v: &mut u32, lo: u32, hi: u32, enabled: bool) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let knob = 70.0;
    let mut changed = false;
    if let Some(pp) = resp.interact_pointer_pos().filter(|_| enabled) {
        let t = ((pp.x - rect.left() - knob / 2.0) / (rect.width() - knob)).clamp(0.0, 1.0);
        let nv = (lo as f32 + t * (hi - lo) as f32).round() as u32;
        changed = nv != *v;
        *v = nv;
    }
    let t = (*v - lo) as f32 / (hi - lo) as f32;
    let p = ui.painter();
    p.rect_filled(rect, 14.0, LAVENDER);
    p.text(rect.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, lo.to_string(), FontId::proportional(14.0), INK);
    let kr = Rect::from_min_size(Pos2::new(rect.left() + t * (rect.width() - knob), rect.top()), Vec2::new(knob, rect.height()));
    p.rect_filled(kr, 14.0, if enabled { PURPLE } else { lighten(PURPLE, 0.4) });
    p.text(kr.center(), Align2::CENTER_CENTER, v.to_string(), FontId::proportional(15.0), Color32::WHITE);
    changed
}

fn buffer_slider(ui: &mut egui::Ui, v: &mut f32) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(170.0, 22.0), Sense::click_and_drag());
    let knob = 50.0;
    if let Some(pp) = resp.interact_pointer_pos() {
        let t = ((pp.x - rect.left() - knob / 2.0) / (rect.width() - knob)).clamp(0.0, 1.0);
        *v = (t * 100.0).round() / 100.0;
    }
    let p = ui.painter();
    p.rect_filled(rect, 11.0, Color32::from_rgb(206, 222, 240));
    p.text(rect.right_center() - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER, "1", FontId::proportional(12.0), INK);
    let kr = Rect::from_min_size(Pos2::new(rect.left() + v.clamp(0.0, 1.0) * (rect.width() - knob), rect.top()), Vec2::new(knob, rect.height()));
    p.rect_filled(kr, 11.0, VBLUE);
    p.text(kr.center(), Align2::CENTER_CENTER, format!("{v:.2}"), FontId::proportional(12.0), Color32::WHITE);
}

fn timer_pill(ui: &mut egui::Ui, secs: u64) {
    let text = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    ui.vertical_centered(|ui| {
        egui::Frame::none().fill(Color32::from_rgb(132, 190, 230)).rounding(4.0).inner_margin(egui::Margin::symmetric(10.0, 4.0)).show(ui, |ui| {
            ui.label(RichText::new(text.clone()).size(19.0).color(Color32::WHITE));
        });
    });
}

fn rates(ui: &mut egui::Ui, r: [f32; 2]) {
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        ui.label(RichText::new(format!("{:.1} kbit/s", r[0])).size(12.0).color(Color32::GRAY));
        let (a, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        arrow(ui.painter(), a.center(), 10.0, true, PINK);
        ui.add_space(36.0);
        ui.label(RichText::new(format!("{:.1} kbit/s", r[1])).size(12.0).color(Color32::GRAY));
        let (a, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        arrow(ui.painter(), a.center(), 10.0, false, Color32::from_rgb(110, 200, 230));
    });
}

/// This machine's LAN address (no packet is sent; the OS just picks the route).
fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

// ---------------------------------------------------------------- setup screen

/// Draws both collab screens when open.
pub fn collab_window(app: &mut App, ctx: &egui::Context) {
    if app.ui.collab_open {
        setup_screen(app, ctx);
    } else if app.ui.collab_control {
        control_screen(app, ctx);
    }
}

fn setup_screen(app: &mut App, ctx: &egui::Context) {
    let screen = ctx.screen_rect();
    // Dimmed backdrop like the VTS collab screen.
    egui::Area::new(egui::Id::new("vnet-dim")).fixed_pos(screen.min).order(egui::Order::Background).interactable(false).show(ctx, |ui| {
        ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(110));
        ui.painter().text(screen.left_bottom() + Vec2::new(16.0, -14.0), Align2::LEFT_BOTTOM, format!("VNet-style collab · protocol {}", crate::net::PROTOCOL), FontId::proportional(14.0), Color32::from_gray(200));
    });
    let mut close = false;
    egui::Area::new(egui::Id::new("vnet-setup")).anchor(Align2::RIGHT_TOP, [-24.0, 40.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.set_width(W);
        ui.spacing_mut().item_spacing.y = 4.0;
        let top = ui.cursor().min;
        card(ui, |ui| setup_body(app, ui));
        // Round badge with the people icon overlapping the top edge, gear at the corner.
        let p = ui.painter();
        let c = Pos2::new(top.x + W / 2.0, top.y);
        match &app.net {
            // During a session the badge becomes the session timer (as in VTS).
            Some(s) => {
                let secs = s.started.elapsed().as_secs();
                let r = Rect::from_center_size(c, Vec2::new(150.0, 34.0));
                p.rect_filled(r, 4.0, Color32::from_rgb(132, 190, 230));
                p.rect_stroke(r, 4.0, Stroke::new(2.0, Color32::WHITE));
                p.text(r.center(), Align2::CENTER_CENTER, format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60), FontId::proportional(20.0), Color32::WHITE);
            }
            None => {
                p.circle_filled(c, 30.0, Color32::WHITE);
                p.circle_filled(c, 26.0, SKY);
                people(p, c, 30.0, Color32::WHITE);
            }
        }
        let g = Pos2::new(top.x + W - 2.0, top.y);
        let gr = ui.interact(Rect::from_center_size(g, Vec2::splat(30.0)), egui::Id::new("vnet-gear"), Sense::click());
        ui.painter().circle_filled(g, 16.0, Color32::WHITE);
        ui.painter().text(g, Align2::CENTER_CENTER, "⚙", FontId::proportional(22.0), Color32::from_rgb(70, 150, 230));
        if gr.on_hover_text(tr("Общие настройки коллаба")).clicked() {
            let id = egui::Id::new("vnet-settings-open");
            let v = ui.ctx().data(|d| d.get_temp::<bool>(id).unwrap_or(false));
            ui.ctx().data_mut(|d| d.insert_temp(id, !v));
        }
        ui.add_space(4.0);
        card(ui, |ui| session_controls(app, ui));
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let b = egui::Button::new(RichText::new(tr("Вернуться")).size(17.0).strong().color(Color32::WHITE)).fill(VBLUE).rounding(3.0).min_size(Vec2::new(200.0, 34.0));
            if ui.add(b).clicked() {
                close = true;
            }
        });
    });
    collab_settings(app, ctx);
    if close {
        app.ui.collab_open = false;
        app.save();
    }
}

fn collab_settings(app: &mut App, ctx: &egui::Context) {
    let id = egui::Id::new("vnet-settings-open");
    let mut open = ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    if !open {
        return;
    }
    egui::Window::new(tr("Общие настройки коллаба")).open(&mut open).collapsible(false).resizable(false).order(egui::Order::Tooltip).show(ctx, |ui| {
        let n = &mut app.settings.net;
        vts_toggle(ui, &mut n.share_items, "Делиться предметами, которые я добавляю");
        vts_toggle(ui, &mut n.smoothing, "VNet: сглаживание параметров (выключите для покадровых ригов)");
        ui.horizontal(|ui| {
            ui.label(tr("Лимит отдачи файлов"));
            ui.add(egui::DragValue::new(&mut n.upload_kbps).range(0..=100_000).suffix(" KB/s"));
            ui.label(RichText::new(tr("0 = без лимита")).color(Color32::GRAY));
        });
        ui.horizontal(|ui| {
            ui.label(tr("TCP-порт"));
            ui.add_enabled(app.net.is_none(), egui::DragValue::new(&mut n.port).range(1024..=65535));
        });
        ui.label(
            RichText::new(tr("Хост должен быть доступен по TCP: та же локальная сеть, проброс порта или виртуальная LAN (Tailscale, ZeroTier, Radmin VPN)."))
                .size(12.0)
                .color(Color32::GRAY),
        );
    });
    ctx.data_mut(|d| d.insert_temp(id, open));
}

fn setup_body(app: &mut App, ui: &mut egui::Ui) {
    let running = app.net.is_some();
    ui.add_space(18.0);
    // Join / Host switch.
    ui.horizontal(|ui| {
        let host = app.settings.net.host;
        ui.add_space(52.0);
        ui.label(RichText::new(tr("Гость")).size(22.0).color(if host { Color32::from_gray(190) } else { INK }));
        let r = ui.add_enabled_ui(!running, |ui| switch(ui, host, Vec2::new(62.0, 30.0))).inner;
        if r.clicked() && !running {
            app.settings.net.host = !host;
        }
        ui.label(RichText::new(tr("Хост")).size(26.0).color(if host { INK } else { Color32::from_gray(190) }));
    });
    ui.add_space(4.0);
    // Password.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
        lock(ui.painter(), r.center(), 28.0, if running { Color32::GRAY } else { ICON });
        let empty = app.settings.net.password.is_empty();
        let frame = egui::Frame::none().fill(FIELD).rounding(4.0).stroke(Stroke::new(1.5, if empty && !running { RED } else { FIELD })).inner_margin(egui::Margin::symmetric(6.0, 3.0));
        frame.show(ui, |ui| {
            ui.set_width(250.0);
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !running,
                    egui::TextEdit::singleline(&mut app.settings.net.password).password(true).hint_text(tr("Пароль…")).frame(false).font(FontId::proportional(18.0)).desired_width(212.0),
                );
                let (cr, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                cross(ui.painter(), cr.center(), 18.0, if empty { RED } else { lighten(RED, 0.4) });
                if resp.clicked() && !running {
                    app.settings.net.password.clear();
                }
            });
        });
    });
    // Nickname.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
        person(ui.painter(), r.center(), 28.0, if running { Color32::GRAY } else { ICON });
        egui::Frame::none().fill(FIELD).rounding(4.0).inner_margin(egui::Margin::symmetric(6.0, 5.0)).show(ui, |ui| {
            ui.add_enabled(!running, egui::TextEdit::singleline(&mut app.settings.net.nickname).hint_text(tr("Ник")).frame(false).font(FontId::proportional(18.0)).desired_width(238.0));
        });
    });
    ui.label(RichText::new(tr("Список гостей")).size(17.0).color(INK));
    guest_list(app, ui);
    ui.add_space(2.0);
    help(ui, "Подключение", "Прямое TCP-подключение к хосту (вместо файлового сервера VTS). Модели и предметы передаются через хост.");
    ui.vertical_centered(|ui| {
        let text = if app.settings.net.host {
            format!("{} : {}", local_ip().unwrap_or_else(|| "0.0.0.0".into()), app.settings.net.port)
        } else if app.settings.net.address.is_empty() {
            tr("Укажите адрес хоста (кнопка +)").to_string()
        } else {
            app.settings.net.address.clone()
        };
        let b = egui::Button::new(RichText::new(text).size(16.0).strong().color(Color32::WHITE)).fill(VBLUE).rounding(2.0).min_size(Vec2::new(W - 50.0, 28.0));
        if ui.add(b).on_hover_text(tr("Нажмите, чтобы скопировать")).clicked() && app.settings.net.host {
            ui.ctx().copy_text(format!("{}:{}", local_ip().unwrap_or_default(), app.settings.net.port));
        }
    });
    help(ui, "Data FPS", "Как часто отправляются данные движения (выставляет хост).");
    let host = app.settings.net.host;
    match app.net.as_mut() {
        Some(s) => {
            let mut fps = s.data_fps;
            if fps_slider(ui, &mut fps, 10, 60, s.role == Role::Host) {
                s.set_data_fps(fps);
                app.settings.net.data_fps = fps;
            }
        }
        None => {
            fps_slider(ui, &mut app.settings.net.data_fps, 10, 60, host);
        }
    }
    rates(ui, app.net.as_ref().map(|s| s.rate).unwrap_or([0.0; 2]));
    help(ui, "Статус", "Состояние подключения.");
    let status: Vec<String> = match &app.net {
        None => vec![tr("Не в сети.").to_string(), tr("Коллаб доступен бесплатно.").to_string()],
        Some(s) => {
            let mut v = vec![if s.connected { format!("{} — {}", tr("В сети"), s.status) } else { s.status.clone() }];
            if s.packing() > 0 {
                v.push(tr("Упаковка модели/предметов…").to_string());
            }
            for (n, e) in s.pack_errors() {
                v.push(format!("⚠ {n}: {e}"));
            }
            v
        }
    };
    for l in status {
        ui.horizontal(|ui| {
            ui.add_space(26.0);
            ui.label(RichText::new(l).size(14.0).color(INK));
        });
    }
}

fn guest_list(app: &mut App, ui: &mut egui::Ui) {
    // Slot 0 is always the host, then the guests in join order; the rest are empty.
    struct Row {
        id: Option<u16>,
        nick: String,
        ping: Option<f32>,
        online: bool,
        me: bool,
        up: bool,
        down: bool,
    }
    let mut rows: Vec<Row> = Vec::new();
    match &app.net {
        Some(s) => {
            let me = Row { id: Some(s.my_id), nick: s.nick.clone(), ping: Some(if s.role == Role::Host { 0.0 } else { s.my_ping }), online: s.connected, me: true, up: s.uploading() > 0, down: !s.transfers().is_empty() };
            if s.role == Role::Host {
                rows.push(me);
                for p in s.peers.values() {
                    rows.push(Row { id: Some(p.id), nick: p.nick.clone(), ping: Some(p.ping_ms), online: p.has_data(), me: false, up: p.uploading > 0, down: s.asset_progress(p.avatar_asset).map(|f| f < 1.0).unwrap_or(false) });
                }
            } else {
                rows.push(Row { id: Some(0), nick: if s.host_nick.is_empty() { app.settings.net.address.clone() } else { s.host_nick.clone() }, ping: Some(s.my_ping), online: s.connected, me: false, up: false, down: false });
                rows.push(me);
                for p in s.peers.values().filter(|p| p.id != 0) {
                    rows.push(Row { id: Some(p.id), nick: p.nick.clone(), ping: Some(p.ping_ms), online: p.has_data(), me: false, up: false, down: false });
                }
            }
        }
        None => {
            let nick = if app.settings.net.nickname.is_empty() { "Player".to_string() } else { app.settings.net.nickname.clone() };
            if app.settings.net.host {
                rows.push(Row { id: None, nick, ping: None, online: false, me: true, up: false, down: false });
            } else if !app.settings.net.address.is_empty() {
                rows.push(Row { id: None, nick: app.settings.net.address.clone(), ping: None, online: false, me: false, up: false, down: false });
            }
        }
    }
    let host_mode = app.net.as_ref().map(|s| s.role == Role::Host).unwrap_or(app.settings.net.host);
    let mut kick = None;
    let mut open_plus = false;
    for slot in 0..MAX_PARTICIPANTS {
        ui.horizontal(|ui| {
            let (ir, _) = ui.allocate_exact_size(Vec2::new(34.0, 38.0), Sense::hover());
            if slot == 0 {
                crown(ui.painter(), ir.center(), 30.0);
            } else {
                person(ui.painter(), ir.center(), 30.0, PERSON);
            }
            let (rect, _) = ui.allocate_exact_size(Vec2::new(W - 24.0 - 52.0, 38.0), Sense::hover());
            match rows.get(slot) {
                Some(r) => {
                    let p = ui.painter();
                    p.rect_filled(rect, 12.0, PURPLE_ROW);
                    let av = Rect::from_min_size(rect.min + Vec2::new(4.0, 4.0), Vec2::splat(30.0));
                    p.rect_filled(av, 7.0, Color32::from_rgb(250, 226, 236));
                    person(p, av.center(), 24.0, Color32::from_rgb(232, 140, 170));
                    p.circle_filled(av.right_top() + Vec2::new(8.0, 9.0), 5.0, if r.online { GREEN } else { RED });
                    p.circle_stroke(av.right_top() + Vec2::new(8.0, 9.0), 5.0, Stroke::new(1.0, Color32::WHITE));
                    p.text(av.right_top() + Vec2::new(16.0, 8.0), Align2::LEFT_CENTER, &r.nick, FontId::proportional(16.0), Color32::WHITE);
                    let ping = match r.ping.filter(|_| r.online) {
                        Some(ms) => format!("{} {:.0} ms", tr("Пинг:"), ms),
                        None => format!("{}  -", tr("Пинг:")),
                    };
                    p.text(av.right_bottom() + Vec2::new(16.0, -7.0), Align2::LEFT_CENTER, ping, FontId::proportional(13.0), Color32::from_gray(235));
                    let mut x = rect.right() - 54.0;
                    if r.up {
                        arrow(p, Pos2::new(x, rect.bottom() - 10.0), 9.0, true, Color32::WHITE);
                        x -= 18.0;
                    }
                    if r.down {
                        arrow(p, Pos2::new(x, rect.bottom() - 10.0), 9.0, false, Color32::WHITE);
                    }
                    if r.me {
                        let tag = Rect::from_center_size(Pos2::new(rect.right() - 4.0, rect.center().y), Vec2::new(30.0, 30.0));
                        p.rect_filled(tag, 6.0, Color32::from_rgb(236, 232, 248));
                        p.text(tag.center() + Vec2::new(0.0, 7.0), Align2::CENTER_CENTER, "YOU", FontId::proportional(10.0), PURPLE);
                        p.text(tag.center() - Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, "=^.^=", FontId::proportional(9.0), INK);
                    } else if host_mode && slot > 0 {
                        let c = Pos2::new(rect.right() - 22.0, rect.center().y);
                        let resp = ui.interact(Rect::from_center_size(c, Vec2::splat(28.0)), egui::Id::new(("vnet-kick", slot)), Sense::click());
                        cross(ui.painter(), c, 22.0, if resp.hovered() { lighten(RED, 0.2) } else { RED });
                        if resp.on_hover_text(tr("Выгнать")).clicked() {
                            kick = r.id;
                        }
                    }
                }
                None => {
                    ui.painter().rect_filled(rect, 12.0, LAVENDER);
                    ui.painter().rect_stroke(rect, 12.0, Stroke::new(1.0, Color32::from_rgb(222, 214, 240)));
                    // "+" is where VTS invites a Steam friend: here it shows how to connect.
                    let show_plus = app.net.is_none() && ((host_mode && slot >= 1) || (!host_mode && slot == 0));
                    if show_plus && plus_button(ui, rect.center(), 15.0, egui::Id::new(("vnet-plus", slot))).clicked() {
                        open_plus = true;
                    }
                }
            }
        });
    }
    if let (Some(id), Some(s)) = (kick, app.net.as_mut()) {
        s.kick(id);
    }
    let pid = egui::Id::new("vnet-plus-open");
    if open_plus {
        ui.ctx().data_mut(|d| d.insert_temp(pid, true));
    }
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(pid).unwrap_or(false));
    if open {
        let title = if host_mode { tr("Пригласить гостей") } else { tr("Адрес хоста") };
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).order(egui::Order::Tooltip).show(ui.ctx(), |ui| {
            if host_mode {
                let addr = format!("{}:{}", local_ip().unwrap_or_else(|| "?".into()), app.settings.net.port);
                ui.label(tr("Отправьте гостям ваш адрес и пароль:"));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&addr).size(18.0).strong());
                    if crate::vts_style::button(ui, tr("Копировать")).clicked() {
                        ui.ctx().copy_text(addr.clone());
                    }
                });
                ui.label(RichText::new(tr("Это адрес в локальной сети. Для игры через интернет нужен проброс порта или виртуальная LAN (Tailscale / ZeroTier / Radmin VPN) — тогда дайте адрес из неё.")).size(12.0).color(Color32::GRAY));
            } else {
                ui.label(tr("IP или IP:порт хоста"));
                ui.add(egui::TextEdit::singleline(&mut app.settings.net.address).hint_text("192.168.1.10:25580").desired_width(240.0));
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(pid, open));
    }
}

fn session_controls(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new(tr("Управление сессией")).size(17.0).color(INK));
    let running = app.net.is_some();
    ui.horizontal(|ui| {
        let label = if app.settings.net.host { format!("{}   🌐", tr("Хост")) } else { format!("{}   🌐", tr("Войти")) };
        if wide_button(ui, &label, SKY, 140.0, !running).clicked() {
            app.save();
            crate::net::start(app);
        }
        ui.add_space(14.0);
        if wide_button(ui, tr("Выйти из коллаба"), PINK, 140.0, running).clicked() {
            crate::net::stop(app);
            app.save();
        }
    });
}

// ---------------------------------------------------------------- session control screen

fn bars(ui: &mut egui::Ui, up: f32, down: f32, tip: String) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(176.0, 26.0), Sense::hover());
    let p = ui.painter();
    let top = Rect::from_min_size(rect.min + Vec2::new(0.0, 2.0), Vec2::new(164.0, 9.0));
    let bot = Rect::from_min_size(rect.min + Vec2::new(0.0, 15.0), Vec2::new(164.0, 9.0));
    p.rect_filled(top, 0.0, Color32::from_rgb(214, 238, 220));
    p.rect_filled(Rect::from_min_size(top.min, Vec2::new(top.width() * up.clamp(0.0, 1.0), top.height())), 0.0, Color32::from_rgb(130, 200, 150));
    p.rect_filled(bot, 0.0, Color32::from_rgb(250, 220, 226));
    p.rect_filled(Rect::from_min_size(bot.min, Vec2::new(bot.width() * down.clamp(0.0, 1.0), bot.height())), 0.0, Color32::from_rgb(236, 140, 160));
    arrow(p, top.right_center() + Vec2::new(7.0, 0.0), 8.0, true, Color32::from_rgb(110, 190, 130));
    arrow(p, bot.right_center() + Vec2::new(7.0, 0.0), 8.0, false, Color32::from_rgb(230, 120, 140));
    if !tip.is_empty() {
        resp.on_hover_text(tip);
    }
}

fn control_screen(app: &mut App, ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("vnet-control")).anchor(Align2::RIGHT_TOP, [-24.0, 20.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.set_width(W);
        ui.spacing_mut().item_spacing.y = 8.0;
        let Some(s) = app.net.as_mut() else {
            card(ui, |ui| {
                ui.label(RichText::new(tr("Вы не в коллабе.")).size(18.0).color(INK));
                if wide_button(ui, tr("Настройка коллаба"), VBLUE, W - 30.0, true).clicked() {
                    app.ui.collab_open = true;
                    app.ui.collab_control = false;
                }
            });
            return;
        };
        timer_pill(ui, s.started.elapsed().as_secs());
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                people(ui.painter(), r.center(), 26.0, ICON);
                ui.label(RichText::new(tr("Людей в коллабе:")).size(19.0).color(INK));
                ui.label(RichText::new(format!("{} / {}", s.participant_count(), MAX_PARTICIPANTS)).size(20.0).strong().color(INK));
            });
            rates(ui, s.rate);
        });
        let mut quit = false;
        card(ui, |ui| {
            let label = tr("Выйти из коллаба").to_string();
            if ui.add_sized([W - 24.0, 36.0], egui::Button::new(RichText::new(label).size(18.0).strong().color(Color32::WHITE)).fill(PINK).rounding(3.0)).clicked() {
                quit = true;
            }
            ui.horizontal(|ui| {
                let oid = egui::Id::new("vnet-ordering");
                if ui.add(egui::Button::new(RichText::new(tr("Порядок")).size(16.0).color(Color32::WHITE)).fill(VBLUE).rounding(3.0).min_size(Vec2::new(150.0, 28.0))).clicked() {
                    let v = ui.ctx().data(|d| d.get_temp::<bool>(oid).unwrap_or(false));
                    ui.ctx().data_mut(|d| d.insert_temp(oid, !v));
                }
                vts_toggle(ui, &mut s.fixed_order, "Фикс. порядок");
            });
        });
        ordering_window(ui.ctx(), s);
        // Me.
        let (up_me, uploading) = (s.uploading(), s.uploading() > 0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                if s.role == Role::Host {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    crown(ui.painter(), r.center(), 18.0);
                }
                ui.label(RichText::new(&s.nick).size(17.0).color(INK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    bars(ui, if uploading { 0.5 } else { 1.0 }, 1.0, if uploading { format!("{} {:.1} MB", tr("Отправка:"), up_me as f64 / 1048576.0) } else { String::new() });
                    ui.label(RichText::new("0 ms").size(14.0).color(INK));
                });
            });
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("=^.^=").size(22.0).color(INK));
                ui.label(RichText::new(tr("Это вы!!")).size(18.0).color(INK));
                if uploading {
                    ui.label(RichText::new(format!("{} {:.1} MB", tr("Отправка:"), up_me as f64 / 1048576.0)).size(15.0).color(Color32::from_rgb(220, 70, 160)));
                }
            });
        });
        let progress: Vec<Option<f32>> = s.peers.values().map(|p| s.asset_progress(p.avatar_asset)).collect();
        for (p, prog) in s.peers.values_mut().zip(progress) {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    if p.id == 0 {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                        crown(ui.painter(), r.center(), 18.0);
                    }
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    ui.painter().circle_stroke(r.center(), 9.0, Stroke::new(1.5, ICON));
                    person(ui.painter(), r.center(), 14.0, ICON);
                    ui.label(RichText::new(&p.nick).size(17.0).color(INK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let down = prog.unwrap_or(0.0);
                        let tip = match prog {
                            Some(f) if f < 1.0 => format!("{} {:.0}%", tr("Загрузка модели:"), f * 100.0),
                            _ => String::new(),
                        };
                        bars(ui, if p.uploading > 0 { 0.5 } else { 1.0 }, down, tip);
                        ui.label(RichText::new(format!("{:.0} ms", p.ping_ms)).size(14.0).color(INK));
                    });
                });
                ui.indent(("vnet-peer", p.id), |ui| {
                    vts_toggle(ui, &mut p.local_control, "Позицию задаю я");
                    if p.local_control {
                        crate::vts_style::slider_ex(ui, &mut p.offset[0], -1.0..=1.0, "", "", "X", "");
                        crate::vts_style::slider_ex(ui, &mut p.offset[1], -0.7..=0.7, "", "", "Y", "");
                        crate::vts_style::slider_ex(ui, &mut p.offset[2], 0.2..=3.0, "", "", "Размер", "");
                    }
                    vts_toggle(ui, &mut p.show_model, "Показывать модель");
                    vts_toggle(ui, &mut p.show_items, "Показывать предметы");
                    vts_toggle(ui, &mut p.front, "Перед моей моделью");
                    ui.horizontal(|ui| {
                        buffer_slider(ui, &mut p.buffer);
                        help(ui, "Буфер (сек)", "Задержка/сглаживание движения этого участника (0-1 с, по умолчанию 0.05).");
                    });
                });
            });
        }
        if quit {
            crate::net::stop(app);
            app.save();
        }
    });
}

fn ordering_window(ctx: &egui::Context, s: &mut crate::net::Session) {
    let oid = egui::Id::new("vnet-ordering");
    let mut open = ctx.data(|d| d.get_temp::<bool>(oid).unwrap_or(false));
    if !open {
        return;
    }
    egui::Window::new(tr("Порядок участников")).open(&mut open).collapsible(false).resizable(false).order(egui::Order::Tooltip).show(ctx, |ui| {
        ui.label(RichText::new(tr("Сверху — дальше всех, снизу — ближе всех. Применяется только у вас (при включённом фиксированном порядке).")).size(12.0).color(Color32::GRAY));
        let mut ids: Vec<u16> = s.peers.keys().copied().collect();
        ids.sort_by_key(|id| s.peers[id].rank);
        let mut swap = None;
        for (i, id) in ids.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&s.peers[id].nick).size(16.0));
                if ui.add_enabled(i > 0, egui::Button::new("▲")).clicked() {
                    swap = Some((i, i - 1));
                }
                if ui.add_enabled(i + 1 < ids.len(), egui::Button::new("▼")).clicked() {
                    swap = Some((i, i + 1));
                }
            });
        }
        if let Some((a, b)) = swap {
            ids.swap(a, b);
        }
        for (r, id) in ids.iter().enumerate() {
            if let Some(p) = s.peers.get_mut(id) {
                p.rank = r as i32;
            }
        }
    });
    ctx.data_mut(|d| d.insert_temp(oid, open));
}

// ====================================================================== phone / mocopi / Discord cards
//
// Settings → General, laid out like the matching VTS cards.

use crate::app::Action;
use crate::ui::{section, status_dot, toggle};

fn status_line(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("Статус:")).strong());
        status_dot(ui, color);
        ui.label(RichText::new(text).size(15.0));
    });
}

fn phone_running(app: &App) -> bool {
    app.tracker.as_ref().map(|t| t.backend() == backend::PHONE).unwrap_or(false)
}

fn start_phone(app: &mut App, ip: String) {
    app.settings.phone_ip = ip;
    app.settings.track_backend = backend::PHONE;
    app.settings.track_type = 0;
    app.actions.push(Action::StartTracker);
    app.save();
}

fn scan(app: &mut App, usb: bool) {
    let slot = Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    let port = app.settings.phone_port;
    std::thread::spawn(move || {
        let extra: &[&str] = &crate::tracking::USB_TETHER_PHONES;
        *s2.lock().unwrap() = Some(crate::tracking::discover_phones(port, extra, std::time::Duration::from_millis(2500)));
    });
    app.phone_scan = Some((usb, slot));
}

use std::sync::{Arc, Mutex};

fn phone_status(app: &App, ui: &mut egui::Ui) {
    let (col, text) = if phone_running(app) {
        let rate = app.tracker.as_ref().map(|t| t.rate).unwrap_or(0.0);
        if rate > 0.5 {
            (Color32::from_rgb(90, 200, 110), format!("{} {} ({rate:.0} FPS)", tr("Телефон подключён:"), app.settings.phone_ip))
        } else {
            (Color32::from_rgb(240, 170, 60), format!("{} {}…", tr("Ожидание данных от"), app.settings.phone_ip))
        }
    } else {
        (Color32::GRAY, tr("Отключено").to_string())
    };
    status_line(ui, col, &text);
    if let (true, Some(e)) = (phone_running(app), &app.tracker_error) {
        ui.label(RichText::new(e).size(13.0).color(Color32::from_rgb(220, 80, 80)));
    }
}

fn input(ui: &mut egui::Ui, text: &mut String, hint: &str, enabled: bool) {
    ui.add_enabled(enabled, egui::TextEdit::singleline(text).hint_text(hint).font(egui::FontId::proportional(17.0)).desired_width(ui.available_width() - 10.0));
}

fn port_field(ui: &mut egui::Ui, v: &mut u16, enabled: bool) -> egui::Response {
    ui.add_enabled(enabled, egui::DragValue::new(v).range(1..=65535).speed(0.0))
}

fn phone_status_vts(app: &App, ui: &mut egui::Ui) {
    let (col, text) = if phone_running(app) {
        let rate = app.tracker.as_ref().map(|t| t.rate).unwrap_or(0.0);
        if rate > 0.5 {
            (Color32::from_rgb(110, 210, 110), format!("{} {}\n{}: {rate:.0} FPS", tr("Подключено к"), app.settings.phone_ip, tr("Частота данных")))
        } else {
            (Color32::from_rgb(240, 220, 60), tr("Подключение…").to_string())
        }
    } else {
        (Color32::GRAY, tr("Отключено").to_string())
    };
    crate::vts_style::status(ui, col, &text);
    if let (true, Some(e)) = (phone_running(app), &app.tracker_error) {
        crate::vts_style::note(ui, e);
    }
}

fn ip_list(ui: &mut egui::Ui, key: &str) {
    let id = egui::Id::new(("show-ip-list", key));
    let mut show = ui.ctx().data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    if crate::vts_style::wide(ui, tr("Показать список IP"), crate::vts_style::BLUE, 200.0, 28.0).clicked() {
        show = !show;
        ui.ctx().data_mut(|d| d.insert_temp(id, show));
    }
    if show {
        ui.label(RichText::new(crate::tracking::local_ips().join("\n")).size(16.0).strong());
    }
}

/// "Wi-Fi connection settings" (VTS: "Stream to PC (WiFi)"): the VTS phone app with
/// "3rd Party PC Clients" enabled.
pub fn wifi_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, tr("Настройки Wi-Fi-соединения"), |ui| {
        let running = phone_running(app);
        let mut on = running;
        ui.horizontal(|ui| {
            crate::vts_style::help(ui, "Трекинг с телефона: приложение VTube Studio на iPhone/Android, в его настройках включите «3rd Party PC Clients». Телефон и ПК должны быть в одной сети.");
            if toggle(ui, &mut on, tr("Подключить телефон")).changed() {
                if on {
                    let ip = app.settings.phone_ip.clone();
                    if ip.trim().is_empty() {
                        scan(app, false);
                    } else {
                        start_phone(app, ip);
                    }
                } else {
                    app.actions.push(Action::StopTracker);
                }
            }
        });
        ui.add_space(4.0);
        crate::vts_style::field_row(ui, "IP", |ui| input(ui, &mut app.settings.phone_ip, "192.168.1.23", !running));
        crate::vts_style::field_row(ui, "Порт", |ui| {
            port_field(ui, &mut app.settings.phone_port, !running);
        });
        ui.add_space(4.0);
        ui.columns(2, |c| {
            phone_status_vts(app, &mut c[0]);
            let busy = app.phone_scan.is_some();
            c[1].add_enabled_ui(!busy && !running, |ui| {
                if crate::vts_style::wide(ui, if busy { tr("Поиск…") } else { tr("Найти телефон") }, crate::vts_style::BLUE, 0.0, 32.0).clicked() {
                    scan(app, false);
                }
            });
            c[1].add_space(6.0);
            ip_list(&mut c[1], "wifi");
        });
        let mut pick = None;
        for (ip, usb) in &app.phone_found {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(RichText::new(format!("📱 {ip}{}", if *usb { "  (USB)" } else { "" })).size(16.0));
                if crate::vts_style::button(ui, tr("Подключить")).clicked() {
                    pick = Some(ip.to_string());
                }
            });
        }
        if let Some(ip) = pick {
            start_phone(app, ip);
        }
        crate::vts_style::note(ui, "Если Windows спросит про брандмауэр, разрешите доступ для частных сетей. Телефон может одновременно отдавать данные и в VBridger.");
    });
}

/// "USB settings": the same protocol over a USB tethering link.
pub fn usb_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, tr("Настройки USB-соединения"), |ui| {
        let usb_ip = crate::tracking::USB_TETHER_PHONES.contains(&app.settings.phone_ip.as_str());
        let mut on = phone_running(app) && usb_ip;
        let busy = app.phone_scan.as_ref().map(|s| s.0).unwrap_or(false);
        ui.horizontal(|ui| {
            crate::vts_style::help(ui, "Через USB-модем: iPhone — «Режим модема» + кабель (нужны iTunes или Apple Devices), Android — «USB-модем». Собственный USB-протокол VTS закрыт, поэтому данные идут по сети поверх кабеля.");
            if toggle(ui, &mut on, tr("Подключиться через USB")).changed() {
                if on {
                    scan(app, true);
                } else {
                    app.actions.push(Action::StopTracker);
                }
            }
        });
        ui.add_space(4.0);
        if busy {
            crate::vts_style::status(ui, Color32::from_rgb(240, 220, 60), tr("Поиск телефона на USB…"));
        } else if on {
            phone_status_vts(app, ui);
        } else {
            crate::vts_style::status(ui, Color32::GRAY, tr("USB выключен."));
        }
    });
}

pub fn mocopi_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, tr("Настройки mocopi"), |ui| {
        ui.horizontal(|ui| {
            crate::vts_style::help(ui, "В приложении mocopi (телефон или ПК) откройте «3rd party connection», укажите IP и порт отсюда и нажмите «Start sending data». Принимается один трекер одновременно.");
            if toggle(ui, &mut app.settings.mocopi, tr("Подключить")).changed() {
                app.mocopi_error = None;
                app.save();
            }
        });
        crate::vts_style::field_row(ui, "IP-адрес", |ui| ip_list(ui, "mocopi"));
        crate::vts_style::field_row(ui, "Порт", |ui| {
            if port_field(ui, &mut app.settings.mocopi_port, app.mocopi.is_none()).changed() {
                app.save();
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            crate::vts_style::help(ui, "Показывать параметры Mocopi* в списке входов при настройке модели. Без данных MocopiAngleX/Y/Z берутся из трекинга лица.");
            toggle(ui, &mut app.settings.mocopi_show_params, tr("Показывать параметры трекинга"));
        });
        let (col, text) = match (&app.mocopi, &app.mocopi_error) {
            (_, Some(e)) => (Color32::from_rgb(220, 80, 80), e.clone()),
            (Some(r), None) if r.connected() => (Color32::from_rgb(110, 210, 110), format!("{}\n{}: {:.0} FPS", tr("Подключено."), tr("Частота кадров движения"), r.rate)),
            (Some(r), None) => (Color32::from_rgb(240, 220, 60), format!("{} {}…", tr("Ожидание данных на порту"), r.port)),
            (None, None) => (Color32::GRAY, tr("Отключено").to_string()),
        };
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 7.0, col);
            ui.label(RichText::new(tr("Состояние подключения")).size(20.0));
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(34.0);
            ui.label(RichText::new(text).size(16.0));
        });
        if let Some(r) = app.mocopi.as_mut() {
            ui.horizontal(|ui| {
                ui.add_space(34.0);
                if crate::vts_style::button(ui, tr("Центрировать позицию тела")).clicked() {
                    r.recenter();
                }
            });
        }
        if app.settings.mocopi_show_params {
            egui::CollapsingHeader::new(tr("Параметры mocopi")).show(ui, |ui| {
                for (n, v) in crate::mocopi::snapshot() {
                    ui.horizontal(|ui| {
                        ui.add_sized([230.0, 16.0], egui::Label::new(n));
                        ui.label(format!("{v:+.1}"));
                    });
                }
            });
        }
    });
}

pub fn discord_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Discord", |ui| {
        let can = !app.settings.discord_app_id.trim().is_empty();
        let mut on = app.settings.discord;
        ui.horizontal(|ui| {
            crate::vts_style::help(ui, "Статус в профиле Discord (модель, коллаб, время). Нужен Application ID: создайте приложение на discord.com/developers/applications — его имя Discord покажет как активность.");
            if ui.add_enabled_ui(can || on, |ui| toggle(ui, &mut on, tr("Войти и активировать"))).inner.changed() {
                app.settings.discord = on;
                app.save();
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(30.0);
            toggle(ui, &mut app.settings.discord_show_model, tr("Показывать название модели"));
        });
        crate::vts_style::field_row(ui, "Application ID", |ui| input(ui, &mut app.settings.discord_app_id, "123456789012345678", !app.settings.discord));
        let (col, text) = match &app.discord {
            Some(p) if p.last_status == "connected" => (Color32::from_rgb(110, 210, 110), tr("Подключено").to_string()),
            Some(p) => (Color32::from_rgb(240, 220, 60), p.last_status.clone()),
            None => (Color32::GRAY, tr("Отключено").to_string()),
        };
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Состояние подключения")).size(20.0));
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 7.0, col);
            ui.label(RichText::new(text).size(16.0));
        });
    });
}

/// Per frame: mocopi receiver, Discord presence, phone discovery.
pub fn pump_extras(app: &mut App, dt: f32) {
    // mocopi
    if app.settings.mocopi {
        let port_changed = app.mocopi.as_ref().map(|r| r.port != app.settings.mocopi_port).unwrap_or(false);
        if port_changed {
            app.mocopi = None;
        }
        if app.mocopi.is_none() && app.mocopi_error.is_none() {
            match crate::mocopi::Receiver::start(app.settings.mocopi_port) {
                Ok(r) => app.mocopi = Some(r),
                Err(e) => app.mocopi_error = Some(format!("{e:#}")),
            }
        }
    } else {
        app.mocopi = None;
    }
    use crate::mapping::input;
    let face = [app.inputs[input::FACE_ANGLE_X], app.inputs[input::FACE_ANGLE_Y], app.inputs[input::FACE_ANGLE_Z]];
    crate::mocopi::update(app.mocopi.as_mut(), face, app.settings.track.mirror, dt);

    // Discord
    if app.settings.discord && !app.settings.discord_app_id.trim().is_empty() {
        let p = app.discord.get_or_insert_with(|| crate::discord::Presence::start(&app.settings.discord_app_id));
        let details = match (&app.avatar, app.settings.discord_show_model) {
            (Some(a), true) => format!("{} {}", tr("Модель:"), a.name),
            _ => "Live2D VTuber".to_string(),
        };
        let state = match &app.net {
            Some(n) => format!("{} ({}/{})", tr("В коллабе"), n.participant_count(), crate::net::MAX_PARTICIPANTS),
            None if app.tracking => tr("Трекинг включён").to_string(),
            None => tr("В rigrs").to_string(),
        };
        p.update(&details, &state, app.started_unix);
    } else {
        app.discord = None;
    }

    // Phone discovery
    let done = app.phone_scan.as_ref().and_then(|(usb, s)| s.lock().unwrap().take().map(|r| (*usb, r)));
    if let Some((usb, found)) = done {
        app.phone_scan = None;
        app.phone_found = found.clone();
        let pick = if usb { found.iter().find(|f| f.1).or(found.first()) } else if found.len() == 1 { found.first() } else { None };
        match pick {
            Some((ip, _)) => start_phone(app, ip.to_string()),
            None if found.is_empty() => app.toast(if usb {
                "USB: телефон не найден — включите режим модема и «3rd Party PC Clients» в VTube Studio"
            } else {
                "Телефон не найден — включите «3rd Party PC Clients» в VTube Studio или введите IP вручную"
            }),
            None => app.toast("Найдено несколько телефонов — выберите нужный в списке"),
        }
    }
}
