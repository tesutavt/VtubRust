//! VTube Studio look for the settings screens (after the screenshots in the VTS wiki):
//! flat blue buttons with almost square corners, light blue-grey input fields, pill
//! sliders with a blue value knob and "Min / Max" captions, blue radio dots, and the
//! "blue selector button + ↳ value" pattern.

use egui::{Align2, Color32, FontId, Pos2, Rect, Response, RichText, Sense, Stroke, Vec2};

use crate::i18n::tr;

pub const BLUE: Color32 = Color32::from_rgb(10, 145, 240);
pub const KNOB: Color32 = Color32::from_rgb(66, 142, 236);
pub const FIELD: Color32 = Color32::from_rgb(238, 243, 250);
pub const FIELD_EDGE: Color32 = Color32::from_rgb(220, 228, 240);
pub const TRACK: Color32 = Color32::from_rgb(236, 242, 250);
pub const TEXT: Color32 = Color32::from_rgb(60, 60, 66);
pub const MUTED: Color32 = Color32::from_rgb(150, 150, 160);
pub const RADIO_OFF: Color32 = Color32::from_rgb(214, 220, 238);
pub const PINK: Color32 = Color32::from_rgb(218, 96, 120);

/// Style for everything inside a settings card: fields and plain widgets look like VTS
/// input boxes; selection is VTS blue.
pub fn apply(style: &mut egui::Style) {
    let v = &mut style.visuals;
    v.extreme_bg_color = FIELD;
    v.selection.bg_fill = BLUE;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = BLUE;
    for (w, fill) in [(&mut v.widgets.inactive, FIELD), (&mut v.widgets.hovered, Color32::from_rgb(228, 236, 248)), (&mut v.widgets.active, Color32::from_rgb(214, 226, 244))] {
        w.weak_bg_fill = fill;
        w.bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, FIELD_EDGE);
        w.rounding = 4.0.into();
        w.fg_stroke = Stroke::new(1.0, TEXT);
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.open = v.widgets.active;
    style.spacing.button_padding = Vec2::new(10.0, 4.0);
    style.spacing.interact_size.y = 26.0;
}

fn flat(ui: &mut egui::Ui, rect: Rect, resp: &Response, fill: Color32, text: &str, size: f32) {
    let p = ui.painter();
    let fill = if !ui.is_enabled() {
        crate::ui::lighten(fill, 0.55)
    } else if resp.is_pointer_button_down_on() {
        crate::ui::mix(fill, Color32::BLACK, 0.12)
    } else if resp.hovered() {
        crate::ui::lighten(fill, 0.12)
    } else {
        fill
    };
    p.rect_filled(rect.translate(Vec2::new(0.0, 2.0)), 3.0, Color32::from_black_alpha(22));
    p.rect_filled(rect, 3.0, fill);
    p.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(size), Color32::WHITE);
}

/// Flat VTS button sized to its text.
pub fn button(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> Response {
    let text = text.into().text().to_string();
    let galley = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(15.0), Color32::WHITE);
    let size = Vec2::new(galley.size().x + 28.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    flat(ui, rect, &resp, BLUE, &text, 15.0);
    resp
}

/// Flat button of a given width (`w <= 0` = full width).
pub fn wide(ui: &mut egui::Ui, text: &str, fill: Color32, w: f32, h: f32) -> Response {
    let w = if w <= 0.0 { ui.available_width() } else { w };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    flat(ui, rect, &resp, fill, text, (h * 0.5).clamp(14.0, 19.0));
    resp
}

/// The "↳" arrow VTS draws under selector buttons.
pub fn hook_arrow(p: &egui::Painter, at: Pos2, s: f32, col: Color32) {
    let st = Stroke::new(s * 0.16, col);
    let corner = at + Vec2::new(0.0, s * 0.45);
    p.line_segment([at - Vec2::new(0.0, s * 0.35), corner], st);
    let tip = corner + Vec2::new(s * 0.9, 0.0);
    p.line_segment([corner, tip], st);
    p.add(egui::Shape::convex_polygon(vec![tip + Vec2::new(s * 0.3, 0.0), tip + Vec2::new(-s * 0.05, -s * 0.28), tip + Vec2::new(-s * 0.05, s * 0.28)], col, Stroke::NONE));
}

/// Blue selector button followed by "↳ value" lines (VTS: Icon, Idle Animation, Key 1…).
pub fn picker(ui: &mut egui::Ui, label: &str, values: &[String], enabled: bool) -> Response {
    let w = (ui.available_width() * 0.6).clamp(200.0, 360.0);
    let r = ui.add_enabled_ui(enabled, |ui| wide(ui, tr(label), BLUE, w, 32.0)).inner;
    for (i, v) in values.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let (ar, _) = ui.allocate_exact_size(Vec2::new(30.0, 26.0), Sense::hover());
            if i == 0 {
                hook_arrow(ui.painter(), ar.left_center() + Vec2::new(6.0, -4.0), 16.0, Color32::from_rgb(70, 70, 76));
            }
            ui.label(RichText::new(tr(v)).size(19.0).color(if enabled { TEXT } else { MUTED }));
        });
    }
    r
}

fn fmt(v: f32, span: f32) -> String {
    if span <= 3.0 {
        format!("{v:.2}")
    } else if span <= 30.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    }
}

/// VTS pill slider for any float range: track with min/max captions, blue knob showing
/// the value. `caption` (if not empty) is written on the knob instead and the value
/// is shown above it (VTS movement sliders: "X", "Y", "Z", "Opacity").
pub fn slider_ex(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, min_label: &str, max_label: &str, caption: &str, suffix: &str) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    let span = (hi - lo).abs().max(1e-6);
    let w = ui.available_width().max(120.0);
    let top = if caption.is_empty() { 0.0 } else { 16.0 };
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, 30.0 + top), Sense::click_and_drag());
    let track = Rect::from_min_max(rect.min + Vec2::new(0.0, top), rect.max);
    let knob_w = (track.width() * 0.22).clamp(64.0, 110.0);
    if let Some(p) = resp.interact_pointer_pos() {
        if ui.is_enabled() {
            let t = ((p.x - track.left() - knob_w / 2.0) / (track.width() - knob_w)).clamp(0.0, 1.0);
            let mut nv = lo + t * (hi - lo);
            let step = if span <= 3.0 { 0.01 } else if span <= 30.0 { 0.1 } else { 1.0 };
            nv = (nv / step).round() * step;
            if (nv - *v).abs() > f32::EPSILON {
                *v = nv;
                resp.mark_changed();
            }
        }
    }
    let t = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
    let p = ui.painter();
    p.rect_filled(track.translate(Vec2::new(0.0, 2.0)), track.height() / 2.0, Color32::from_black_alpha(14));
    p.rect_filled(track, track.height() / 2.0, TRACK);
    let cap = |s: &str, default: String| if s.is_empty() { default } else { tr(s).to_string() };
    p.text(track.left_center() + Vec2::new(14.0, 0.0), Align2::LEFT_CENTER, cap(min_label, fmt(lo, span)), FontId::proportional(13.0), TEXT);
    p.text(track.right_center() - Vec2::new(14.0, 0.0), Align2::RIGHT_CENTER, cap(max_label, fmt(hi, span)), FontId::proportional(13.0), TEXT);
    let kx = track.left() + t * (track.width() - knob_w);
    let knob = Rect::from_min_size(Pos2::new(kx, track.top()), Vec2::new(knob_w, track.height()));
    p.rect_filled(knob.translate(Vec2::new(0.0, 2.0)), track.height() / 2.0, Color32::from_black_alpha(30));
    p.rect_filled(knob, track.height() / 2.0, if ui.is_enabled() { KNOB } else { crate::ui::lighten(KNOB, 0.5) });
    let value = format!("{}{}", fmt(*v, span), suffix);
    if caption.is_empty() {
        p.text(knob.center(), Align2::CENTER_CENTER, value, FontId::proportional(17.0), Color32::WHITE);
    } else {
        p.text(knob.center(), Align2::CENTER_CENTER, tr(caption), FontId::proportional(17.0), Color32::WHITE);
        p.text(Pos2::new(knob.center().x, rect.top() + 7.0), Align2::CENTER_CENTER, value, FontId::proportional(14.0), MUTED);
    }
    resp
}

pub fn slider(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> Response {
    slider_ex(ui, v, range, "", "", "", "")
}

/// VTS radio option: pale dot when off, blue dot with a white ring when on.
pub fn radio<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, value: T, label: &str) -> Response {
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        let on = *current == value;
        let (r, mut resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
        let l = ui.add(egui::Label::new(RichText::new(tr(label)).size(18.0).color(if on { TEXT } else { MUTED })).sense(Sense::click()));
        let p = ui.painter();
        p.circle_filled(r.center(), 11.0, RADIO_OFF);
        if on {
            p.circle_filled(r.center(), 9.5, Color32::WHITE);
            p.circle_filled(r.center(), 7.5, KNOB);
        }
        if (resp.clicked() || l.clicked()) && !on {
            *current = value;
            resp.mark_changed();
        }
        resp
    })
    .inner
}

/// Grey label on the left, VTS input field on the right (IP / Port rows).
pub fn field_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.add_sized([110.0, 26.0], egui::Label::new(RichText::new(tr(label)).size(16.0).color(TEXT)));
        add(ui);
    });
}

/// "Status: ●" with the text on the next line, as in the VTS connection cards.
pub fn status(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(RichText::new(tr("Статус:")).size(16.0).strong().color(TEXT));
        ui.add_space(10.0);
        let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 7.0, color);
        ui.painter().circle_stroke(r.center(), 7.0, Stroke::new(1.0, Color32::from_black_alpha(60)));
    });
    ui.horizontal_wrapped(|ui| {
        ui.add_space(28.0);
        ui.label(RichText::new(text).size(16.0).color(TEXT));
    });
}

/// Small dark "?" help circle with a tooltip.
pub fn help(ui: &mut egui::Ui, tip: &str) {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 10.0, Color32::from_rgb(64, 64, 70));
    ui.painter().text(r.center(), Align2::CENTER_CENTER, "?", FontId::proportional(14.0), Color32::WHITE);
    resp.on_hover_text(tr(tip));
}

/// Grey explanation text.
pub fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(tr(text)).size(13.5).color(MUTED));
}
