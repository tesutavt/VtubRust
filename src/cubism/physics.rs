//! physics3.json evaluator — pendulum particle chains, following the publicly documented
//! behaviour of the Cubism Framework physics (fixed step + output interpolation).

use std::ops::{Add, Mul, Sub};

use crate::formats::{NormRange, Physics3};

use super::model::Model;

const AIR_RESISTANCE: f32 = 5.0;
const MAX_WEIGHT: f32 = 100.0;
const MOVEMENT_THRESHOLD: f32 = 0.001;
const MAX_DELTA: f32 = 5.0;

#[derive(Clone, Copy, Default, Debug)]
struct V2 {
    x: f32,
    y: f32,
}

impl V2 {
    const fn new(x: f32, y: f32) -> V2 {
        V2 { x, y }
    }
    fn normalized(self) -> V2 {
        let l = (self.x * self.x + self.y * self.y).sqrt();
        if l > 0.0 {
            V2::new(self.x / l, self.y / l)
        } else {
            self
        }
    }
}
impl Add for V2 {
    type Output = V2;
    fn add(self, o: V2) -> V2 {
        V2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for V2 {
    type Output = V2;
    fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for V2 {
    type Output = V2;
    fn mul(self, s: f32) -> V2 {
        V2::new(self.x * s, self.y * s)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    X,
    Y,
    Angle,
}

fn kind(s: &str) -> Kind {
    match s {
        "X" => Kind::X,
        "Y" => Kind::Y,
        _ => Kind::Angle,
    }
}

struct Input {
    param: usize,
    weight: f32,
    kind: Kind,
    reflect: bool,
}

struct Output {
    param: usize,
    vertex: usize,
    scale: f32,
    weight: f32,
    kind: Kind,
    reflect: bool,
}

#[derive(Default, Clone)]
struct Particle {
    mobility: f32,
    delay: f32,
    acceleration: f32,
    radius: f32,
    position: V2,
    last_position: V2,
    last_gravity: V2,
    velocity: V2,
}

struct Setting {
    inputs: Vec<Input>,
    outputs: Vec<Output>,
    particles: Vec<Particle>,
    norm_pos: NormRange,
    norm_angle: NormRange,
    cur_out: Vec<f32>,
    prev_out: Vec<f32>,
}

pub struct Physics {
    settings: Vec<Setting>,
    gravity: V2,
    wind: V2,
    fps: Option<f32>,
    /// None = rate from the physics file; Some(None) = every app frame; Some(Some(f)) = f Hz.
    pub rate_override: Option<Option<f32>>,
    /// Output multiplier (1.0 = as authored).
    pub strength: f32,
    remain: f32,
    values: Vec<f32>,
    param_cache: Vec<f32>,
    input_cache: Vec<f32>,
    initialized: bool,
    min: Vec<f32>,
    max: Vec<f32>,
}

fn direction_to_radian(from: V2, to: V2) -> f32 {
    let mut r = to.y.atan2(to.x) - from.y.atan2(from.x);
    while r < -std::f32::consts::PI {
        r += std::f32::consts::TAU;
    }
    while r > std::f32::consts::PI {
        r -= std::f32::consts::TAU;
    }
    r
}

fn normalize(value: f32, pmin: f32, pmax: f32, n: NormRange, inverted: bool) -> f32 {
    let max_v = pmax.max(pmin);
    let min_v = pmin.min(pmax);
    let v = value.clamp(min_v, max_v);
    let min_n = n.minimum.min(n.maximum);
    let max_n = n.minimum.max(n.maximum);
    let mid_n = n.default;
    let mid_v = min_v + (max_v - min_v) / 2.0;
    let pv = v - mid_v;
    let result = if pv > 0.0 {
        let pl = max_v - mid_v;
        if pl != 0.0 { pv * ((max_n - mid_n) / pl) + mid_n } else { 0.0 }
    } else if pv < 0.0 {
        let pl = min_v - mid_v;
        if pl != 0.0 { pv * ((min_n - mid_n) / pl) + mid_n } else { 0.0 }
    } else {
        mid_n
    };
    if inverted { result } else { -result }
}

fn update_output(target: &mut f32, min: f32, max: f32, value: f32, out: &Output) {
    let v = (value * out.scale).clamp(min.min(max), max.max(min));
    let w = out.weight / MAX_WEIGHT;
    *target = if w >= 1.0 { v } else { *target * (1.0 - w) + v * w };
}

fn update_particles(strand: &mut [Particle], translation: V2, angle: f32, wind: V2, threshold: f32, dt: f32) {
    strand[0].position = translation;
    let rad = angle.to_radians();
    let gravity = V2::new(rad.sin(), rad.cos()).normalized();
    for i in 1..strand.len() {
        let prev = strand[i - 1].position;
        let s = &mut strand[i];
        let force = gravity * s.acceleration + wind;
        s.last_position = s.position;
        let delay = s.delay * dt * 30.0;
        let mut dir = s.position - prev;
        let r = direction_to_radian(s.last_gravity, gravity) / AIR_RESISTANCE;
        // Matches the reference implementation, which reuses the rotated X.
        dir.x = r.cos() * dir.x - dir.y * r.sin();
        dir.y = r.sin() * dir.x + dir.y * r.cos();
        s.position = prev + dir;
        s.position = s.position + s.velocity * delay + force * (delay * delay);
        let nd = (s.position - prev).normalized();
        s.position = prev + nd * s.radius;
        if s.position.x.abs() < threshold {
            s.position.x = 0.0;
        }
        if delay != 0.0 {
            s.velocity = (s.position - s.last_position) * (s.mobility / delay);
        }
        s.last_gravity = gravity;
    }
}

impl Physics {
    pub fn new(json: &Physics3, model: &Model) -> Physics {
        let mut settings = Vec::new();
        for s in &json.physics_settings {
            let inputs: Vec<Input> = s
                .input
                .iter()
                .filter_map(|i| {
                    Some(Input { param: model.param(&i.source.id)?, weight: i.weight, kind: kind(&i.kind), reflect: i.reflect })
                })
                .collect();
            let outputs: Vec<Output> = s
                .output
                .iter()
                .filter_map(|o| {
                    Some(Output {
                        param: model.param(&o.destination.id)?,
                        vertex: o.vertex_index,
                        scale: o.scale,
                        weight: o.weight,
                        kind: kind(&o.kind),
                        reflect: o.reflect,
                    })
                })
                .collect();
            let mut particles: Vec<Particle> = Vec::with_capacity(s.vertices.len());
            for (i, v) in s.vertices.iter().enumerate() {
                let pos = if i == 0 { V2::default() } else { particles[i - 1].position + V2::new(0.0, v.radius) };
                particles.push(Particle {
                    mobility: v.mobility,
                    delay: v.delay,
                    acceleration: v.acceleration,
                    radius: v.radius,
                    position: pos,
                    last_position: pos,
                    last_gravity: V2::new(0.0, 1.0),
                    velocity: V2::default(),
                });
            }
            if particles.is_empty() || outputs.is_empty() {
                continue;
            }
            let n = outputs.len();
            settings.push(Setting {
                inputs,
                outputs,
                particles,
                norm_pos: s.normalization.position,
                norm_angle: s.normalization.angle,
                cur_out: vec![0.0; n],
                prev_out: vec![0.0; n],
            });
        }
        let forces = json.meta.effective_forces.as_ref();
        Physics {
            settings,
            gravity: forces.map(|f| V2::new(f.gravity.x, f.gravity.y)).unwrap_or(V2::new(0.0, -1.0)),
            wind: forces.map(|f| V2::new(f.wind.x, f.wind.y)).unwrap_or_default(),
            fps: json.meta.fps.filter(|f| *f > 0.0),
            rate_override: None,
            strength: 1.0,
            remain: 0.0,
            values: Vec::new(),
            param_cache: Vec::new(),
            input_cache: Vec::new(),
            initialized: false,
            min: model.param_min.clone(),
            max: model.param_max.clone(),
        }
    }

    /// Parameters written by physics.
    pub fn output_params(&self) -> Vec<usize> {
        self.settings.iter().flat_map(|s| s.outputs.iter().map(|o| o.param)).collect()
    }

    pub fn chain_count(&self) -> usize {
        self.settings.len()
    }

    pub fn evaluate(&mut self, model: &mut Model, dt: f32) {
        if dt <= 0.0 || self.settings.is_empty() {
            return;
        }
        let fps = match self.rate_override {
            Some(o) => o,
            None => self.fps,
        };
        let step = fps.map(|f| 1.0 / f).unwrap_or(dt);
        self.remain += dt;
        if self.remain > MAX_DELTA {
            self.remain = 0.0;
        }
        self.values.clear();
        self.values.extend_from_slice(model.params());
        if !self.initialized {
            self.param_cache = self.values.clone();
            self.input_cache = self.values.clone();
            self.initialized = true;
        }

        while self.remain >= step {
            let w = step / self.remain;
            for j in 0..self.values.len() {
                let v = self.input_cache[j] * (1.0 - w) + self.values[j] * w;
                self.param_cache[j] = v;
                self.input_cache[j] = v;
            }
            for s in &mut self.settings {
                s.prev_out.copy_from_slice(&s.cur_out);
                let mut tt = V2::default();
                let mut ta = 0.0f32;
                for inp in &s.inputs {
                    let p = inp.param;
                    let weight = inp.weight / MAX_WEIGHT;
                    let v = self.param_cache[p];
                    match inp.kind {
                        Kind::X => tt.x += normalize(v, self.min[p], self.max[p], s.norm_pos, inp.reflect) * weight,
                        Kind::Y => tt.y += normalize(v, self.min[p], self.max[p], s.norm_pos, inp.reflect) * weight,
                        Kind::Angle => ta += normalize(v, self.min[p], self.max[p], s.norm_angle, inp.reflect) * weight,
                    }
                }
                let r = (-ta).to_radians();
                tt.x = tt.x * r.cos() - tt.y * r.sin();
                tt.y = tt.x * r.sin() + tt.y * r.cos();
                update_particles(&mut s.particles, tt, ta, self.wind, MOVEMENT_THRESHOLD * s.norm_pos.maximum, step);

                for (k, out) in s.outputs.iter().enumerate() {
                    let pi = out.vertex;
                    if pi < 1 || pi >= s.particles.len() {
                        continue;
                    }
                    let tr = s.particles[pi].position - s.particles[pi - 1].position;
                    let sign = if out.reflect { -1.0 } else { 1.0 };
                    let v = match out.kind {
                        Kind::X => tr.x * sign,
                        Kind::Y => tr.y * sign,
                        Kind::Angle => {
                            let parent = if pi >= 2 {
                                s.particles[pi - 1].position - s.particles[pi - 2].position
                            } else {
                                self.gravity * -1.0
                            };
                            direction_to_radian(parent, tr) * sign
                        }
                    };
                    s.cur_out[k] = v;
                    let p = out.param;
                    update_output(&mut self.param_cache[p], self.min[p], self.max[p], v, out);
                }
            }
            self.remain -= step;
        }

        let alpha = self.remain / step;
        for s in &self.settings {
            for (k, out) in s.outputs.iter().enumerate() {
                let p = out.param;
                let v = (s.prev_out[k] * (1.0 - alpha) + s.cur_out[k] * alpha) * self.strength;
                update_output(&mut self.values[p], self.min[p], self.max[p], v, out);
            }
        }
        model.params_mut().copy_from_slice(&self.values);
    }
}
