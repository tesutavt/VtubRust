//! A loaded Live2D model instance: simulation + GPU geometry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cubism::core::*;
use crate::cubism::model::Model;
use crate::cubism::motion::Motion;
use crate::cubism::physics::Physics;
use crate::cubism::pose::Pose;
use crate::formats::*;
use crate::gpu::{Affine, Batch, Blend, Clip, DrawData, Gpu, MaskAlloc, MaskPainter, Painter};
use crate::mapping::{default_rules, input, Inputs, MapCtx, Mapping, Rule};
use crate::settings::ModelCfg;

/// Placement in world space (pixels from window centre, Y up).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Xform {
    pub pos: [f32; 2],
    pub scale: f32,
    /// degrees, counter-clockwise
    pub rot: f32,
    pub flip: bool,
}

impl Default for Xform {
    fn default() -> Self {
        Xform { pos: [0.0, 0.0], scale: 1.0, rot: 0.0, flip: false }
    }
}

impl Xform {
    pub fn to_world(&self, p: [f32; 2]) -> [f32; 2] {
        let (s, c) = self.rot.to_radians().sin_cos();
        let x = if self.flip { -p[0] } else { p[0] } * self.scale;
        let y = p[1] * self.scale;
        [self.pos[0] + c * x - s * y, self.pos[1] + s * x + c * y]
    }

    pub fn to_local(&self, w: [f32; 2]) -> [f32; 2] {
        let (s, c) = self.rot.to_radians().sin_cos();
        let (dx, dy) = (w[0] - self.pos[0], w[1] - self.pos[1]);
        let x = (c * dx + s * dy) / self.scale;
        let y = (-s * dx + c * dy) / self.scale;
        [if self.flip { -x } else { x }, y]
    }

    pub fn affine(&self, vw: f32, vh: f32, flip_v: bool) -> Affine {
        let (s, c) = self.rot.to_radians().sin_cos();
        let fx = if self.flip { -1.0 } else { 1.0 };
        let (kx, ky) = (2.0 / vw, 2.0 / vh);
        Affine {
            m: [kx * self.scale * c * fx, ky * self.scale * s * fx, -kx * self.scale * s, ky * self.scale * c],
            t: [kx * self.pos[0], ky * self.pos[1], if flip_v { 1.0 } else { 0.0 }, 0.0],
        }
    }
}

struct Blinker {
    t: f32,
    next: f32,
    rng: u32,
}

impl Blinker {
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 10000) as f32 / 10000.0
    }

    fn value(&mut self, dt: f32) -> f32 {
        self.t += dt;
        if self.t < self.next {
            return 1.0;
        }
        let p = self.t - self.next;
        let v = if p < 0.08 {
            1.0 - p / 0.08
        } else if p < 0.12 {
            0.0
        } else if p < 0.25 {
            (p - 0.12) / 0.13
        } else {
            self.t = 0.0;
            self.next = 2.0 + self.rand() * 4.0;
            1.0
        };
        v
    }
}

#[derive(Clone, Copy)]
enum ExpBlend {
    Add,
    Multiply,
    Overwrite,
}

pub struct Expression {
    pub name: String,
    pub file: String,
    params: Vec<(usize, f32, ExpBlend)>,
    weight: f32,
    /// Fade time (seconds) used for the next transition.
    pub fade: f32,
    pub active: bool,
}

pub struct Pick {
    pub drawable: usize,
    pub tri: [u16; 3],
    pub bary: [f32; 3],
}

pub struct Live2D {
    pub name: String,
    pub model3: PathBuf,
    pub dir: PathBuf,
    pub model: Model,
    physics: Option<Physics>,
    pub physics_enabled: bool,
    motion: Option<Motion>,
    pub idle: Option<String>,
    lost_motion: Option<Motion>,
    pub cfg: ModelCfg,
    /// ModelID from .vtube.json (32 hex), or a stable hash of the path.
    pub model_id: String,
    /// Hotkeys defined in the model's .vtube.json.
    pub vts_hotkeys: Vec<crate::hotkeys::Hotkey>,
    /// Animation played once on top of the idle one (TriggerAnimation hotkeys).
    oneshot: Option<(Motion, String, f32)>,
    /// Per-drawable RGBA multiply set through the plugin API (ColorTint).
    pub art_tint: Vec<[f32; 4]>,
    pub load_time: std::time::Instant,
    pub load_ms: u64,
    /// Physics strength forced by a plugin (SetCurrentModelPhysics).
    pub api_strength: Option<f32>,
    pub texture_count: usize,
    pub texture_size: u32,
    /// Multiplicative colour from the lighting overlay.
    pub tint: [f32; 3],
    /// Smoothed head-follow offset: x, y (px) and relative zoom.
    move_off: [f32; 3],
    pose: Option<Pose>,
    pub mapping: Mapping,
    pub file_rules: Vec<Rule>,
    pub expressions: Vec<Expression>,
    pub xf: Xform,
    pub texture_bytes: u64,
    batch: Batch,
    pos_scratch: Vec<[f32; 2]>,
    draw_scratch: Vec<DrawData>,
    order: Vec<usize>,
    mask_key: Vec<usize>,
    /// Smooth-mask set per drawable for the current frame (-1 = stencil path).
    mask_set: Vec<i32>,
    /// (mask set, representative clipped drawable) to render into the atlas.
    mask_jobs: Vec<(u32, usize)>,
    first_upload: bool,
    blink: Blinker,
    time: f32,
}

fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    let file_name = Path::new(name).file_name()?.to_owned();
    let rd = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            subdirs.push(p);
        } else if p.file_name() == Some(&file_name) {
            return Some(p);
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs.iter().find_map(|d| find_file(d, name, depth - 1))
}

/// All motion3.json files near the model (for the idle-animation picker).
pub fn list_motions(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, base: &Path, out: &mut Vec<String>, depth: u32) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth > 0 {
                walk(&p, base, out, depth - 1);
            } else if p.to_string_lossy().ends_with(".motion3.json") {
                if let Ok(r) = p.strip_prefix(base) {
                    out.push(r.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v, 3);
    v.sort();
    v
}

impl Live2D {
    pub fn load(core: &'static Core, gpu: &Gpu, model3: &Path, vtube: Option<&Path>, idle_override: Option<&str>, rules_override: Option<&Vec<Rule>>) -> Result<Live2D> {
        let dir = model3.parent().unwrap_or(Path::new(".")).to_path_buf();
        let m3: Model3 = read_json(model3)?;
        let moc = std::fs::read(dir.join(&m3.file_references.moc)).context("reading .moc3")?;
        let model = Model::load(core, &moc)?;

        // Decode textures in parallel (PNG decode dominates load time).
        let images: Vec<Result<image::RgbaImage>> = std::thread::scope(|s| {
            let handles: Vec<_> = m3
                .file_references
                .textures
                .iter()
                .map(|t| {
                    let p = dir.join(t);
                    s.spawn(move || -> Result<image::RgbaImage> {
                        Ok(image::open(&p).with_context(|| format!("texture {}", p.display()))?.to_rgba8())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let mut textures = Vec::new();
        let mut texture_bytes = 0u64;
        let mut tex_size = 0u32;
        for img in images {
            let img = img?;
            let (w, h) = img.dimensions();
            tex_size = tex_size.max(w);
            texture_bytes += w as u64 * h as u64 * 4;
            textures.push(gpu.texture(img.into_raw(), w, h));
        }

        let vt: Option<VtubeJson> = vtube.and_then(|p| match read_json(p) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("warning: {e:#}");
                None
            }
        });

        let physics = match &m3.file_references.physics {
            Some(p) => match read_json::<Physics3>(&dir.join(p)) {
                Ok(j) => Some(Physics::new(&j, &model)),
                Err(e) => {
                    eprintln!("warning: physics: {e:#}");
                    None
                }
            },
            None => None,
        };
        let pose = m3
            .file_references
            .pose
            .as_ref()
            .and_then(|p| read_json::<Pose3>(&dir.join(p)).ok())
            .map(|j| Pose::new(&j, &model));

        let idle = idle_override
            .map(str::to_string)
            .or_else(|| vt.as_ref().map(|v| v.file_references.idle_animation.clone()).filter(|s| !s.is_empty()))
            .or_else(|| {
                m3.file_references
                    .motions
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("idle"))
                    .and_then(|(_, v)| v.first())
                    .map(|m| m.file.clone())
            })
            .filter(|s| !s.is_empty());

        let file_rules: Vec<Rule> = match &vt {
            Some(v) if !v.parameter_settings.is_empty() => v.parameter_settings.iter().map(Rule::from).collect(),
            _ => default_rules(&model),
        };
        let rules = rules_override.cloned().unwrap_or_else(|| file_rules.clone());
        let mapping = Mapping::new(rules, &model);

        let mut expressions = Vec::new();
        for e in &m3.file_references.expressions {
            if let Ok(j) = read_json::<Exp3>(&dir.join(&e.file)) {
                expressions.push(Self::compile_expression(&model, e.name.clone(), e.file.clone(), &j));
            }
        }
        // VTS models often keep extra exp3 files that are not listed in model3.json.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for f in rd.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                if name.ends_with(".exp3.json") && !expressions.iter().any(|e| e.file.ends_with(&name)) {
                    if let Ok(j) = read_json::<Exp3>(&f.path()) {
                        let n = name.trim_end_matches(".exp3.json").to_string();
                        expressions.push(Self::compile_expression(&model, n, name, &j));
                    }
                }
            }
        }

        let vcount: usize = model.drawables.iter().map(|d| d.vertex_count).sum();
        let mut uvs = Vec::with_capacity(vcount);
        let mut idx = Vec::new();
        for i in 0..model.drawables.len() {
            uvs.extend(model.uvs(i).iter().map(|v| [v.x, v.y]));
            idx.extend_from_slice(model.indices(i));
        }
        let pos = vec![[0.0f32; 2]; vcount];
        let tex_count = textures.len();
        let batch = Batch::new(gpu, &pos, &uvs, &idx, model.drawables.len(), textures);

        let mut sets: HashMap<Vec<usize>, usize> = HashMap::new();
        let mask_key = model
            .drawables
            .iter()
            .map(|d| {
                let n = sets.len();
                *sets.entry(d.masks.clone()).or_insert(n)
            })
            .collect();

        let physics_enabled = vt.as_ref().and_then(|v| v.physics_settings.as_ref()).map(|p| p.enabled).unwrap_or(true);
        let name = vt
            .as_ref()
            .map(|v| v.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| model3.file_name().unwrap().to_string_lossy().trim_end_matches(".model3.json").to_string());

        let n = model.drawables.len();
        let mut me = Live2D {
            name,
            model3: model3.to_path_buf(),
            dir,
            physics,
            physics_enabled,
            motion: None,
            idle: None,
            lost_motion: None,
            cfg: ModelCfg {
                move_enabled: vt.as_ref().and_then(|v| v.model_position_movement.as_ref()).map(|m| m.enabled).unwrap_or(false),
                move_xyz: vt
                    .as_ref()
                    .and_then(|v| v.model_position_movement.as_ref())
                    .map(|m| [m.x, m.y, m.z])
                    .unwrap_or([6.0, 8.0, 11.0]),
                physics_on: physics_enabled,
                ..ModelCfg::default()
            },
            move_off: [0.0; 3],
            model_id: vt
                .as_ref()
                .map(|v| v.model_id.clone())
                .filter(|s| s.len() == 32)
                .unwrap_or_else(|| {
                    use std::hash::{Hash, Hasher};
                    let mut h1 = std::collections::hash_map::DefaultHasher::new();
                    model3.hash(&mut h1);
                    let mut h2 = std::collections::hash_map::DefaultHasher::new();
                    (model3, 7u8).hash(&mut h2);
                    format!("{:016x}{:016x}", h1.finish(), h2.finish())
                }),
            vts_hotkeys: vt.as_ref().map(|v| v.hotkeys.clone()).unwrap_or_default(),
            oneshot: None,
            art_tint: vec![[1.0; 4]; n],
            load_time: std::time::Instant::now(),
            load_ms: 0,
            api_strength: None,
            texture_count: tex_count,
            texture_size: tex_size,
            tint: [1.0; 3],
            pose,
            mapping,
            file_rules,
            expressions,
            xf: Xform::default(),
            texture_bytes,
            batch,
            pos_scratch: pos,
            draw_scratch: vec![DrawData::default(); n],
            order: (0..n).collect(),
            mask_key,
            mask_set: vec![-1; n],
            mask_jobs: Vec::new(),
            first_upload: true,
            blink: Blinker { t: 0.0, next: 2.0, rng: 0x9E37_79B9 ^ n as u32 },
            time: 0.0,
            model,
        };
        if let Some(i) = idle {
            me.set_idle(Some(&i));
        }
        Ok(me)
    }

    fn compile_expression(model: &Model, name: String, file: String, j: &Exp3) -> Expression {
        let params = j
            .parameters
            .iter()
            .filter_map(|p| {
                let b = match p.blend.as_str() {
                    "Multiply" => ExpBlend::Multiply,
                    "Overwrite" => ExpBlend::Overwrite,
                    _ => ExpBlend::Add,
                };
                Some((model.param(&p.id)?, p.value, b))
            })
            .collect();
        Expression { name, file, params, weight: 0.0, fade: 0.3, active: false }
    }

    pub fn set_idle(&mut self, file: Option<&str>) {
        self.motion = None;
        self.idle = None;
        let Some(f) = file.filter(|f| !f.is_empty()) else { return };
        match find_file(&self.dir, f, 3).map(|p| read_json::<Motion3>(&p)) {
            Some(Ok(j)) => {
                self.motion = Some(Motion::new(&j, &self.model));
                self.idle = Some(f.to_string());
            }
            Some(Err(e)) => eprintln!("warning: idle animation: {e:#}"),
            None => eprintln!("warning: idle animation {f} not found"),
        }
    }

    fn load_motion(&self, f: &str) -> Option<Motion> {
        match find_file(&self.dir, f, 3).map(|p| read_json::<Motion3>(&p)) {
            Some(Ok(j)) => Some(Motion::new(&j, &self.model)),
            Some(Err(e)) => {
                eprintln!("warning: animation {f}: {e:#}");
                None
            }
            None => None,
        }
    }

    /// Plays a motion once over the idle animation. Returns false if the file is missing.
    pub fn play_once(&mut self, file: &str) -> bool {
        match self.load_motion(file) {
            Some(m) => {
                self.oneshot = Some((m, file.to_string(), 0.0));
                true
            }
            None => false,
        }
    }

    pub fn drawable_index(&self, id: &str) -> Option<usize> {
        self.model.drawables.iter().position(|d| d.id == id)
    }

    /// Animation played instead of the idle one while tracking is lost.
    pub fn set_lost_idle(&mut self, file: &str) {
        self.cfg.lost_idle = file.to_string();
        self.lost_motion = if file.is_empty() { None } else { self.load_motion(file) };
    }

    /// Applies a stored per-model config (after loading).
    pub fn apply_cfg(&mut self, cfg: ModelCfg) {
        let lost = cfg.lost_idle.clone();
        if cfg.save_expressions {
            for e in &mut self.expressions {
                e.active = cfg.active_expressions.contains(&e.file);
            }
        }
        self.cfg = cfg;
        self.set_lost_idle(&lost);
        if !self.cfg.display_name.is_empty() {
            self.name = self.cfg.display_name.clone();
        }
    }

    /// Placement including the head-follow offset (what is actually drawn).
    pub fn eff_xf(&self) -> Xform {
        let mut x = self.xf;
        x.pos[0] += self.move_off[0];
        x.pos[1] += self.move_off[1];
        x.scale *= 1.0 + self.move_off[2];
        x
    }

    pub fn physics_outputs(&self) -> Vec<usize> {
        self.physics.as_ref().map(|p| p.output_params()).unwrap_or_default()
    }

    pub fn has_physics(&self) -> bool {
        self.physics.is_some()
    }

    pub fn physics_chains(&self) -> usize {
        self.physics.as_ref().map(|p| p.chain_count()).unwrap_or(0)
    }

    /// Fits the model into a viewport (used when no saved placement exists).
    pub fn fit(&mut self, vw: f32, vh: f32, fraction: f32) {
        let b = self.local_bounds();
        let (w, h) = ((b[2] - b[0]).max(1e-3), (b[3] - b[1]).max(1e-3));
        let s = (vh * fraction / h).min(vw * 0.95 / w);
        let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        self.xf = Xform { pos: [-c[0] * s, -c[1] * s], scale: s, rot: 0.0, flip: false };
    }

    pub fn update(&mut self, dt: f32, inputs: &Inputs, tracking: bool, physics_on: bool) {
        self.time += dt;
        // Head-follow movement (smoothed).
        let target = if self.cfg.move_enabled {
            let k = self.cfg.move_xyz;
            [
                inputs[input::FACE_POSITION_X] * k[0] * 2.0,
                inputs[input::FACE_POSITION_Y] * k[1] * 2.0,
                (inputs[input::FACE_POSITION_Z] * k[2] * 0.002).clamp(-0.5, 0.5),
            ]
        } else {
            [0.0; 3]
        };
        let a = 1.0 - (-dt / 0.12).exp();
        for i in 0..3 {
            self.move_off[i] += (target[i] - self.move_off[i]) * a;
        }
        self.physics_enabled = self.cfg.physics_on;
        if let Some(ph) = &mut self.physics {
            ph.strength = self.api_strength.unwrap_or(self.cfg.physics_strength) / 50.0;
            ph.rate_override = match self.cfg.physics_fps {
                0 => None,
                1 => Some(None),
                f => Some(Some(f as f32)),
            };
        }
        let m = &mut self.model;
        m.reset_pose();
        let mo = if !tracking && self.lost_motion.is_some() { &mut self.lost_motion } else { &mut self.motion };
        if let Some(mo) = mo {
            mo.apply(m, dt);
        }
        if let Some((mo, _, w)) = &mut self.oneshot {
            *w = (*w + dt / 0.3).min(1.0);
            if !mo.apply_once(m, dt, *w) {
                self.oneshot = None;
            }
        }
        let ctx = MapCtx {
            tracking,
            blink: self.blink.value(dt),
            breath: 0.5 + 0.5 * (self.time * std::f32::consts::TAU / 3.5).sin(),
        };
        self.mapping.apply(m, inputs, &ctx, dt);

        for e in &mut self.expressions {
            let target = if e.active { 1.0 } else { 0.0 };
            let step = if e.fade <= 0.001 { 1.0 } else { dt / e.fade };
            e.weight = if e.weight < target { (e.weight + step).min(target) } else { (e.weight - step).max(target) };
            if e.weight <= 0.0 {
                continue;
            }
            for &(p, v, b) in &e.params {
                let cur = m.params()[p];
                let nv = match b {
                    ExpBlend::Add => cur + v * e.weight,
                    ExpBlend::Multiply => cur * (1.0 + (v - 1.0) * e.weight),
                    ExpBlend::Overwrite => cur + (v - cur) * e.weight,
                };
                m.set_param(p, nv);
            }
        }

        if let Some(p) = &self.pose {
            p.apply(m);
        }
        if physics_on && self.physics_enabled {
            if let Some(ph) = &mut self.physics {
                ph.evaluate(m, dt);
            }
        }
        m.update();
    }

    pub fn rebind(&mut self, gpu: &Gpu) {
        self.batch.rebind(gpu);
    }

    /// Allocates smooth masks: one per distinct mask list, bounds = union of clipped drawables (NDC).
    fn assign_masks(&mut self, a: &Affine, al: &mut MaskAlloc) {
        let m = &self.model;
        let (flags, op) = (m.dynamic_flags(), m.opacities());
        let mut groups: Vec<(usize, [f32; 4], usize)> = Vec::new();
        for (i, d) in m.drawables.iter().enumerate() {
            if d.masks.is_empty() || flags[i] & IS_VISIBLE == 0 || op[i] < 0.001 {
                continue;
            }
            let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for p in &self.pos_scratch[d.vertex_offset..d.vertex_offset + d.vertex_count] {
                let x = a.m[0] * p[0] + a.m[2] * p[1] + a.t[0];
                let y = a.m[1] * p[0] + a.m[3] * p[1] + a.t[1];
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
            let key = self.mask_key[i];
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.1 = [g.1[0].min(b[0]), g.1[1].min(b[1]), g.1[2].max(b[2]), g.1[3].max(b[3])],
                None => groups.push((key, b, i)),
            }
        }
        for (key, b, rep) in groups {
            if b[0] > b[2] || b[2] < -1.0 || b[0] > 1.0 || b[3] < -1.0 || b[1] > 1.0 {
                continue; // off-screen
            }
            let Some(set) = al.alloc(b) else { break };
            self.mask_jobs.push((set, rep));
            for (i, k) in self.mask_key.iter().enumerate() {
                if *k == key {
                    self.mask_set[i] = set as i32;
                }
            }
        }
    }

    /// Renders this model's smooth masks into the atlas.
    pub fn draw_masks(&self, mp: &mut MaskPainter, al: &MaskAlloc) {
        if self.mask_jobs.is_empty() {
            return;
        }
        mp.bind(&self.batch);
        let m = &self.model;
        for &(set, rep) in &self.mask_jobs {
            mp.cell(al, set);
            for &mi in &m.drawables[rep].masks {
                let md = &m.drawables[mi];
                let r = md.index_offset as u32..(md.index_offset + md.index_count) as u32;
                mp.draw(&self.batch, md.texture, r, md.vertex_offset as i32, set);
            }
        }
    }

    pub fn upload(&mut self, gpu: &Gpu, vw: f32, vh: f32, masks: Option<&mut MaskAlloc>) {
        let m = &self.model;
        let flags = m.dynamic_flags();
        let mut changed = false;
        for (i, d) in m.drawables.iter().enumerate() {
            if self.first_upload || flags[i] & VERTEX_POSITIONS_DID_CHANGE != 0 {
                let src = m.positions(i);
                let dst = &mut self.pos_scratch[d.vertex_offset..d.vertex_offset + d.vertex_count];
                for (o, v) in dst.iter_mut().zip(src) {
                    *o = [v.x, v.y];
                }
                changed = true;
            }
        }
        self.first_upload = false;
        if changed {
            self.batch.set_positions(&gpu.queue, &self.pos_scratch);
        }
        let affine = self.eff_xf().affine(vw, vh, true);
        self.mask_set.fill(-1);
        self.mask_jobs.clear();
        if let Some(al) = masks {
            self.assign_masks(&affine, al);
        }
        let m = &self.model;
        let (op, mul, scr) = (m.opacities(), m.multiply_colors(), m.screen_colors());
        for i in 0..m.drawables.len() {
            let (a, b) = (mul[i], scr[i]);
            let at = self.art_tint[i];
            self.draw_scratch[i] = DrawData {
                multiply: [a.x * self.tint[0] * at[0], a.y * self.tint[1] * at[1], a.z * self.tint[2] * at[2], 1.0],
                screen: [b.x, b.y, b.z, 0.0],
                opacity: [op[i] * at[3], 0.0, 0.0, 0.0],
                mask: [self.mask_set[i] as f32, if m.drawables[i].constant_flags & IS_INVERTED_MASK != 0 { 1.0 } else { 0.0 }, 0.0, 0.0],
            };
        }
        self.batch.set_draws(&gpu.queue, &self.draw_scratch);
        for (i, &r) in m.render_orders().iter().enumerate() {
            if let Some(slot) = self.order.get_mut(r as usize) {
                *slot = i;
            }
        }
        self.batch.set_xf(&gpu.queue, &affine);
    }

    pub fn draw(&self, p: &mut Painter) {
        let m = &self.model;
        let (flags, op) = (m.dynamic_flags(), m.opacities());
        p.bind(&self.batch);
        for &i in &self.order {
            let d = &m.drawables[i];
            if d.index_count == 0 || flags[i] & IS_VISIBLE == 0 || op[i] < 0.001 {
                continue;
            }
            let blend = if d.constant_flags & BLEND_ADDITIVE != 0 {
                Blend::Additive
            } else if d.constant_flags & BLEND_MULTIPLICATIVE != 0 {
                Blend::Multiply
            } else {
                Blend::Normal
            };
            let clip = if d.masks.is_empty() || self.mask_set[i] >= 0 {
                Clip::None
            } else {
                if p.begin_mask(self.mask_key[i]) {
                    for &mi in &d.masks {
                        let md = &m.drawables[mi];
                        let r = md.index_offset as u32..(md.index_offset + md.index_count) as u32;
                        p.mask(&self.batch, md.texture, r, md.vertex_offset as i32, mi as u32);
                    }
                }
                if d.constant_flags & IS_INVERTED_MASK != 0 { Clip::Outside } else { Clip::Inside }
            };
            let r = d.index_offset as u32..(d.index_offset + d.index_count) as u32;
            p.draw(&self.batch, d.texture, r, d.vertex_offset as i32, i as u32, blend, clip);
        }
    }

    fn visible(&self, i: usize) -> bool {
        self.model.dynamic_flags()[i] & IS_VISIBLE != 0 && self.model.opacities()[i] > 0.05
    }

    /// AABB (model units) of currently visible drawables: [min_x, min_y, max_x, max_y].
    pub fn local_bounds(&self) -> [f32; 4] {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for i in 0..self.model.drawables.len() {
            if !self.visible(i) {
                continue;
            }
            for v in self.model.positions(i) {
                b = [b[0].min(v.x), b[1].min(v.y), b[2].max(v.x), b[3].max(v.y)];
            }
        }
        if b[0] > b[2] {
            let c = &self.model.canvas;
            let (w, h) = (c.size.x / c.pixels_per_unit, c.size.y / c.pixels_per_unit);
            return [-w / 2.0, -h / 2.0, w / 2.0, h / 2.0];
        }
        b
    }

    /// Topmost visible triangle under a world-space point.
    pub fn pick(&self, world: [f32; 2]) -> Option<Pick> {
        let p = self.eff_xf().to_local(world);
        for &i in self.order.iter().rev() {
            if !self.visible(i) {
                continue;
            }
            let pos = self.model.positions(i);
            for t in self.model.indices(i).chunks_exact(3) {
                let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
                let d = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
                if d.abs() < 1e-12 {
                    continue;
                }
                let l1 = ((b.y - c.y) * (p[0] - c.x) + (c.x - b.x) * (p[1] - c.y)) / d;
                let l2 = ((c.y - a.y) * (p[0] - c.x) + (a.x - c.x) * (p[1] - c.y)) / d;
                let l3 = 1.0 - l1 - l2;
                if l1 >= 0.0 && l2 >= 0.0 && l3 >= 0.0 {
                    return Some(Pick { drawable: i, tri: [t[0], t[1], t[2]], bary: [l1, l2, l3] });
                }
            }
        }
        None
    }

    pub fn tri_world(&self, drawable: usize, tri: [u16; 3]) -> Option<[[f32; 2]; 3]> {
        let pos = self.model.positions(drawable);
        let xf = self.eff_xf();
        let g = |k: usize| pos.get(tri[k] as usize).map(|v| xf.to_world([v.x, v.y]));
        Some([g(0)?, g(1)?, g(2)?])
    }
}
