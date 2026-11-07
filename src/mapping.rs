//! Tracking input → Live2D parameter mapping, compatible with the `ParameterSettings`
//! section of a `.vtube.json` (input range → output range + smoothing).

use serde::{Deserialize, Serialize};

use crate::cubism::model::Model;
use crate::formats::VtubeParam;

pub mod input {
    pub const FACE_POSITION_X: usize = 0;
    pub const FACE_POSITION_Y: usize = 1;
    pub const FACE_POSITION_Z: usize = 2;
    pub const FACE_ANGLE_X: usize = 3;
    pub const FACE_ANGLE_Y: usize = 4;
    pub const FACE_ANGLE_Z: usize = 5;
    pub const MOUTH_SMILE: usize = 6;
    pub const MOUTH_OPEN: usize = 7;
    pub const BROWS: usize = 8;
    pub const TONGUE_OUT: usize = 9;
    pub const EYE_OPEN_LEFT: usize = 10;
    pub const EYE_OPEN_RIGHT: usize = 11;
    pub const EYE_LEFT_X: usize = 12;
    pub const EYE_LEFT_Y: usize = 13;
    pub const EYE_RIGHT_X: usize = 14;
    pub const EYE_RIGHT_Y: usize = 15;
    pub const CHEEK_PUFF: usize = 16;
    pub const FACE_ANGRY: usize = 17;
    pub const BROW_LEFT_Y: usize = 18;
    pub const BROW_RIGHT_Y: usize = 19;
    pub const MOUTH_X: usize = 20;
    pub const MOUSE_POSITION_X: usize = 21;
    pub const MOUSE_POSITION_Y: usize = 22;
    pub const HAND_LEFT_FOUND: usize = 23;
    pub const HAND_RIGHT_FOUND: usize = 24;
    pub const BOTH_HANDS_FOUND: usize = 25;
    pub const HAND_DISTANCE: usize = 26;
    pub const HAND_LEFT_POSITION_X: usize = 27;
    pub const HAND_LEFT_POSITION_Y: usize = 28;
    pub const HAND_LEFT_POSITION_Z: usize = 29;
    pub const HAND_RIGHT_POSITION_X: usize = 30;
    pub const HAND_RIGHT_POSITION_Y: usize = 31;
    pub const HAND_RIGHT_POSITION_Z: usize = 32;
    pub const HAND_LEFT_ANGLE_X: usize = 33;
    pub const HAND_LEFT_ANGLE_Z: usize = 34;
    pub const HAND_RIGHT_ANGLE_X: usize = 35;
    pub const HAND_RIGHT_ANGLE_Z: usize = 36;
    pub const HAND_LEFT_OPEN: usize = 37;
    pub const HAND_RIGHT_OPEN: usize = 38;
    pub const HAND_LEFT_FINGER_1: usize = 39;
    pub const HAND_LEFT_FINGER_2: usize = 40;
    pub const HAND_LEFT_FINGER_3: usize = 41;
    pub const HAND_LEFT_FINGER_4: usize = 42;
    pub const HAND_LEFT_FINGER_5: usize = 43;
    pub const HAND_RIGHT_FINGER_1: usize = 44;
    pub const HAND_RIGHT_FINGER_2: usize = 45;
    pub const HAND_RIGHT_FINGER_3: usize = 46;
    pub const HAND_RIGHT_FINGER_4: usize = 47;
    pub const HAND_RIGHT_FINGER_5: usize = 48;
    pub const COUNT: usize = 49;
}

pub const INPUT_NAMES: [&str; input::COUNT] = [
    "FacePositionX", "FacePositionY", "FacePositionZ", "FaceAngleX", "FaceAngleY", "FaceAngleZ",
    "MouthSmile", "MouthOpen", "Brows", "TongueOut", "EyeOpenLeft", "EyeOpenRight", "EyeLeftX",
    "EyeLeftY", "EyeRightX", "EyeRightY", "CheekPuff", "FaceAngry", "BrowLeftY", "BrowRightY", "MouthX",
    "MousePositionX", "MousePositionY",
    "HandLeftFound", "HandRightFound", "BothHandsFound", "HandDistance",
    "HandLeftPositionX", "HandLeftPositionY", "HandLeftPositionZ",
    "HandRightPositionX", "HandRightPositionY", "HandRightPositionZ",
    "HandLeftAngleX", "HandLeftAngleZ", "HandRightAngleX", "HandRightAngleZ",
    "HandLeftOpen", "HandRightOpen",
    "HandLeftFinger_1_Thumb", "HandLeftFinger_2_Index", "HandLeftFinger_3_Middle", "HandLeftFinger_4_Ring", "HandLeftFinger_5_Pinky",
    "HandRightFinger_1_Thumb", "HandRightFinger_2_Index", "HandRightFinger_3_Middle", "HandRightFinger_4_Ring", "HandRightFinger_5_Pinky",
];

pub type Inputs = [f32; input::COUNT];

/// Values of each input with a neutral face (hands: not found).
pub const NEUTRAL: Inputs = {
    let mut v = [0.0; input::COUNT];
    v[input::MOUTH_SMILE] = 0.5;
    v[input::BROWS] = 0.5;
    v[input::EYE_OPEN_LEFT] = 1.0;
    v[input::EYE_OPEN_RIGHT] = 1.0;
    v[input::BROW_LEFT_Y] = 0.5;
    v[input::BROW_RIGHT_Y] = 0.5;
    v
};

pub fn input_index(name: &str) -> Option<usize> {
    let name = match name {
        "VoiceVolumePlusMouthOpen" => "MouthOpen",
        "VoiceFrequencyPlusMouthSmile" => "MouthSmile",
        n => n,
    };
    INPUT_NAMES.iter().position(|n| *n == name)
}

/// One user-editable mapping rule (serialisable; same meaning as in `.vtube.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    pub input: String,
    pub in_range: [f32; 2],
    pub out_range: [f32; 2],
    pub clamp_input: bool,
    pub clamp_output: bool,
    pub auto_blink: bool,
    pub auto_breath: bool,
    pub output: String,
    pub smoothing: f32,
}

impl From<&VtubeParam> for Rule {
    fn from(p: &VtubeParam) -> Rule {
        Rule {
            name: p.name.clone(),
            input: p.input.clone(),
            in_range: [p.input_range_lower, p.input_range_upper],
            out_range: [p.output_range_lower, p.output_range_upper],
            clamp_input: p.clamp_input,
            clamp_output: p.clamp_output,
            auto_blink: p.use_blinking,
            auto_breath: p.use_breathing,
            output: p.output_live2d.clone(),
            smoothing: p.smoothing,
        }
    }
}

fn rule(name: &str, input: &str, i: [f32; 2], o: [f32; 2], out: &str, s: f32) -> Rule {
    Rule {
        name: name.into(),
        input: input.into(),
        in_range: i,
        out_range: o,
        clamp_input: false,
        clamp_output: false,
        auto_blink: false,
        auto_breath: false,
        output: out.into(),
        smoothing: s,
    }
}

/// Reasonable defaults for models without a `.vtube.json` (standard Cubism parameter IDs).
pub fn default_rules(model: &Model) -> Vec<Rule> {
    let pick = |a: &str, b: &str| -> Option<String> {
        if model.param(a).is_some() {
            Some(a.into())
        } else if model.param(b).is_some() {
            Some(b.into())
        } else {
            None
        }
    };
    let mut v = Vec::new();
    let mut add = |name: &str, input: &str, i, o, ids: (&str, &str), s| {
        if let Some(id) = pick(ids.0, ids.1) {
            v.push(rule(name, input, i, o, &id, s));
        }
    };
    add("Face Left/Right", "FaceAngleX", [-30.0, 30.0], [-30.0, 30.0], ("ParamAngleX", "PARAM_ANGLE_X"), 15.0);
    add("Face Up/Down", "FaceAngleY", [-20.0, 20.0], [-30.0, 30.0], ("ParamAngleY", "PARAM_ANGLE_Y"), 15.0);
    add("Face Lean", "FaceAngleZ", [-30.0, 30.0], [-30.0, 30.0], ("ParamAngleZ", "PARAM_ANGLE_Z"), 15.0);
    add("Body Left/Right", "FaceAngleX", [-30.0, 30.0], [-10.0, 10.0], ("ParamBodyAngleX", "PARAM_BODY_ANGLE_X"), 20.0);
    add("Body Up/Down", "FaceAngleY", [-20.0, 20.0], [-10.0, 10.0], ("ParamBodyAngleY", "PARAM_BODY_ANGLE_Y"), 20.0);
    add("Body Lean", "FaceAngleZ", [-30.0, 30.0], [-10.0, 10.0], ("ParamBodyAngleZ", "PARAM_BODY_ANGLE_Z"), 20.0);
    add("Eye Open Left", "EyeOpenLeft", [0.0, 1.0], [0.0, 1.0], ("ParamEyeLOpen", "PARAM_EYE_L_OPEN"), 10.0);
    add("Eye Open Right", "EyeOpenRight", [0.0, 1.0], [0.0, 1.0], ("ParamEyeROpen", "PARAM_EYE_R_OPEN"), 10.0);
    add("Eye X", "EyeRightX", [-1.0, 1.0], [1.0, -1.0], ("ParamEyeBallX", "PARAM_EYE_BALL_X"), 8.0);
    add("Eye Y", "EyeRightY", [-1.0, 1.0], [-1.0, 1.0], ("ParamEyeBallY", "PARAM_EYE_BALL_Y"), 8.0);
    add("Mouth Open", "MouthOpen", [0.0, 1.0], [0.0, 1.0], ("ParamMouthOpenY", "PARAM_MOUTH_OPEN_Y"), 5.0);
    add("Mouth Smile", "MouthSmile", [0.0, 1.0], [-1.0, 1.0], ("ParamMouthForm", "PARAM_MOUTH_FORM"), 10.0);
    add("Brow Left", "Brows", [0.0, 1.0], [-1.0, 1.0], ("ParamBrowLY", "PARAM_BROW_L_Y"), 10.0);
    add("Brow Right", "Brows", [0.0, 1.0], [-1.0, 1.0], ("ParamBrowRY", "PARAM_BROW_R_Y"), 10.0);
    for r in v.iter_mut() {
        if r.input.starts_with("EyeOpen") {
            r.auto_blink = true;
        }
    }
    if let Some(id) = pick("ParamBreath", "PARAM_BREATH") {
        let mut r = rule("Breath", "", [0.0, 1.0], [0.0, 1.0], &id, 0.0);
        r.auto_breath = true;
        v.push(r);
    }
    v
}

struct Compiled {
    input: Option<usize>,
    param: Option<usize>,
    value: f32,
    primed: bool,
}

pub struct MapCtx {
    pub tracking: bool,
    pub blink: f32,
    pub breath: f32,
}

pub struct Mapping {
    pub rules: Vec<Rule>,
    compiled: Vec<Compiled>,
    /// Last raw input per rule (for the editor's live display).
    pub last_in: Vec<f32>,
}

impl Mapping {
    pub fn new(rules: Vec<Rule>, model: &Model) -> Mapping {
        let mut m = Mapping { rules, compiled: Vec::new(), last_in: Vec::new() };
        m.recompile(model);
        m
    }

    /// Re-resolves input/parameter names (call after editing rules).
    pub fn recompile(&mut self, model: &Model) {
        self.compiled = self
            .rules
            .iter()
            .map(|r| Compiled { input: input_index(&r.input), param: model.param(&r.output), value: 0.0, primed: false })
            .collect();
        self.last_in = vec![0.0; self.rules.len()];
    }

    pub fn apply(&mut self, model: &mut Model, inputs: &Inputs, ctx: &MapCtx, dt: f32) {
        for (i, (r, c)) in self.rules.iter().zip(self.compiled.iter_mut()).enumerate() {
            let Some(param) = c.param else { continue };
            let x = match c.input {
                Some(idx) => {
                    let v = inputs[idx];
                    if r.auto_blink && !ctx.tracking { v.min(ctx.blink) } else { v }
                }
                None if r.auto_breath => ctx.breath,
                None => continue,
            };
            self.last_in[i] = x;
            let span = r.in_range[1] - r.in_range[0];
            let mut t = if span.abs() > 1e-6 { (x - r.in_range[0]) / span } else { 0.0 };
            if r.clamp_input {
                t = t.clamp(0.0, 1.0);
            }
            let mut out = r.out_range[0] + t * (r.out_range[1] - r.out_range[0]);
            if r.clamp_output {
                let (lo, hi) = (r.out_range[0].min(r.out_range[1]), r.out_range[0].max(r.out_range[1]));
                out = out.clamp(lo, hi);
            }
            // VTS smoothing 0..100 → exponential time constant.
            let tau = r.smoothing.max(0.0) * 0.004;
            if !c.primed || tau <= 0.0 {
                c.value = out;
                c.primed = true;
            } else {
                c.value += (out - c.value) * (1.0 - (-dt / tau).exp());
            }
            model.set_param(param, c.value);
        }
    }
}
