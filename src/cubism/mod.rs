pub mod core;
pub mod model;
pub mod motion;
pub mod physics;
pub mod pose;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::core::Core;
    use super::model::Model;
    use super::motion::Motion;
    use super::physics::Physics;
    use crate::formats::*;
    use crate::mapping::{default_rules, MapCtx, Mapping, Rule, NEUTRAL};

    #[test]
    fn load_and_simulate_all_vts_models() {
        let Some(root) = crate::settings::detect_vts_dir() else {
            eprintln!("skipped: VTube Studio not found");
            return;
        };
        let vts_dir = root.join("VTube Studio_Data").to_string_lossy().into_owned();
        #[allow(non_snake_case)]
        let VTS = vts_dir.as_str();
        let core_path = Path::new(VTS).join(r"Plugins\x86_64\Live2DCubismCore.dll");
        if !core_path.exists() {
            eprintln!("skipped: no core");
            return;
        }
        let core: &'static Core = Box::leak(Box::new(Core::load(&core_path).unwrap()));
        println!("core {}", core.version_string());
        let lib = crate::library::scan(Some(&Path::new(VTS).join("StreamingAssets")), Path::new("."), &[]);
        eprintln!("models {} items {}", lib.models.len(), lib.items.len());
        assert!(!lib.models.is_empty());
        for e in lib.models.iter().chain(lib.items.iter().filter_map(|i| match i {
            crate::library::ItemEntry::Live2D(m) => Some(m),
            _ => None,
        })) {
            let dir = e.model3.parent().unwrap();
            let m3: Model3 = read_json(&e.model3).unwrap();
            let moc = std::fs::read(dir.join(&m3.file_references.moc)).unwrap();
            let mut model = Model::load(core, &moc).unwrap_or_else(|err| panic!("{}: {err:#}", e.name));
            let mut phys = m3.file_references.physics.as_ref().map(|p| Physics::new(&read_json(&dir.join(p)).unwrap(), &model));
            let vt: Option<VtubeJson> = e.vtube.as_ref().map(|p| read_json(p).unwrap());
            let rules: Vec<Rule> = match &vt {
                Some(v) if !v.parameter_settings.is_empty() => v.parameter_settings.iter().map(Rule::from).collect(),
                _ => default_rules(&model),
            };
            let mut map = Mapping::new(rules, &model);
            let mut motion = vt
                .as_ref()
                .map(|v| v.file_references.idle_animation.clone())
                .filter(|s| !s.is_empty())
                .and_then(|f| {
                    walk(dir, &f).map(|p| Motion::new(&read_json(&p).unwrap(), &model))
                });
            let mut inputs = NEUTRAL;
            let t0 = std::time::Instant::now();
            for f in 0..300 {
                let t = f as f32 / 60.0;
                inputs[crate::mapping::input::FACE_ANGLE_X] = (t * 3.0).sin() * 25.0;
                inputs[crate::mapping::input::FACE_ANGLE_Y] = (t * 2.0).cos() * 15.0;
                model.reset_pose();
                if let Some(m) = &mut motion {
                    m.apply(&mut model, 1.0 / 60.0);
                }
                map.apply(&mut model, &inputs, &MapCtx { tracking: true, blink: 1.0, breath: 0.5 }, 1.0 / 60.0);
                if let Some(p) = &mut phys {
                    p.evaluate(&mut model, 1.0 / 60.0);
                }
                model.update();
            }
            let per_frame = t0.elapsed().as_secs_f64() * 1000.0 / 300.0;
            assert!(model.params().iter().all(|v| v.is_finite()), "{}: NaN param", e.name);
            for i in 0..model.drawables.len() {
                assert!(model.positions(i).iter().all(|v| v.x.is_finite() && v.y.is_finite()), "{}: NaN vertex", e.name);
            }
            eprintln!(
                "{:<28} drawables {:4} params {:4} physics {:3} idle {:5}  {:.3} ms/frame",
                e.name,
                model.drawables.len(),
                model.param_ids.len(),
                phys.as_ref().map(|p| p.chain_count()).unwrap_or(0),
                motion.is_some(),
                per_frame
            );
        }
        eprintln!("ALL DONE");
    }

    fn walk(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = walk(&p, name) {
                    return Some(f);
                }
            } else if p.file_name().map(|n| n.to_string_lossy() == name).unwrap_or(false) {
                return Some(p);
            }
        }
        None
    }
}
