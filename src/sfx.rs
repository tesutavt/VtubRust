//! Model sound settings (VTS): model volume / mute (Toggle Model Volume hotkey), spatial
//! audio (volume by size, pan by X), and parameter sound triggers (value, speed, direction
//! change, value pass → / ←) with conditions, cooldown and speed → volume / pitch modifiers,
//! plus item load / delete sounds.

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::audio::SoundParams;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum TriggerKind {
    Value,
    Speed,
    DirectionChange,
    PassRight,
    PassLeft,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct SoundTrigger {
    pub name: String,
    pub active: bool,
    pub parameter: String,
    pub kind: TriggerKind,
    /// Value / speed range (speed normalized to -1..1); for pass triggers `range[0]` is the value.
    pub range: [f32; 2],
    /// Optional condition: another parameter must be within a range.
    pub condition_parameter: String,
    pub condition_range: [f32; 2],
    pub cooldown: [f32; 2],
    pub sound: SoundParams,
    pub speed_to_volume: bool,
    pub speed_to_volume_range: [f32; 2],
    pub speed_to_pitch: bool,
    pub speed_to_pitch_range: [f32; 2],
    pub item_load_sound: bool,
    pub item_delete_sound: bool,
}

impl Default for SoundTrigger {
    fn default() -> Self {
        SoundTrigger {
            name: String::new(),
            active: true,
            parameter: String::new(),
            kind: TriggerKind::Value,
            range: [0.0, 0.1],
            condition_parameter: String::new(),
            condition_range: [0.0, 1.0],
            cooldown: [0.0, 0.0],
            sound: SoundParams::default(),
            speed_to_volume: false,
            speed_to_volume_range: [0.5, 1.5],
            speed_to_pitch: false,
            speed_to_pitch_range: [0.8, 1.2],
            item_load_sound: false,
            item_delete_sound: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelSound {
    /// 0..100 (VTS GeneralSettings.ModelVolume).
    pub volume: f32,
    pub muted: bool,
    pub volume_by_size: bool,
    pub size_range: [f32; 2],
    pub pan_by_x: bool,
    pub x_range: [f32; 2],
    pub triggers: Vec<SoundTrigger>,
}

impl Default for ModelSound {
    fn default() -> Self {
        ModelSound { volume: 90.0, muted: false, volume_by_size: false, size_range: [-50.0, 50.0], pan_by_x: false, x_range: [-1.0, 1.0], triggers: Vec::new() }
    }
}

impl ModelSound {
    pub fn gain(&self) -> f32 {
        if self.muted { 0.0 } else { (self.volume / 100.0).clamp(0.0, 1.0) }
    }

    /// Spatial volume / pan from the model size (VTS size units) and X position (-1..1).
    pub fn spatial(&self, size: f32, x: f32) -> (f32, f32) {
        let vol = if self.volume_by_size {
            ((size - self.size_range[0]) / (self.size_range[1] - self.size_range[0]).max(1e-3)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let pan = if self.pan_by_x { (((x - self.x_range[0]) / (self.x_range[1] - self.x_range[0]).max(1e-3)) * 2.0 - 1.0).clamp(-1.0, 1.0) } else { 0.0 };
        (vol, pan)
    }
}

/// Runtime state per (model key, trigger index).
#[derive(Default)]
pub struct TriggerState {
    prev: Option<f32>,
    prev_speed: f32,
    inside: bool,
    next_allowed: Option<Instant>,
}

#[derive(Default)]
pub struct Runtime {
    pub states: HashMap<(String, usize), TriggerState>,
}

/// Evaluates the triggers of one model; returns the triggers that fire with their speed.
pub fn evaluate(rt: &mut Runtime, key: &str, ms: &ModelSound, ids: &[String], vals: &[f32], mins: &[f32], maxs: &[f32], dt: f32) -> Vec<(usize, f32)> {
    let mut fired = Vec::new();
    if dt <= 0.0 {
        return fired;
    }
    let get = |name: &str| ids.iter().position(|i| i == name).map(|k| (vals[k], mins[k], maxs[k]));
    for (ti, t) in ms.triggers.iter().enumerate() {
        if !t.active {
            continue;
        }
        let Some((v, mn, mx)) = get(&t.parameter) else { continue };
        let st = rt.states.entry((key.to_string(), ti)).or_default();
        let span = (mx - mn).abs().max(1e-4);
        let prev = st.prev.unwrap_or(v);
        // Speed normalized to -1..1 (full range in ~0.25 s = 1).
        let speed = (((v - prev) / span) / (dt * 4.0)).clamp(-1.0, 1.0);
        st.prev = Some(v);
        let cond_ok = t.condition_parameter.is_empty() || get(&t.condition_parameter).map(|(c, _, _)| c >= t.condition_range[0] && c <= t.condition_range[1]).unwrap_or(false);
        let (lo, hi) = (t.range[0].min(t.range[1]), t.range[0].max(t.range[1]));
        let in_range = |x: f32| x >= lo && x <= hi;
        let hit = match t.kind {
            TriggerKind::Value => {
                let now = in_range(v);
                let h = now && !st.inside;
                st.inside = now;
                h
            }
            TriggerKind::Speed => {
                let now = in_range(speed) || in_range(-speed) && lo >= 0.0;
                let h = now && !st.inside;
                st.inside = now;
                h
            }
            TriggerKind::DirectionChange => {
                let changed = st.prev_speed.signum() != speed.signum() && st.prev_speed.abs() > 0.02 && speed.abs() > 0.0;
                let mag = st.prev_speed.abs();
                changed && (mag >= lo.abs().min(hi.abs()) && mag <= lo.abs().max(hi.abs()))
            }
            TriggerKind::PassRight => prev < t.range[0] && v >= t.range[0],
            TriggerKind::PassLeft => prev > t.range[0] && v <= t.range[0],
        };
        st.prev_speed = speed;
        if !hit || !cond_ok {
            continue;
        }
        let now = Instant::now();
        if st.next_allowed.map(|n| now < n).unwrap_or(false) {
            continue;
        }
        let cd = t.cooldown[0] + (t.cooldown[1] - t.cooldown[0]) * crate::audio::rnd();
        st.next_allowed = Some(now + std::time::Duration::from_secs_f32(cd.max(0.0)));
        fired.push((ti, speed.abs()));
    }
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(kind: TriggerKind, range: [f32; 2]) -> ModelSound {
        ModelSound { triggers: vec![SoundTrigger { parameter: "P".into(), kind, range, ..Default::default() }], ..Default::default() }
    }

    #[test]
    fn value_trigger_fires_on_entry_only() {
        let mut rt = Runtime::default();
        let m = ms(TriggerKind::Value, [0.0, 0.1]);
        let ids = vec!["P".to_string()];
        let run = |rt: &mut Runtime, v: f32| evaluate(rt, "k", &m, &ids, &[v], &[0.0], &[1.0], 0.016).len();
        assert_eq!(run(&mut rt, 1.0), 0);
        assert_eq!(run(&mut rt, 0.05), 1);
        assert_eq!(run(&mut rt, 0.02), 0);
        assert_eq!(run(&mut rt, 0.8), 0);
        assert_eq!(run(&mut rt, 0.0), 1);
    }

    #[test]
    fn pass_triggers_are_directional() {
        let mut rt = Runtime::default();
        let m = ms(TriggerKind::PassLeft, [0.5, 0.5]);
        let ids = vec!["P".to_string()];
        let run = |rt: &mut Runtime, v: f32| evaluate(rt, "k", &m, &ids, &[v], &[0.0], &[1.0], 0.016).len();
        assert_eq!(run(&mut rt, 0.2), 0);
        assert_eq!(run(&mut rt, 0.8), 0); // passing → does not fire a ← trigger
        assert_eq!(run(&mut rt, 0.3), 1);
    }

    #[test]
    fn spatial_audio() {
        let m = ModelSound { volume_by_size: true, size_range: [0.0, 100.0], pan_by_x: true, x_range: [-1.0, 1.0], ..Default::default() };
        let (v, p) = m.spatial(50.0, 1.0);
        assert!((v - 0.5).abs() < 1e-5 && (p - 1.0).abs() < 1e-5);
    }
}
