//! Per-ArtMesh model settings in the VTube Studio format (`ArtMeshDetails` in `.vtube.json`):
//! pin exclusion, "delete items dropped here", lighting multipliers, multiply/screen colors,
//! ArtMesh groups. Plus `vts_ignore_raycast` tags from the model's userdata3.json.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct IdValue {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Value")]
    pub value: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct MeshGroup {
    pub name: String,
    pub art_meshes: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct ArtMeshDetails {
    pub art_meshes_excluded_from_pinning: Vec<String>,
    pub art_meshes_that_delete_items_on_drop: Vec<String>,
    pub art_mesh_scene_lighting_multipliers: Vec<IdValue>,
    pub art_mesh_multiply_and_screen_colors: Vec<IdValue>,
    pub model_art_mesh_groups: Vec<MeshGroup>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn hex_rgba(s: &str) -> Option<[f32; 4]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|b| b as f32 / 255.0);
    Some([v(0)?, v(2)?, v(4)?, if s.len() >= 8 { v(6)? } else { 1.0 }])
}

pub fn rgba_hex(c: [f32; 4]) -> String {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{:02X}{:02X}{:02X}{:02X}", b(c[0]), b(c[1]), b(c[2]), b(c[3]))
}

/// "MMMMMMAA|SSSSSSAA" → (multiply, screen). VTS stores the multiply color first.
pub fn parse_colors(v: &Value) -> Option<([f32; 4], [f32; 4])> {
    let s = v.as_str()?;
    let (m, sc) = s.split_once('|')?;
    Some((hex_rgba(m)?, hex_rgba(sc)?))
}

pub fn format_colors(mul: [f32; 4], scr: [f32; 4]) -> Value {
    Value::String(format!("{}|{}", rgba_hex(mul), rgba_hex(scr)))
}

impl ArtMeshDetails {
    pub fn excluded(&self, id: &str) -> bool {
        id.contains("vts_ignore_raycast") || self.art_meshes_excluded_from_pinning.iter().any(|x| x == id)
    }
    pub fn deletes(&self, id: &str) -> bool {
        self.art_meshes_that_delete_items_on_drop.iter().any(|x| x == id)
    }
    pub fn light_mult(&self, id: &str) -> f32 {
        self.art_mesh_scene_lighting_multipliers
            .iter()
            .find(|x| x.id == id)
            .and_then(|x| x.value.as_f64().or_else(|| x.value.as_str().and_then(|s| s.parse().ok())))
            .map(|v| v as f32)
            .unwrap_or(1.0)
    }
    pub fn colors(&self, id: &str) -> Option<([f32; 4], [f32; 4])> {
        self.art_mesh_multiply_and_screen_colors.iter().find(|x| x.id == id).and_then(|x| parse_colors(&x.value))
    }
    pub fn set_list(list: &mut Vec<String>, id: &str, on: bool) {
        list.retain(|x| x != id);
        if on {
            list.push(id.to_string());
        }
    }
    pub fn set_light(&mut self, id: &str, v: f32) {
        self.art_mesh_scene_lighting_multipliers.retain(|x| x.id != id);
        if (v - 1.0).abs() > 1e-3 {
            self.art_mesh_scene_lighting_multipliers.push(IdValue { id: id.into(), value: serde_json::json!(v) });
        }
    }
    pub fn set_colors(&mut self, id: &str, mul: [f32; 4], scr: [f32; 4]) {
        self.art_mesh_multiply_and_screen_colors.retain(|x| x.id != id);
        let neutral = mul == [1.0, 1.0, 1.0, 1.0] && scr[..3] == [0.0, 0.0, 0.0];
        if !neutral {
            self.art_mesh_multiply_and_screen_colors.push(IdValue { id: id.into(), value: format_colors(mul, scr) });
        }
    }
}

/// `vts_ignore_raycast` tags from `<model>.userdata3.json` (ArtMesh ids).
pub fn userdata_ignored(model3: &std::path::Path, user_data_file: Option<&str>) -> Vec<String> {
    let dir = model3.parent().unwrap_or(std::path::Path::new("."));
    let path = match user_data_file {
        Some(f) => dir.join(f),
        None => {
            let stem = model3.file_name().map(|n| n.to_string_lossy().trim_end_matches(".model3.json").to_string()).unwrap_or_default();
            dir.join(format!("{stem}.userdata3.json"))
        }
    };
    let Ok(v) = crate::formats::read_json::<Value>(&path) else { return Vec::new() };
    v["UserData"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| u["Target"].as_str() == Some("ArtMesh") && u["Value"].as_str().map(|s| s.contains("vts_ignore_raycast")).unwrap_or(false))
        .filter_map(|u| u["Id"].as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_roundtrip() {
        let (m, s) = parse_colors(&Value::String("FFFFFF00|000000FF".into())).unwrap();
        assert_eq!(m, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(s, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(format_colors(m, s), Value::String("FFFFFF00|000000FF".into()));
    }

    #[test]
    fn vts_block_parses() {
        let j = r#"{"ArtMeshesExcludedFromPinning":["A"],"ArtMeshesThatDeleteItemsOnDrop":["Mouth"],"ArtMeshSceneLightingMultipliers":[{"ID":"Eye","Value":0.5}],"ArtMeshMultiplyAndScreenColors":[{"ID":"Hair","Value":"FF000000|00000000"}],"ModelArtMeshGroups":[]}"#;
        let d: ArtMeshDetails = serde_json::from_str(j).unwrap();
        assert!(d.excluded("A") && d.excluded("x_vts_ignore_raycast") && !d.excluded("B"));
        assert!(d.deletes("Mouth"));
        assert_eq!(d.light_mult("Eye"), 0.5);
        assert_eq!(d.colors("Hair").unwrap().0, [1.0, 0.0, 0.0, 0.0]);
    }
}
