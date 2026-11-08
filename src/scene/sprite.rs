//! Image / animated GIF sprite (items and backgrounds).

use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};
use image::AnimationDecoder;

use crate::gpu::{Affine, Batch, Blend, Clip, Gpu, Painter};

pub struct Sprite {
    batch: Batch,
    delays: Vec<f32>,
    total: f32,
    time: f32,
    frame: usize,
    pub size: [f32; 2],
    pub texture_bytes: u64,
    tint: [f32; 3],
    alpha: f32,
    mosaic: f32,
    file_delays: Vec<f32>,
}

impl Sprite {
    pub fn load(gpu: &Gpu, path: &Path) -> Result<Sprite> {
        let is_gif = path.extension().map(|e| e.eq_ignore_ascii_case("gif")).unwrap_or(false);
        let mut frames: Vec<(image::RgbaImage, f32)> = Vec::new();
        if path.is_dir() {
            // VTS: a folder of PNGs in "Items" is an animated item (frames in alphabetical order).
            let mut files: Vec<_> = std::fs::read_dir(path)?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("png")).unwrap_or(false)).collect();
            files.sort();
            for f in files.iter().take(600) {
                let img = image::open(f).with_context(|| format!("open {}", f.display()))?.to_rgba8();
                frames.push((img, 1.0 / 12.0));
            }
            if frames.is_empty() {
                anyhow::bail!("{}: no PNG frames", path.display());
            }
        } else if is_gif {
            let f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
            let dec = image::codecs::gif::GifDecoder::new(BufReader::new(f))?;
            for fr in dec.into_frames().collect_frames()? {
                let (n, d) = fr.delay().numer_denom_ms();
                let ms = if d == 0 { 100.0 } else { n as f32 / d as f32 };
                frames.push((fr.into_buffer(), (ms.max(20.0)) / 1000.0));
            }
        }
        if frames.is_empty() {
            let img = image::open(path).with_context(|| format!("open {}", path.display()))?.to_rgba8();
            frames.push((img, 1.0));
        }
        let (w, h) = frames[0].0.dimensions();
        let mut texture_bytes = 0;
        let mut textures = Vec::with_capacity(frames.len());
        let mut delays = Vec::with_capacity(frames.len());
        for (img, d) in frames {
            let (fw, fh) = img.dimensions();
            texture_bytes += fw as u64 * fh as u64 * 4;
            textures.push(gpu.texture(img.into_raw(), fw, fh));
            delays.push(d);
        }
        let (hw, hh) = (w as f32 / 2.0, h as f32 / 2.0);
        let pos = [[-hw, hh], [hw, hh], [hw, -hh], [-hw, -hh]];
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let batch = Batch::new(gpu, &pos, &uv, &[0, 1, 2, 0, 2, 3], 1, textures);
        Ok(Sprite { batch, total: delays.iter().sum(), file_delays: delays.clone(), delays, time: 0.0, frame: 0, size: [w as f32, h as f32], texture_bytes, tint: [1.0; 3], alpha: 1.0, mosaic: 0.0 })
    }

    pub fn update(&mut self, dt: f32) {
        if self.delays.len() < 2 {
            return;
        }
        self.time = (self.time + dt) % self.total.max(1e-3);
        let mut acc = 0.0;
        self.frame = 0;
        for (i, d) in self.delays.iter().enumerate() {
            acc += d;
            if self.time < acc {
                self.frame = i;
                break;
            }
        }
    }

    pub fn rebind(&mut self, gpu: &Gpu) {
        self.batch.rebind(gpu);
    }

    pub fn set_tint(&mut self, gpu: &Gpu, t: [f32; 3]) {
        let (a, m) = (self.alpha, self.mosaic);
        self.set_look(gpu, t, a, m);
    }

    /// Tint, opacity (fade) and mosaic block size (censor).
    pub fn set_look(&mut self, gpu: &Gpu, t: [f32; 3], alpha: f32, mosaic: f32) {
        if (0..3).any(|i| (self.tint[i] - t[i]).abs() > 1e-3) || (self.alpha - alpha).abs() > 1e-3 || (self.mosaic - mosaic).abs() > 1e-6 {
            self.tint = t;
            self.alpha = alpha;
            self.mosaic = mosaic;
            let mut d = crate::gpu::DrawData::plain(alpha);
            d.multiply = [t[0], t[1], t[2], 1.0];
            d.opacity[1] = mosaic;
            self.batch.set_draws(&gpu.queue, &[d]);
        }
    }

    pub fn tint(&self) -> [f32; 3] {
        self.tint
    }

    pub fn frame_count(&self) -> usize {
        self.delays.len()
    }

    /// Animated items: fps > 0 overrides the frame timing (VTS item FPS, 0–60).
    pub fn set_fps(&mut self, fps: f32) {
        if self.delays.len() < 2 {
            return;
        }
        if fps > 0.0 {
            let d = 1.0 / fps.min(60.0);
            self.delays.iter_mut().for_each(|x| *x = d);
        } else {
            self.delays = self.file_delays.clone();
        }
        self.total = self.delays.iter().sum();
    }

    pub fn current_frame(&self) -> usize {
        self.frame
    }

    pub fn set_affine(&self, gpu: &Gpu, a: &Affine) {
        self.batch.set_xf(&gpu.queue, a);
    }

    pub fn draw(&self, p: &mut Painter) {
        p.bind(&self.batch);
        p.draw(&self.batch, self.frame, 0..6, 0, 0, Blend::Normal, Clip::None);
    }

    pub fn contains_local(&self, l: [f32; 2]) -> bool {
        l[0].abs() <= self.size[0] / 2.0 && l[1].abs() <= self.size[1] / 2.0
    }
}
