//! VTS-layout cards and dialogs: Sound Effects (dB meters, General Sound Settings, parameter
//! sound triggers, sound test, stop), Controller Input (Controller Config floating widget),
//! Model tab "Customize model" menu with the ArtMesh lists, and Model Volume.

use egui::{Align2, Color32, Context, FontId, Pos2, RichText, Sense, Stroke, Vec2};

use crate::app::App;
use crate::artmesh::{ArtMeshDetails, MeshGroup};
use crate::i18n::tr;
use crate::ui::{section, toggle, TEXT};
use crate::ui_hotkeys::{dim, group_title, pink_row};

const BLUE_BTN: Color32 = crate::vts_style::BLUE;
const RED_BTN: Color32 = Color32::from_rgb(222, 88, 116);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dlg {
    SoundSettings,
    SoundTriggers,
    Customize,
    AutoSetup,
    AutoConfirm,
    CopyFrom,
    /// ArtMesh list: 0 = pin exclusion, 1 = delete items, 2 = lighting, 3 = colors, 4 = groups.
    Meshes(u8),
}

#[derive(Default)]
pub struct VtsState {
    pub dlg: Option<Dlg>,
    pub pad_widget: bool,
    pub draft: Option<ArtMeshDetails>,
    pub filter: String,
    pub eye: Option<String>,
    pub group: usize,
    pub group_name: String,
}

fn wide_button(ui: &mut egui::Ui, icon: &str, text: &str, w: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, if resp.hovered() { crate::ui::lighten(BLUE_BTN, 0.1) } else { BLUE_BTN });
    p.text(rect.left_center() + Vec2::new(18.0, 0.0), Align2::CENTER_CENTER, icon, FontId::proportional(20.0), Color32::WHITE);
    p.text(rect.center(), Align2::CENTER_CENTER, tr(text), FontId::proportional(18.0), Color32::WHITE);
    resp.clicked()
}

/// dB meter (-60…0, green / yellow / red) like the VTS sound card.
fn db_meter(ui: &mut egui::Ui, level: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 38.0), Sense::hover());
    let p = ui.painter();
    let bar = egui::Rect::from_min_size(rect.min, Vec2::new(w, 16.0));
    let x_of = |db: f32| bar.left() + (db + 60.0) / 60.0 * bar.width();
    let db = 20.0 * level.max(1e-5).log10();
    for (a, b, c) in [(-60.0, -20.0, Color32::from_rgb(28, 110, 30)), (-20.0, -9.0, Color32::from_rgb(140, 110, 20)), (-9.0, 0.0, Color32::from_rgb(130, 20, 30))] {
        let r = egui::Rect::from_min_max(Pos2::new(x_of(a), bar.top()), Pos2::new(x_of(b), bar.bottom()));
        p.rect_filled(r, 0.0, c.gamma_multiply(0.35));
        let lit = egui::Rect::from_min_max(r.min, Pos2::new(x_of(db.clamp(a, b)), r.bottom()));
        if db > a {
            p.rect_filled(lit, 0.0, c);
        }
    }
    for k in 0..=12 {
        let v = -60.0 + k as f32 * 5.0;
        let x = x_of(v);
        p.line_segment([Pos2::new(x, bar.bottom()), Pos2::new(x, bar.bottom() + 4.0)], Stroke::new(1.0f32, TEXT));
        p.text(Pos2::new(x, bar.bottom() + 12.0), Align2::CENTER_CENTER, format!("{v:.0}"), FontId::proportional(12.0), TEXT);
    }
}

fn vol_slider(ui: &mut egui::Ui, v: &mut f32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("🔊").size(18.0));
        crate::vts_style::slider_ex(ui, v, 0.0..=100.0, "0%", "100%", "", "%");
    });
}

/// General tab "Sound Effects".
pub fn sound_card(app: &mut App, ui: &mut egui::Ui) {
    let (pk, pk_obs) = app.audio.as_ref().map(|a| (a.peak(), a.peak_obs())).unwrap_or((0.0, 0.0));
    let mut open = None;
    let mut test = false;
    let mut stop = false;
    section(ui, "Звуковые эффекты", |ui| {
        let s = &mut app.settings.sound;
        let db = |v: f32| format!("{:.1} dB", (20.0 * v.max(1e-5).log10()).max(-60.0));
        ui.horizontal(|ui| {
            toggle(ui, &mut s.play_local, tr("Воспроизводить в приложении"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(db(pk)).size(17.0)));
        });
        db_meter(ui, pk);
        vol_slider(ui, &mut s.volume);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            toggle(ui, &mut s.vban, tr("Отправлять в OBS"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(db(pk_obs)).size(17.0)));
        });
        db_meter(ui, pk_obs);
        vol_slider(ui, &mut s.volume_obs);
        let w = ui.available_width();
        if wide_button(ui, "⚙", "Настройки", w) {
            open = Some(Dlg::SoundSettings);
        }
        ui.add_space(4.0);
        if wide_button(ui, "🎵", "Звуковые триггеры параметров", w) {
            open = Some(Dlg::SoundTriggers);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let half = (w - 8.0) / 2.0;
            if wide_button(ui, "♪", "Тест звука", half) {
                test = true;
            }
            if wide_button(ui, "🔇", "Остановить звуки", half) {
                stop = true;
            }
        });
        if let Some(e) = app.audio.as_ref().and_then(|a| a.error()) {
            ui.colored_label(Color32::from_rgb(220, 80, 80), e);
        }
    });
    if open.is_some() {
        app.ui_vts.dlg = open;
    }
    if test {
        sound_test(app);
    }
    if stop {
        if let Some(a) = &app.audio {
            a.stop_all();
        }
    }
}

/// Plays a short test chime (generated once into the Sounds folder).
fn sound_test(app: &mut App) {
    let p = app.paths.data.join("Sounds").join("_rigrs_test.wav");
    if !p.exists() {
        let rate = 48000u32;
        let n = (rate as f32 * 0.6) as usize;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&((36 + n * 2) as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&((n * 2) as u32).to_le_bytes());
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let f = if t < 0.2 { 880.0 } else if t < 0.4 { 1108.7 } else { 1318.5 };
            let env = (1.0 - (t % 0.2) / 0.2).powi(2);
            w.extend_from_slice(&(((t * f * std::f32::consts::TAU).sin() * env * 9000.0) as i16).to_le_bytes());
        }
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(&p, w);
    }
    app.play_sound(crate::audio::Play { file: p, volume: 1.0, pitch: 1.0, pan: 0.0, delay: 0.0, echo_count: 0, echo_delay: 0.2, echo_volume: 0.5, group: "test".into() });
}

fn frame_dialog(ctx: &Context, id: &str, title: &str, header: &str, add: impl FnOnce(&mut egui::Ui)) -> (bool, bool) {
    dim(ctx, &format!("{id}-dim"));
    let (mut ok, mut cancel) = (false, false);
    let h = ctx.screen_rect().height() - 220.0;
    egui::Area::new(egui::Id::new(id)).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, -20.0]).show(ctx, |ui| {
        let resp = egui::Frame::none().fill(Color32::WHITE).rounding(4.0).inner_margin(egui::Margin { left: 14.0, right: 14.0, top: 40.0, bottom: 14.0 }).show(ui, |ui| {
            ui.set_width(600.0);
            ui.style_mut().visuals.override_text_color = Some(TEXT);
            if !title.is_empty() {
                ui.label(RichText::new(tr(title)).size(19.0));
            }
            if !header.is_empty() {
                ui.add(egui::Label::new(RichText::new(tr(header)).size(17.0)).wrap());
            }
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(h).show(ui, add);
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 44.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).strong().color(Color32::WHITE)).fill(BLUE_BTN)).clicked() {
                    ok = true;
                }
                ui.add_space(12.0);
                if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new(tr("Отмена")).size(17.0).strong().color(Color32::WHITE)).fill(RED_BTN)).clicked() {
                    cancel = true;
                }
            });
        });
        let c = Pos2::new(resp.response.rect.center().x, resp.response.rect.top());
        let p = ui.painter();
        p.circle_filled(c, 36.0, Color32::WHITE);
        p.circle_filled(c, 31.0, Color32::from_rgb(166, 214, 240));
        p.text(c, Align2::CENTER_CENTER, "🐱", FontId::proportional(30.0), Color32::WHITE);
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    (ok, cancel)
}

/// All VTS-style dialogs of this module.
pub fn dialogs(app: &mut App, ctx: &Context) {
    controller_widget(app, ctx);
    update_banner(app, ctx);
    let Some(d) = app.ui_vts.dlg else { return };
    match d {
        Dlg::SoundSettings => {
            let (ok, cancel) = frame_dialog(ctx, "snd-settings", "Общие настройки звука", "", |ui| {
                group_title(ui, "Настройки VBAN");
                let s = &mut app.settings.sound;
                ui.horizontal(|ui| {
                    ui.label(tr("Порт VBAN (OBS)"));
                    let mut p = s.vban_port as f32;
                    if ui.add(egui::DragValue::new(&mut p).range(1.0..=65535.0)).changed() {
                        s.vban_port = p as u16;
                    }
                    if crate::vts_style::button(ui, "↺").clicked() {
                        s.vban_port = 6980;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("IP");
                    ui.add(egui::TextEdit::singleline(&mut s.vban_ip).desired_width(140.0));
                    ui.label(tr("Имя потока"));
                    ui.add(egui::TextEdit::singleline(&mut s.vban_stream).desired_width(120.0));
                });
                ui.label(RichText::new(tr("В OBS: плагин VBAN Audio → источник «VBAN Audio Source», порт 6980.")).size(13.0).weak());
                group_title(ui, "Звуки в мультиплеере");
                ui.label(RichText::new(tr("Появится вместе с мультиплеером rigrs.")).weak());
            });
            if ok || cancel {
                app.ui_vts.dlg = None;
                app.save();
            }
        }
        Dlg::SoundTriggers => {
            let Some(key) = app.model_key() else {
                app.ui_vts.dlg = None;
                app.toast("Сначала загрузите модель");
                return;
            };
            let params = app.avatar.as_ref().map(|a| a.model.param_ids.clone()).unwrap_or_default();
            let dir = app.avatar.as_ref().map(|a| a.dir.clone()).unwrap_or_default();
            let (ok, cancel) = frame_dialog(ctx, "snd-triggers", "Звуковые триггеры параметров", "", |ui| {
                crate::ui_hotkeys::model_sound_for(app, ui, "", key, params, dir);
            });
            if ok || cancel {
                app.ui_vts.dlg = None;
                app.save();
            }
        }
        Dlg::Customize => customize_menu(app, ctx),
        Dlg::AutoSetup => autosetup_menu(app, ctx),
        Dlg::AutoConfirm => autosetup_confirm(app, ctx),
        Dlg::CopyFrom => {
            let (ok, cancel) = frame_dialog(ctx, "copy-from-dlg", "", "", |ui| {
                crate::ui_model_extra::copy_card(app, ui);
            });
            if ok || cancel {
                app.ui_vts.dlg = None;
            }
        }
        Dlg::Meshes(kind) => mesh_list(app, ctx, kind),
    }
}

// ------------------------------------------------------------------ Model: autosetup

fn alert_frame(ctx: &Context, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    dim(ctx, &format!("{id}-dim"));
    egui::Area::new(egui::Id::new(id)).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, -20.0]).show(ctx, |ui| {
        let w = (ctx.screen_rect().width() - 120.0).clamp(500.0, 1200.0);
        let resp = egui::Frame::none().fill(Color32::WHITE).rounding(4.0).inner_margin(egui::Margin { left: 22.0, right: 22.0, top: 48.0, bottom: 18.0 }).show(ui, |ui| {
            ui.set_width(w);
            add(ui);
        });
        let c = Pos2::new(resp.response.rect.center().x, resp.response.rect.top());
        let p = ui.painter();
        p.circle_filled(c, 38.0, Color32::WHITE);
        p.circle_filled(c, 33.0, Color32::from_rgb(166, 214, 240));
        p.text(c, Align2::CENTER_CENTER, "!", FontId::proportional(48.0), Color32::WHITE);
    });
}

fn rich_line(ui: &mut egui::Ui, parts: &[(&str, bool)]) {
    let mut job = egui::text::LayoutJob::default();
    for (t, hi) in parts {
        let fmt = egui::TextFormat {
            font_id: FontId::proportional(21.0),
            color: if *hi { Color32::from_rgb(236, 30, 160) } else { Color32::from_rgb(110, 110, 118) },
            ..Default::default()
        };
        job.append(t, 0.0, fmt);
    }
    job.wrap.max_width = ui.available_width();
    ui.label(job);
}

fn big_button(ui: &mut egui::Ui, text: &str, fill: Color32) -> bool {
    ui.add_sized([ui.available_width(), 50.0], egui::Button::new(RichText::new(tr(text)).size(18.0).strong().color(Color32::WHITE)).fill(fill).rounding(2.0)).clicked()
}

fn autosetup_menu(app: &mut App, ctx: &Context) {
    let name = app.avatar.as_ref().map(|a| a.name.clone()).unwrap_or_default();
    let mut next = None;
    let mut close = false;
    alert_frame(ctx, "autosetup", |ui| {
        rich_line(ui, &[(tr("Хотите запустить "), false), (tr("автонастройку"), true), (&format!(" {} {name}?", tr("модели")), false)]);
        ui.add_space(50.0);
        if big_button(ui, "Автонастройка", BLUE_BTN) {
            next = Some(Dlg::AutoConfirm);
        }
        ui.add_space(14.0);
        if big_button(ui, "Копировать настройки из другой модели", BLUE_BTN) {
            next = Some(Dlg::CopyFrom);
        }
        ui.add_space(14.0);
        if big_button(ui, "Отмена", RED_BTN) {
            close = true;
        }
    });
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui_vts.dlg = None;
    } else if next.is_some() {
        app.ui_vts.dlg = next;
    }
}

fn autosetup_confirm(app: &mut App, ctx: &Context) {
    let Some(key) = app.model_key() else {
        app.ui_vts.dlg = None;
        return;
    };
    let (old, new) = app.avatar.as_ref().map(|a| (a.mapping.rules.len(), crate::mapping::default_rules(&a.model).len())).unwrap_or((0, 0));
    let (mut ok, mut cancel) = (false, false);
    alert_frame(ctx, "autosetup-ok", |ui| {
        ui.set_width(860.0);
        rich_line(ui, &[
            (tr("Будет удалено "), false),
            (&old.to_string(), true),
            (tr(" существующих параметров и создано "), false),
            (&new.to_string(), true),
            (tr(" новых параметров на основе стандартных имён и значений параметров Live2D."), false),
        ]);
        ui.add_space(70.0);
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 44.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_sized([170.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).strong().color(Color32::WHITE)).fill(BLUE_BTN)).clicked() {
                ok = true;
            }
            ui.add_space(20.0);
            if ui.add_sized([170.0, 40.0], egui::Button::new(RichText::new(tr("Отмена")).size(17.0).strong().color(Color32::WHITE)).fill(RED_BTN)).clicked() {
                cancel = true;
            }
        });
    });
    if ok {
        if let Some(av) = app.avatar.as_mut() {
            av.mapping.rules = crate::mapping::default_rules(&av.model);
            av.mapping.recompile(&av.model);
            app.settings.rules_override.insert(key, av.mapping.rules.clone());
        }
        app.ui.collapsed.clear();
        app.ui_vts.dlg = None;
        app.save();
        app.toast("Автонастройка: маппинг собран по стандартным параметрам модели");
    } else if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui_vts.dlg = None;
    }
}

// ------------------------------------------------------------------ Model: customize menu

fn customize_menu(app: &mut App, ctx: &Context) {
    dim(ctx, "cust-dim");
    let rows: [(&str, &str); 6] = [
        ("🕸", "Настроить группы слоёв"),
        ("🎵", "Настроить звуковые триггеры параметров"),
        ("📌", "Отключить прикрепление к сеткам ArtMesh"),
        ("🗑", "Выбрать сетки ArtMesh для удаления элементов"),
        ("💡", "Настроить освещение для сеток ArtMesh"),
        ("🖌", "Настроить цвета умножения и осветления"),
    ];
    let mut pick = None;
    let mut cancel = false;
    egui::Area::new(egui::Id::new("cust-menu")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, -20.0]).show(ctx, |ui| {
        let resp = egui::Frame::none().fill(Color32::WHITE).rounding(6.0).inner_margin(egui::Margin { left: 26.0, right: 26.0, top: 46.0, bottom: 18.0 }).show(ui, |ui| {
            ui.set_width(780.0);
            ui.label(RichText::new(tr("Что вы хотите сделать?")).size(21.0).color(Color32::from_rgb(110, 110, 118)));
            ui.add_space(30.0);
            for (k, (icon, text)) in rows.iter().enumerate() {
                let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::click());
                let p = ui.painter();
                p.rect_filled(rect.translate(Vec2::new(1.5, 2.5)), 8.0, Color32::from_black_alpha(60));
                p.rect_filled(rect, 8.0, if r.hovered() { Color32::from_rgb(222, 222, 234) } else { Color32::from_rgb(232, 230, 240) });
                p.text(rect.left_center() + Vec2::new(26.0, 0.0), Align2::CENTER_CENTER, *icon, FontId::proportional(26.0), Color32::from_rgb(236, 90, 110));
                p.text(rect.left_center() + Vec2::new(56.0, 0.0), Align2::LEFT_CENTER, tr(text), FontId::proportional(20.0), Color32::from_rgb(60, 60, 66));
                if r.clicked() {
                    pick = Some(k);
                }
                ui.add_space(10.0);
            }
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 44.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([170.0, 40.0], egui::Button::new(RichText::new(tr("Отмена")).size(17.0).strong().color(Color32::WHITE)).fill(RED_BTN)).clicked() {
                    cancel = true;
                }
            });
        });
        let c = Pos2::new(resp.response.rect.center().x, resp.response.rect.top());
        let p = ui.painter();
        p.circle_filled(c, 36.0, Color32::WHITE);
        p.circle_filled(c, 31.0, Color32::from_rgb(166, 214, 240));
        p.text(c, Align2::CENTER_CENTER, "!", FontId::proportional(46.0), Color32::WHITE);
    });
    if let Some(k) = pick {
        app.ui_vts.draft = app.artmesh_details().or(Some(ArtMeshDetails::default()));
        app.ui_vts.filter.clear();
        app.ui_vts.dlg = Some(match k {
            0 => Dlg::Meshes(4),
            1 => Dlg::SoundTriggers,
            2 => Dlg::Meshes(0),
            3 => Dlg::Meshes(1),
            4 => Dlg::Meshes(2),
            _ => Dlg::Meshes(3),
        });
    }
    if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui_vts.dlg = None;
    }
}

fn mesh_list(app: &mut App, ctx: &Context, kind: u8) {
    let Some(key) = app.model_key() else {
        app.ui_vts.dlg = None;
        return;
    };
    let ids: Vec<String> = app.avatar.as_ref().map(|a| a.model.drawables.iter().map(|d| d.id.clone()).collect()).unwrap_or_default();
    let header = match kind {
        0 => "Элементы не будут прикрепляться к включённым сеткам ArtMesh из этого списка. Выключено = прикрепление разрешено.",
        1 => "Элементы будут удаляться при их перетаскивании на включённые сетки ArtMesh из этого списка.",
        2 => "Насколько наложение освещения влияет на каждую сетку ArtMesh (1 = полностью).",
        3 => "Цвета умножения (с прозрачностью: 0 скрывает часть) и осветления для сеток ArtMesh. Перекрашивайте модель только с разрешения художника.",
        _ => "Группы слоёв: выберите группу и отметьте сетки ArtMesh, которые в неё входят.",
    };
    let mut d = app.ui_vts.draft.take().unwrap_or_default();
    let st = &mut app.ui_vts;
    let mut eye = st.eye.clone();
    let (ok, cancel) = frame_dialog(ctx, "mesh-list", "", header, |ui| {
        if kind == 4 {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("grp").selected_text(d.model_art_mesh_groups.get(st.group).map(|g| g.name.clone()).unwrap_or_else(|| tr("(нет групп)").into())).show_ui(ui, |ui| {
                    for (k, g) in d.model_art_mesh_groups.iter().enumerate() {
                        ui.selectable_value(&mut st.group, k, &g.name);
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut st.group_name).hint_text(tr("Новая группа")).desired_width(160.0));
                if crate::vts_style::button(ui, tr("Создать")).clicked() && !st.group_name.trim().is_empty() {
                    d.model_art_mesh_groups.push(MeshGroup { name: st.group_name.trim().into(), ..Default::default() });
                    st.group = d.model_art_mesh_groups.len() - 1;
                    st.group_name.clear();
                }
                if !d.model_art_mesh_groups.is_empty() && crate::vts_style::button(ui, tr("Удалить группу")).clicked() {
                    d.model_art_mesh_groups.remove(st.group.min(d.model_art_mesh_groups.len() - 1));
                    st.group = 0;
                }
            });
        }
        let f = st.filter.to_lowercase();
        if kind <= 1 || (kind == 4 && !d.model_art_mesh_groups.is_empty()) {
            // Bulk switches for the meshes matching the filter.
            let mut set_all = None;
            ui.horizontal(|ui| {
                if crate::vts_style::button(ui, tr("Включить все")).clicked() {
                    set_all = Some(true);
                }
                if crate::vts_style::button(ui, tr("Выключить все")).clicked() {
                    set_all = Some(false);
                }
            });
            if let Some(on) = set_all {
                for id in ids.iter().filter(|i| f.is_empty() || i.to_lowercase().contains(&f)) {
                    match kind {
                        0 => ArtMeshDetails::set_list(&mut d.art_meshes_excluded_from_pinning, id, on),
                        1 => ArtMeshDetails::set_list(&mut d.art_meshes_that_delete_items_on_drop, id, on),
                        _ => {
                            if let Some(g) = d.model_art_mesh_groups.get_mut(st.group) {
                                ArtMeshDetails::set_list(&mut g.art_meshes, id, on);
                            }
                        }
                    }
                }
            }
        }
        let row_w = ui.available_width() - 16.0;
        for id in ids.iter().filter(|i| f.is_empty() || i.to_lowercase().contains(&f)).take(600) {
            ui.horizontal(|ui| {
                ui.set_width(row_w);
                match kind {
                    0 | 1 | 4 => {
                        let mut on = match kind {
                            0 => d.excluded(id),
                            1 => d.deletes(id),
                            _ => d.model_art_mesh_groups.get(st.group).map(|g| g.art_meshes.contains(id)).unwrap_or(false),
                        };
                        ui.allocate_ui(Vec2::new(row_w - 40.0, 40.0), |ui| {
                            if pink_row(ui, &mut on, id, "") {
                                match kind {
                                    0 => ArtMeshDetails::set_list(&mut d.art_meshes_excluded_from_pinning, id, on),
                                    1 => ArtMeshDetails::set_list(&mut d.art_meshes_that_delete_items_on_drop, id, on),
                                    _ => {
                                        if let Some(g) = d.model_art_mesh_groups.get_mut(st.group) {
                                            ArtMeshDetails::set_list(&mut g.art_meshes, id, on);
                                        }
                                    }
                                }
                            }
                        });
                    }
                    2 => {
                        ui.add_sized([280.0, 24.0], egui::Label::new(RichText::new(id).size(16.0)));
                        let mut v = d.light_mult(id);
                        if crate::vts_style::slider(ui, &mut v, 0.0..=2.0).changed() {
                            d.set_light(id, v);
                        }
                    }
                    _ => {
                        ui.add_sized([280.0, 24.0], egui::Label::new(RichText::new(id).size(16.0)));
                        let (m, s) = d.colors(id).unwrap_or(([1.0; 4], [0.0, 0.0, 0.0, 1.0]));
                        let mut mc = egui::Rgba::from_rgba_unmultiplied(m[0], m[1], m[2], m[3]);
                        let mut sc = egui::Rgba::from_rgba_unmultiplied(s[0], s[1], s[2], 1.0);
                        let a = egui::widgets::color_picker::color_edit_button_rgba(ui, &mut mc, egui::widgets::color_picker::Alpha::OnlyBlend).changed();
                        let b = egui::widgets::color_picker::color_edit_button_rgba(ui, &mut sc, egui::widgets::color_picker::Alpha::Opaque).changed();
                        if a || b {
                            let v = sc.to_rgba_unmultiplied();
                            d.set_colors(id, mc.to_rgba_unmultiplied(), [v[0], v[1], v[2], 1.0]);
                        }
                    }
                }
                let lit = eye.as_deref() == Some(id.as_str());
                if ui.add(egui::Button::new(RichText::new("👁").size(18.0).color(if lit { Color32::WHITE } else { Color32::from_rgb(236, 120, 140) })).fill(if lit { Color32::from_rgb(236, 120, 140) } else { Color32::TRANSPARENT })).on_hover_text(tr("Подсветить эту часть на модели")).clicked() {
                    eye = if lit { None } else { Some(id.clone()) };
                }
            });
        }
    });
    st.eye = eye.clone();
    // Filter field under the dialog, like the other VTS lists.
    egui::Area::new(egui::Id::new("mesh-filter")).order(egui::Order::Tooltip).anchor(Align2::CENTER_BOTTOM, [0.0, -10.0]).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).rounding(10.0).inner_margin(8.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut app.ui_vts.filter).hint_text(tr("Фильтр")).font(FontId::proportional(20.0)).desired_width(380.0));
                if ui.add(egui::Button::new(RichText::new("✖").size(20.0).color(Color32::from_rgb(240, 120, 130))).frame(false)).clicked() {
                    app.ui_vts.filter.clear();
                }
                ui.label(RichText::new("🔍").size(24.0));
            });
        });
    });
    // Live preview of the edited settings.
    if let Some(a) = app.avatar.as_mut() {
        a.apply_artmesh(&d);
        a.highlight = eye.as_ref().and_then(|e| a.drawable_index(e));
    }
    if ok {
        app.settings.model_artmesh.insert(key, d);
        app.ui_vts.dlg = None;
        app.ui_vts.eye = None;
        if let Some(a) = app.avatar.as_mut() {
            a.highlight = None;
        }
        app.apply_artmesh();
        app.save();
    } else if cancel {
        app.ui_vts.dlg = None;
        app.ui_vts.eye = None;
        if let Some(a) = app.avatar.as_mut() {
            a.highlight = None;
        }
        app.apply_artmesh();
    } else {
        app.ui_vts.draft = Some(d);
    }
}

/// Model tab: model volume (VTS "Model Volume").
pub fn model_volume(sounds: &mut std::collections::HashMap<String, crate::sfx::ModelSound>, key: &str, ui: &mut egui::Ui) {
    section(ui, "Громкость модели", |ui| {
        let ms = sounds.entry(key.to_string()).or_default();
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = ui.available_width() - 120.0;
            crate::vts_style::slider_ex(ui, &mut ms.volume, 0.0..=100.0, "0%", "100%", "", "%");
            let mut on = !ms.muted;
            if ui.add(egui::Button::new(RichText::new(if on { "🔊" } else { "🔇" }).size(20.0))).on_hover_text(tr("Вкл./выкл. звук модели")).clicked() {
                on = !on;
                ms.muted = !on;
            }
        });
        ui.horizontal(|ui| {
            crate::ui::toggle(ui, &mut ms.volume_by_size, tr("Громкость по размеру"));
            crate::ui::toggle(ui, &mut ms.pan_by_x, tr("Панорама по X"));
        });
    });
}

// ------------------------------------------------------------------ Controller Input

pub fn controller_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Ввод с контроллера", |ui| {
        let w = ui.available_width();
        if wide_button(ui, "🎮", "Настройка контроллера", w) {
            app.ui_vts.pad_widget = !app.ui_vts.pad_widget;
        }
        let cur = match app.settings.controller_mode {
            0 => tr("Без контроллера").to_string(),
            1 => tr("Любой контроллер").to_string(),
            _ => app.pads.devices.iter().find(|d| d.id == app.settings.controller_id).map(|d| d.name.clone()).unwrap_or_else(|| tr("контроллер отключён").into()),
        };
        ui.label(RichText::new(format!("   » {cur}")).size(18.0).color(Color32::GRAY));
    });
}

/// Floating controller widget (left side): selection, live drawing, button config, parameter list.
fn controller_widget(app: &mut App, ctx: &Context) {
    if !app.ui_vts.pad_widget {
        return;
    }
    let mut close = false;
    egui::Area::new(egui::Id::new("pad-widget")).order(egui::Order::Foreground).anchor(Align2::LEFT_TOP, [10.0, 90.0]).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).rounding(8.0).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(430.0);
            ui.style_mut().visuals.override_text_color = Some(TEXT);
            ui.horizontal(|ui| {
                ui.label(RichText::new("🎮").size(22.0));
                let devs = app.pads.devices.clone();
                let cur = match app.settings.controller_mode {
                    0 => tr("Без контроллера").to_string(),
                    1 => tr("Любой контроллер").to_string(),
                    _ => devs.iter().find(|d| d.id == app.settings.controller_id).map(|d| d.name.clone()).unwrap_or_else(|| format!("⚠ {}", tr("контроллер отключён"))),
                };
                egui::ComboBox::from_id_salt("pad-sel").selected_text(cur).width(320.0).show_ui(ui, |ui| {
                    if ui.selectable_label(app.settings.controller_mode == 0, tr("Без контроллера")).clicked() {
                        app.settings.controller_mode = 0;
                    }
                    if ui.selectable_label(app.settings.controller_mode == 1, tr("Любой контроллер")).clicked() {
                        app.settings.controller_mode = 1;
                    }
                    for d in &devs {
                        if ui.selectable_label(app.settings.controller_mode == 2 && app.settings.controller_id == d.id, &d.name).clicked() {
                            app.settings.controller_mode = 2;
                            app.settings.controller_id = d.id.clone();
                        }
                    }
                });
                if ui.add(egui::Button::new(RichText::new("✖").size(20.0).color(Color32::from_rgb(240, 96, 96))).frame(false)).clicked() {
                    close = true;
                }
            });
            draw_pad(ui, &app.pads.pad);
            let w = ui.available_width();
            ui.horizontal(|ui| {
                ui.add_enabled_ui(false, |ui| {
                    wide_button(ui, "✚", "Настройка кнопок", w - 50.0);
                })
                .response
                .on_hover_text(tr("Ручная настройка нужна только для нестандартных геймпадов (DirectInput) — пока не поддерживается"));
                if crate::vts_style::button(ui, "↺").on_hover_text(tr("Сбросить ориентацию гироскопа")).clicked() {
                    app.pads.reset_orientation();
                }
            });
            ui.add_space(4.0);
            egui::CollapsingHeader::new(tr("Показать список параметров")).id_salt("pad-list").show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    let base = crate::mapping::input::CONTROLLER_FIRST;
                    egui::Grid::new("pad-grid2").num_columns(2).striped(true).show(ui, |ui| {
                        for (k, n) in crate::mapping::CONTROLLER_NAMES.iter().enumerate() {
                            ui.label(*n);
                            ui.label(format!("{:+.3}", app.inputs[base + k]));
                            ui.end_row();
                        }
                    });
                });
            });
        });
    });
    if close {
        app.ui_vts.pad_widget = false;
    }
}

/// Gamepad drawing with live buttons, sticks and triggers.
fn draw_pad(ui: &mut egui::Ui, p: &crate::controller::Pad) {
    use crate::controller::btn;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 300.0), Sense::hover());
    let pt = ui.painter_at(rect);
    let on = |b: u64| p.buttons & b != 0;
    let lit = Color32::from_rgb(70, 140, 250);
    let body = Color32::from_rgb(236, 238, 245);
    let c = rect.center_top() + Vec2::new(0.0, 100.0);
    // Body
    pt.rect_filled(egui::Rect::from_center_size(c, Vec2::new(300.0, 120.0)), 50.0, body);
    pt.circle_filled(c + Vec2::new(-120.0, 40.0), 60.0, body);
    pt.circle_filled(c + Vec2::new(120.0, 40.0), 60.0, body);
    pt.rect_filled(egui::Rect::from_center_size(c + Vec2::new(0.0, -30.0), Vec2::new(110.0, 50.0)), 8.0, if on(btn::TOUCH) { lit } else { Color32::from_rgb(210, 214, 224) });
    for t in p.touch.iter().filter(|t| t.0) {
        pt.circle_filled(c + Vec2::new(-55.0 + t.1 * 110.0, -55.0 + t.2 * 50.0), 5.0, Color32::from_rgb(240, 80, 120));
    }
    // D-pad
    let dp = c + Vec2::new(-110.0, -10.0);
    for (b, off) in [(btn::UP, Vec2::new(0.0, -18.0)), (btn::DOWN, Vec2::new(0.0, 18.0)), (btn::LEFT, Vec2::new(-18.0, 0.0)), (btn::RIGHT, Vec2::new(18.0, 0.0))] {
        pt.rect_filled(egui::Rect::from_center_size(dp + off, Vec2::splat(16.0)), 3.0, if on(b) { lit } else { Color32::from_rgb(90, 90, 100) });
    }
    // Face buttons
    let fb = c + Vec2::new(110.0, -10.0);
    for (b, off, s) in [(btn::TRIANGLE, Vec2::new(0.0, -20.0), "△"), (btn::CROSS, Vec2::new(0.0, 20.0), "✕"), (btn::SQUARE, Vec2::new(-20.0, 0.0), "□"), (btn::CIRCLE, Vec2::new(20.0, 0.0), "○")] {
        pt.circle_filled(fb + off, 11.0, if on(b) { lit } else { Color32::from_rgb(90, 90, 100) });
        pt.text(fb + off, Align2::CENTER_CENTER, s, FontId::proportional(13.0), Color32::WHITE);
    }
    // Sticks
    for (base, x, y, b) in [(c + Vec2::new(-50.0, 35.0), p.lx, p.ly, btn::L3), (c + Vec2::new(50.0, 35.0), p.rx, p.ry, btn::R3)] {
        pt.circle_stroke(base, 22.0, Stroke::new(2.0f32, Color32::GRAY));
        pt.circle_filled(base + Vec2::new(x, -y) * 14.0, 13.0, if on(b) { lit } else { Color32::from_rgb(80, 80, 90) });
    }
    // Options
    pt.circle_filled(c + Vec2::new(-70.0, -45.0), 6.0, if on(btn::OPT_L) { lit } else { Color32::from_rgb(120, 120, 130) });
    pt.circle_filled(c + Vec2::new(70.0, -45.0), 6.0, if on(btn::OPT_R) { lit } else { Color32::from_rgb(120, 120, 130) });
    pt.circle_filled(c + Vec2::new(0.0, 20.0), 7.0, if on(btn::HOME) { lit } else { Color32::from_rgb(220, 120, 160) });
    // Shoulders and triggers (second view below).
    let s = rect.center_top() + Vec2::new(0.0, 245.0);
    for (b, x, col) in [(btn::L1, -95.0, Color32::from_rgb(60, 90, 200)), (btn::R1, 95.0, Color32::from_rgb(200, 50, 60))] {
        pt.rect_filled(egui::Rect::from_center_size(s + Vec2::new(x, -20.0), Vec2::new(80.0, 18.0)), 8.0, if on(b) { lit } else { col.gamma_multiply(0.6) });
    }
    for (v, x) in [(p.lt, -95.0), (p.rt, 95.0)] {
        let r = egui::Rect::from_center_size(s + Vec2::new(x, 15.0), Vec2::new(80.0, 30.0));
        pt.rect_filled(r, 8.0, Color32::from_rgb(230, 210, 150));
        pt.rect_filled(egui::Rect::from_min_size(r.min, Vec2::new(80.0 * v, 30.0)), 8.0, Color32::from_rgb(240, 170, 40));
    }
}

// ------------------------------------------------------------------ Updates

fn update_status(app: &mut App, ui: &mut egui::Ui, full: bool) {
    use crate::updater::State;
    match app.updater.state() {
        State::Idle => {}
        State::Checking => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Проверка обновлений…"));
            });
        }
        State::UpToDate => {
            ui.label(RichText::new(format!("✔ {}", tr("У вас последняя версия"))).color(Color32::from_rgb(60, 160, 80)));
        }
        State::Available(r) => {
            ui.label(RichText::new(format!("{} {}", tr("Доступна новая версия"), r.tag)).size(18.0).strong().color(Color32::from_rgb(236, 30, 160)));
            if full && !r.notes.is_empty() {
                egui::CollapsingHeader::new(tr("Что нового")).id_salt("upd-notes").show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&r.notes).size(13.0)).wrap());
                    });
                });
            }
            let w = ui.available_width().min(420.0);
            if wide_button(ui, "⬇", "Обновить и перезапустить", w) {
                app.save();
                app.updater.install(r.clone());
            }
            if full && ui.link(tr("Открыть страницу релиза")).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&r.page));
            }
        }
        State::Downloading(done, total) => {
            let f = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            ui.label(tr("Загрузка обновления…"));
            ui.add(egui::ProgressBar::new(f).text(format!("{:.1} / {:.1} МБ", done as f32 / 1048576.0, total as f32 / 1048576.0)));
            ui.ctx().request_repaint();
        }
        State::Installing | State::Ready => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Установка, программа перезапустится…"));
            });
        }
        State::Error(e) => {
            ui.colored_label(Color32::from_rgb(220, 80, 80), format!("{}: {e}", tr("Ошибка обновления")));
        }
    }
}

/// General tab: version, auto-check and manual update.
pub fn update_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Обновления", |ui| {
        ui.label(RichText::new(format!("{} {}", tr("Текущая версия:"), crate::updater::current_version())).size(17.0));
        if toggle(ui, &mut app.settings.update_check, tr("Проверять обновления при запуске")).changed() {
            app.save();
        }
        if !app.updater.busy() && crate::vts_style::button(ui, tr("Проверить сейчас")).clicked() {
            app.updater.check();
        }
        update_status(app, ui, true);
    });
}

/// Small banner at the top of the window when a new version is available or installing.
fn update_banner(app: &mut App, ctx: &Context) {
    use crate::updater::State;
    let st = app.updater.state();
    let show = match st {
        State::Available(_) => !app.update_banner_closed,
        State::Downloading(..) | State::Installing | State::Ready => true,
        State::Error(_) => false,
        _ => false,
    };
    if !show || app.ui.settings_open {
        return;
    }
    egui::Area::new(egui::Id::new("upd-banner")).order(egui::Order::Foreground).anchor(Align2::CENTER_TOP, [0.0, 12.0]).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).rounding(8.0).inner_margin(12.0).stroke(Stroke::new(1.0f32, Color32::from_rgb(166, 214, 240))).show(ui, |ui| {
            ui.set_width(440.0);
            ui.style_mut().visuals.override_text_color = Some(TEXT);
            ui.horizontal(|ui| {
                ui.label(RichText::new("🚀").size(22.0));
                ui.vertical(|ui| update_status(app, ui, false));
                if matches!(app.updater.state(), State::Available(_)) && ui.add(egui::Button::new(RichText::new("✖").size(18.0).color(Color32::GRAY)).frame(false)).on_hover_text(tr("Позже")).clicked() {
                    app.update_banner_closed = true;
                }
            });
        });
    });
}
