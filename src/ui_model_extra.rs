//! Model tab extras (VTS "Customize Model"): per-ArtMesh settings (pin exclusion, item
//! deletion, lighting multiplier, multiply/screen recoloring), ArtMesh groups, and
//! "Copy setup from other model".

use egui::RichText;

use crate::app::App;
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
