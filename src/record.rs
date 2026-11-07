//! Records the avatar's parameter values into a standard Live2D .motion3.json file.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use serde_json::json;

pub struct Recorder {
    start: Instant,
    pub duration: f32,
    ids: Vec<String>,
    /// Parameters excluded up front (physics outputs when not recording physics).
    skip: Vec<bool>,
    keep_unchanged: bool,
    times: Vec<f32>,
    frames: Vec<Vec<f32>>,
    out_dir: PathBuf,
}

const FPS: f32 = 30.0;

impl Recorder {
    pub fn new(ids: Vec<String>, physics_outputs: &[usize], record_physics: bool, keep_unchanged: bool, duration: f32, out_dir: PathBuf) -> Recorder {
        let mut skip = vec![false; ids.len()];
        if !record_physics {
            for &i in physics_outputs {
                if let Some(s) = skip.get_mut(i) {
                    *s = true;
                }
            }
        }
        Recorder { start: Instant::now(), duration, ids, skip, keep_unchanged, times: Vec::new(), frames: Vec::new(), out_dir }
    }

    pub fn elapsed(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    pub fn done(&self) -> bool {
        self.elapsed() >= self.duration
    }

    /// Stores one frame (sampled at ~30 FPS to keep files small).
    pub fn push(&mut self, params: &[f32]) {
        let t = self.elapsed().min(self.duration);
        if let Some(&last) = self.times.last() {
            if t - last < 1.0 / FPS - 1e-4 {
                return;
            }
        }
        self.times.push(t);
        self.frames.push(params.to_vec());
    }

    /// Builds the motion3 JSON and writes it on a worker thread. Returns the target path.
    pub fn finish(self) -> PathBuf {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let path = self.out_dir.join(format!("recording_{secs}.motion3.json"));
        let p2 = path.clone();
        std::thread::spawn(move || {
            if let Err(e) = self.write(&p2) {
                eprintln!("recording: {e:#}");
            }
        });
        path
    }

    fn write(&self, path: &Path) -> Result<()> {
        let duration = self.times.last().copied().unwrap_or(0.0).max(1.0 / FPS);
        let mut curves = Vec::new();
        let (mut seg_total, mut pt_total) = (0usize, 0usize);
        for (pi, id) in self.ids.iter().enumerate() {
            if self.skip[pi] || self.frames.is_empty() {
                continue;
            }
            let vals: Vec<f32> = self.frames.iter().map(|f| f[pi]).collect();
            let (lo, hi) = vals.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
            if !self.keep_unchanged && hi - lo < 1e-4 {
                continue;
            }
            // Linear segments; drop points that lie on the line between their neighbours.
            let mut keep = vec![true; vals.len()];
            for i in 1..vals.len().saturating_sub(1) {
                let (t0, t1, t2) = (self.times[i - 1], self.times[i], self.times[i + 1]);
                let lerp = vals[i - 1] + (vals[i + 1] - vals[i - 1]) * ((t1 - t0) / (t2 - t0).max(1e-6));
                if (vals[i] - lerp).abs() < 1e-3 * (hi - lo).max(1.0) {
                    keep[i] = false;
                }
            }
            let mut seg: Vec<f32> = Vec::new();
            let mut first = true;
            let mut count = 0usize;
            for i in 0..vals.len() {
                if !keep[i] {
                    continue;
                }
                if first {
                    seg.extend([self.times[i], vals[i]]);
                    first = false;
                } else {
                    seg.extend([0.0, self.times[i], vals[i]]);
                    count += 1;
                }
            }
            if count == 0 {
                seg.extend([0.0, duration, vals[vals.len() - 1]]);
                count = 1;
            }
            seg_total += count;
            pt_total += count + 1;
            curves.push(json!({ "Target": "Parameter", "Id": id, "Segments": seg }));
        }
        let doc = json!({
            "Version": 3,
            "Meta": {
                "Duration": duration,
                "Fps": FPS,
                "Loop": true,
                "AreBeziersRestricted": true,
                "CurveCount": curves.len(),
                "TotalSegmentCount": seg_total,
                "TotalPointCount": pt_total,
                "UserDataCount": 0,
                "TotalUserDataSize": 0
            },
            "Curves": curves
        });
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(&doc)?)?;
        Ok(())
    }
}
