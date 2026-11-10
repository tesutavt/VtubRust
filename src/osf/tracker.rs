//! The tracking pipeline: face detection → 66 landmarks → pupils → head pose → features.
//! A single-face port of OpenSeeFace's `tracker.py` (static 3D model, no RetinaFace), kept
//! deliberately close to the original, including its (row, column) landmark convention,
//! so the numbers mean the same as those of `facetracker.exe`.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};

use super::face3d::FACE_3D;
use super::math::{self, Camera, M3, V3};
use super::ort::{Runtime, Session};

/// An RGB8 camera frame.
pub struct Frame<'a> {
    pub w: usize,
    pub h: usize,
    pub rgb: &'a [u8],
}

const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
const CONTOUR: [usize; 14] = [0, 1, 8, 15, 16, 27, 28, 29, 30, 31, 32, 33, 34, 35];

/// pixel (0..255) → network input: `p * K[c] + B[c]`.
fn norm_k(c: usize) -> (f32, f32) {
    (1.0 / (STD[c] * 255.0), -MEAN[c] / STD[c])
}

fn clamp_to_im(x: f32, y: f32, w: usize, h: usize) -> (i32, i32) {
    let x = x.clamp(0.0, w as f32 - 1.0);
    let y = y.clamp(0.0, h as f32 - 1.0);
    (x as i32, (y + 1.0) as i32)
}

/// Float image, interleaved channels.
struct Img {
    w: usize,
    h: usize,
    c: usize,
    d: Vec<f32>,
}

impl Img {
    fn crop(f: &Frame, x1: usize, y1: usize, x2: usize, y2: usize) -> Img {
        let (w, h) = (x2 - x1, y2 - y1);
        let mut d = Vec::with_capacity(w * h * 3);
        for y in y1..y2 {
            let row = &f.rgb[(y * f.w + x1) * 3..(y * f.w + x2) * 3];
            d.extend(row.iter().map(|&v| v as f32));
        }
        Img { w, h, c: 3, d }
    }

    /// Bilinear resize with OpenCV's `INTER_LINEAR` sampling (pixel centres, replicated border).
    fn resize(&self, dw: usize, dh: usize) -> Img {
        let mut d = vec![0.0f32; dw * dh * self.c];
        let axis = |n_src: usize, n_dst: usize| -> Vec<(usize, usize, f32)> {
            let scale = n_src as f32 / n_dst as f32;
            (0..n_dst)
                .map(|i| {
                    let s = (i as f32 + 0.5) * scale - 0.5;
                    let f = s.floor();
                    let t = s - f;
                    let a = (f as i64).clamp(0, n_src as i64 - 1) as usize;
                    let b = (f as i64 + 1).clamp(0, n_src as i64 - 1) as usize;
                    (a, b, t)
                })
                .collect()
        };
        let (xs, ys) = (axis(self.w, dw), axis(self.h, dh));
        let c = self.c;
        for (y, &(y0, y1, ty)) in ys.iter().enumerate() {
            let (r0, r1) = (&self.d[y0 * self.w * c..(y0 + 1) * self.w * c], &self.d[y1 * self.w * c..(y1 + 1) * self.w * c]);
            let out = &mut d[y * dw * c..(y + 1) * dw * c];
            for (x, &(x0, x1, tx)) in xs.iter().enumerate() {
                for k in 0..c {
                    let top = r0[x0 * c + k] + (r0[x1 * c + k] - r0[x0 * c + k]) * tx;
                    let bot = r1[x0 * c + k] + (r1[x1 * c + k] - r1[x0 * c + k]) * tx;
                    out[x * c + k] = top + (bot - top) * ty;
                }
            }
        }
        Img { w: dw, h: dh, c, d }
    }

    /// Bilinear sample, zero outside (OpenCV `warpAffine` constant border).
    fn sample(&self, x: f32, y: f32, k: usize) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let px = |xi: i64, yi: i64| -> f32 {
            if xi < 0 || yi < 0 || xi >= self.w as i64 || yi >= self.h as i64 {
                0.0
            } else {
                self.d[(yi as usize * self.w + xi as usize) * self.c + k]
            }
        };
        let (xi, yi) = (x0 as i64, y0 as i64);
        let top = px(xi, yi) * (1.0 - tx) + px(xi + 1, yi) * tx;
        let bot = px(xi, yi + 1) * (1.0 - tx) + px(xi + 1, yi + 1) * tx;
        top * (1.0 - ty) + bot * ty
    }

    /// NCHW tensor, normalised.
    fn to_nchw(&self) -> Vec<f32> {
        let n = self.w * self.h;
        let mut t = vec![0.0f32; n * 3];
        for k in 0..3 {
            let (m, b) = norm_k(k);
            for i in 0..n {
                t[k * n + i] = self.d[i * 3 + k] * m + b;
            }
        }
        t
    }
}

fn logit(p: f32, factor: f32) -> f32 {
    let p = p.clamp(0.0000001, 0.9999999);
    (p / (1.0 - p)).ln() / factor
}

fn angle(p1: [f64; 2], p2: [f64; 2]) -> f64 {
    let a = (p2[1] - p1[1]).atan2(p2[0] - p1[0]);
    a.rem_euclid(2.0 * std::f64::consts::PI)
}

fn rotate(origin: [f64; 2], point: [f64; 2], a: f64) -> [f64; 2] {
    let a = -a;
    let (dx, dy) = (point[0] - origin[0], point[1] - origin[1]);
    [origin[0] + a.cos() * dx - a.sin() * dy, origin[1] + a.sin() * dx + a.cos() * dy]
}

// ------------------------------------------------------------------ features

/// Running median of medians ("remedian", buffers of 64 values).
#[derive(Default)]
struct Remedian {
    all: Vec<f64>,
    more: Option<Box<Remedian>>,
}

fn median_of(v: &[f64]) -> f64 {
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n < 3 {
        return (v[0] + v[n - 1]) / 2.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if n % 2 == 0 {
        (s[n / 2] + s[n / 2 - 1]) / 2.0
    } else {
        s[n / 2]
    }
}

impl Remedian {
    fn add(&mut self, x: f64) {
        self.all.push(x);
        if self.all.len() == 64 {
            let m = median_of(&self.all);
            self.more.get_or_insert_with(Default::default).add(m);
            self.all.clear();
        }
    }

    fn median(&self) -> f64 {
        match &self.more {
            Some(m) => m.median(),
            None => median_of(&self.all),
        }
    }
}

/// One self-calibrating feature: learns the median and the extremes of a measurement and
/// reports it as -1..1 around the median.
struct Feature {
    median: Remedian,
    min: Option<f64>,
    max: Option<f64>,
    hard_min: f64,
    hard_max: f64,
    threshold: f64,
    last: f64,
    current_median: f64,
    first_seen: f64,
    updating: bool,
    max_updates: f64,
}

const ALPHA: f64 = 0.2;
const HARD_FACTOR: f64 = 0.15;
const DECAY: f64 = 0.001;

impl Feature {
    fn new(threshold: f64, max_updates: f64) -> Feature {
        Feature { median: Remedian::default(), min: None, max: None, hard_min: 0.0, hard_max: 0.0, threshold, last: 0.0, current_median: 0.0, first_seen: -1.0, updating: true, max_updates }
    }

    fn update(&mut self, x: f64, now: f64) -> f64 {
        if self.max_updates > 0.0 && self.first_seen == -1.0 {
            self.first_seen = now;
        }
        let new = self.update_state(x, now);
        let filtered = self.last * ALPHA + new * (1.0 - ALPHA);
        self.last = if filtered.is_finite() { filtered } else { 0.0 };
        self.last
    }

    fn update_state(&mut self, x: f64, now: f64) -> f64 {
        let updating = self.updating && (self.max_updates == 0.0 || now - self.first_seen < self.max_updates);
        if updating {
            self.median.add(x);
            self.current_median = self.median.median();
        } else {
            self.updating = false;
        }
        let median = self.current_median;
        match self.min {
            None => {
                if x < median && (median - x) / median > self.threshold {
                    if updating {
                        self.min = Some(x);
                        self.hard_min = x + HARD_FACTOR * (median - x);
                    }
                    return -1.0;
                }
                return 0.0;
            }
            Some(mn) => {
                if x < mn {
                    if updating {
                        self.min = Some(x);
                        self.hard_min = x + HARD_FACTOR * (median - x);
                    }
                    return -1.0;
                }
            }
        }
        match self.max {
            None => {
                if x > median && (x - median) / median > self.threshold {
                    if updating {
                        self.max = Some(x);
                        self.hard_max = x - HARD_FACTOR * (x - median);
                    }
                    return 1.0;
                }
                return 0.0;
            }
            Some(mx) => {
                if x > mx {
                    if updating {
                        self.max = Some(x);
                        self.hard_max = x - HARD_FACTOR * (x - median);
                    }
                    return 1.0;
                }
            }
        }
        let (mut mn, mut mx) = (self.min.unwrap(), self.max.unwrap());
        if updating {
            if mn < self.hard_min {
                mn = self.hard_min * DECAY + mn * (1.0 - DECAY);
                self.min = Some(mn);
            }
            if mx > self.hard_max {
                mx = self.hard_max * DECAY + mx * (1.0 - DECAY);
                self.max = Some(mx);
            }
        }
        if x < median {
            -(1.0 - (x - mn) / (median - mn))
        } else if x > median {
            (x - median) / (mx - median)
        } else {
            0.0
        }
    }
}

/// Feature order of the tracker packet.
pub const FEATURE_NAMES: [&str; 14] = [
    "eye_l", "eye_r", "eyebrow_steepness_l", "eyebrow_updown_l", "eyebrow_quirk_l", "eyebrow_steepness_r", "eyebrow_updown_r", "eyebrow_quirk_r",
    "mouth_corner_updown_l", "mouth_corner_inout_l", "mouth_corner_updown_r", "mouth_corner_inout_r", "mouth_open", "mouth_wide",
];

struct FeatureExtractor {
    f: Vec<Feature>,
}

impl FeatureExtractor {
    fn new(max_updates: f64) -> FeatureExtractor {
        // Thresholds in FEATURE_NAMES order.
        let th = [0.15, 0.15, 0.05, 0.15, 0.05, 0.05, 0.15, 0.05, 0.15, 0.02, 0.15, 0.02, 0.15, 0.02];
        FeatureExtractor { f: th.iter().map(|t| Feature::new(*t, max_updates)).collect() }
    }

    fn align(a: [f64; 2], b: [f64; 2], pts: &[[f64; 2]]) -> (f64, Vec<[f64; 2]>) {
        let mut alpha = angle(a, b).to_degrees();
        if alpha >= 90.0 {
            alpha = -(alpha - 180.0);
        }
        if alpha <= -90.0 {
            alpha = -(alpha + 180.0);
        }
        let alpha = alpha.to_radians();
        (alpha, pts.iter().map(|p| rotate(a, *p, alpha)).collect())
    }

    /// `p`: the 2D (x, y) part of the normalised 3D points.
    fn update(&mut self, p: &[[f64; 2]], now: f64) -> [f32; 14] {
        let mut out = [0.0f64; 14];
        let ndx = ((p[0][0] - p[16][0]) + (p[1][0] - p[15][0])) / 2.0;
        let ndy = ((p[27][1] - p[28][1]) + (p[28][1] - p[29][1]) + (p[29][1] - p[30][1])) / 3.0;
        let eye = |a: usize, b: usize, q: [usize; 4]| {
            let (al, f) = Self::align(p[a], p[b], &[p[q[0]], p[q[1]], p[q[2]], p[q[3]]]);
            (al, (((f[0][1] + f[1][1]) / 2.0 - (f[2][1] + f[3][1]) / 2.0) / ndy).abs())
        };
        let (a1, v) = eye(42, 45, [43, 44, 47, 46]);
        out[0] = self.f[0].update(v, now);
        let (a2, v) = eye(36, 39, [37, 38, 41, 40]);
        out[1] = self.f[1].update(v, now);

        let (a3, _) = Self::align(p[0], p[16], &[]);
        let (a4, _) = Self::align(p[31], p[35], &[]);
        let norm_angle = (a1.to_degrees() + a2.to_degrees() + a3.to_degrees() + a4.to_degrees()) / 4.0;
        // The original subtracts the first point's y from both coordinates of the next three.
        let quirk = |f: &[[f64; 2]]| f[1..4].iter().flat_map(|q| [(q[0] - f[0][1]).abs(), (q[1] - f[0][1]).abs()]).fold(0.0f64, f64::max) / ndy;
        let (a, f) = Self::align(p[22], p[26], &[p[22], p[23], p[24], p[25], p[26]]);
        out[2] = self.f[2].update(-a.to_degrees() - norm_angle, now);
        out[4] = self.f[4].update(quirk(&f), now);
        let (a, f) = Self::align(p[17], p[21], &[p[17], p[18], p[19], p[20], p[21]]);
        out[5] = self.f[5].update(a.to_degrees() - norm_angle, now);
        out[7] = self.f[7].update(quirk(&f), now);

        out[3] = self.f[3].update(((p[22][1] + p[26][1]) / 2.0 - p[27][1]) / ndy, now);
        out[6] = self.f[6].update(((p[17][1] + p[21][1]) / 2.0 - p[27][1]) / ndy, now);

        let upper = (p[49][1] + p[50][1] + p[51][1]) / 3.0;
        let center = (p[50][0] + p[60][0] + p[27][0] + p[30][0] + p[64][0] + p[55][0]) / 6.0;
        out[8] = self.f[8].update((upper - p[62][1]) / ndy, now);
        out[9] = self.f[9].update((center - p[62][0]).abs() / ndx, now);
        out[10] = self.f[10].update((upper - p[58][1]) / ndy, now);
        out[11] = self.f[11].update((center - p[58][0]).abs() / ndx, now);
        out[12] = self.f[12].update((((p[59][1] + p[60][1] + p[61][1]) / 3.0) - ((p[63][1] + p[64][1] + p[65][1]) / 3.0)).abs() / ndy, now);
        out[13] = self.f[13].update((p[58][0] - p[62][0]).abs() / ndx, now);
        let mut r = [0.0f32; 14];
        for k in 0..14 {
            r[k] = out[k] as f32;
        }
        r
    }
}

// ------------------------------------------------------------------ tracker

/// Result for one frame.
#[derive(Clone, Debug)]
pub struct Face {
    pub conf: f32,
    /// 66 face landmarks + 2 pupils as (row, column, confidence).
    pub lms: [[f32; 3]; 68],
    pub success: bool,
    pub pnp_error: f32,
    pub euler: [f32; 3],
    pub translation: [f32; 3],
    pub quaternion: [f32; 4],
    /// [right eye, left eye] openness 0..1 (None until the features are calibrated).
    pub eye_blink: Option<[f32; 2]>,
    pub features: [f32; 14],
}

pub struct Tracker {
    width: usize,
    height: usize,
    landmark: Session,
    gaze: Session,
    detection: Session,
    res: usize,
    out_res: usize,
    logit_factor: f32,
    threshold: f32,
    detection_threshold: f32,
    camera: Camera,
    /// Last face box (x, y, w, h).
    face: Option<[f32; 4]>,
    rotation: Option<V3>,
    translation: V3,
    fail_count: u32,
    features: FeatureExtractor,
    current_features: [f32; 14],
    eye_blink: Option<[f32; 2]>,
    start: Instant,
    base_scale_v: [f64; 3],
    base_scale_h: [f64; 3],
}

/// File name of the landmark model for an OpenSeeFace `--model` value.
pub fn landmark_model(model_type: i32) -> &'static str {
    match model_type {
        0 => "lm_model0_opt.onnx",
        1 => "lm_model1_opt.onnx",
        2 => "lm_model2_opt.onnx",
        4 => "lm_model4_opt.onnx",
        -2 => "lm_modelV_opt.onnx",
        -3 => "lm_modelU_opt.onnx",
        _ => "lm_model3_opt.onnx",
    }
}

pub const GAZE_MODEL: &str = "mnv3_gaze32_split_opt.onnx";
pub const DETECTION_MODEL: &str = "mnv3_detection_opt.onnx";

impl Tracker {
    pub fn new(rt: &'static Runtime, models: &Path, width: usize, height: usize, model_type: i32, threads: u32) -> Result<Tracker> {
        let load = |name: &str| std::fs::read(models.join(name)).with_context(|| format!("model {} not found in {}", name, models.display()));
        let landmark = rt.session(&load(landmark_model(model_type))?, threads, "input", &["output"])?;
        let gaze = rt.session(&load(GAZE_MODEL)?, 1, "input", &["output"])?;
        let detection = rt.session(&load(DETECTION_MODEL)?, 1, "input", &["output", "maxpool"])?;
        let small = model_type < -1;
        let f = &FACE_3D;
        Ok(Tracker {
            width,
            height,
            landmark,
            gaze,
            detection,
            res: if small { 112 } else { 224 },
            out_res: if small { 13 } else { 27 },
            logit_factor: 16.0,
            threshold: if model_type < 0 { 0.87 } else { 0.6 },
            detection_threshold: 0.6,
            camera: Camera::new(width as f64, height as f64),
            face: None,
            rotation: None,
            translation: [0.0; 3],
            fail_count: 0,
            features: FeatureExtractor::new(900.0),
            current_features: [0.0; 14],
            eye_blink: None,
            start: Instant::now(),
            base_scale_v: [f[27][1] - f[28][1], f[28][1] - f[29][1], f[29][1] - f[30][1]],
            base_scale_h: [(f[0][0] - f[16][0]).abs(), (f[36][0] - f[39][0]).abs(), (f[42][0] - f[45][0]).abs()],
        })
    }

    fn detect(&self, frame: &Frame) -> Result<Option<[f32; 4]>> {
        let im = Img::crop(frame, 0, 0, frame.w, frame.h).resize(224, 224);
        let mut t = im.to_nchw();
        let out = self.detection.run(&mut t, &[1, 3, 224, 224])?;
        let (o, mp) = (&out[0].data, &out[1].data);
        let n = 56 * 56;
        // Keep local maxima only, take the strongest.
        let mut best = (0usize, f32::MIN);
        for i in 0..n {
            let v = if o[i] != mp[i] { 0.0 } else { o[i] };
            if v > best.1 {
                best = (i, v);
            }
        }
        let (idx, c) = best;
        if c < self.detection_threshold {
            return Ok(None);
        }
        let (y, x) = ((idx / 56) as f32 * 4.0, (idx % 56) as f32 * 4.0);
        let r = o[n + idx] * 112.0;
        let (sx, sy) = (frame.w as f32 / 224.0, frame.h as f32 / 224.0);
        Ok(Some([(x - r) * sx, (y - r) * sy, 2.0 * r * sx, 2.0 * r * sy]))
    }

    /// Decodes the heatmaps: (average confidence, 66 × (row, column, confidence)).
    fn landmarks(&self, t: &[f32], crop: (i32, i32, f32, f32)) -> (f32, [[f32; 3]; 66]) {
        let (crop_x1, crop_y1, scale_x, scale_y) = crop;
        let o = self.out_res + 1;
        let plane = o * o;
        let res = self.res as f32 - 1.0;
        let mut lms = [[0.0f32; 3]; 66];
        let mut sum = 0.0;
        for (i, lm) in lms.iter_mut().enumerate() {
            let h = &t[i * plane..(i + 1) * plane];
            let mut m = 0;
            for k in 1..plane {
                if h[k] > h[m] {
                    m = k;
                }
            }
            let conf = h[m];
            let off_x = res * logit(t[(66 + i) * plane + m], self.logit_factor);
            let off_y = res * logit(t[(132 + i) * plane + m], self.logit_factor);
            let row = crop_y1 as f32 + scale_y * (res * (m / o) as f32 / self.out_res as f32 + off_x);
            let col = crop_x1 as f32 + scale_x * (res * (m % o) as f32 / self.out_res as f32 + off_y);
            sum += conf;
            *lm = if row.is_finite() && col.is_finite() && conf.is_finite() { [row, col, conf] } else { [0.0; 3] };
        }
        (sum / 66.0, lms)
    }

    /// Pupil positions: [(open, row, column, confidence); 2] for the right and the left eye.
    fn eye_state(&self, frame: &Frame, lms: &[[f32; 3]; 66]) -> Option<[[f32; 4]; 2]> {
        // Face box around the landmarks, in (x, y).
        let (mut x1, mut y1, mut x2, mut y2) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for l in lms {
            x1 = x1.min(l[1]);
            x2 = x2.max(l[1]);
            y1 = y1.min(l[0]);
            y2 = y2.max(l[0]);
        }
        let (rx, ry) = (1.2 * (x2 - x1) / 2.0, 1.2 * (y2 - y1) / 2.0);
        let (cx, cy) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
        let (fx1, fy1) = clamp_to_im(cx - rx, cy - ry, frame.w, frame.h);
        let (fx2, fy2) = clamp_to_im(cx + rx + 1.0, cy + ry + 1.0, frame.w, frame.h);
        let fy2 = fy2.min(frame.h as i32);
        if fx2 <= fx1 || fy2 <= fy1 {
            return None;
        }
        let face = Img::crop(frame, fx1 as usize, fy1 as usize, fx2 as usize, fy2 as usize);
        let pt = |i: usize| [(lms[i][1] - fx1 as f32) as i32 as f64, (lms[i][0] - fy1 as f32) as i32 as f64];

        let mut input = vec![0.0f32; 2 * 3 * 32 * 32];
        let mut info = [([0.0f64; 2], 0.0f64, 0i32, 0i32, [0.0f32; 2]); 2];
        for (e, (outer, inner, flip)) in [(36usize, 39usize, false), (42, 45, true)].into_iter().enumerate() {
            let c1 = pt(outer);
            let a = angle(c1, pt(inner));
            let c2 = rotate(c1, pt(inner), a);
            let center = [(c1[0] + c2[0]) / 2.0, (c1[1] + c2[1]) / 2.0];
            let r = ((c1[0] - c2[0]).powi(2) + (c1[1] - c2[1]).powi(2)).sqrt() / 2.0;
            let (ex1, ey1) = clamp_to_im((center[0] - r * 1.4) as f32, (center[1] - r * 1.2) as f32, face.w, face.h);
            let (ex2, ey2) = clamp_to_im((center[0] + r * 1.4) as f32, (center[1] + r * 1.2) as f32, face.w, face.h);
            let ey2 = ey2.min(face.h as i32);
            if ex2 <= ex1 || ey2 <= ey1 {
                return None;
            }
            // The eye patch of the face image rotated by `a` around the outer corner
            // (cv2.getRotationMatrix2D + warpAffine): destination → source mapping.
            let (cs, sn) = (a.cos(), a.sin());
            let (ew, eh) = ((ex2 - ex1) as usize, (ey2 - ey1) as usize);
            let mut patch = Img { w: ew, h: eh, c: 3, d: vec![0.0; ew * eh * 3] };
            for y in 0..eh {
                for x in 0..ew {
                    let (dx, dy) = ((ex1 as usize + x) as f64 - c1[0], (ey1 as usize + y) as f64 - c1[1]);
                    let (sx, sy) = (c1[0] + cs * dx - sn * dy, c1[1] + sn * dx + cs * dy);
                    let xo = if flip { ew - 1 - x } else { x };
                    for k in 0..3 {
                        patch.d[(y * ew + xo) * 3 + k] = face.sample(sx as f32, sy as f32, k);
                    }
                }
            }
            let small = patch.resize(32, 32);
            // Tensor layout (N, C, W, H): the original transposes rows and columns.
            for k in 0..3 {
                let (m, b) = norm_k(k);
                for x in 0..32 {
                    for y in 0..32 {
                        input[((e * 3 + k) * 32 + x) * 32 + y] = small.d[(y * 32 + x) * 3 + k] * m + b;
                    }
                }
            }
            info[e] = (c1, a, ex1, ey1, [ew as f32 / 32.0, eh as f32 / 32.0]);
        }
        let out = self.gaze.run(&mut input, &[2, 3, 32, 32]).ok()?;
        let r = &out[0].data;
        let mut state = [[1.0f32, 0.0, 0.0, 0.0]; 2];
        for e in 0..2 {
            let base = e * 3 * 64;
            let h = &r[base..base + 64];
            let mut m = 0;
            for k in 1..64 {
                if h[k] > h[m] {
                    m = k;
                }
            }
            let (x, y) = ((m / 8) as f32, (m % 8) as f32);
            let conf = h[m];
            let mut eye_x = 32.0 * x / 8.0 + 32.0 * logit(r[base + 64 + m], 8.0);
            let eye_y = 32.0 * y / 8.0 + 32.0 * logit(r[base + 128 + m], 8.0);
            let (c1, a, ex1, ey1, scale) = info[e];
            if e == 1 {
                eye_x = 32.0 - eye_x;
            }
            let px = ex1 as f32 + scale[0] * eye_x;
            let py = ey1 as f32 + scale[1] * eye_y;
            let p = rotate(c1, [px as f64, py as f64], -a);
            let (gx, gy) = (p[0] as f32 + fx1 as f32, p[1] as f32 + fy1 as f32);
            state[e] = if gx.is_finite() && gy.is_finite() && conf.is_finite() { [1.0, gy, gx, conf] } else { [1.0, 0.0, 0.0, 0.0] };
        }
        Some(state)
    }

    fn lose_face(&mut self) {
        self.rotation = None;
        self.translation = [0.0; 3];
        self.eye_blink = None;
        self.fail_count = 0;
        self.face = None;
    }

    /// Head pose, 3D points, features.
    fn estimate(&mut self, conf: f32, lms: &[[f32; 3]; 68]) -> Face {
        let mut face = Face { conf, lms: *lms, success: false, pnp_error: 99999.0, euler: [0.0; 3], translation: [0.0; 3], quaternion: [0.0; 4], eye_blink: self.eye_blink, features: self.current_features };
        let obj: Vec<V3> = CONTOUR.iter().map(|&i| FACE_3D[i]).collect();
        let img: Vec<[f64; 2]> = CONTOUR.iter().map(|&i| [lms[i][0] as f64, lms[i][1] as f64]).collect();
        let (r0, t0) = match self.rotation {
            Some(r) => (r, self.translation),
            None => math::initial_pose(&self.camera, &obj, &img),
        };
        let Some((rvec, tvec, _)) = math::solve_pnp(&self.camera, &obj, &img, r0, t0) else {
            self.rotation = None;
            self.translation = [0.0; 3];
            return face;
        };
        self.rotation = Some(rvec);
        self.translation = tvec;
        let rmat: M3 = math::rodrigues(rvec);
        let rinv = math::transpose(&rmat);
        let cam = self.camera;
        let to_cam = |p: V3| {
            let q = math::mat_vec(&rmat, p);
            cam.apply([q[0] + tvec[0], q[1] + tvec[1], q[2] + tvec[2]])
        };
        // Lifts an image point (row, column) to model space at a given depth.
        let lift = |row: f64, col: f64, depth: f64| {
            let p = cam.unapply([row * depth, col * depth, depth]);
            math::mat_vec(&rinv, [p[0] - tvec[0], p[1] - tvec[1], p[2] - tvec[2]])
        };
        let mut pts = [[0.0f64; 3]; 70];
        let mut err = 0.0;
        for i in 0..66 {
            let q = to_cam(FACE_3D[i]);
            let depth = if q[2] == 0.0 { 0.000001 } else { q[2] };
            pts[i] = lift(lms[i][0] as f64, lms[i][1] as f64, depth);
            if i < 17 || i == 30 {
                err += (lms[i][0] as f64 - q[0] / depth).powi(2) + (lms[i][1] as f64 - q[1] / depth).powi(2);
            }
        }
        if !err.is_finite() {
            err = 9999999.0;
        }
        let dist3 = |a: V3, b: V3| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        let dist2 = |a: usize, b: usize| ((lms[a][0] - lms[b][0]).powi(2) + (lms[a][1] - lms[b][1]).powi(2)).sqrt() as f64;
        // Pupils (66, 67): depth interpolated between the eye corners.
        for (k, (c0, c1)) in [(36usize, 39usize), (42, 45)].into_iter().enumerate() {
            let (d1, d2) = (dist2(66 + k, c0), dist2(66 + k, c1));
            let d = d1 + d2;
            let p = [(pts[c0][0] * d1 + pts[c1][0] * d2) / d, (pts[c0][1] * d1 + pts[c1][1] * d2) / d, (pts[c0][2] * d1 + pts[c1][2] * d2) / d];
            let depth = to_cam(p)[2];
            pts[66 + k] = lift(lms[66 + k][0] as f64, lms[66 + k][1] as f64, depth);
            // Eyeball centres (68, 69): behind the middle of the eye.
            let centre = [(pts[c0][0] + pts[c1][0]) / 2.0, (pts[c0][1] + pts[c1][1]) / 2.0, (pts[c0][2] + pts[c1][2]) / 2.0];
            pts[68 + k] = [centre[0], centre[1], centre[2] - 0.385 * dist3(pts[c0], pts[c1])];
        }
        for p in pts.iter_mut() {
            if p.iter().any(|v| !v.is_finite()) {
                *p = [0.0; 3];
            }
        }
        let pnp_error = (err / (2.0 * CONTOUR.len() as f64)).sqrt();
        if pnp_error > 300.0 {
            self.fail_count += 1;
            if self.fail_count > 5 {
                self.rotation = None;
                self.translation = [0.0; 3];
            }
        } else {
            self.fail_count = 0;
        }
        let e = math::rq_euler(&rmat);
        let q = math::quaternion(&rmat);
        face.success = true;
        face.pnp_error = pnp_error as f32;
        face.euler = [e[0] as f32, e[1] as f32, e[2] as f32];
        face.translation = [tvec[0] as f32, tvec[1] as f32, tvec[2] as f32];
        face.quaternion = [q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32];

        if conf >= 0.4 && pnp_error <= 300.0 {
            let norm = self.normalize(pts);
            let p2: Vec<[f64; 2]> = norm.iter().map(|p| [p[0], p[1]]).collect();
            let f = self.features.update(&p2, self.start.elapsed().as_secs_f64());
            self.current_features = f;
            self.eye_blink = Some([1.0 - (-f[1]).clamp(0.0, 1.0), 1.0 - (-f[0]).clamp(0.0, 1.0)]);
            face.features = f;
            face.eye_blink = self.eye_blink;
        }
        face
    }

    /// Removes roll and scale from the 3D points (nose tip at the origin, bridge straight up).
    fn normalize(&self, mut p: [[f64; 3]; 70]) -> [[f64; 3]; 70] {
        let o = [p[30][0], p[30][1]];
        for q in p.iter_mut() {
            q[0] -= o[0];
            q[1] -= o[1];
        }
        let alpha = angle([p[30][0], p[30][1]], [p[27][0], p[27][1]]) - std::f64::consts::FRAC_PI_2;
        let (s, c) = alpha.sin_cos();
        for q in p.iter_mut() {
            let (x, y) = (q[0], q[1]);
            q[0] = x * c + y * s;
            q[1] = -x * s + y * c;
        }
        let v = ((p[27][1] - p[28][1]) / self.base_scale_v[0] + (p[28][1] - p[29][1]) / self.base_scale_v[1] + (p[29][1] - p[30][1]) / self.base_scale_v[2]) / 3.0;
        let h = ((p[0][0] - p[16][0]).abs() / self.base_scale_h[0] + (p[36][0] - p[39][0]).abs() / self.base_scale_h[1] + (p[42][0] - p[45][0]).abs() / self.base_scale_h[2]) / 3.0;
        for q in p.iter_mut() {
            q[1] /= v;
            q[0] /= h;
        }
        p
    }

    /// Tracks the face in one frame.
    pub fn predict(&mut self, frame: &Frame) -> Result<Option<Face>> {
        anyhow::ensure!(frame.w == self.width && frame.h == self.height && frame.rgb.len() >= frame.w * frame.h * 3, "unexpected frame size");
        // Candidate boxes: the previous face (with a confidence bonus), else a fresh detection.
        let cand = match self.face {
            Some(b) => Some(b),
            None => self.detect(frame)?,
        };
        let Some([x, y, w, h]) = cand else {
            self.lose_face();
            return Ok(None);
        };
        let (cx1, cy1) = clamp_to_im(x - (w * 0.1) as i32 as f32, y - (h * 0.125) as i32 as f32, self.width, self.height);
        let (cx2, cy2) = clamp_to_im(x + w + (w * 0.1) as i32 as f32, y + h + (h * 0.125) as i32 as f32, self.width, self.height);
        let cy2 = cy2.min(self.height as i32);
        if cx2 - cx1 < 4 || cy2 - cy1 < 4 {
            self.lose_face();
            return Ok(None);
        }
        let res = self.res;
        let scale_x = (cx2 - cx1) as f32 / res as f32;
        let scale_y = (cy2 - cy1) as f32 / res as f32;
        let mut t = Img::crop(frame, cx1 as usize, cy1 as usize, cx2 as usize, cy2 as usize).resize(res, res).to_nchw();
        let out = self.landmark.run(&mut t, &[1, 3, res as i64, res as i64])?;
        let (conf, lm66) = self.landmarks(&out[0].data, (cx1, cy1, scale_x, scale_y));
        if !(conf > self.threshold) {
            self.lose_face();
            return Ok(None);
        }
        let eyes = self.eye_state(frame, &lm66).unwrap_or([[1.0, 0.0, 0.0, 0.0]; 2]);
        let mut lms = [[0.0f32; 3]; 68];
        lms[..66].copy_from_slice(&lm66);
        lms[66] = [eyes[0][1], eyes[0][2], eyes[0][3]];
        lms[67] = [eyes[1][1], eyes[1][2], eyes[1][3]];
        let face = self.estimate(conf, &lms);
        // Next frame starts from this face's bounding box.
        let (mut r1, mut c1, mut r2, mut c2) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for l in &lm66 {
            r1 = r1.min(l[0]);
            r2 = r2.max(l[0]);
            c1 = c1.min(l[1]);
            c2 = c2.max(l[1]);
        }
        let b = [c1, r1, c2 - c1, r2 - r1];
        self.face = b.iter().all(|v| v.is_finite()).then_some(b);
        Ok(Some(face))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remedian_and_feature() {
        let mut r = Remedian::default();
        for i in 0..200 {
            r.add(i as f64);
        }
        assert!((r.median() - 95.5).abs() < 40.0);
        assert_eq!(median_of(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median_of(&[4.0, 1.0]), 2.5);
        let mut f = Feature::new(0.15, 0.0);
        for _ in 0..50 {
            f.update(1.0, 0.0);
        }
        assert!(f.update(0.5, 0.0) < -0.5, "well below the median reads as negative");
        for _ in 0..10 {
            f.update(1.0, 0.0);
        }
        assert!(f.update(2.0, 0.0) > 0.5);
    }

    /// End to end on a photo (set RIGRS_ORT, RIGRS_OSF_MODELS, RIGRS_FACE_IMG, optional
    /// RIGRS_OUT for an overlay PNG): `cargo test track_photo -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn track_photo() {
        let rt: &'static Runtime = Box::leak(Box::new(Runtime::load(Path::new(&std::env::var("RIGRS_ORT").unwrap())).unwrap()));
        let img = image::open(std::env::var("RIGRS_FACE_IMG").unwrap()).unwrap().to_rgb8();
        let (w, h) = (640usize, 480usize);
        let model: i32 = std::env::var("RIGRS_OSF_MODEL").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
        let mut tr = Tracker::new(rt, Path::new(&std::env::var("RIGRS_OSF_MODELS").unwrap()), w, h, model, 1).unwrap();
        let mut last = None;
        let mut tracked = 0;
        let t0 = Instant::now();
        let n = 60;
        let mut canvas = image::RgbImage::new(w as u32, h as u32);
        for i in 0..n {
            // Synthetic camera motion: scale, shift and rotate the photo.
            let fi = i as f32;
            let zoom: f32 = std::env::var("RIGRS_FACE_ZOOM").ok().and_then(|v| v.parse().ok()).unwrap_or(0.55);
            let s = (w as f32 / img.width() as f32).min(h as f32 / img.height() as f32) * zoom * (1.0 + 0.05 * (fi * 0.3).sin());
            let ang = (8.0 * (fi * 0.2).sin()).to_radians();
            let (dx, dy) = (15.0 * (fi * 0.17).sin(), 10.0 * (fi * 0.23).cos());
            for y in 0..h {
                for x in 0..w {
                    let (px, py) = (x as f32 - w as f32 / 2.0 - dx, y as f32 - h as f32 / 2.0 - dy);
                    let sx = (ang.cos() * px + ang.sin() * py) / s + img.width() as f32 / 2.0;
                    let sy = (-ang.sin() * px + ang.cos() * py) / s + img.height() as f32 / 2.0;
                    let p = if sx >= 0.0 && sy >= 0.0 && (sx as u32) < img.width() && (sy as u32) < img.height() { *img.get_pixel(sx as u32, sy as u32) } else { image::Rgb([0, 0, 0]) };
                    canvas.put_pixel(x as u32, y as u32, p);
                }
            }
            let f = tr.predict(&Frame { w, h, rgb: canvas.as_raw() }).unwrap();
            if let Some(f) = f {
                tracked += 1;
                if i % 10 == 0 || i == n - 1 {
                    println!("frame {i}: conf {:.2} euler {:?} t {:?} pnp {:.1} blink {:?} roll-in {:.1}°", f.conf, f.euler, f.translation, f.pnp_error, f.eye_blink, ang.to_degrees());
                }
                last = Some(f);
            } else {
                println!("frame {i}: no face");
            }
        }
        println!("{tracked}/{n} frames tracked, {:.1} ms/frame incl. synthesis", t0.elapsed().as_secs_f64() * 1000.0 / n as f64);
        let f = last.expect("face tracked");
        assert!(tracked >= n - 2);
        assert!(f.success && f.conf > 0.6);
        if let Ok(out) = std::env::var("RIGRS_OUT") {
            for (i, l) in f.lms.iter().enumerate() {
                let col = if i >= 66 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 255, 0]) };
                for (ox, oy) in [(0i32, 0i32), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (-1, -1)] {
                    let (x, y) = (l[1] as i32 + ox, l[0] as i32 + oy);
                    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                        canvas.put_pixel(x as u32, y as u32, col);
                    }
                }
            }
            canvas.save(out).unwrap();
        }
    }
}
