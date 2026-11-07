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
}

impl Sprite {
    pub fn load(gpu: &Gpu, path: &Path) -> Result<Sprite> {
        let is_gif = path.extension().map(|e| e.eq_ignore_ascii_case("gif")).unwrap_or(false);
        let mut frames: Vec<(image::RgbaImage, f32)> = Vec::new();
        if is_gif {
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
        Ok(Sprite { batch, total: delays.iter().sum(), delays, time: 0.0, frame: 0, size: [w as f32, h as f32], texture_bytes, tint: [1.0; 3] })
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
        if (0..3).any(|i| (self.tint[i] - t[i]).abs() > 1e-3) {
            self.tint = t;
            let mut d = crate::gpu::DrawData::plain(1.0);
            d.multiply = [t[0], t[1], t[2], 1.0];
            self.batch.set_draws(&gpu.queue, &[d]);
        }
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
