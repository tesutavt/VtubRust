//! pose3.json: mutually exclusive part groups (e.g. alternate arm poses).
//! Simplified: switches instantly instead of cross-fading.

use crate::formats::Pose3;

use super::model::Model;

struct Entry {
    part: usize,
    param: Option<usize>,
    links: Vec<usize>,
}

pub struct Pose {
    groups: Vec<Vec<Entry>>,
}

impl Pose {
    pub fn new(json: &Pose3, model: &Model) -> Pose {
        let groups = json
            .groups
            .iter()
            .map(|g| {
                g.iter()
                    .filter_map(|p| {
                        Some(Entry {
                            part: model.part(&p.id)?,
                            param: model.param(&p.id),
                            links: p.link.iter().filter_map(|l| model.part(l)).collect(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|g| !g.is_empty())
            .collect();
        Pose { groups }
    }

    pub fn apply(&self, model: &mut Model) {
        for g in &self.groups {
            let visible = g
                .iter()
                .position(|e| e.param.map(|p| model.params()[p] > 0.001).unwrap_or(false))
                .unwrap_or(0);
            for (i, e) in g.iter().enumerate() {
                let o = if i == visible { 1.0 } else { 0.0 };
                let parts = model.parts_mut();
                parts[e.part] = o;
                for &l in &e.links {
                    parts[l] = o;
                }
            }
        }
    }
}
