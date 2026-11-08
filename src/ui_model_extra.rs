//! Model tab extras (VTS "Customize Model"): per-ArtMesh settings (pin exclusion, item
//! deletion, lighting multiplier, multiply/screen recoloring), ArtMesh groups, and
//! "Copy setup from other model".

use egui::RichText;

use crate::app::App;
use crate::artmesh::{ArtMeshDetails, MeshGroup};
use crate::i18n::tr;
use crate::ui::{section, toggle};

#[derive(Default)]
pub struct ModelExtraState {
    pub filter: String,
    pub group: Option<usize>,
    pub group_name: String,
    pub add_by: String,
    pub copy_from: Option<usize>,
    pub copy: [bool; 8],
    pub rename: bool,
}

fn rgba(c: [f32; 4]) -> egui::Rgba {
    egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

pub fn artmesh_card(app: &mut App, ui: &mut egui::Ui) {
    let Some(key) = app.model_key() else { return };
    let Some(mut d) = app.artmesh_details() else {
        if app.avatar.is_none() {
            return;
        }
        app.settings.model_artmesh.insert(key.clone(), ArtMeshDetails::default());
        return;
    };
    let before = d.clone();
    let mut highlight = None;
    section(ui, "Настройка частей модели (ArtMesh)", |ui| {
        ui.label(RichText::new(tr("Как «Customize Model» в VTS: закрепление предметов, удаление брошенных предметов, влияние освещения, перекраска. Пожалуйста, спросите разрешения художника перед перекраской.")).size(13.0).weak());
        let a = app.avatar.as_ref().unwrap();
        let ids: Vec<String> = a.model.drawables.iter().map(|d| d.id.clone()).collect();
        let st = &mut app.ui_extra;
        // ---- groups
        ui.horizontal(|ui| {
            ui.label(tr("Группа"));
            egui::ComboBox::from_id_salt("mesh-group").selected_text(st.group.and_then(|g| d.model_art_mesh_groups.get(g)).map(|g| g.name.clone()).unwrap_or_else(|| tr("(все части)").into())).show_ui(ui, |ui| {
                if ui.selectable_label(st.group.is_none(), tr("(все части)")).clicked() {
                    st.group = None;
                }
                for (k, g) in d.model_art_mesh_groups.iter().enumerate() {
                    if ui.selectable_label(st.group == Some(k), format!("{} ({})", g.name, g.art_meshes.len())).clicked() {
                        st.group = Some(k);
                    }
                }
            });
            if let Some(g) = st.group {
                if ui.small_button(tr("Удалить группу")).clicked() && g < d.model_art_mesh_groups.len() {
                    d.model_art_mesh_groups.remove(g);
                    st.group = None;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.group_name).hint_text(tr("Новая группа")).desired_width(150.0));
            ui.add(egui::TextEdit::singleline(&mut st.add_by).hint_text(tr("Добавить по ID / UserData (например, hair)")).desired_width(230.0));
            if ui.button(tr("Создать")).clicked() && !st.group_name.trim().is_empty() {
                let f = st.add_by.to_lowercase();
                let tags = &a.ignore_raycast;
                let _ = tags;
                let meshes = if f.is_empty() { Vec::new() } else { ids.iter().filter(|i| i.to_lowercase().contains(&f)).cloned().collect() };
                d.model_art_mesh_groups.push(MeshGroup { name: st.group_name.trim().to_string(), art_meshes: meshes, ..Default::default() });
                st.group = Some(d.model_art_mesh_groups.len() - 1);
                st.group_name.clear();
            }
        });
        ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(tr("Фильтр по ID")).desired_width(f32::INFINITY));
        let f = st.filter.to_lowercase();
        let group = st.group.and_then(|g| d.model_art_mesh_groups.get(g).cloned());
        // ---- bulk color for the visible list
        let shown: Vec<String> = ids
            .iter()
            .filter(|i| f.is_empty() || i.to_lowercase().contains(&f))
            .filter(|i| group.as_ref().map(|g| g.art_meshes.contains(i)).unwrap_or(true))
            .cloned()
            .collect();
        ui.horizontal(|ui| {
            ui.label(format!("{}: {}", tr("Показано"), shown.len()));
            if ui.small_button(tr("Сбросить цвета показанных")).clicked() {
                for i in &shown {
                    d.set_colors(i, [1.0; 4], [0.0, 0.0, 0.0, 1.0]);
                }
            }
            if ui.small_button(tr("Исключить показанные из закрепления")).clicked() {
                for i in &shown {
                    ArtMeshDetails::set_list(&mut d.art_meshes_excluded_from_pinning, i, true);
                }
            }
            if let Some(gk) = st.group {
                if ui.small_button(tr("Добавить отфильтрованные в группу")).clicked() {
                    if let Some(g) = d.model_art_mesh_groups.get_mut(gk) {
                        for i in ids.iter().filter(|i| !f.is_empty() && i.to_lowercase().contains(&f)) {
                            if !g.art_meshes.contains(i) {
                                g.art_meshes.push(i.clone());
                            }
                        }
                    }
                }
            }
        });
        egui::ScrollArea::vertical().id_salt("artmesh-list").max_height(360.0).show(ui, |ui| {
            egui::Grid::new("artmesh-grid").striped(true).num_columns(6).show(ui, |ui| {
                ui.label(RichText::new("ArtMesh").strong());
                ui.label(RichText::new(tr("Закреп.")).strong()).on_hover_text(tr("Предметы могут закрепляться к этой части"));
                ui.label(RichText::new(tr("Удаляет")).strong()).on_hover_text(tr("Брошенные на эту часть предметы удаляются"));
                ui.label(RichText::new(tr("Свет")).strong()).on_hover_text(tr("Насколько наложение освещения влияет на часть"));
                ui.label(RichText::new(tr("Умножение")).strong());
                ui.label(RichText::new(tr("Экран")).strong());
                ui.end_row();
                for id in shown.iter().take(400) {
                    let r = ui.label(id);
                    if r.hovered() {
                        highlight = Some(id.clone());
                    }
                    let mut pin = !d.excluded(id);
                    if ui.checkbox(&mut pin, "").changed() {
                        ArtMeshDetails::set_list(&mut d.art_meshes_excluded_from_pinning, id, !pin);
                    }
                    let mut del = d.deletes(id);
                    if ui.checkbox(&mut del, "").changed() {
                        ArtMeshDetails::set_list(&mut d.art_meshes_that_delete_items_on_drop, id, del);
                    }
                    let mut lm = d.light_mult(id);
                    if ui.add(egui::DragValue::new(&mut lm).speed(0.01).range(0.0..=3.0)).changed() {
                        d.set_light(id, lm);
                    }
                    let (mut m, mut s) = d.colors(id).unwrap_or(([1.0; 4], [0.0, 0.0, 0.0, 1.0]));
                    let mut mc = rgba(m);
                    let mut sc = rgba(s);
                    let c1 = egui::widgets::color_picker::color_edit_button_rgba(ui, &mut mc, egui::widgets::color_picker::Alpha::OnlyBlend).changed();
                    let c2 = egui::widgets::color_picker::color_edit_button_rgba(ui, &mut sc, egui::widgets::color_picker::Alpha::Opaque).changed();
                    if c1 || c2 {
                        let u = mc.to_rgba_unmultiplied();
                        m = u;
                        let v = sc.to_rgba_unmultiplied();
                        s = [v[0], v[1], v[2], 1.0];
                        d.set_colors(id, m, s);
                    }
                    ui.end_row();
                }
            });
        });
    });
    if let Some(a) = app.avatar.as_mut() {
        a.highlight = highlight.and_then(|h| a.drawable_index(&h));
    }
    if d != before {
        app.settings.model_artmesh.insert(key, d);
        app.apply_artmesh();
        app.api_events.push(("ModelConfigChangedEvent".into(), crate::api::model_config_event(app)));
    }
}

/// PARAM_ANGLE_X ↔ ParamAngleX style conversion for "Rename missing parameters".
fn param_key(id: &str) -> String {
    id.chars().filter(|c| *c != '_').collect::<String>().to_lowercase()
}

pub fn copy_card(app: &mut App, ui: &mut egui::Ui) {
    let Some(key) = app.model_key() else { return };
    let models: Vec<(String, std::path::PathBuf, Option<std::path::PathBuf>, Option<std::path::PathBuf>)> =
        app.library.models.iter().filter(|m| m.model3.to_string_lossy() != key).map(|m| (m.name.clone(), m.model3.clone(), m.vtube.clone(), m.icon.clone())).collect();
    let mut go = false;
    section(ui, "Копировать настройки из другой модели", |ui| {
        let st = &mut app.ui_extra;
        egui::ComboBox::from_id_salt("copy-from").selected_text(st.copy_from.and_then(|i| models.get(i)).map(|m| m.0.clone()).unwrap_or_else(|| tr("Выберите модель").into())).width(300.0).show_ui(ui, |ui| {
            for (i, m) in models.iter().enumerate() {
                if ui.selectable_label(st.copy_from == Some(i), &m.0).clicked() {
                    st.copy_from = Some(i);
                }
            }
        });
        let labels = ["Основные настройки", "Физика и ветер", "Настройка параметров", "Горячие клавиши", "Файлы выражений", "Файлы анимаций", "Иконка модели", "Настройки частей модели"];
        for (k, l) in labels.iter().enumerate() {
            toggle(ui, &mut st.copy[k], tr(l));
        }
        toggle(ui, &mut st.rename, tr("Переименовать недостающие параметры"));
        ui.label(RichText::new(tr("Настройки и файлы копируются в текущую модель. Анимации и выражения работают, только если параметры моделей совпадают.")).size(13.0).weak());
        go = ui.add_enabled(st.copy_from.is_some() && st.copy.iter().any(|b| *b), egui::Button::new(tr("Копировать"))).clicked();
    });
    if !go {
        return;
    }
    let Some((name, src3, src_vt, src_icon)) = app.ui_extra.copy_from.and_then(|i| models.get(i).cloned()) else { return };
    let skey = src3.to_string_lossy().into_owned();
    let c = app.ui_extra.copy;
    let rename = app.ui_extra.rename;
    let src_json = src_vt.as_ref().and_then(|p| crate::formats::read_json::<crate::formats::VtubeJson>(p).ok());
    let dst_dir = app.avatar.as_ref().map(|a| a.dir.clone()).unwrap_or_default();
    let src_dir = src3.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let target_params: Vec<String> = app.avatar.as_ref().map(|a| a.model.param_ids.clone()).unwrap_or_default();
    let mut done = Vec::new();
    if c[0] || c[1] {
        let mut cfg = app.settings.model_cfg.get(&key).cloned().unwrap_or_default();
        let src_cfg = app.settings.model_cfg.get(&skey).cloned().unwrap_or_default();
        if c[0] {
            cfg.lost_idle = src_cfg.lost_idle.clone();
            cfg.move_enabled = src_cfg.move_enabled;
            cfg.move_xyz = src_cfg.move_xyz;
            if let Some(i) = app.settings.idle_override.get(&skey).cloned() {
                app.settings.idle_override.insert(key.clone(), i);
            }
            done.push(tr("основные настройки"));
        }
        if c[1] {
            cfg.physics_on = src_cfg.physics_on;
            cfg.physics_fps = src_cfg.physics_fps;
            cfg.physics_strength = src_cfg.physics_strength;
            done.push(tr("физика"));
        }
        app.settings.model_cfg.insert(key.clone(), cfg.clone());
        if let Some(a) = app.avatar.as_mut() {
            a.apply_cfg(cfg);
        }
    }
    if c[2] {
        let mut rules = app.settings.rules_override.get(&skey).cloned().unwrap_or_else(|| src_json.as_ref().map(|v| v.parameter_settings.iter().map(crate::mapping::Rule::from).collect()).unwrap_or_default());
        if rename {
            for r in &mut rules {
                if !target_params.contains(&r.output) {
                    if let Some(t) = target_params.iter().find(|t| param_key(t) == param_key(&r.output)) {
                        r.output = t.clone();
                    }
                }
            }
        }
        app.settings.rules_override.insert(key.clone(), rules.clone());
        if let Some(a) = app.avatar.as_mut() {
            a.mapping.rules = rules;
            a.mapping.recompile(&a.model);
        }
        done.push(tr("параметры"));
    }
    if c[3] {
        let hk = app.settings.model_hotkeys.get(&skey).cloned().unwrap_or_else(|| src_json.as_ref().map(|v| v.hotkeys.clone()).unwrap_or_default());
        let fresh: Vec<_> = hk.into_iter().map(|mut h| {
            h.id = crate::hotkeys::new_id();
            h
        }).collect();
        app.settings.model_hotkeys.insert(key.clone(), fresh);
        done.push(tr("горячие клавиши"));
    }
    let copy_ext = |ext: &str| -> usize {
        let mut n = 0;
        for e in walk(&src_dir) {
            if e.to_string_lossy().to_lowercase().ends_with(ext) {
                let rel = e.strip_prefix(&src_dir).unwrap_or(&e);
                let dst = dst_dir.join(rel);
                if !dst.exists() {
                    if let Some(p) = dst.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    if std::fs::copy(&e, &dst).is_ok() {
                        n += 1;
                    }
                }
            }
        }
        n
    };
    if c[4] {
        let n = copy_ext(".exp3.json");
        done.push(format!("{} {n}", tr("выражений:")).leak());
    }
    if c[5] {
        let n = copy_ext(".motion3.json");
        done.push(format!("{} {n}", tr("анимаций:")).leak());
    }
    if c[6] {
        if let Some(icon) = &src_icon {
            if let Some(fname) = icon.file_name() {
                let _ = std::fs::copy(icon, dst_dir.join(fname));
                done.push(tr("иконка"));
            }
        }
    }
    if c[7] {
        let d = app.settings.model_artmesh.get(&skey).cloned().or_else(|| src_json.as_ref().and_then(|v| v.art_mesh_details.clone())).unwrap_or_default();
        app.settings.model_artmesh.insert(key.clone(), d);
        app.apply_artmesh();
        done.push(tr("части модели"));
    }
    app.rescan();
    app.save();
    app.toast(format!("{} «{name}»: {}", tr("Скопировано из"), done.join(", ")));
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
    }
    out
}
