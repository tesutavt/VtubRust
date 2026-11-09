//! "Perfect Sync" / VBridger-compatible tracking inputs.
//!
//! Trackers that deliver ARKit blendshapes (iPhone / Android over the VTube Studio phone
//! protocol, NVIDIA Broadcast via VTS's `ExpressionApp.exe`, Mediapipe) expose:
//!
//! * the raw ARKit blendshapes under their ARKit names (`EyeBlinkLeft`, `JawOpen`,
//!   `MouthFunnel`, ...), and
//! * the combined parameters VBridger rigs are built around (`MouthPressLipOpen`,
//!   `MouthPuckerWiden`, `MouthShrug`, `CheekSquint`, ...).
//!
//! They work like plugin-created custom parameters: a model's `.vtube.json` can map them to
//! Live2D parameters by name, so a model rigged for VBridger moves without VBridger running.
//! If VBridger itself is connected through the plugin API and creates a parameter with the
//! same name, the plugin value wins (see `mapping::Mapping::apply`).
//!
//! "Left"/"Right" follow the rest of the app: with mirror mode on they refer to the avatar's
//! side, so the ARKit left/right values are swapped.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::tracking::{FaceFrame, FaceKind};

/// ARKit blendshape names in Mediapipe blendshape order (index 0 = Mediapipe `_neutral`,
/// unused). Names are capitalised the way VTube Studio's phone app sends them.
pub const ARKIT_NAMES: [&str; 52] = [
    "", "BrowDownLeft", "BrowDownRight", "BrowInnerUp", "BrowOuterUpLeft", "BrowOuterUpRight", "CheekPuff",
    "CheekSquintLeft", "CheekSquintRight", "EyeBlinkLeft", "EyeBlinkRight", "EyeLookDownLeft", "EyeLookDownRight",
    "EyeLookInLeft", "EyeLookInRight", "EyeLookOutLeft", "EyeLookOutRight", "EyeLookUpLeft", "EyeLookUpRight",
    "EyeSquintLeft", "EyeSquintRight", "EyeWideLeft", "EyeWideRight", "JawForward", "JawLeft", "JawOpen", "JawRight",
    "MouthClose", "MouthDimpleLeft", "MouthDimpleRight", "MouthFrownLeft", "MouthFrownRight", "MouthFunnel",
    "MouthLeft", "MouthLowerDownLeft", "MouthLowerDownRight", "MouthPressLeft", "MouthPressRight", "MouthPucker",
    "MouthRight", "MouthRollLower", "MouthRollUpper", "MouthShrugLower", "MouthShrugUpper", "MouthSmileLeft",
    "MouthSmileRight", "MouthStretchLeft", "MouthStretchRight", "MouthUpperUpLeft", "MouthUpperUpRight",
    "NoseSneerLeft", "NoseSneerRight",
];

/// Index into `ARKIT_NAMES` / `FaceFrame::bs` for an ARKit blendshape name (case-insensitive,
/// accepts `eyeBlink_L` style too).
pub fn arkit_index(name: &str) -> Option<usize> {
    let n = if let Some(s) = name.strip_suffix("_L") {
        format!("{s}Left")
    } else if let Some(s) = name.strip_suffix("_R") {
        format!("{s}Right")
    } else {
        name.to_string()
    };
    ARKIT_NAMES.iter().position(|a| !a.is_empty() && a.eq_ignore_ascii_case(&n))
}

/// VBridger-style combined parameters: (name, min, max, explanation).
pub const VB_PARAMS: [(&str, f32, f32, &str); 6] = [
    ("MouthPressLipOpen", -1.0, 1.0, "Lips pressed together (-1) to lips apart / teeth showing (+1)"),
    ("MouthPuckerWiden", -1.0, 1.0, "Mouth puckered (-1) to wide / stretched (+1)"),
    ("MouthShrug", 0.0, 1.0, "Lower and upper lip shrug"),
    ("CheekSquint", 0.0, 1.0, "Both cheeks squinting"),
    ("EyeSquint", 0.0, 1.0, "Both eyes squinting"),
    ("BrowOuterUp", 0.0, 1.0, "Both outer brows raised"),
];

/// Every input name this module provides (raw ARKit except the two VTS already has as
/// default inputs, then the VBridger combinations).
pub fn input_names() -> Vec<&'static str> {
    let mut v: Vec<&str> = ARKIT_NAMES.iter().copied().filter(|n| !n.is_empty() && *n != "CheekPuff").collect();
    v.extend(VB_PARAMS.iter().map(|p| p.0));
    v
}

pub fn range_of(name: &str) -> (f32, f32, f32) {
    VB_PARAMS.iter().find(|p| p.0 == name).map(|p| (p.1, p.2, 0.0)).unwrap_or((0.0, 1.0, 0.0))
}

static VALUES: Mutex<Option<HashMap<&'static str, f32>>> = Mutex::new(None);

pub fn value(name: &str) -> Option<f32> {
    VALUES.lock().unwrap().as_ref().and_then(|m| m.get(name).copied())
}

/// Snapshot of all current values (for the UI / API).
pub fn snapshot() -> Vec<(&'static str, f32)> {
    let g = VALUES.lock().unwrap();
    input_names().into_iter().map(|n| (n, g.as_ref().and_then(|m| m.get(n).copied()).unwrap_or(0.0))).collect()
}

/// Resets everything to the neutral face.
pub fn clear() {
    *VALUES.lock().unwrap() = None;
}

/// Computes all values from a tracking frame. Frames without blendshapes (OpenSeeFace)
/// clear the values.
pub fn update(f: &FaceFrame, mirror: bool) {
    if f.kind == FaceKind::OpenSeeFace {
        clear();
        return;
    }
    let b = &f.bs;
    // With mirror mode the avatar's left is the subject's right.
    let swap = |name: &'static str| -> &'static str {
        if !mirror {
            return name;
        }
        let other = if let Some(s) = name.strip_suffix("Left") {
            format!("{s}Right")
        } else if let Some(s) = name.strip_suffix("Right") {
            format!("{s}Left")
        } else {
            return name;
        };
        ARKIT_NAMES.iter().copied().find(|n| *n == other).unwrap_or(name)
    };
    let mut m: HashMap<&'static str, f32> = HashMap::with_capacity(64);
    for (i, &n) in ARKIT_NAMES.iter().enumerate().skip(1) {
        m.insert(swap(n), b[i].clamp(0.0, 1.0));
    }
    // Mirror also flips the horizontal mouth/jaw movement.
    if mirror {
        let (ml, mr) = (m["MouthLeft"], m["MouthRight"]);
        m.insert("MouthLeft", mr);
        m.insert("MouthRight", ml);
        let (jl, jr) = (m["JawLeft"], m["JawRight"]);
        m.insert("JawLeft", jr);
        m.insert("JawRight", jl);
    }
    let avg = |a: &str, c: &str| (m[a] + m[c]) * 0.5;
    let open = avg("MouthUpperUpLeft", "MouthUpperUpRight") * 0.6 + avg("MouthLowerDownLeft", "MouthLowerDownRight") * 0.6;
    let press = avg("MouthPressLeft", "MouthPressRight") + (m["MouthRollLower"] + m["MouthRollUpper"]) * 0.35 + m["MouthClose"] * 0.5;
    let widen = avg("MouthStretchLeft", "MouthStretchRight") * 0.8 + avg("MouthDimpleLeft", "MouthDimpleRight") * 0.4;
    let pucker = m["MouthPucker"] + m["MouthFunnel"] * 0.3;
    let vb = [
        (open - press).clamp(-1.0, 1.0),
        (widen - pucker).clamp(-1.0, 1.0),
        ((m["MouthShrugLower"] + m["MouthShrugUpper"]) * 0.5).clamp(0.0, 1.0),
        avg("CheekSquintLeft", "CheekSquintRight").clamp(0.0, 1.0),
        avg("EyeSquintLeft", "EyeSquintRight").clamp(0.0, 1.0),
        avg("BrowOuterUpLeft", "BrowOuterUpRight").clamp(0.0, 1.0),
    ];
    for (p, v) in VB_PARAMS.iter().zip(vb) {
        m.insert(p.0, v);
    }
    *VALUES.lock().unwrap() = Some(m);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(arkit_index("EyeBlinkLeft"), Some(9));
        assert_eq!(arkit_index("eyeBlink_L"), Some(9));
        assert_eq!(arkit_index("mouthSmile_R"), Some(45));
        assert_eq!(arkit_index("jawOpen"), Some(25));
        assert_eq!(arkit_index("noseSneerRight"), Some(51));
        assert_eq!(arkit_index("mouthRollLower"), Some(40));
        assert_eq!(arkit_index("mouthShrugUpper"), Some(43));
        assert_eq!(arkit_index("browInnerUp"), Some(3));
        assert_eq!(arkit_index("mouthFunnel"), Some(32));
        assert_eq!(arkit_index("tongueOut"), None);
        assert!(!input_names().contains(&"CheekPuff"));
    }
}
