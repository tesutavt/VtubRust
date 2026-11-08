//! Settings tab 4 (hotkeys, VTS layout), the per-hotkey Twitch trigger dialog, on-screen
//! hotkey buttons, and the General-tab cards for the plugin API and Twitch.

use egui::{Align2, Color32, Context, FontId, Pos2, RichText, Sense, Stroke, Vec2};

use crate::app::{Action, App};
use crate::hotkeys::{self, Hotkey, Source, ACTIONS, GESTURES, TWITCH_FEATURES};
use crate::i18n::tr;
use crate::ui::{blue_button, card_frame, row, section, status_dot, toggle, Modal, Row, BLUE, TEXT};

const TWITCH: Color32 = Color32::from_rgb(145, 70, 255);
const PINK: Color32 = Color32::from_rgb(236, 30, 160);
const GREEN: Color32 = Color32::from_rgb(28, 150, 90);

fn header(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(tr(text)).size(24.0).color(TEXT));
}

/// Light-blue selector button + "↳ value" line (VTS style).
fn selector(ui: &mut egui::Ui, label: &str, value: &str) -> egui::Response {
    let r = ui.horizontal(|ui| {
        ui.add_space(24.0);
        ui.add(egui::Button::new(RichText::new(tr(label)).size(17.0).color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240)).rounding(3.0).min_size(Vec2::new(270.0, 30.0)))
    });
    ui.horizontal(|ui| {
        ui.add_space(30.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::hover());
        let p = ui.painter();
        let c = Color32::from_rgb(70, 70, 78);
        p.line_segment([rect.left_top() + Vec2::new(4.0, 0.0), rect.left_top() + Vec2::new(4.0, 14.0)], Stroke::new(2.5, c));
        p.line_segment([rect.left_top() + Vec2::new(4.0, 14.0), rect.left_top() + Vec2::new(20.0, 14.0)], Stroke::new(2.5, c));
        p.line_segment([rect.left_top() + Vec2::new(15.0, 9.0), rect.left_top() + Vec2::new(20.0, 14.0)], Stroke::new(2.5, c));
        p.line_segment([rect.left_top() + Vec2::new(15.0, 19.0), rect.left_top() + Vec2::new(20.0, 14.0)], Stroke::new(2.5, c));
        ui.label(RichText::new(value).size(20.0).color(Color32::from_rgb(70, 70, 78)));
    });
    r.inner
}

/// Small Twitch glyph (speech bubble with two bars).
fn twitch_icon(ui: &mut egui::Ui, size: f32, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let col = if active { TWITCH } else { Color32::from_rgb(206, 190, 240) };
    let p = ui.painter();
    let r = rect.shrink(size * 0.12);
    p.rect_stroke(r, size * 0.08, Stroke::new(size * 0.09, col));
    let bx = r.center().x;
    for dx in [-0.17, 0.17] {
        let x = bx + dx * r.width();
        p.line_segment([Pos2::new(x, r.top() + r.height() * 0.25), Pos2::new(x, r.top() + r.height() * 0.55)], Stroke::new(size * 0.09, col));
    }
    resp
}

fn icon_button(ui: &mut egui::Ui, icon: &str, color: Color32, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(icon).size(22.0).color(color)).frame(false)).on_hover_text(tr(tip))
}

fn keys_text(h: &Hotkey) -> String {
    let k = h.triggers.list();
    if k.is_empty() {
        String::new()
    } else {
        k.join(" + ")
    }
}

// ------------------------------------------------------------------ tab

pub fn hotkeys_tab(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Настройки горячих клавиш", |ui| {
        toggle(ui, &mut app.settings.hotkeys_enabled, tr("Горячие клавиши клавиатуры"));
        toggle(ui, &mut app.settings.screen_buttons, tr("Горячие кнопки на экране"));
        toggle(ui, &mut app.settings.send_to_pc, tr("Отправлять команды на ПК")).on_hover_text(tr("Глобальные горячие клавиши (🌐) срабатывают, даже когда окно rigrs не активно"));
        ui.add_space(6.0);
        crate::ui::pill_slider(ui, &mut app.settings.screen_buttons_opacity, 0.0..=100.0, "0%", "100%").on_hover_text(tr("Видимость экранных кнопок"));
    });
    if app.avatar.is_none() {
        section(ui, "", |ui| {
            ui.label(RichText::new(tr("Загрузите модель: горячие клавиши хранятся у каждой модели отдельно.")).size(18.0));
        });
        return;
    }
    let expressions: Vec<String> = app.avatar.as_ref().map(|a| a.expressions.iter().map(|e| e.file.clone()).collect()).unwrap_or_default();
    let n = app.hotkeys().len();
    let mut remove = None;
    let mut test = None;
    let mut open: Option<Modal> = None;
    let recording = app.hk.recording.as_ref().map(|r| (r.0, r.1.clone()));
    let mut start_rec = None;
    let cur_xf = app.avatar.as_ref().map(|a| a.xf);
    let (vw, vh) = app.viewport();
    let art_colors = app.artmesh_details().map(|d| d.art_mesh_multiply_and_screen_colors.clone()).unwrap_or_default();
    let light_now = {
        let st = &app.settings;
        serde_json::json!({
            "On": st.light_mode != 0, "IsStaticColor": st.light_mode == 3, "Display": st.light_monitor as i64,
            "WindowName": if st.light_mode == 2 { st.light_window.clone() } else { String::new() },
            "IncludeLeft": false, "IncludeMid": true, "IncludeRight": false,
            "BaseValue": st.light_brightness, "OverlayValue": st.light_overlay, "Smoothing": st.light_smoothing, "IncludeItems": st.light_items,
            "StaticColor": { "r": st.light_color[0], "g": st.light_color[1], "b": st.light_color[2], "a": 1.0 },
        })
    };
    let scenes_active = app.active_scene.clone();
    let _ = scenes_active;
    for i in 0..n {
        let Some(list) = app.hotkeys_mut() else { break };
        let h = &mut list[i];
        card_frame().inner_margin(egui::Margin::symmetric(10.0, 12.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // ---- header row
            ui.horizontal(|ui| {
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
                ui.painter().circle_filled(r.center(), 15.0, if h.is_active { GREEN } else { Color32::from_rgb(190, 190, 196) });
                ui.painter().text(r.center(), Align2::CENTER_CENTER, if h.is_active { "ON" } else { "OFF" }, FontId::proportional(11.0), Color32::WHITE);
                if resp.on_hover_text(tr("Включить/выключить горячую клавишу")).clicked() {
                    h.is_active = !h.is_active;
                }
                let g = if h.is_global { Color32::from_rgb(20, 80, 160) } else { Color32::from_rgb(190, 200, 214) };
                if icon_button(ui, "🌐", g, "Глобальная: срабатывает, даже когда окно не в фокусе").clicked() {
                    h.is_global = !h.is_global;
                }
                ui.add(egui::TextEdit::singleline(&mut h.name).hint_text(tr("Имя горячей клавиши")).font(FontId::proportional(18.0)).desired_width(210.0));
                let snd = if h.sound_config.sound_active { BLUE } else { Color32::from_rgb(170, 206, 240) };
                if icon_button(ui, "🎵", snd, "Звук при срабатывании").clicked() {
                    open = Some(Modal::HkSound(i));
                }
                if twitch_icon(ui, 28.0, h.twitch_triggers.active).on_hover_text(tr("Триггеры Twitch")).clicked() {
                    open = Some(Modal::HkTwitch(i));
                }
                if icon_button(ui, "▶", Color32::from_rgb(90, 170, 90), "Проверить (выполнить сейчас)").clicked() {
                    test = Some(h.id.clone());
                }
                if icon_button(ui, "🗑", Color32::from_rgb(230, 80, 80), "Удалить").clicked() {
                    remove = Some(i);
                }
                let m = if h.minimized { "⊕" } else { "⊖" };
                if icon_button(ui, m, Color32::GRAY, if h.minimized { "Развернуть" } else { "Свернуть" }).clicked() {
                    h.minimized = !h.minimized;
                }
            });
            if h.minimized {
                let mut s = hotkeys::action_label(&h.action);
                if !h.file.is_empty() {
                    s += &format!(" · {}", h.file);
                }
                let k = keys_text(h);
                if !k.is_empty() {
                    s += &format!(" · {k}");
                }
                ui.label(RichText::new(s).size(15.0).weak());
                return;
            }
            // ---- action
            header(ui, "Действие горячей клавиши");
            if selector(ui, "Действие", &hotkeys::action_label(&h.action)).clicked() {
                open = Some(Modal::HkAction(i));
            }
            if hotkeys::action_needs_file(&h.action) {
                let shown = if h.action == "ToggleTwitchFeature" {
                    TWITCH_FEATURES.iter().find(|f| f.0 == h.file).map(|f| f.1.to_string()).unwrap_or_default()
                } else {
                    h.file.clone()
                };
                if selector(ui, hotkeys::file_caption(&h.action), &shown).clicked() {
                    open = Some(Modal::HkFile(i));
                }
            }
            match h.action.as_str() {
                "MoveModel" => {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        if ui.button(tr("Записать текущую позицию модели")).clicked() {
                            if let Some(xf) = cur_xf {
                                h.position = hotkeys::HkPosition { x: xf.pos[0] / (vw / 2.0), y: xf.pos[1] / (vh / 2.0), z: xf.scale, rotation: xf.rot };
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label("X");
                        ui.add(egui::DragValue::new(&mut h.position.x).speed(0.01));
                        ui.label("Y");
                        ui.add(egui::DragValue::new(&mut h.position.y).speed(0.01));
                        ui.label(tr("Масштаб"));
                        ui.add(egui::DragValue::new(&mut h.position.z).speed(0.001));
                        ui.label(tr("Поворот"));
                        ui.add(egui::DragValue::new(&mut h.position.rotation).speed(1.0).suffix("°"));
                    });
                }
                "ChangeVTSModel" => {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        toggle(ui, &mut h.load_model_settings.load_at_current_position, tr("Загрузить в текущей позиции"));
                    });
                }
                "ArtMeshColorPreset" => {
                    let n = h.extra.get("ColorScreenMultiplyPreset").and_then(|p| p["ArtMeshMultiplyAndScreenColors"].as_array()).map(|a| a.len()).unwrap_or(0);
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        if ui.button(tr("Записать текущие цвета модели")).clicked() {
                            let colors = art_colors.clone();
                            h.extra.insert("ColorScreenMultiplyPreset".into(), serde_json::json!({ "ArtMeshMultiplyAndScreenColors": colors }));
                        }
                        if ui.button(tr("Очистить")).on_hover_text(tr("Пустой пресет снимает загруженный пресет")).clicked() {
                            h.extra.insert("ColorScreenMultiplyPreset".into(), serde_json::json!({ "ArtMeshMultiplyAndScreenColors": [] }));
                        }
                        ui.label(format!("{}: {n}", tr("частей в пресете")));
                    });
                }
                "ScreenColorOverlay" => {
                    let on = h.extra.get("ColorOverlay").map(|o| o["On"].as_bool().unwrap_or(false)).unwrap_or(false);
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        if ui.button(tr("Записать текущие настройки освещения")).clicked() {
                            h.extra.insert("ColorOverlay".into(), light_now.clone());
                        }
                        ui.label(if on { tr("наложение: вкл.") } else { tr("наложение: выкл.") });
                    });
                }
                _ => {}
            }
            // ---- keys
            header(ui, "Горячая клавиша");
            let rec = recording.as_ref().filter(|r| r.0 == i);
            let shown = match rec {
                Some((_, k)) if !k.is_empty() => format!("{} …", k.join(" + ")),
                Some(_) => tr("Нажмите клавиши… (Esc — сброс)").to_string(),
                None => keys_text(h),
            };
            if selector(ui, if rec.is_some() { "Запись…" } else { "Record" }, &shown).clicked() {
                start_rec = Some(i);
            }
            // ---- gestures
            header(ui, "Активация жестом");
            let g = &h.hand_gesture_settings;
            let gl = hotkeys::gesture_label(&g.gesture_left);
            let gr = hotkeys::gesture_label(&g.gesture_right);
            let gs = match (g.gesture_left.is_empty(), g.gesture_right.is_empty()) {
                (true, true) => String::new(),
                (false, true) => format!("{}: {gl}", tr("Левая")),
                (true, false) => format!("{}: {gr}", tr("Правая")),
                _ => format!("{}: {gl} {} {}: {gr}", tr("Левая"), if g.gesture_combinator == "OR" { tr("ИЛИ") } else { tr("И") }, tr("Правая")),
            };
            if selector(ui, "Жест", &gs).clicked() {
                open = Some(Modal::HkGesture(i, true));
            }
            if !(g.gesture_left.is_empty() && g.gesture_right.is_empty()) {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    if ui.button(tr("Правая рука…")).clicked() {
                        open = Some(Modal::HkGesture(i, false));
                    }
                    let mut or = h.hand_gesture_settings.gesture_combinator == "OR";
                    if ui.checkbox(&mut or, tr("Любая из рук (ИЛИ)")).changed() {
                        h.hand_gesture_settings.gesture_combinator = if or { "OR" } else { "AND" }.into();
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.checkbox(&mut h.hand_gesture_settings.allow_mirrored_gesture, tr("Разрешить зеркальный жест"));
                    ui.checkbox(&mut h.hand_gesture_settings.deactivate_exp_when_gesture_not_detected, tr("Выключать, когда жест пропал"));
                });
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.label(tr("Удерживать жест (сек.)"));
                    ui.add(egui::DragValue::new(&mut h.hand_gesture_settings.seconds_until_detection).speed(0.05).range(0.0..=5.0));
                    if app_tracks_hands_hint() {
                        ui.label(RichText::new(tr("нужен трекинг рук")).weak());
                    }
                });
            }
            // ---- deactivation
            header(ui, "Настройки деактивации");
            egui::Grid::new(("deact", i)).num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(60.0);
                    ui.label(RichText::new(tr("Затухание (сек.)")).size(16.0));
                });
                ui.add(egui::DragValue::new(&mut h.fade_seconds_amount).speed(0.01).range(0.0..=10.0));
                ui.end_row();
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    toggle(ui, &mut h.deactivate_after_seconds, tr("Время до остановки (сек.)"));
                });
                ui.add_enabled(h.deactivate_after_seconds, egui::DragValue::new(&mut h.deactivate_after_seconds_amount).speed(0.1).range(0.1..=3600.0));
                ui.end_row();
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    toggle(ui, &mut h.deactivate_after_key_up, tr("Остановка при отпускании клавиши"));
                });
                ui.end_row();
            });
            // ---- on-screen button
            header(ui, "Экранная горячая клавиша");
            ui.horizontal_wrapped(|ui| {
                ui.add_space(20.0);
                for b in 1..=8 {
                    let sel = h.triggers.screen_button == b;
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(54.0), Sense::click());
                    let c = h.on_screen_hotkey_color.clone();
                    let base = Color32::from_rgb((c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8);
                    let fill = if sel { crate::ui::mix(BLUE, base, 0.35) } else { Color32::from_rgb(170, 210, 246) };
                    ui.painter().circle_filled(r.center(), 25.0, fill);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, b.to_string(), FontId::proportional(28.0), Color32::WHITE);
                    if resp.clicked() {
                        h.triggers.screen_button = if sel { -1 } else { b };
                    }
                    if b == 4 {
                        ui.end_row();
                        ui.add_space(20.0);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.label(tr("Цвет кнопки"));
                let c = &mut h.on_screen_hotkey_color;
                let mut rgb = [c.r, c.g, c.b];
                if ui.color_edit_button_rgb(&mut rgb).changed() {
                    c.r = rgb[0];
                    c.g = rgb[1];
                    c.b = rgb[2];
                    c.a = 1.0;
                }
            });
        });
        ui.add_space(14.0);
    }
    if let Some(i) = start_rec {
        app.hk.recording = if recording.map(|r| r.0) == Some(i) { None } else { Some((i, Vec::new(), false)) };
    }
    if let Some(i) = remove {
        if let Some(l) = app.hotkeys_mut() {
            l.remove(i);
        }
        app.hk.recording = None;
        app.save();
    }
    if let Some(id) = test {
        hotkeys::trigger(app, &id, Source::Test);
    }
    if let Some(m) = open {
        app.ui.modal = Some(m);
        app.ui.modal_sel = None;
    }
    section(ui, "", |ui| {
        if blue_button(ui, &format!("+  {}", tr("Добавить горячую клавишу")), BLUE, 44.0).clicked() {
            let file = expressions.first().cloned().unwrap_or_default();
            if let Some(l) = app.hotkeys_mut() {
                l.push(Hotkey { file, ..Hotkey::default() });
            }
        }
    });
}

fn app_tracks_hands_hint() -> bool {
    false
}

// ------------------------------------------------------------------ list modals

/// Rows for the hotkey-related list modals (shown by ui::modal_dialog).
pub fn modal_rows(app: &mut App, kind: Modal) -> Option<(String, Vec<Row>)> {
    let hk = |i: usize| app.hotkeys().get(i).cloned();
    Some(match kind {
        Modal::HkAction(i) => {
            let cur = hk(i).map(|h| h.action).unwrap_or_default();
            (
                tr("Выберите действие горячей клавиши.").into(),
                ACTIONS
                    .iter()
                    .map(|a| Row {
                        bold: if a.0 == cur { "✔".into() } else { String::new() },
                        text: if a.2 { tr(a.1).to_string() } else { format!("{}  ({})", tr(a.1), tr("недоступно в rigrs")) },
                        enabled: a.2,
                    })
                    .collect(),
            )
        }
        Modal::HkFile(i) => {
            let action = hk(i).map(|h| h.action).unwrap_or_default();
            let files = file_choices(app, &action);
            (format!("{} «{}».", tr("Выберите"), tr(hotkeys::file_caption(&action)).to_lowercase()), files.into_iter().map(|f| row(f.1)).collect())
        }
        Modal::HkGesture(_, left) => (
            if left { tr("Выберите жест левой руки.") } else { tr("Выберите жест правой руки.") }.into(),
            GESTURES.iter().map(|g| row(tr(g.1))).collect(),
        ),
        Modal::TwitchReward(_) => {
            let list = app.twitch.as_ref().map(|t| t.rewards.clone()).unwrap_or_default();
            let mut v: Vec<Row> = list.iter().map(|r| Row { bold: format!("{} ", r.cost), text: r.title.clone(), enabled: true }).collect();
            if v.is_empty() {
                v.push(Row { bold: String::new(), text: tr("Награды не загружены — войдите в Twitch (нужен статус компаньона или партнёра)").into(), enabled: false });
            }
            (tr("Выберите награду Twitch.").into(), v)
        }
        Modal::TwitchFeatures => (
            tr("Выберите функцию интеграции Twitch, которую вы хотите настроить.").into(),
            vec![
                Row { bold: "Twitch Emote Dropper".into(), text: format!("  {}", if app.settings.twitch.dropper.enabled { tr("вкл.") } else { tr("выкл.") }), enabled: true },
                Row { bold: tr("Пролетающий чат Twitch").into(), text: format!("  {}", if app.settings.twitch.chat.enabled { tr("вкл.") } else { tr("выкл.") }), enabled: true },
                Row { bold: tr("Триггеры горячей клавиши Twitch").into(), text: format!("  {}", if app.settings.twitch.triggers_on { tr("вкл.") } else { tr("выкл.") }), enabled: true },
            ],
        ),
        _ => return None,
    })
}

/// (stored value, shown label)
fn file_choices(app: &mut App, action: &str) -> Vec<(String, String)> {
    match action {
        "TriggerAnimation" | "ChangeIdleAnimation" => {
            let dir = app.avatar.as_ref().map(|a| a.dir.clone());
            let mut v: Vec<(String, String)> = dir.map(|d| crate::scene::live2d::list_motions(&d)).unwrap_or_default().into_iter().map(|m| (m.clone(), m)).collect();
            if action == "ChangeIdleAnimation" {
                v.insert(0, (String::new(), tr("Нет").into()));
            }
            v
        }
        "ToggleExpression" => app.avatar.as_ref().map(|a| a.expressions.iter().map(|e| (e.file.clone(), e.file.clone())).collect()).unwrap_or_default(),
        "ChangeBackground" => app.library.backgrounds.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).map(|n| (n.clone(), n)).collect(),
        "ChangeVTSModel" => app.library.models.iter().map(|m| (m.name.clone(), m.name.clone())).collect(),
        "ToggleItemScene" => app.settings.item_scenes.iter().map(|s| (s.name.clone(), s.name.clone())).collect(),
        "ExecuteItemAction" => {
            let mut v = Vec::new();
            for e in &app.library.items {
                let crate::library::ItemEntry::Live2D(m) = e else { continue };
                let Some(vt) = m.vtube.as_ref().and_then(|p| crate::formats::read_json::<crate::formats::VtubeJson>(p).ok()) else { continue };
                let folder = hotkeys::item_folder(&m.model3);
                for h in vt.hotkeys.iter().filter(|h| !h.name.is_empty() && ["ToggleExpression", "RemoveAllExpressions", "TriggerAnimation", "ChangeIdleAnimation"].contains(&h.action.as_str())) {
                    v.push((format!("{folder}::{}", h.name), format!("{folder} → {}", h.name)));
                }
            }
            v
        }
        "ToggleTwitchFeature" => TWITCH_FEATURES.iter().map(|f| (f.0.to_string(), tr(f.1).to_string())).collect(),
        "LoadEffectPreset" => {
            let mut v = vec![(String::new(), tr("Вкл./выкл. все эффекты").to_string())];
            let dir = app.paths.data.join("vfx_presets");
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().map(|x| x == "json").unwrap_or(false) {
                        let n = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                        v.push((n.clone(), n));
                    }
                }
            }
            v
        }
        _ => Vec::new(),
    }
}

/// Applies a choice from one of the list modals above. Returns true when handled.
pub fn modal_choose(app: &mut App, kind: Modal, i: usize) -> bool {
    match kind {
        Modal::HkAction(k) => {
            let Some(a) = ACTIONS.get(i) else { return true };
            let needs = a.3;
            if let Some(h) = app.hotkeys_mut().and_then(|l| l.get_mut(k)) {
                if h.action != a.0 {
                    h.action = a.0.to_string();
                    h.file.clear();
                }
            }
            app.ui.modal = if needs { Some(Modal::HkFile(k)) } else { None };
        }
        Modal::HkFile(k) => {
            let action = app.hotkeys().get(k).map(|h| h.action.clone()).unwrap_or_default();
            if let Some((v, _)) = file_choices(app, &action).get(i).cloned() {
                if let Some(h) = app.hotkeys_mut().and_then(|l| l.get_mut(k)) {
                    h.file = v;
                }
            }
            app.ui.modal = None;
        }
        Modal::HkGesture(k, left) => {
            if let Some(g) = GESTURES.get(i) {
                if let Some(h) = app.hotkeys_mut().and_then(|l| l.get_mut(k)) {
                    if left {
                        h.hand_gesture_settings.gesture_left = g.0.into();
                    } else {
                        h.hand_gesture_settings.gesture_right = g.0.into();
                    }
                }
            }
            app.ui.modal = None;
        }
        Modal::TwitchReward(k) => {
            let r = app.twitch.as_ref().and_then(|t| t.rewards.get(i).cloned());
            if let (Some(r), Some(h)) = (r, app.hotkeys_mut().and_then(|l| l.get_mut(k))) {
                h.twitch_triggers.redeem_id = r.id;
                h.twitch_triggers.redeem_name = r.title;
            }
            app.ui.modal = Some(Modal::HkTwitch(k));
        }
        Modal::TwitchFeatures => {
            app.ui.modal = Some(match i {
                0 => Modal::TwitchDropper,
                1 => Modal::TwitchChat,
                _ => Modal::TwitchTriggersCfg,
            });
        }
        _ => return false,
    }
    app.save();
    true
}

// ------------------------------------------------------------------ custom dialogs

fn dim(ctx: &Context, id: &str) {
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new(id)).fixed_pos(Pos2::ZERO).order(egui::Order::Foreground).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(120));
    });
}

/// Pink VTS-style option row: switch + title + subtitle. Returns true when changed.
fn pink_row(ui: &mut egui::Ui, on: &mut bool, title: &str, sub: &str) -> bool {
    let w = ui.available_width();
    let fill = if *on { Color32::from_rgb(214, 234, 252) } else { Color32::from_rgb(255, 222, 228) };
    let stroke = if *on { Color32::from_rgb(120, 180, 240) } else { Color32::from_rgb(240, 150, 166) };
    let mut changed = false;
    egui::Frame::none().fill(fill).stroke(Stroke::new(1.5, stroke)).rounding(8.0).inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(ui, |ui| {
        ui.set_width(w - 18.0);
        ui.horizontal(|ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(46.0, 24.0), Sense::click());
            let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.15);
            let p = ui.painter();
            p.rect_filled(rect, 12.0, crate::ui::mix(Color32::from_rgb(255, 196, 206), BLUE, t));
            p.circle_filled(Pos2::new(egui::lerp(rect.left() + 12.0..=rect.right() - 12.0, t), rect.center().y), 9.0, Color32::WHITE);
            let lr = ui
                .vertical(|ui| {
                    let a = ui.add(egui::Label::new(RichText::new(tr(title)).size(19.0).color(Color32::from_rgb(90, 80, 86))).sense(Sense::click()));
                    if !sub.is_empty() {
                        ui.label(RichText::new(tr(sub)).size(13.0).color(Color32::from_rgb(110, 100, 106)));
                    }
                    a
                })
                .inner;
            if resp.clicked() || lr.clicked() {
                *on = !*on;
                changed = true;
            }
        });
    });
    ui.add_space(4.0);
    changed
}

fn group_title(ui: &mut egui::Ui, t: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(tr(t)).size(17.0).color(Color32::from_rgb(110, 110, 118)));
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([Pos2::new(r.left(), r.top()), Pos2::new(r.left() + 260.0, r.top())], Stroke::new(1.0, Color32::from_rgb(220, 220, 226)));
    ui.add_space(6.0);
}

fn range_row(ui: &mut egui::Ui, r: &mut hotkeys::Range, lo: f32, hi: f32, dflt: (f32, f32)) {
    ui.horizontal(|ui| {
        ui.add_space(52.0);
        ui.label(RichText::new(tr("Мин.")).size(14.0));
        ui.add(egui::DragValue::new(&mut r.min).speed(1.0).range(lo..=hi));
        ui.label(RichText::new(tr("Макс.")).size(14.0));
        let inf = r.max >= 1e6;
        let mut m = if inf { hi } else { r.max };
        ui.add(egui::DragValue::new(&mut m).speed(1.0).range(lo..=hi).custom_formatter(|v, _| if v >= hi as f64 && hi >= 1e6 { "∞".into() } else { format!("{v:.0}") }));
        r.max = m;
        if r.max < r.min {
            r.max = r.min;
        }
        if ui.button("↺").on_hover_text(tr("Сбросить")).clicked() {
            r.min = dflt.0;
            r.max = dflt.1;
        }
    });
}

/// Handles the dialogs that are not simple lists. Returns true when `kind` was one of them.
pub fn custom_modal(app: &mut App, ctx: &Context, kind: Modal) -> bool {
    match kind {
        Modal::HkTwitch(i) => twitch_triggers_dialog(app, ctx, i),
        Modal::HkSound(i) => sound_dialog(app, ctx, i),
        Modal::TwitchDropper => dropper_dialog(app, ctx),
        Modal::TwitchChat => chat_dialog(app, ctx),
        Modal::TwitchTriggersCfg => triggers_cfg_dialog(app, ctx),
        _ => return false,
    }
    true
}

fn dialog_frame(ctx: &Context, id: &str, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    let screen = ctx.screen_rect();
    let w = width.min(screen.width() - 40.0);
    egui::Area::new(egui::Id::new(id)).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, 10.0]).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).rounding(4.0).inner_margin(egui::Margin { left: 14.0, right: 14.0, top: 14.0, bottom: 14.0 }).show(ui, |ui| {
            ui.set_width(w - 28.0);
            ui.set_max_height(screen.height() - 150.0);
            add(ui);
        });
    });
}

fn top_bar(app: &mut App, ctx: &Context, id: &str, on: &mut bool, test: &mut bool) {
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new(id)).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - 300.0, 14.0)).show(ctx, |ui| {
        ui.horizontal(|ui| {
            let (txt, fill) = if *on { ("ON", Color32::from_rgb(120, 200, 140)) } else { ("OFF", Color32::from_rgb(250, 170, 176)) };
            if ui.add_sized([150.0, 48.0], egui::Button::new(RichText::new(txt).size(26.0).strong().color(Color32::from_rgb(90, 60, 66))).fill(fill).rounding(8.0)).clicked() {
                *on = !*on;
            }
            ui.add_space(70.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            ui.painter().rect_filled(r, 10.0, Color32::from_rgb(230, 228, 250));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(8.0)));
            twitch_icon(&mut child, 48.0, true);
            ui.add_space(20.0);
            if ui.add_sized([190.0, 34.0], egui::Button::new(RichText::new(tr("Тест")).size(17.0).color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240)).rounding(4.0)).clicked() {
                *test = true;
            }
        });
    });
    let _ = app;
}

fn twitch_triggers_dialog(app: &mut App, ctx: &Context, i: usize) {
    let Some(mut h) = app.hotkeys().get(i).cloned() else {
        app.ui.modal = None;
        return;
    };
    dim(ctx, "tw-dim");
    let mut test = false;
    let mut cancel_cd = false;
    let cooldown_left = app.hk.cooldown.get(&h.id).map(|t| h.twitch_triggers.cooldown_seconds - t.elapsed().as_secs_f32()).filter(|l| *l > 0.0 && h.twitch_triggers.cooldown_active);
    let mut active = h.twitch_triggers.active;
    top_bar(app, ctx, "tw-top", &mut active, &mut test);
    h.twitch_triggers.active = active;
    let mut ok = false;
    let mut pick_reward = false;
    let connected = app.twitch.as_ref().map(|t| t.status == crate::twitch::Status::Connected).unwrap_or(false);
    let eventsub = app.twitch.as_ref().and_then(|t| t.eventsub.clone()).map(|r| r.is_ok()).unwrap_or(false);
    dialog_frame(ctx, "tw-dialog", 600.0, |ui| {
        ui.style_mut().visuals.override_text_color = Some(TEXT);
        let t = &mut h.twitch_triggers;
        let name = if h.name.is_empty() { tr("Название не задано").to_string() } else { h.name.clone() };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Настройка триггеров Twitch для горячей клавиши с названием:")).size(17.0));
            ui.label(RichText::new(format!("\"{name}\"")).size(17.0).color(PINK));
            ui.label(RichText::new(format!(", {}:", tr("триггеры"))).size(17.0));
            ui.label(RichText::new(t.count().to_string()).size(17.0).color(PINK));
        });
        if !connected {
            ui.label(RichText::new(tr("Twitch не подключён: Настройки → Общие → Настройки Twitch.")).color(Color32::from_rgb(220, 90, 90)));
        } else if !eventsub {
            ui.label(RichText::new(tr("Без входа в Twitch работают: текст, битсы, подписки, подарочные подписки, рейды и Power-Up. Для фолловов, наград, отметок и рекламы войдите в Twitch.")).size(13.0).weak());
        }
        egui::ScrollArea::vertical().max_height(ctx.screen_rect().height() - 330.0).show(ui, |ui| {
            group_title(ui, "Вкл./Выкл.");
            let mut a = t.active;
            pink_row(ui, &mut a, &format!("{} {}", tr("Активно. Триггеры:"), t.count()), "");
            t.active = a;
            group_title(ui, "Общие настройки");
            pink_row(ui, &mut t.cooldown_active, "Период ожидания", "Разрешить срабатывание каждые X секунд");
            if t.cooldown_active {
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    ui.add(egui::Slider::new(&mut t.cooldown_seconds, 1.0..=3600.0).logarithmic(true).suffix(tr(" с")));
                });
            }
            pink_row(ui, &mut t.reset_active, "Сброс после периода ожидания", "Автоматический сброс горячей клавиши");
            pink_row(ui, &mut t.hide_model_reset_dialog, "Скрыть диалог загрузки модели", "");
            group_title(ui, "Триггеры Twitch");
            pink_row(ui, &mut t.gift_sub, "Триггер подарочной подписки Twitch", "Клавиша срабатывает при значении в диапазоне");
            if t.gift_sub {
                range_row(ui, &mut t.gift_range, 1.0, 100.0, (1.0, 100.0));
            }
            pink_row(ui, &mut t.bits, "Триггер битсов Twitch", "Клавиша срабатывает при значении в диапазоне");
            if t.bits {
                range_row(ui, &mut t.bit_range, 1.0, 1e6, (100.0, 1e6));
            }
            pink_row(ui, &mut t.redeem, "Триггер активации награды Twitch", "Горячая клавиша срабатывает при активации награды");
            if t.redeem {
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    let label = if t.redeem_name.is_empty() { tr("Выберите награду Twitch").to_string() } else { t.redeem_name.clone() };
                    if ui.add_sized([300.0, 30.0], egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240))).clicked() {
                        pick_reward = true;
                    }
                    if ui.button("↺").on_hover_text(tr("Сбросить")).clicked() {
                        t.redeem_id.clear();
                        t.redeem_name.clear();
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    ui.label(RichText::new(tr("или название вручную:")).size(13.0));
                    if ui.add(egui::TextEdit::singleline(&mut t.redeem_name).desired_width(200.0)).changed() {
                        t.redeem_id.clear();
                    }
                });
            }
            pink_row(ui, &mut t.power_up, "Twitch Power-Up trigger", "Hotkey triggered on Power-Up");
            if t.power_up {
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    for (id, label) in [("", "Любой"), ("gigantify_an_emote", "Гигантский эмоут"), ("message_effect", "Эффект сообщения"), ("celebration", "Празднование")] {
                        if ui.selectable_label(t.power_up_id == id, tr(label)).clicked() {
                            t.power_up_id = id.into();
                            t.power_up_name = tr(label).into();
                        }
                    }
                });
            }
            pink_row(ui, &mut t.text, "Текстовый триггер Twitch", "Любое слово из списка активирует горячую клавишу");
            if t.text {
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    ui.add(egui::TextEdit::singleline(&mut t.text_words).hint_text(tr("!команда, слово, другое слово")).desired_width(380.0));
                });
            }
            pink_row(ui, &mut t.restrict_text, "Ограничить текстовый триггер", "Кто может его использовать?");
            if t.restrict_text {
                let r = &mut t.restrict;
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(52.0);
                    ui.checkbox(&mut r.you, tr("Вы"));
                    ui.checkbox(&mut r.subs, tr("Подписчики"));
                    ui.checkbox(&mut r.mods, tr("Модераторы"));
                    ui.checkbox(&mut r.vips, "VIP");
                    ui.checkbox(&mut r.artists, tr("Художники"));
                });
                ui.horizontal(|ui| {
                    ui.add_space(52.0);
                    ui.checkbox(&mut r.by_name, tr("Имена:"));
                    let mut names = r.user_names.join(", ");
                    if ui.add_enabled(r.by_name, egui::TextEdit::singleline(&mut names).desired_width(300.0)).changed() {
                        r.user_names = names.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                    }
                });
            }
            pink_row(ui, &mut t.sub, "Триггер подписки Twitch", "Активируется при подписке");
            pink_row(ui, &mut t.follow, "Триггер отслеживания Twitch", "Активируется при отслеживании");
            pink_row(ui, &mut t.allow_repeated_follow, "Разрешить повторное отслеживание", "Разрешить активацию при повторном отслеживании");
            pink_row(ui, &mut t.raid, "Триггер рейда Twitch", "Активируется при рейде на ваш канал");
            pink_row(ui, &mut t.shoutout, "Триггер отметки Twitch", "Активируется при отметке любого пользователя");
            pink_row(ui, &mut t.ad_break, "Триггер рекламной паузы Twitch", "Активируется при запуске рекламной паузы");
        });
        if let Some(left) = cooldown_left {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} {:.0} {}", tr("Период ожидания активен:"), left, tr("с"))).color(PINK));
                if ui.small_button("✖").on_hover_text(tr("Отменить период ожидания и сброс")).clicked() {
                    cancel_cd = true;
                }
            });
        }
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240)).rounding(3.0)).clicked() {
                ok = true;
            }
        });
    });
    let id = h.id.clone();
    if cancel_cd {
        hotkeys::cancel(app, &id);
    }
    if let Some(slot) = app.hotkeys_mut().and_then(|l| l.get_mut(i)) {
        *slot = h;
    }
    if test {
        hotkeys::trigger(app, &id, Source::Test);
    }
    if pick_reward {
        if let Some(t) = &app.twitch {
            t.fetch_rewards(&app.settings.twitch);
        }
        app.ui.modal = Some(Modal::TwitchReward(i));
        app.ui.modal_sel = None;
    }
    if ok || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.modal = None;
        app.save();
    }
}

/// Editor for a VTS sound list + randomized playback settings (hotkey slots and triggers).
pub fn sound_params_ui(ui: &mut egui::Ui, salt: &str, p: &mut crate::audio::SoundParams, dir_hint: &std::path::Path) {
    let mut del = None;
    for (k, f) in p.files.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&f.file).size(14.0));
            ui.add(egui::DragValue::new(&mut f.volume).speed(0.01).range(0.0..=2.0).prefix(tr("громк. ")));
            ui.add(egui::DragValue::new(&mut f.pitch).speed(0.01).range(0.1..=3.0).prefix(tr("тон ")));
            if ui.small_button("🗑").clicked() {
                del = Some(k);
            }
        });
    }
    if let Some(k) = del {
        p.files.remove(k);
    }
    if ui.button(tr("+ Добавить звук…")).clicked() {
        let _ = std::fs::create_dir_all(dir_hint);
        if let Some(files) = rfd::FileDialog::new().set_directory(dir_hint).add_filter("Audio", &["wav", "mp3", "ogg"]).pick_files() {
            for f in files {
                let name = if f.starts_with(dir_hint) { f.strip_prefix(dir_hint).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default() } else { f.to_string_lossy().into_owned() };
                p.files.push(crate::audio::SoundFile { file: name, ..Default::default() });
            }
        }
    }
    let range = |ui: &mut egui::Ui, label: &str, r: &mut [f32; 2], lo: f32, hi: f32| {
        ui.horizontal(|ui| {
            ui.add_sized([150.0, 18.0], egui::Label::new(tr(label)));
            ui.add(egui::DragValue::new(&mut r[0]).speed(0.01).range(lo..=hi));
            ui.label("…");
            ui.add(egui::DragValue::new(&mut r[1]).speed(0.01).range(lo..=hi));
        });
    };
    egui::CollapsingHeader::new(tr("Параметры воспроизведения")).id_salt((salt, "params")).show(ui, |ui| {
        range(ui, "Громкость", &mut p.volume, 0.0, 2.0);
        range(ui, "Высота тона", &mut p.pitch, 0.1, 3.0);
        range(ui, "Панорама (L…R)", &mut p.pan, -1.0, 1.0);
        range(ui, "Задержка (сек.)", &mut p.delay, 0.0, 10.0);
        ui.horizontal(|ui| {
            ui.label(tr("Эхо: повторов"));
            ui.add(egui::DragValue::new(&mut p.echo_count).range(0..=8));
            ui.label(tr("задержка"));
            ui.add(egui::DragValue::new(&mut p.echo_delay).speed(0.01).range(0.01..=2.0));
            ui.label(tr("громкость"));
            ui.add(egui::DragValue::new(&mut p.echo_volume).speed(0.01).range(0.0..=1.0));
        });
    });
}

fn sound_dialog(app: &mut App, ctx: &Context, i: usize) {
    let Some(mut h) = app.hotkeys().get(i).cloned() else {
        app.ui.modal = None;
        return;
    };
    dim(ctx, "snd-dim");
    let dir = app.avatar.as_ref().map(|a| a.dir.clone()).unwrap_or_else(|| hotkeys::sounds_dir(app));
    let mut ok = false;
    let mut test = false;
    let slot_id = egui::Id::new(("snd-slot", i));
    let mut slot: usize = ctx.data(|d| d.get_temp(slot_id)).unwrap_or(0);
    dialog_frame(ctx, "snd-dialog", 680.0, |ui| {
        ui.style_mut().visuals.override_text_color = Some(TEXT);
        ui.label(RichText::new(tr("Звук горячей клавиши")).size(24.0));
        ui.label(RichText::new(tr("WAV, MP3 или OGG из папки модели (как в VTS) или из папки Sounds. Из списка каждый раз выбирается случайный файл.")).size(13.0).weak());
        let sc = &mut h.sound_config;
        toggle(ui, &mut sc.sound_active, tr("Проигрывать звук"));
        ui.horizontal(|ui| {
            toggle(ui, &mut sc.stop_when_inactive, tr("Останавливать звук при выключении горячей клавиши"));
            ui.add(egui::DragValue::new(&mut sc.stop_fade_out_seconds).speed(0.05).range(0.0..=10.0).suffix(tr(" с")));
        });
        // Up to 10 sound slots.
        ui.horizontal_wrapped(|ui| {
            for k in 0..10 {
                let used = sc.sound_slots.get(k).map(|s| !s.params().files.is_empty()).unwrap_or(false);
                let label = format!("{}{}", k + 1, if used { " ●" } else { "" });
                if ui.selectable_label(slot == k, label).clicked() {
                    slot = k;
                }
            }
        });
        while sc.sound_slots.len() <= slot {
            sc.sound_slots.push(Default::default());
        }
        let s = &mut sc.sound_slots[slot];
        if s.params.files.is_empty() && !s.file.is_empty() {
            s.params = s.params();
            s.file.clear();
        }
        egui::ScrollArea::vertical().max_height(ctx.screen_rect().height() - 380.0).show(ui, |ui| {
            sound_params_ui(ui, &format!("hk{i}-{slot}"), &mut s.params, &dir);
        });
        // Drop trailing empty slots.
        while sc.sound_slots.last().map(|s| s.params.files.is_empty() && s.file.is_empty()).unwrap_or(false) {
            sc.sound_slots.pop();
        }
        ui.horizontal(|ui| {
            if ui.button(tr("▶ Проверить")).clicked() {
                test = true;
            }
            if ui.button(tr("Открыть папку модели")).clicked() {
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        });
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240))).clicked() {
                ok = true;
            }
        });
    });
    ctx.data_mut(|d| d.insert_temp(slot_id, slot));
    let id = h.id.clone();
    if let Some(sl) = app.hotkeys_mut().and_then(|l| l.get_mut(i)) {
        *sl = h;
    }
    if test {
        hotkeys::trigger(app, &id, Source::Test);
    }
    if ok || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.modal = None;
        app.save();
    }
}

/// General tab: sound output (VTS "Sound effects" card).
pub fn sound_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Звуковые эффекты", |ui| {
        let s = &mut app.settings.sound;
        toggle(ui, &mut s.play_local, tr("Воспроизводить в приложении"));
        toggle(ui, &mut s.vban, tr("Отправлять в OBS (VBAN)"));
        ui.horizontal(|ui| {
            ui.label(tr("Громкость"));
            ui.add(egui::Slider::new(&mut s.volume, 0.0..=100.0).suffix("%"));
        });
        if s.vban {
            ui.horizontal(|ui| {
                ui.label("IP");
                ui.add(egui::TextEdit::singleline(&mut s.vban_ip).desired_width(120.0));
                ui.label(tr("Порт"));
                let mut p = s.vban_port as f32;
                if ui.add(egui::DragValue::new(&mut p).range(1.0..=65535.0)).changed() {
                    s.vban_port = p as u16;
                }
                ui.label(tr("Имя потока"));
                ui.add(egui::TextEdit::singleline(&mut s.vban_stream).desired_width(100.0));
            });
            ui.label(RichText::new(tr("В OBS: плагин VBAN Audio → источник «VBAN Audio Source», порт 6980.")).size(13.0).weak());
        }
        let peak = app.audio.as_ref().map(|a| a.peak()).unwrap_or(0.0);
        ui.add(egui::ProgressBar::new(peak).desired_width(300.0).text(format!("{:.0} dB", 20.0 * peak.max(1e-4).log10())));
        if let Some(e) = app.audio.as_ref().and_then(|a| a.error()) {
            ui.colored_label(Color32::from_rgb(220, 80, 80), e);
        }
    });
}

/// Model tab: model volume, spatial audio and parameter sound triggers.
pub fn model_sound_card(app: &mut App, ui: &mut egui::Ui) {
    let Some(key) = app.model_key() else { return };
    let params: Vec<String> = app.avatar.as_ref().map(|a| a.model.param_ids.clone()).unwrap_or_default();
    let dir = app.avatar.as_ref().map(|a| a.dir.clone()).unwrap_or_default();
    let mut test = None;
    section(ui, "Звук модели", |ui| {
        let ms = app.settings.model_sound.entry(key.clone()).or_default();
        ui.horizontal(|ui| {
            ui.label(tr("Громкость модели"));
            ui.add(egui::Slider::new(&mut ms.volume, 0.0..=100.0).suffix("%"));
            ui.checkbox(&mut ms.muted, tr("Без звука"));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut ms.volume_by_size, tr("Громкость по размеру"));
            ui.add_enabled(ms.volume_by_size, egui::DragValue::new(&mut ms.size_range[0]).speed(1.0).prefix("min "));
            ui.add_enabled(ms.volume_by_size, egui::DragValue::new(&mut ms.size_range[1]).speed(1.0).prefix("max "));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut ms.pan_by_x, tr("Панорама по X"));
            ui.add_enabled(ms.pan_by_x, egui::DragValue::new(&mut ms.x_range[0]).speed(0.01).prefix("min "));
            ui.add_enabled(ms.pan_by_x, egui::DragValue::new(&mut ms.x_range[1]).speed(0.01).prefix("max "));
        });
        ui.separator();
        ui.label(RichText::new(tr("Звуковые триггеры параметров")).strong());
        let mut del = None;
        for (k, t) in ms.triggers.iter_mut().enumerate() {
            egui::CollapsingHeader::new(if t.name.is_empty() { format!("{} {}", tr("Триггер"), k + 1) } else { t.name.clone() }).id_salt(("trg", k)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut t.active, tr("Активен"));
                    ui.add(egui::TextEdit::singleline(&mut t.name).hint_text(tr("Название")).desired_width(160.0));
                    if ui.small_button("▶").clicked() {
                        test = Some(k);
                    }
                    if ui.small_button("🗑").clicked() {
                        del = Some(k);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Параметр"));
                    egui::ComboBox::from_id_salt(("trg-p", k)).selected_text(t.parameter.clone()).height(300.0).show_ui(ui, |ui| {
                        for p in &params {
                            ui.selectable_value(&mut t.parameter, p.clone(), p);
                        }
                    });
                    use crate::sfx::TriggerKind::*;
                    for (kind, label) in [(Value, "Значение"), (Speed, "Скорость"), (DirectionChange, "Смена направления"), (PassRight, "Проход →"), (PassLeft, "Проход ←")] {
                        ui.selectable_value(&mut t.kind, kind, tr(label));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Диапазон"));
                    ui.add(egui::DragValue::new(&mut t.range[0]).speed(0.01));
                    ui.add(egui::DragValue::new(&mut t.range[1]).speed(0.01));
                    ui.label(tr("Перезарядка (сек.)"));
                    ui.add(egui::DragValue::new(&mut t.cooldown[0]).speed(0.01).range(0.0..=60.0));
                    ui.add(egui::DragValue::new(&mut t.cooldown[1]).speed(0.01).range(0.0..=60.0));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Условие: параметр"));
                    egui::ComboBox::from_id_salt(("trg-c", k)).selected_text(if t.condition_parameter.is_empty() { tr("(нет)").to_string() } else { t.condition_parameter.clone() }).height(300.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut t.condition_parameter, String::new(), tr("(нет)"));
                        for p in &params {
                            ui.selectable_value(&mut t.condition_parameter, p.clone(), p);
                        }
                    });
                    ui.add(egui::DragValue::new(&mut t.condition_range[0]).speed(0.01));
                    ui.add(egui::DragValue::new(&mut t.condition_range[1]).speed(0.01));
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut t.speed_to_volume, tr("Скорость → громкость"));
                    ui.add(egui::DragValue::new(&mut t.speed_to_volume_range[0]).speed(0.01));
                    ui.add(egui::DragValue::new(&mut t.speed_to_volume_range[1]).speed(0.01));
                    ui.checkbox(&mut t.speed_to_pitch, tr("Скорость → тон"));
                    ui.add(egui::DragValue::new(&mut t.speed_to_pitch_range[0]).speed(0.01));
                    ui.add(egui::DragValue::new(&mut t.speed_to_pitch_range[1]).speed(0.01));
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut t.item_load_sound, tr("Звук загрузки предмета"));
                    ui.checkbox(&mut t.item_delete_sound, tr("Звук удаления предмета"));
                });
                sound_params_ui(ui, &format!("trg{k}"), &mut t.sound, &dir);
            });
        }
        if let Some(k) = del {
            ms.triggers.remove(k);
        }
        if ui.button(tr("+ Добавить триггер")).clicked() {
            ms.triggers.push(crate::sfx::SoundTrigger { parameter: params.first().cloned().unwrap_or_default(), ..Default::default() });
        }
    });
    if let Some(k) = test {
        let t = app.settings.model_sound.get(&key).and_then(|m| m.triggers.get(k).cloned());
        if let Some(t) = t {
            let data = app.paths.data.join("Sounds");
            if let Some(req) = crate::audio::request(&t.sound, |n| crate::hotkeys::resolve_sound(&[dir.clone()], &data, n), &key, 1.0, 1.0, 0.0) {
                app.play_sound(req);
            }
        }
    }
}

fn slider(ui: &mut egui::Ui, v: &mut f32, r: std::ops::RangeInclusive<f32>, label: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([250.0, 22.0], egui::Label::new(RichText::new(tr(label)).size(15.0)));
        ui.add(egui::Slider::new(v, r));
    });
}

fn feature_dialog(app: &mut App, ctx: &Context, id: &str, title: &str, dropper: bool, body: impl FnOnce(&mut App, &mut egui::Ui)) {
    dim(ctx, &format!("{id}-dim"));
    let mut test = false;
    let mut on = if dropper { app.settings.twitch.dropper.enabled } else { app.settings.twitch.chat.enabled };
    top_bar(app, ctx, &format!("{id}-top"), &mut on, &mut test);
    if dropper {
        app.settings.twitch.dropper.enabled = on;
    } else {
        app.settings.twitch.chat.enabled = on;
    }
    let mut ok = false;
    let h = ctx.screen_rect().height() - 300.0;
    let screen = ctx.screen_rect();
    let w = 640.0f32.min(screen.width() - 40.0);
    egui::Area::new(egui::Id::new(id)).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, 10.0]).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).rounding(4.0).inner_margin(14.0).show(ui, |ui| {
            ui.set_width(w - 28.0);
            ui.style_mut().visuals.override_text_color = Some(TEXT);
            ui.label(RichText::new(tr(title)).size(22.0));
            egui::ScrollArea::vertical().max_height(h).show(ui, |ui| body(app, ui));
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240))).clicked() {
                    ok = true;
                }
            });
        });
    });
    if test {
        crate::twitch_fx::test(app, dropper);
    }
    if ok || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.modal = None;
        app.save();
    }
}

fn dropper_dialog(app: &mut App, ctx: &Context) {
    feature_dialog(app, ctx, "drop", "Twitch Emote Dropper", true, |app, ui| {
        let d = &mut app.settings.twitch.dropper;
        group_title(ui, "Вкл./Выкл.");
        pink_row(ui, &mut d.enabled, "Включить", "Эмоуты из чата падают на экран и отскакивают от модели");
        group_title(ui, "Фильтры");
        let mut f = d.filter.join(", ");
        ui.horizontal(|ui| {
            ui.label(tr("Список фильтров (не ронять эти эмоуты):"));
            if ui.add(egui::TextEdit::singleline(&mut f).desired_width(240.0)).changed() {
                d.filter = f.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
        });
        pink_row(ui, &mut d.subs_only, "Только подписчики", "");
        pink_row(ui, &mut d.privileged_only, "Только вы, модераторы и VIP", "");
        group_title(ui, "Количество");
        pink_row(ui, &mut d.smooth_borders, "Плавные края", "Плавное появление и исчезновение");
        slider(ui, &mut d.max_per_message, 1.0..=30.0, "Макс. эмоутов в сообщении");
        slider(ui, &mut d.max_on_screen, 1.0..=400.0, "Макс. эмоутов на экране");
        slider(ui, &mut d.spawn_chance, 0.01..=100.0, "Шанс появления (%)");
        group_title(ui, "Эмоуты сторонних сервисов");
        pink_row(ui, &mut d.bttv_channel, "BTTV: эмоуты канала", "");
        pink_row(ui, &mut d.bttv_global, "BTTV: глобальные эмоуты", "");
        pink_row(ui, &mut d.seventv_channel, "7TV: эмоуты канала", "");
        pink_row(ui, &mut d.seventv_global, "7TV: глобальные эмоуты", "");
        pink_row(ui, &mut d.ffz_channel, "FFZ: эмоуты канала", "");
        pink_row(ui, &mut d.ffz_global, "FFZ: глобальные эмоуты", "");
        group_title(ui, "Физика и вид");
        slider(ui, &mut d.lifetime, 1.0..=15.0, "Время жизни (сек.)");
        slider(ui, &mut d.anim_speed, 0.0..=2.0, "Скорость анимации");
        slider(ui, &mut d.fall_speed, 0.0..=1.0, "Скорость падения");
        slider(ui, &mut d.gravity, 0.1..=2.0, "Гравитация");
        slider(ui, &mut d.opacity, 0.0..=1.0, "Прозрачность");
        slider(ui, &mut d.bounciness, 0.0..=1.0, "Упругость");
        slider(ui, &mut d.size, 0.5..=3.0, "Размер эмоутов");
        slider(ui, &mut d.rotation, 0.0..=1.0, "Вращение");
        pink_row(ui, &mut d.bounce_off_model, "Отскакивать от модели", "");
        slider(ui, &mut d.bounce_bottom, 0.0..=20.0, "Отскоков от низа");
        slider(ui, &mut d.bounce_top, 0.0..=20.0, "Отскоков от верха");
        slider(ui, &mut d.bounce_left, 0.0..=20.0, "Отскоков от левого края");
        slider(ui, &mut d.bounce_right, 0.0..=20.0, "Отскоков от правого края");
        pink_row(ui, &mut d.notify, "Показывать уведомления", "");
    });
}

fn chat_dialog(app: &mut App, ctx: &Context) {
    feature_dialog(app, ctx, "chat", "Пролетающий чат Twitch", false, |app, ui| {
        let c = &mut app.settings.twitch.chat;
        group_title(ui, "Вкл./Выкл.");
        pink_row(ui, &mut c.enabled, "Включить", "Сообщения чата пролетают по экрану");
        group_title(ui, "Вид");
        pink_row(ui, &mut c.foreground, "На переднем плане", "Поверх модели");
        slider(ui, &mut c.opacity, 0.0..=1.0, "Прозрачность");
        pink_row(ui, &mut c.grayscale, "Оттенки серого", "");
        pink_row(ui, &mut c.fly_in, "Анимация влёта", "");
        slider(ui, &mut c.speed, 0.1..=4.0, "Базовая скорость");
        slider(ui, &mut c.speed_random, 0.0..=1.0, "Случайная скорость");
        slider(ui, &mut c.size, 0.3..=4.0, "Базовый размер");
        slider(ui, &mut c.size_random, 0.0..=1.0, "Случайный размер");
        slider(ui, &mut c.shadow_opacity, 0.0..=1.0, "Прозрачность тени");
        slider(ui, &mut c.shadow_offset, 0.0..=1.0, "Смещение тени");
        slider(ui, &mut c.zone_width, 0.05..=1.0, "Ширина зоны появления");
        slider(ui, &mut c.zone_center, 0.0..=1.0, "Центр зоны появления");
        slider(ui, &mut c.angle, -45.0..=45.0, "Начальный угол");
        slider(ui, &mut c.angle_random, 0.0..=1.0, "Случайный угол");
        slider(ui, &mut c.max_len, 10.0..=600.0, "Макс. длина сообщения");
        pink_row(ui, &mut c.sync_emote_anim, "Синхронизировать анимацию эмодзи", "");
        slider(ui, &mut c.emote_anim_speed, 0.0..=2.0, "Скорость анимации эмодзи");
        group_title(ui, "Количество");
        slider(ui, &mut c.max_on_screen, 1.0..=320.0, "Макс. сообщений на экране");
        pink_row(ui, &mut c.remove_old, "Удалять старые", "Когда лимит достигнут");
        pink_row(ui, &mut c.per_user_limit, "Ограничение на пользователя", "Одно сообщение пользователя на экране");
        group_title(ui, "Фильтры");
        pink_row(ui, &mut c.subs_only, "Только подписчики", "");
        pink_row(ui, &mut c.privileged_only, "Только вы, модераторы и VIP", "");
        ui.label(tr("Список заблокированных слов (через запятую):"));
        ui.add(egui::TextEdit::multiline(&mut c.blocked_words).desired_rows(2).desired_width(f32::INFINITY));
        pink_row(ui, &mut c.block_scam_bots, "Блокировать ботов-мошенников", "");
        pink_row(ui, &mut c.block_obscene, "Блокировать нецензурную лексику", "");
        pink_row(ui, &mut c.block_profanity, "Блокировать ненормативную лексику", "");
        pink_row(ui, &mut c.block_urls, "Блокировать URL", "");
        pink_row(ui, &mut c.exclude_you, "Исключить вас из фильтров", "");
        group_title(ui, "Эмоуты сторонних сервисов");
        pink_row(ui, &mut c.bttv, "BTTV", "");
        pink_row(ui, &mut c.seventv, "7TV", "");
        pink_row(ui, &mut c.ffz, "FFZ", "");
        group_title(ui, "Особая анимация");
        slider(ui, &mut c.special_privileged, 0.0..=100.0, "Шанс (VIP/модераторы), %");
        slider(ui, &mut c.special_others, 0.0..=100.0, "Шанс (остальные), %");
        pink_row(ui, &mut c.notify, "Показывать уведомления", "");
    });
}

fn triggers_cfg_dialog(app: &mut App, ctx: &Context) {
    dim(ctx, "trg-dim");
    let mut ok = false;
    dialog_frame(ctx, "trg-dialog", 600.0, |ui| {
        ui.style_mut().visuals.override_text_color = Some(TEXT);
        ui.label(RichText::new(tr("Триггеры горячей клавиши Twitch")).size(22.0));
        let t = &mut app.settings.twitch;
        pink_row(ui, &mut t.triggers_on, "Включить", "Горячие клавиши срабатывают от событий Twitch");
        pink_row(ui, &mut t.trigger_log, "Записывать чат в лог", "События пишутся в rigrs.log");
        pink_row(ui, &mut t.trigger_notify, "Показывать уведомления", "");
        ui.label(RichText::new(tr("Сами триггеры настраиваются у каждой горячей клавиши (значок Twitch на вкладке «Горячие клавиши»).")).size(14.0).weak());
        if let Some(tw) = &app.twitch {
            group_title(ui, "Последние события");
            for e in tw.recent.iter().take(8) {
                ui.label(RichText::new(e).size(14.0));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_sized([180.0, 40.0], egui::Button::new(RichText::new("OK").size(18.0).color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240))).clicked() {
                ok = true;
            }
        });
    });
    if ok || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.modal = None;
        app.save();
    }
}

// ------------------------------------------------------------------ overlays

/// Round on-screen hotkey buttons (1–8) on the right edge.
pub fn screen_buttons(app: &mut App, ctx: &Context) {
    if !app.settings.screen_buttons || !app.ui.visible || app.ui.settings_open {
        return;
    }
    let mut list: Vec<(i32, String, String, Color32)> = app
        .hotkeys()
        .iter()
        .filter(|h| h.is_active && (1..=8).contains(&h.triggers.screen_button))
        .map(|h| {
            let c = &h.on_screen_hotkey_color;
            (h.triggers.screen_button, h.id.clone(), if h.name.is_empty() { hotkeys::action_label(&h.action) } else { h.name.clone() }, Color32::from_rgb((c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8))
        })
        .collect();
    if list.is_empty() {
        return;
    }
    list.sort_by_key(|l| l.0);
    let alpha = (app.settings.screen_buttons_opacity / 100.0).clamp(0.05, 1.0);
    let screen = ctx.screen_rect();
    let mut fire = None;
    egui::Area::new(egui::Id::new("screen-hotkeys")).fixed_pos(Pos2::new(screen.right() - 84.0, screen.center().y - 4.0 * 37.0)).show(ctx, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        for (b, id, name, col) in &list {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::click());
            let fill = crate::ui::mix(BLUE, *col, 0.35).gamma_multiply(alpha);
            ui.painter().circle_filled(r.center(), 30.0, if resp.hovered() { crate::ui::lighten(fill, 0.15) } else { fill });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, b.to_string(), FontId::proportional(30.0), Color32::WHITE.gamma_multiply(alpha));
            if resp.on_hover_text(name).clicked() {
                fire = Some(id.clone());
            }
        }
    });
    if let Some(id) = fire {
        hotkeys::trigger(app, &id, Source::Screen);
    }
}

/// Plugin asking for access (token or permission).
pub fn api_prompt(app: &mut App, ctx: &Context) {
    let Some(p) = app.api_prompts.front() else { return };
    dim(ctx, "api-dim");
    let (plugin, dev) = (p.plugin.clone(), p.developer.clone());
    let what = match &p.kind {
        crate::api::PromptKind::Token => tr("запрашивает доступ к API rigrs (совместимо с VTube Studio). Плагин сможет управлять моделью, горячими клавишами, предметами и параметрами.").to_string(),
        crate::api::PromptKind::Permission(perm) => format!("{} «{perm}».", tr("запрашивает дополнительное разрешение")),
    };
    let icon = p.icon_png.clone();
    let mut answer = None;
    egui::Area::new(egui::Id::new("api-prompt")).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        egui::Frame::none().fill(Color32::WHITE).rounding(6.0).inner_margin(26.0).show(ui, |ui| {
            ui.set_width(620.0);
            ui.horizontal(|ui| {
                if let Some(png) = &icon {
                    let id = format!("bytes://api-icon-{plugin}.png");
                    ui.add(egui::Image::from_bytes(id, png.clone()).max_size(Vec2::splat(96.0)));
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr("Запрос доступа плагина")).size(26.0).color(PINK));
                    ui.label(RichText::new(format!("«{plugin}» ({} {dev})", tr("автор:"))).size(20.0).strong().color(TEXT));
                });
            });
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(what).size(18.0).color(Color32::from_rgb(110, 110, 118))).wrap());
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_sized([170.0, 42.0], egui::Button::new(RichText::new(tr("Разрешить")).size(18.0).color(Color32::WHITE)).fill(Color32::from_rgb(14, 150, 240))).clicked() {
                    answer = Some(true);
                }
                ui.add_space(12.0);
                if ui.add_sized([170.0, 42.0], egui::Button::new(RichText::new(tr("Запретить")).size(18.0).color(Color32::WHITE)).fill(Color32::from_rgb(222, 88, 116))).clicked() {
                    answer = Some(false);
                }
            });
        });
    });
    if let Some(a) = answer {
        crate::api::resolve_prompt(app, a);
    }
}

// ------------------------------------------------------------------ General tab cards

pub fn api_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "API для плагинов", |ui| {
        let mut on = app.settings.api_on;
        if toggle(ui, &mut on, tr("Запустить API (плагины)")).changed() {
            app.actions.push(Action::SetApi(on));
        }
        let mut lan = app.settings.api_lan;
        if toggle(ui, &mut lan, tr("Разрешить подключения из локальной сети")).changed() {
            app.settings.api_lan = lan;
            if app.settings.api_on {
                app.actions.push(Action::SetApi(true));
            }
        }
        ui.horizontal(|ui| {
            ui.label(tr("Порт"));
            let mut port = app.settings.api_port as f32;
            if ui.add(egui::DragValue::new(&mut port).range(1024.0..=65535.0)).changed() {
                app.settings.api_port = port as u16;
            }
            if app.settings.api_on && app.api.as_ref().map(|s| s.port != app.settings.api_port).unwrap_or(false) && ui.button(tr("Применить")).clicked() {
                app.actions.push(Action::SetApi(true));
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Статус:")).strong());
            match (&app.api, &app.api_error) {
                (Some(s), _) => {
                    status_dot(ui, Color32::from_rgb(60, 190, 90));
                    ui.label(format!("{} ws://{}:{}  ·  {}: {}", tr("Запущен"), if app.settings.api_lan { "0.0.0.0" } else { "localhost" }, s.port, tr("плагинов"), s.authenticated_count()));
                }
                (None, Some(e)) => {
                    status_dot(ui, Color32::from_rgb(230, 80, 80));
                    ui.label(e.clone());
                }
                _ => {
                    status_dot(ui, Color32::GRAY);
                    ui.label(tr("API выключен"));
                }
            }
        });
        ui.label(RichText::new(tr("Совместимо с VTube Studio Public API 1.0: существующие плагины подключаются к rigrs так же, как к VTS.")).size(13.0).weak());
        if let Some(s) = &app.api {
            let names: Vec<String> = s.sessions.values().filter_map(|x| x.plugin.as_ref().map(|p| format!("{} ({})", p.0, p.1))).collect();
            if !names.is_empty() {
                ui.label(format!("{}: {}", tr("Подключены"), names.join(", ")));
            }
        }
        if !app.settings.api_tokens.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Разрешённые плагины")).strong());
            let mut revoke = None;
            for (k, t) in app.settings.api_tokens.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{} — {}", t.plugin, t.developer));
                    if ui.small_button(tr("Отозвать")).clicked() {
                        revoke = Some(k);
                    }
                });
            }
            if let Some(k) = revoke {
                let t = app.settings.api_tokens.remove(k);
                let pre = format!("{}|{}|", t.plugin, t.developer);
                app.settings.api_permissions.retain(|p| !p.starts_with(&pre));
                app.save();
            }
        }
        if !app.api_custom.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(tr("Параметры, созданные плагинами")).strong());
            for p in app.api_custom.values() {
                ui.label(format!("{} = {:.3}  [{} … {}]  · {}", p.name, p.value, p.min, p.max, p.added_by));
            }
        }
    });
}

/// ArtMeshSelectionRequest window.
pub fn mesh_select(app: &mut App, ctx: &Context) {
    let Some(sel) = app.mesh_select.as_mut() else { return };
    let ids: Vec<String> = app.avatar.as_ref().map(|a| a.model.drawables.iter().map(|d| d.id.clone()).collect()).unwrap_or_default();
    if ids.is_empty() {
        let (s, r) = (sel.session, sel.request_id.clone());
        app.mesh_select = None;
        crate::api::reply(app, s, &r, "APIError", serde_json::json!({ "errorID": 1000, "message": "No model loaded." }));
        return;
    }
    let mut done: Option<bool> = None;
    let mut hover = None;
    egui::Window::new(tr("Выбор частей модели"))
        .id(egui::Id::new("mesh-select"))
        .anchor(Align2::RIGHT_TOP, [-20.0, 20.0])
        .default_width(420.0)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&sel.text).size(16.0)).wrap());
                if ui.button("?").clicked() {
                    sel.show_help = !sel.show_help;
                }
            });
            if sel.show_help {
                ui.label(RichText::new(&sel.help).weak());
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut sel.filter).hint_text(tr("Фильтр")).desired_width(220.0));
                if sel.at_click.is_some() && ui.button(tr("Сбросить фильтр по клику")).clicked() {
                    sel.at_click = None;
                }
            });
            let f = sel.filter.to_lowercase();
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                for id in ids.iter().rev() {
                    if !f.is_empty() && !id.to_lowercase().contains(&f) {
                        continue;
                    }
                    if let Some(c) = &sel.at_click {
                        if !c.contains(id) {
                            continue;
                        }
                    }
                    let mut on = sel.active.contains(id);
                    let r = ui.checkbox(&mut on, id);
                    if r.hovered() {
                        hover = Some(id.clone());
                    }
                    if r.changed() {
                        if on {
                            sel.active.insert(id.clone());
                        } else {
                            sel.active.remove(id);
                        }
                    }
                }
            });
            let n = sel.active.len() as i64;
            let ok = if sel.count > 0 { n == sel.count } else { n >= 1 };
            ui.label(if sel.count > 0 { format!("{} {n} / {}", tr("Выбрано:"), sel.count) } else { format!("{} {n}", tr("Выбрано:")) });
            ui.horizontal(|ui| {
                if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                    done = Some(true);
                }
                if ui.button(tr("Отмена")).clicked() {
                    done = Some(false);
                }
            });
        });
    if let Some(a) = app.avatar.as_mut() {
        a.highlight = hover.and_then(|h| a.drawable_index(&h));
    }
    if let Some(success) = done {
        let sel = app.mesh_select.take().unwrap();
        let active: Vec<&String> = ids.iter().filter(|i| sel.active.contains(*i)).collect();
        let inactive: Vec<&String> = ids.iter().filter(|i| !sel.active.contains(*i)).collect();
        crate::api::reply(app, sel.session, &sel.request_id, "ArtMeshSelectionResponse", serde_json::json!({ "success": success, "activeArtMeshes": active, "inactiveArtMeshes": inactive }));
        if let Some(a) = app.avatar.as_mut() {
            a.highlight = None;
        }
    }
}

pub fn controller_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Ввод с контроллера", |ui| {
        let devs = app.pads.devices.clone();
        let cur = match app.settings.controller_mode {
            0 => tr("Без контроллера").to_string(),
            1 => tr("Все контроллеры").to_string(),
            _ => devs.iter().find(|d| d.id == app.settings.controller_id).map(|d| d.name.clone()).unwrap_or_else(|| format!("⚠ {}", tr("контроллер отключён"))),
        };
        egui::ComboBox::from_id_salt("pad-select").selected_text(cur).width(320.0).show_ui(ui, |ui| {
            if ui.selectable_label(app.settings.controller_mode == 0, tr("Без контроллера")).clicked() {
                app.settings.controller_mode = 0;
            }
            if ui.selectable_label(app.settings.controller_mode == 1, tr("Все контроллеры")).clicked() {
                app.settings.controller_mode = 1;
            }
            for d in &devs {
                let label = if d.extended { format!("{} ({})", d.name, tr("тачпад и гироскоп")) } else { d.name.clone() };
                if ui.selectable_label(app.settings.controller_mode == 2 && app.settings.controller_id == d.id, label).clicked() {
                    app.settings.controller_mode = 2;
                    app.settings.controller_id = d.id.clone();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Статус:")).strong());
            status_dot(ui, if app.pads.connected { Color32::from_rgb(60, 190, 90) } else { Color32::GRAY });
            ui.label(if app.pads.connected { tr("Контроллер активен") } else { tr("Нет данных от контроллера") });
            if ui.button(tr("Сбросить ориентацию")).on_hover_text(tr("Или удерживайте L1+R1+L2+R2+Треугольник 2 секунды")).clicked() {
                app.pads.reset_orientation();
            }
        });
        // Simple live widget: sticks, triggers and pressed buttons.
        let p = app.pads.pad;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(420.0), 110.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 18.0, Color32::from_rgb(236, 240, 247));
        let stick = |c: Pos2, x: f32, y: f32, pressed: bool| {
            painter.circle_stroke(c, 24.0, Stroke::new(2.0f32, Color32::GRAY));
            painter.circle_filled(c + Vec2::new(x, -y) * 16.0, 10.0, if pressed { BLUE } else { Color32::from_rgb(90, 90, 100) });
        };
        stick(rect.left_center() + Vec2::new(60.0, 10.0), p.lx, p.ly, p.buttons & crate::controller::btn::L3 != 0);
        stick(rect.right_center() + Vec2::new(-60.0, 10.0), p.rx, p.ry, p.buttons & crate::controller::btn::R3 != 0);
        let bar = |x: f32, v: f32| {
            let r = egui::Rect::from_min_size(rect.left_top() + Vec2::new(x, 8.0), Vec2::new(60.0, 10.0));
            painter.rect_filled(r, 4.0, Color32::from_rgb(210, 214, 224));
            painter.rect_filled(egui::Rect::from_min_size(r.min, Vec2::new(60.0 * v, 10.0)), 4.0, BLUE);
        };
        bar(30.0, p.lt);
        bar(rect.width() - 90.0, p.rt);
        let names: Vec<&str> = crate::controller::PAD_KEYS.iter().filter(|k| p.buttons & k.1 != 0).map(|k| k.0.trim_start_matches("Controller")).collect();
        painter.text(rect.center() + Vec2::new(0.0, 30.0), Align2::CENTER_CENTER, names.join(" "), FontId::proportional(13.0), TEXT);
        egui::CollapsingHeader::new(tr("Показать список параметров")).id_salt("pad-params").show(ui, |ui| {
            let base = crate::mapping::input::CONTROLLER_FIRST;
            egui::Grid::new("pad-grid").num_columns(2).striped(true).show(ui, |ui| {
                for (k, n) in crate::mapping::CONTROLLER_NAMES.iter().enumerate() {
                    ui.label(*n);
                    ui.label(format!("{:+.3}", app.inputs[base + k]));
                    ui.end_row();
                }
            });
        });
        ui.label(RichText::new(tr("Кнопки контроллера и свайпы по тачпаду можно назначать на горячие клавиши (кнопка Record).")).size(13.0).weak());
    });
}

pub fn mic_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Настройки микрофона", |ui| {
        let mut on = app.settings.mic.on;
        if toggle(ui, &mut on, tr("Микрофон (продвинутый липсинк)")).changed() {
            app.set_mic(on);
            app.save();
        }
        ui.horizontal(|ui| {
            ui.label(tr("Устройство"));
            let cur = if app.settings.mic.device_name.is_empty() { tr("По умолчанию").to_string() } else { app.settings.mic.device_name.clone() };
            let resp = egui::ComboBox::from_id_salt("mic-dev").selected_text(cur).width(300.0).show_ui(ui, |ui| {
                let mut pick = None;
                if ui.selectable_label(app.settings.mic.device.is_empty(), tr("По умолчанию")).clicked() {
                    pick = Some((String::new(), String::new()));
                }
                for (id, name) in &app.mic_devices {
                    if ui.selectable_label(&app.settings.mic.device == id, name).clicked() {
                        pick = Some((id.clone(), name.clone()));
                    }
                }
                pick
            });
            if resp.response.clicked() || app.mic_devices.is_empty() {
                app.mic_devices = crate::lipsync::list_devices();
            }
            if let Some(Some((id, name))) = resp.inner {
                app.settings.mic.device = id;
                app.settings.mic.device_name = name;
                if app.settings.mic.on {
                    app.set_mic(true);
                }
            }
            if ui.button(tr("Перезагрузить")).clicked() {
                app.set_mic(true);
            }
        });
        let m = &mut app.settings.mic;
        let row = |ui: &mut egui::Ui, label: &str, v: &mut f32| {
            ui.horizontal(|ui| {
                ui.add_sized([200.0, 20.0], egui::Label::new(tr(label)));
                ui.add(egui::Slider::new(v, 0.0..=1.0));
            });
        };
        row(ui, "Усиление громкости", &mut m.gain);
        row(ui, "Порог громкости", &mut m.cutoff);
        row(ui, "Усиление частоты", &mut m.freq_gain);
        toggle(ui, &mut m.monitor, tr("Прослушивание (вывод микрофона в динамики)"));
        // Live meters.
        let inp = app.inputs;
        use crate::mapping::input::*;
        let meter = |ui: &mut egui::Ui, label: &str, v: f32| {
            ui.horizontal(|ui| {
                ui.add_sized([110.0, 18.0], egui::Label::new(label));
                ui.add(egui::ProgressBar::new(v.clamp(0.0, 1.0)).desired_width(260.0));
            });
        };
        meter(ui, "VoiceVolume", inp[VOICE_VOLUME]);
        meter(ui, "VoiceFrequency", inp[VOICE_FREQUENCY]);
        for (k, idx) in [VOICE_A, VOICE_I, VOICE_U, VOICE_E, VOICE_O].into_iter().enumerate() {
            meter(ui, &format!("Voice{}", crate::lipsync::VOWELS[k]), inp[idx]);
        }
        meter(ui, "VoiceSilence", inp[VOICE_SILENCE]);
        if let Some(mic) = &app.mic {
            if let Some(e) = mic.error() {
                ui.colored_label(Color32::from_rgb(220, 80, 80), e);
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tr("Калибровка: нажмите кнопку и тяните гласный звук ~2 секунды.")).size(13.0).weak());
        let calib = app.mic.as_ref().and_then(|m| m.calib_progress());
        ui.horizontal_wrapped(|ui| {
            for k in ["A", "I", "U", "E", "O", "Silence"] {
                let done = app.settings.mic.profiles.contains_key(k);
                let label = match &calib {
                    Some((c, p)) if c == k => format!("{k} {:.0}%", p * 100.0),
                    _ => format!("{}{k}", if done { "✔ " } else { "" }),
                };
                if ui.add_enabled(app.mic.is_some(), egui::Button::new(label)).clicked() {
                    if let Some(mic) = &app.mic {
                        mic.calibrate(k);
                    }
                }
            }
            if ui.button(tr("Сбросить калибровку")).clicked() {
                app.settings.mic.profiles.clear();
            }
        });
        ui.horizontal(|ui| {
            ui.label(tr("Веса VoiceFrequency A/I/U/E/O"));
            for w in app.settings.mic.freq_weights.iter_mut() {
                ui.add(egui::DragValue::new(w).speed(0.01).range(0.0..=1.0));
            }
        });
    });
}

pub fn editor_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Live2D Cubism Editor API", |ui| {
        let mut on = app.settings.editor_on;
        if toggle(ui, &mut on, tr("Запустить")).changed() {
            app.set_editor(on);
            app.save();
        }
        let mut remote = app.settings.editor_remote;
        if toggle(ui, &mut remote, tr("Подключиться к Editor на удалённом ПК")).changed() {
            app.settings.editor_remote = remote;
            if app.settings.editor_on {
                app.set_editor(true);
            }
        }
        ui.horizontal(|ui| {
            if app.settings.editor_remote {
                ui.label("IP");
                ui.add(egui::TextEdit::singleline(&mut app.settings.editor_ip).desired_width(140.0));
            }
            ui.label(tr("Порт"));
            let mut p = app.settings.editor_port as f32;
            if ui.add(egui::DragValue::new(&mut p).range(1.0..=65535.0)).changed() {
                app.settings.editor_port = p as u16;
            }
            if app.settings.editor_on && ui.button(tr("Переподключиться")).clicked() {
                app.set_editor(true);
            }
        });
        let mut send = app.settings.editor_send;
        if toggle(ui, &mut send, tr("Отправлять значения в редактор")).changed() {
            app.settings.editor_send = send;
            app.push_editor_event();
        }
        toggle(ui, &mut app.settings.editor_physics_only, tr("Только в окне физики"));
        toggle(ui, &mut app.settings.editor_auto_import, tr("Копировать экспортированные модели в папку моделей"));
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Статус:")).strong());
            match &app.editor {
                Some(e) => {
                    status_dot(ui, if e.connected && e.approved { Color32::from_rgb(60, 190, 90) } else if e.connected { Color32::from_rgb(230, 180, 60) } else { Color32::from_rgb(230, 80, 80) });
                    ui.label(tr(&e.status).to_string());
                    if !e.edit_mode.is_empty() {
                        ui.label(RichText::new(format!("· {}", e.edit_mode)).weak());
                    }
                }
                None => {
                    status_dot(ui, Color32::GRAY);
                    ui.label(tr("Выкл."));
                }
            }
        });
        ui.label(RichText::new(tr("В Cubism Editor: «Файл» → «Настройки внешнего приложения» → запустить API, затем поставить галочку «Разрешить» для rigrs.")).size(13.0).weak());
    });
}

pub fn twitch_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Настройки Twitch", |ui| {
        let mut on = app.settings.twitch.enabled;
        ui.horizontal(|ui| {
            ui.label(tr("Канал"));
            ui.add(egui::TextEdit::singleline(&mut app.settings.twitch.channel).hint_text(tr("имя канала")).desired_width(220.0));
        });
        if toggle(ui, &mut on, tr("Подключиться к чату и активировать")).changed() {
            app.actions.push(Action::SetTwitch(on));
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Статус:")).strong());
            match app.twitch.as_ref().map(|t| t.status.clone()) {
                Some(crate::twitch::Status::Connected) => {
                    status_dot(ui, Color32::from_rgb(60, 190, 90));
                    let ch = app.twitch.as_ref().map(|t| t.channel.clone()).unwrap_or_default();
                    let es = match app.twitch.as_ref().and_then(|t| t.eventsub.clone()) {
                        Some(Ok(n)) => format!(" · EventSub: {n} {}", tr("подписок")),
                        Some(Err(e)) => format!(" · EventSub: {e}"),
                        None => String::new(),
                    };
                    ui.label(format!("{} #{ch}{es}", tr("Подключено к чату")));
                }
                Some(crate::twitch::Status::Connecting) => {
                    status_dot(ui, Color32::from_rgb(230, 180, 60));
                    ui.label(tr("Подключение…"));
                }
                Some(crate::twitch::Status::Error(e)) => {
                    status_dot(ui, Color32::from_rgb(230, 80, 80));
                    ui.label(e);
                }
                None => {
                    status_dot(ui, Color32::GRAY);
                    ui.label(tr("Отключено"));
                }
            }
        });
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new(tr("Вход в Twitch (фолловы, награды, Power-Up, отметки, реклама)")).size(16.0)).id_salt("tw-login").show(ui, |ui| {
            ui.label(RichText::new(tr("Создайте приложение на dev.twitch.tv/console (тип клиента «Public», OAuth Redirect: http://localhost) и вставьте его Client ID.")).size(13.0).weak());
            ui.horizontal(|ui| {
                ui.label("Client ID");
                ui.add(egui::TextEdit::singleline(&mut app.settings.twitch.client_id).desired_width(260.0));
                if ui.small_button("dev.twitch.tv").clicked() {
                    app.actions.push(Action::OpenUrl("https://dev.twitch.tv/console/apps/create".into()));
                }
            });
            ui.horizontal(|ui| {
                if app.settings.twitch.login.is_empty() {
                    if ui.button(tr("Войти через Twitch")).clicked() {
                        app.twitch_login();
                    }
                } else {
                    ui.label(format!("{} {}", tr("Вход выполнен:"), app.settings.twitch.login));
                    if ui.button(tr("Выйти")).clicked() {
                        let t = &mut app.settings.twitch;
                        t.access_token.clear();
                        t.refresh_token.clear();
                        t.login.clear();
                        t.user_id.clear();
                        if app.settings.twitch.enabled {
                            app.actions.push(Action::SetTwitch(true));
                        }
                    }
                }
            });
            if let Some(a) = &app.twitch_auth {
                ui.label(RichText::new(a).size(16.0).color(TWITCH).strong());
            }
        });
        ui.add_space(6.0);
        if blue_button(ui, tr("Функции интеграции Twitch"), TWITCH, 40.0).clicked() {
            app.ui.modal = Some(Modal::TwitchFeatures);
            app.ui.modal_sel = None;
        }
    });
}

/// VTS: a pending "load previous model" auto-reset shows a notice (top right) that can cancel it.
pub fn reset_notice(app: &mut App, ctx: &Context) {
    let list: Vec<(String, String, f32)> = app
        .hk
        .resets
        .iter()
        .filter(|r| !r.hide_dialog)
        .filter_map(|r| match &r.undo {
            hotkeys::Undo::Model(_, name) => Some((r.hotkey.clone(), name.clone(), r.due.saturating_duration_since(std::time::Instant::now()).as_secs_f32())),
            _ => None,
        })
        .collect();
    if list.is_empty() {
        return;
    }
    let mut cancel = None;
    egui::Area::new(egui::Id::new("reset-notice")).anchor(Align2::RIGHT_TOP, [-16.0, 16.0]).order(egui::Order::Foreground).show(ctx, |ui| {
        for (id, name, left) in &list {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{} «{name}» {} {:.0} {}", tr("Возврат к модели"), tr("через"), left, tr("с")));
                    if ui.button("✖").on_hover_text(tr("Отменить")).clicked() {
                        cancel = Some(id.clone());
                    }
                });
            });
        }
    });
    if let Some(id) = cancel {
        app.hk.resets.retain(|r| r.hotkey != id);
    }
    ctx.request_repaint();
}
