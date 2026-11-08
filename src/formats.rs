//! Serde definitions for the public Live2D JSON formats and the model-side `.vtube.json`
//! config (read only, for interoperability with existing avatars).

use std::collections::HashMap;

use serde::Deserialize;

// ---------------------------------------------------------------- model3.json

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Model3 {
    pub file_references: Model3Files,
    #[serde(default)]
    pub groups: Vec<Model3Group>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Model3Files {
    pub moc: String,
    #[serde(default)]
    pub textures: Vec<String>,
    pub physics: Option<String>,
    pub pose: Option<String>,
    #[serde(default)]
    pub motions: HashMap<String, Vec<Model3Motion>>,
    #[serde(default)]
    pub expressions: Vec<Model3Expression>,
    pub user_data: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Model3Expression {
    #[serde(default)]
    pub name: String,
    pub file: String,
}

// ---------------------------------------------------------------- exp3.json

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Exp3 {
    #[serde(default)]
    pub parameters: Vec<Exp3Param>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Exp3Param {
    pub id: String,
    pub value: f32,
    #[serde(default)]
    pub blend: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Model3Motion {
    pub file: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Model3Group {
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ids: Vec<String>,
}

// ---------------------------------------------------------------- physics3.json

#[derive(Deserialize, Debug, Clone, Copy, Default)]
#[serde(rename_all = "PascalCase")]
pub struct JVec2 {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Physics3 {
    #[serde(default)]
    pub meta: PhysicsMeta,
    #[serde(default)]
    pub physics_settings: Vec<PhysicsSetting>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicsMeta {
    pub effective_forces: Option<EffectiveForces>,
    pub fps: Option<f32>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "PascalCase")]
pub struct EffectiveForces {
    #[serde(default)]
    pub gravity: JVec2,
    #[serde(default)]
    pub wind: JVec2,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicsSetting {
    #[serde(default)]
    pub input: Vec<PhysicsInput>,
    #[serde(default)]
    pub output: Vec<PhysicsOutput>,
    #[serde(default)]
    pub vertices: Vec<PhysicsVertex>,
    pub normalization: PhysicsNormalization,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicsTarget {
    pub id: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicsInput {
    pub source: PhysicsTarget,
    pub weight: f32,
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(default)]
    pub reflect: bool,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicsOutput {
    pub destination: PhysicsTarget,
    pub vertex_index: usize,
    pub scale: f32,
    pub weight: f32,
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(default)]
    pub reflect: bool,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicsVertex {
    #[serde(default)]
    pub position: JVec2,
    pub mobility: f32,
    pub delay: f32,
    pub acceleration: f32,
    pub radius: f32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicsNormalization {
    pub position: NormRange,
    pub angle: NormRange,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "PascalCase")]
pub struct NormRange {
    pub minimum: f32,
    pub default: f32,
    pub maximum: f32,
}

// ---------------------------------------------------------------- motion3.json

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Motion3 {
    pub meta: MotionMeta,
    #[serde(default)]
    pub curves: Vec<MotionCurve>,
    /// Animation events ("Add Event" in the Cubism Animator, exported with "Export Event").
    #[serde(default)]
    pub user_data: Vec<MotionUserData>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct MotionUserData {
    #[serde(default)]
    pub time: f32,
    #[serde(default)]
    pub value: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct MotionMeta {
    pub duration: f32,
    #[serde(default = "yes")]
    pub r#loop: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct MotionCurve {
    pub target: String,
    pub id: String,
    pub segments: Vec<f32>,
}

// ---------------------------------------------------------------- pose3.json

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Pose3 {
    #[serde(default)]
    pub groups: Vec<Vec<PosePart>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PosePart {
    pub id: String,
    #[serde(default)]
    pub link: Vec<String>,
}

// ---------------------------------------------------------------- .vtube.json (subset)

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct VtubeJson {
    pub name: String,
    #[serde(rename = "ModelID")]
    pub model_id: String,
    pub hotkeys: Vec<crate::hotkeys::Hotkey>,
    pub art_mesh_details: Option<crate::artmesh::ArtMeshDetails>,
    pub general_settings: Option<serde_json::Value>,
    pub file_references: VtubeFiles,
    pub parameter_settings: Vec<VtubeParam>,
    pub physics_settings: Option<VtubePhysics>,
    pub model_position_movement: Option<VtubeMovement>,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "PascalCase", default)]
pub struct VtubeMovement {
    #[serde(rename = "Use")]
    pub enabled: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct VtubeFiles {
    pub model: String,
    pub icon: String,
    pub idle_animation: String,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct VtubePhysics {
    #[serde(rename = "Use")]
    pub enabled: bool,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "PascalCase", default)]
pub struct VtubeParam {
    pub name: String,
    pub input: String,
    pub input_range_lower: f32,
    pub input_range_upper: f32,
    pub output_range_lower: f32,
    pub output_range_upper: f32,
    pub clamp_input: bool,
    pub clamp_output: bool,
    pub use_blinking: bool,
    pub use_breathing: bool,
    #[serde(rename = "OutputLive2D")]
    pub output_live2d: String,
    pub smoothing: f32,
}

/// Reads a JSON file, tolerating a UTF-8 BOM (common in files written by Unity/.NET).
pub fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> anyhow::Result<T> {
    use anyhow::Context;
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    serde_json::from_slice(bytes).with_context(|| format!("parsing {}", path.display()))
}
