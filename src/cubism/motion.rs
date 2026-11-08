//! Minimal motion3.json player (used for the looping idle animation).

use crate::formats::Motion3;

use super::model::Model;

#[derive(Clone, Copy)]
enum SegKind {
    Linear,
    Bezier,
    Stepped,
    InverseStepped,
}

#[derive(Clone, Copy)]
struct Seg {
    kind: SegKind,
    // p[0] = start point, p[3] (bezier) / p[1] (others) = end point
    p: [(f32, f32); 4],
}

impl Seg {
    fn end(&self) -> f32 {
        match self.kind {
            SegKind::Bezier => self.p[3].0,
            _ => self.p[1].0,
        }
    }

    fn eval(&self, t: f32) -> f32 {
        let p = &self.p;
        match self.kind {
            SegKind::Linear => {
                let span = p[1].0 - p[0].0;
                let u = if span > 0.0 { ((t - p[0].0) / span).clamp(0.0, 1.0) } else { 1.0 };
                p[0].1 + (p[1].1 - p[0].1) * u
            }
            SegKind::Bezier => {
                let span = p[3].0 - p[0].0;
                let u = if span > 0.0 { ((t - p[0].0) / span).clamp(0.0, 1.0) } else { 1.0 };
                let iu = 1.0 - u;
                iu * iu * iu * p[0].1 + 3.0 * iu * iu * u * p[1].1 + 3.0 * iu * u * u * p[2].1 + u * u * u * p[3].1
            }
            SegKind::Stepped => p[0].1,
            SegKind::InverseStepped => p[1].1,
        }
    }
}

enum Target {
    Param(usize),
    Part(usize),
}

struct Curve {
    target: Target,
    first: f32,
    segs: Vec<Seg>,
    cursor: usize,
}

impl Curve {
    fn eval(&mut self, t: f32) -> f32 {
        if self.segs.is_empty() || t <= self.segs[0].p[0].0 {
            return self.first;
        }
        if self.cursor >= self.segs.len() || self.segs[self.cursor].p[0].0 > t {
            self.cursor = 0;
        }
        while self.cursor + 1 < self.segs.len() && self.segs[self.cursor].end() < t {
            self.cursor += 1;
        }
        self.segs[self.cursor].eval(t)
    }
}

pub struct Motion {
    duration: f32,
    looped: bool,
    curves: Vec<Curve>,
    time: f32,
}

fn parse_segments(s: &[f32]) -> (f32, Vec<Seg>) {
    if s.len() < 2 {
        return (0.0, Vec::new());
    }
    let mut last = (s[0], s[1]);
    let mut out = Vec::new();
    let mut i = 2;
    while i < s.len() {
        let kind = s[i] as i32;
        let need = if kind == 1 { 7 } else { 3 };
        if i + need > s.len() {
            break;
        }
        let seg = match kind {
            0 => Seg { kind: SegKind::Linear, p: [last, (s[i + 1], s[i + 2]), (0.0, 0.0), (0.0, 0.0)] },
            1 => Seg {
                kind: SegKind::Bezier,
                p: [last, (s[i + 1], s[i + 2]), (s[i + 3], s[i + 4]), (s[i + 5], s[i + 6])],
            },
            2 => Seg { kind: SegKind::Stepped, p: [last, (s[i + 1], s[i + 2]), (0.0, 0.0), (0.0, 0.0)] },
            3 => Seg { kind: SegKind::InverseStepped, p: [last, (s[i + 1], s[i + 2]), (0.0, 0.0), (0.0, 0.0)] },
            _ => break,
        };
        last = if kind == 1 { seg.p[3] } else { seg.p[1] };
        out.push(seg);
        i += need;
    }
    (s[1], out)
}

impl Motion {
    pub fn new(json: &Motion3, model: &Model) -> Motion {
        let curves = json
            .curves
            .iter()
            .filter_map(|c| {
                let target = match c.target.as_str() {
                    "Parameter" => Target::Param(model.param(&c.id)?),
                    "PartOpacity" => Target::Part(model.part(&c.id)?),
                    _ => return None,
                };
                let (first, segs) = parse_segments(&c.segments);
                Some(Curve { target, first, segs, cursor: 0 })
            })
            .collect();
        Motion { duration: json.meta.duration.max(0.0), looped: json.meta.r#loop, curves, time: 0.0 }
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Plays once (no looping); returns false when finished.
    pub fn apply_once(&mut self, model: &mut Model, dt: f32, weight: f32) -> bool {
        self.time += dt;
        if self.time >= self.duration {
            return false;
        }
        let t = self.time;
        for c in &mut self.curves {
            let v = c.eval(t);
            match c.target {
                Target::Param(i) => {
                    let cur = model.params()[i];
                    model.set_param(i, cur + (v - cur) * weight);
                }
                Target::Part(i) => model.parts_mut()[i] = v,
            }
        }
        true
    }

    pub fn apply(&mut self, model: &mut Model, dt: f32) {
        self.time += dt;
        if self.duration > 0.0 {
            if self.looped || true {
                // Idle animations are always looped, whatever the file says.
                self.time %= self.duration;
            }
        }
        let t = self.time;
        for c in &mut self.curves {
            let v = c.eval(t);
            match c.target {
                Target::Param(i) => model.set_param(i, v),
                Target::Part(i) => model.parts_mut()[i] = v,
            }
        }
    }
}
