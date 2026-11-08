//! Item config window (VTS item menu): order, flip, lock, censor, smoothing, FPS,
//! between-layer insertion, Live2D item split, pinning options and Live2D item tracking.

use egui::{Align2, Context, RichText};

use crate::app::{Action, App};
use crate::i18n::tr;
use crate::scene::{Body, LAYER_BACK, LAYER_FRONT};

fn layer_label(v: &str) -> String {
    match v {
        "" => tr("(как по порядку)").to_string(),
        LAYER_FRONT => tr("Всегда перед моделью").to_string(),
        LAYER_BACK => tr("Всегда за моделью").to_string(),
        id => id.to_string(),
    }
}

/// Layer selector: Front / Back / any ArtMesh of the main model.
fn layer_combo(ui: &mut egui::Ui, salt: &str, value: &mut String, meshes: &[String], allow_none: bool) -> bool {
    let mut changed = false;
    let missing = !value.is_empty() && !value.starts_with("__") && !meshes.contains(value);
    let text = if missing { format!("⚠ {}", layer_label(value)) } else { layer_label(value) };
    egui::ComboBox::from_id_salt(salt).selected_text(text).width(260.0).height(380.0).show_ui(ui, |ui| {
        let mut opts: Vec<String> = Vec::new();
        if allow_none {
            opts.push(String::new());
        }
        opts.push(LAYER_FRONT.into());
        opts.push(LAYER_BACK.into());
        opts.extend(meshes.iter().rev().cloned());
        for o in opts {
            if ui.selectable_label(*value == o, layer_label(&o)).clicked() {
                *value = o;
                changed = true;
            }
        }
    });
    if missing {
        ui.label(RichText::new(tr("слоя нет в этой модели")).small().weak());
    }
    changed
}

pub fn item_config(app: &mut App, ctx: &Context) {
    let Some(id) = app.ui.selected else { return };
    let Some(idx) = app.items.iter().position(|i| i.id == id) else {
        app.ui.selected = None;
        return;
    };
    // Main model ArtMeshes in render order (back → front).
    let meshes: Vec<String> = app
        .avatar
        .as_ref()
        .map(|a| (0..a.order_len()).filter_map(|p| a.order_at(p)).map(|d| a.model.drawables[d].id.clone()).collect())
        .unwrap_or_default();
    let mut open = true;
    let mut remove = false;
    let mut repin = false;
    let mut fps_changed = None;
    egui::Window::new(format!("{}: {}", tr("Предмет"), app.items[idx].name))
        .id(egui::Id::new("item-panel"))
        .open(&mut open)
        .collapsible(true)
        .resizable(false)
        .default_width(380.0)
        .anchor(Align2::LEFT_BOTTOM, [96.0, -120.0])
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(ctx.screen_rect().height() * 0.7).show(ui, |ui| {
                let it = &mut app.items[idx];
                ui.label(RichText::new(if it.pin.is_some() { tr("📌 Закреплён на модели (перетащите, чтобы перевесить)") } else { tr("Не закреплён — положите предмет на модель") }).weak());
                ui.horizontal(|ui| {
                    ui.label(tr("Порядок"));
                    let mut o = it.cfg.order;
                    if ui.add(egui::DragValue::new(&mut o).range(-100..=100)).on_hover_text(tr("Модель на уровне 0: больше — впереди, отрицательные — за моделью")).changed() {
                        it.cfg.order = if o == 0 { if it.cfg.order > 0 { -1 } else { 1 } } else { o };
                    }
                    ui.checkbox(&mut it.xf.flip, tr("Отразить"));
                    ui.checkbox(&mut it.locked, tr("Заблокировать"));
                    ui.checkbox(&mut it.cfg.censor, tr("Цензура"));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Размер"));
                    let mut s = it.xf.scale;
                    if ui.add(egui::DragValue::new(&mut s).speed(0.01).range(0.01..=50.0)).changed() {
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
                ui.horizontal(|ui| {
                    ui.label(tr("Сглаживание"));
                    ui.add(egui::Slider::new(&mut it.cfg.smoothing, 0.0..=1.0)).on_hover_text(tr("Работает, когда предмет закреплён и заблокирован"));
                });
                if let Body::Sprite(s) = &it.body {
                    if s.frame_count() > 1 {
                        ui.horizontal(|ui| {
                            ui.label(tr("FPS анимации"));
                            if ui.add(egui::Slider::new(&mut it.cfg.fps, 0.0..=60.0)).on_hover_text(tr("0 — как в файле")).changed() {
                                fps_changed = Some((it.source.to_string_lossy().into_owned(), it.cfg.fps));
                            }
                        });
                    }
                }
                ui.separator();
                let mut in_model = !it.cfg.layer.is_empty();
                if ui.checkbox(&mut in_model, tr("Поместить предмет в модель")).changed() {
                    it.cfg.layer = if in_model { meshes.last().cloned().unwrap_or_else(|| LAYER_FRONT.to_string()) } else { String::new() };
                }
                if in_model {
                    ui.horizontal(|ui| {
                        ui.label(tr("Над слоем"));
                        let mut v = it.cfg.layer.clone();
                        if layer_combo(ui, "item-layer", &mut v, &meshes, false) {
                            it.cfg.layer = v;
                        }
                    });
                }
                let item_meshes: Vec<String> = match &it.body {
                    Body::Live2D(l) => (0..l.order_len()).filter_map(|p| l.order_at(p)).map(|d| l.model.drawables[d].id.clone()).collect(),
                    _ => Vec::new(),
                };
                if !item_meshes.is_empty() {
                    ui.separator();
                    ui.label(RichText::new(tr("Live2D-предмет")).strong());
                    ui.horizontal(|ui| {
                        ui.label(tr("Точка разделения"));
                        egui::ComboBox::from_id_salt("item-split").selected_text(if it.cfg.split.is_empty() { tr("(нет)").to_string() } else { it.cfg.split.clone() }).height(380.0).show_ui(ui, |ui| {
                            if ui.selectable_label(it.cfg.split.is_empty(), tr("(нет)")).clicked() {
                                it.cfg.split.clear();
                            }
                            for m in item_meshes.iter().rev() {
                                if ui.selectable_label(&it.cfg.split == m, m).clicked() {
                                    it.cfg.split = m.clone();
                                }
                            }
                        });
                    });
                    if !it.cfg.split.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label(tr("Передняя часть"));
                            let mut v = it.cfg.split_front.clone();
                            if layer_combo(ui, "item-split-front", &mut v, &meshes, true) {
                                it.cfg.split_front = v;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("Задняя часть"));
                            let mut v = it.cfg.split_back.clone();
                            if layer_combo(ui, "item-split-back", &mut v, &meshes, true) {
                                it.cfg.split_back = v;
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr("Множители углов X/Y/Z"));
                        for k in 0..3 {
                            ui.add(egui::DragValue::new(&mut it.cfg.angle_mult[k]).speed(0.01).range(-3.0..=3.0));
                        }
                    });
                    ui.checkbox(&mut it.cfg.default_unpinned, tr("Возвращать к умолчанию, когда не закреплён"));
                    ui.checkbox(&mut it.cfg.normal_hotkeys, tr("Использовать обычные горячие клавиши предмета"));
                }
                ui.separator();
                ui.label(RichText::new(tr("Закрепление")).strong());
                ui.checkbox(&mut it.cfg.pin_only_below, tr("Закреплять только к слоям ниже предмета"));
                let mut only = !it.cfg.pin_only_layer.is_empty();
                if ui.checkbox(&mut only, tr("Закреплять только к определённому слою")).changed() {
                    it.cfg.pin_only_layer = if only { meshes.last().cloned().unwrap_or_default() } else { String::new() };
                }
                if only {
                    egui::ComboBox::from_id_salt("item-pin-layer").selected_text(it.cfg.pin_only_layer.clone()).height(380.0).show_ui(ui, |ui| {
                        for m in meshes.iter().rev() {
                            if ui.selectable_label(&it.cfg.pin_only_layer == m, m).clicked() {
                                it.cfg.pin_only_layer = m.clone();
                            }
                        }
                    });
                }
                ui.checkbox(&mut it.cfg.pin_notify, tr("Показывать уведомления о закреплении"));
                ui.horizontal(|ui| {
                    if ui.button(tr("📌 Перезакрепить здесь")).clicked() {
                        repin = true;
                    }
                    if ui.button(tr("Открепить")).clicked() {
                        it.pin = None;
                    }
                    if ui.button(tr("🗑 Удалить")).clicked() {
                        remove = true;
                    }
                });
            });
        });
    if let Some((src, fps)) = fps_changed {
        app.settings.item_fps.insert(src, fps);
    }
    if repin {
        let details = app.model_key().and_then(|k| app.settings.model_artmesh.get(&k).cloned());
        if let (Some(a), Some(it)) = (&app.avatar, app.items.get_mut(idx)) {
            it.pin = None;
            match it.pin_to_filtered(a, details.as_ref().or(a.vts_artmesh.as_ref())) {
                Some(mesh) if it.cfg.pin_notify || app.settings.item_show_pin_notifications => app.toasts.push((format!("{} {mesh}", tr("Закреплено к слою")), std::time::Instant::now())),
                None => app.toasts.push((tr("Под предметом нет подходящего слоя модели").to_string(), std::time::Instant::now())),
                _ => {}
            }
        }
    }
    if remove {
        app.actions.push(Action::RemoveItem(id));
    }
    if !open || remove {
        app.ui.selected = None;
    }
}
