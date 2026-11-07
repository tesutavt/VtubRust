pub mod live2d;
pub mod sprite;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::gpu::{Gpu, MaskAlloc, MaskPainter, Painter};
use crate::mapping::Inputs;
use live2d::{Live2D, Xform};
use sprite::Sprite;

pub enum Body {
    Live2D(Box<Live2D>),
    Sprite(Sprite),
}

/// Attachment of an item to a triangle of the avatar mesh.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pin {
    pub drawable: usize,
    pub tri: [u16; 3],
    pub bary: [f32; 3],
    pub angle0: f32,
    pub rot0: f32,
    pub scale_ratio: f32,
}

pub struct Item {
    pub id: u64,
    pub source: PathBuf,
    pub name: String,
    pub body: Body,
    pub xf: Xform,
    pub front: bool,
    pub locked: bool,
    pub pin: Option<Pin>,
    /// Local point that `xf.pos` refers to (visual centre of Live2D items).
    pub pivot: [f32; 2],
}

fn frame_of(av: &Live2D, drawable: usize, tri: [u16; 3], bary: [f32; 3]) -> Option<([f32; 2], f32)> {
    let t = av.tri_world(drawable, tri)?;
    let p = [
        t[0][0] * bary[0] + t[1][0] * bary[1] + t[2][0] * bary[2],
        t[0][1] * bary[0] + t[1][1] * bary[1] + t[2][1] * bary[2],
    ];
    let ang = (t[1][1] - t[0][1]).atan2(t[1][0] - t[0][0]);
    Some((p, ang))
}

impl Item {
    fn render_xf(&self) -> Xform {
        let mut x = self.xf;
        let w = x.to_world(self.pivot);
        x.pos = [2.0 * self.xf.pos[0] - w[0], 2.0 * self.xf.pos[1] - w[1]];
        x
    }

    pub fn hit(&self, world: [f32; 2]) -> bool {
        match &self.body {
            Body::Sprite(s) => s.contains_local(self.xf.to_local(world)),
            Body::Live2D(l) => l.pick(world).is_some(),
        }
    }

    /// Pins the item to whatever avatar triangle lies under its centre.
    pub fn pin_to(&mut self, av: &Live2D) -> bool {
        let Some(pk) = av.pick(self.xf.pos) else { return false };
        let Some((_, ang)) = frame_of(av, pk.drawable, pk.tri, pk.bary) else { return false };
        self.pin = Some(Pin {
            drawable: pk.drawable,
            tri: pk.tri,
            bary: pk.bary,
            angle0: ang,
            rot0: self.xf.rot,
            scale_ratio: self.xf.scale / av.xf.scale,
        });
        true
    }

    pub fn follow(&mut self, av: &Live2D) {
        let Some(pin) = &self.pin else { return };
        if let Some((p, ang)) = frame_of(av, pin.drawable, pin.tri, pin.bary) {
            self.xf.pos = p;
            self.xf.rot = pin.rot0 + (ang - pin.angle0).to_degrees();
            self.xf.scale = pin.scale_ratio * av.xf.scale;
        }
    }

    pub fn update(&mut self, dt: f32, inputs: &Inputs, neutral: &Inputs, tracking: bool, physics: bool) {
        let rxf = self.render_xf();
        let pinned = self.pin.is_some();
        match &mut self.body {
            Body::Sprite(s) => s.update(dt),
            Body::Live2D(l) => {
                // Pinned Live2D items follow the head, like the avatar does.
                let inp = if pinned { inputs } else { neutral };
                l.xf = rxf;
                l.update(dt, inp, tracking && pinned, physics);
            }
        }
    }

    pub fn set_tint(&mut self, gpu: &Gpu, t: [f32; 3]) {
        match &mut self.body {
            Body::Sprite(s) => s.set_tint(gpu, t),
            Body::Live2D(l) => l.tint = t,
        }
    }

    pub fn rebind(&mut self, gpu: &Gpu) {
        match &mut self.body {
            Body::Sprite(s) => s.rebind(gpu),
            Body::Live2D(l) => l.rebind(gpu),
        }
    }

    pub fn draw_masks(&self, mp: &mut MaskPainter, al: &MaskAlloc) {
        if let Body::Live2D(l) = &self.body {
            l.draw_masks(mp, al);
        }
    }

    pub fn upload(&mut self, gpu: &Gpu, vw: f32, vh: f32, masks: Option<&mut MaskAlloc>) {
        let rxf = self.render_xf();
        match &mut self.body {
            Body::Sprite(s) => s.set_affine(gpu, &self.xf.affine(vw, vh, false)),
            Body::Live2D(l) => {
                l.xf = rxf;
                l.upload(gpu, vw, vh, masks);
            }
        }
    }

    pub fn draw(&self, p: &mut Painter) {
        match &self.body {
            Body::Sprite(s) => s.draw(p),
            Body::Live2D(l) => l.draw(p),
        }
    }

    pub fn texture_bytes(&self) -> u64 {
        match &self.body {
            Body::Sprite(s) => s.texture_bytes,
            Body::Live2D(l) => l.texture_bytes,
        }
    }
}

/// What gets drawn, back to front.
pub fn draw_scene(p: &mut Painter, bg: Option<&Sprite>, items: &[Item], avatar: Option<&Live2D>) {
    if let Some(b) = bg {
        b.draw(p);
    }
    for it in items.iter().filter(|i| !i.front) {
        it.draw(p);
    }
    if let Some(a) = avatar {
        a.draw(p);
    }
    for it in items.iter().filter(|i| i.front) {
        it.draw(p);
    }
}
