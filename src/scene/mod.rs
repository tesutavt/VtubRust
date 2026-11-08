pub mod live2d;
pub mod sprite;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::gpu::{Gpu, MaskAlloc, MaskPainter, Painter};
use crate::mapping::{input, Inputs};
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

/// Special values of `ItemCfg::layer` / split placements.
pub const LAYER_FRONT: &str = "__front";
pub const LAYER_BACK: &str = "__back";

/// Item options as in the VTS item config menu.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ItemCfg {
    /// VTS item order: the model is at 0, higher is in front, negative is behind the model.
    pub order: i32,
    /// Smoothing when pinned and locked (0 = rigid, 1 = very loose).
    pub smoothing: f32,
    /// Mosaic censor.
    pub censor: bool,
    /// Animated items: frames per second (0 = from the file).
    pub fps: f32,
    /// "Put item in model": "" (off), LAYER_FRONT, LAYER_BACK or an ArtMesh ID (item goes above it).
    pub layer: String,
    /// Live2D items: split ArtMesh and where the back / front parts go.
    pub split: String,
    pub split_front: String,
    pub split_back: String,
    /// Pinning options.
    pub pin_only_below: bool,
    pub pin_only_layer: String,
    pub pin_notify: bool,
    /// Live2D items: multipliers for the head angles the item receives.
    pub angle_mult: [f32; 3],
    /// Live2D items: don't follow tracking while not pinned.
    pub default_unpinned: bool,
    /// Live2D items: the item's own keyboard hotkeys work.
    pub normal_hotkeys: bool,
}

impl Default for ItemCfg {
    fn default() -> Self {
        ItemCfg {
            order: 1,
            smoothing: 0.0,
            censor: false,
            fps: 0.0,
            layer: String::new(),
            split: String::new(),
            split_front: String::new(),
            split_back: String::new(),
            pin_only_below: false,
            pin_only_layer: String::new(),
            pin_notify: false,
            angle_mult: [1.0; 3],
            default_unpinned: true,
            normal_hotkeys: false,
        }
    }
}

pub struct Item {
    pub id: u64,
    pub source: PathBuf,
    pub name: String,
    pub body: Body,
    pub xf: Xform,
    pub locked: bool,
    pub pin: Option<Pin>,
    /// Local point that `xf.pos` refers to (visual centre of Live2D items).
    pub pivot: [f32; 2],
    pub cfg: ItemCfg,
    /// Item scene that loaded this item.
    pub scene: Option<String>,
    /// Fade in/out: current opacity, target and speed (opacity per second).
    pub alpha: f32,
    pub alpha_target: f32,
    pub fade_speed: f32,
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
    pub fn new(id: u64, source: PathBuf, name: String, body: Body, xf: Xform, pivot: [f32; 2]) -> Item {
        let mut cfg = ItemCfg::default();
        if let Body::Live2D(l) = &body {
            // VTS: the first layer with "split" in its name is the default split point.
            if let Some(d) = l.model.drawables.iter().find(|d| d.id.to_lowercase().contains("split")) {
                cfg.split = d.id.clone();
            }
        }
        Item { id, source, name, body, xf, locked: false, pin: None, pivot, cfg, scene: None, alpha: 1.0, alpha_target: 1.0, fade_speed: 0.0 }
    }

    /// In front of the model (when not placed inside it).
    pub fn front(&self) -> bool {
        self.cfg.order > 0
    }

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

    /// ArtMesh filter used for pinning: model exclusions, `vts_ignore_raycast`, and the
    /// item's "only pin to layers below item" / "only pin to specific layer" options.
    pub fn pin_filter<'a>(&'a self, av: &'a Live2D, details: Option<&'a crate::artmesh::ArtMeshDetails>) -> impl Fn(usize) -> bool + 'a {
        let below = if self.cfg.pin_only_below && !self.cfg.layer.is_empty() && !self.cfg.layer.starts_with("__") { av.order_pos(&self.cfg.layer) } else { None };
        move |i: usize| {
            let id = &av.model.drawables[i].id;
            if av.ignore_raycast.iter().any(|x| x == id) || details.map(|d| d.excluded(id)).unwrap_or(id.contains("vts_ignore_raycast")) {
                return false;
            }
            if !self.cfg.pin_only_layer.is_empty() && *id != self.cfg.pin_only_layer {
                return false;
            }
            if let Some(b) = below {
                if av.order_pos_of(i).map(|p| p > b).unwrap_or(true) {
                    return false;
                }
            }
            true
        }
    }

    /// Pins the item to the uppermost allowed ArtMesh under its centre (VTS behaviour).
    pub fn pin_to_filtered(&mut self, av: &Live2D, details: Option<&crate::artmesh::ArtMeshDetails>) -> Option<String> {
        let pk = {
            let f = self.pin_filter(av, details);
            av.pick_filtered(self.xf.pos, f)?
        };
        let (_, ang) = frame_of(av, pk.drawable, pk.tri, pk.bary)?;
        self.pin = Some(Pin { drawable: pk.drawable, tri: pk.tri, bary: pk.bary, angle0: ang, rot0: self.xf.rot, scale_ratio: self.xf.scale / av.xf.scale });
        Some(av.model.drawables[pk.drawable].id.clone())
    }

    /// Pins the item to whatever avatar triangle lies under its centre.
    pub fn pin_to(&mut self, av: &Live2D) -> bool {
        self.pin_to_filtered(av, av.vts_artmesh.as_ref()).is_some()
    }

    /// Pins the item to a given triangle / barycentric point of the avatar.
    pub fn pin_exact(&mut self, av: &Live2D, drawable: usize, tri: [u16; 3], bary: [f32; 3]) -> bool {
        let Some((p, ang)) = frame_of(av, drawable, tri, bary) else { return false };
        self.xf.pos = p;
        self.pin = Some(Pin { drawable, tri, bary, angle0: ang, rot0: self.xf.rot, scale_ratio: self.xf.scale / av.xf.scale });
        true
    }

    pub fn follow(&mut self, av: &Live2D, dt: f32) {
        let Some(pin) = &self.pin else { return };
        if let Some((p, ang)) = frame_of(av, pin.drawable, pin.tri, pin.bary) {
            let target = Xform { pos: p, rot: pin.rot0 + (ang - pin.angle0).to_degrees(), scale: pin.scale_ratio * av.xf.scale, flip: self.xf.flip };
            // VTS: smoothing only applies to locked items.
            let s = if self.locked { self.cfg.smoothing.clamp(0.0, 0.98) } else { 0.0 };
            let k = if s <= 0.0 { 1.0 } else { (1.0 - s.powf(dt * 60.0)).clamp(0.0, 1.0) };
            self.xf.pos = [self.xf.pos[0] + (target.pos[0] - self.xf.pos[0]) * k, self.xf.pos[1] + (target.pos[1] - self.xf.pos[1]) * k];
            self.xf.rot += (target.rot - self.xf.rot) * k;
            self.xf.scale += (target.scale - self.xf.scale) * k;
        }
    }

    pub fn update(&mut self, dt: f32, inputs: &Inputs, neutral: &Inputs, tracking: bool, physics: bool) {
        // Fade in / out.
        if self.fade_speed > 0.0 {
            let d = self.fade_speed * dt;
            self.alpha = if self.alpha < self.alpha_target { (self.alpha + d).min(self.alpha_target) } else { (self.alpha - d).max(self.alpha_target) };
        } else {
            self.alpha = self.alpha_target;
        }
        let rxf = self.render_xf();
        let pinned = self.pin.is_some();
        match &mut self.body {
            Body::Sprite(s) => s.update(dt),
            Body::Live2D(l) => {
                // VTS: Live2D items react to face tracking ("glasses that rotate with your head"),
                // with angle multipliers; "return to default when not pinned" keeps loose items still.
                l.xf = rxf;
                l.alpha = self.alpha;
                let follow = pinned || !self.cfg.default_unpinned;
                if follow {
                    let mut inp = *inputs;
                    for (k, idx) in [input::FACE_ANGLE_X, input::FACE_ANGLE_Y, input::FACE_ANGLE_Z].into_iter().enumerate() {
                        inp[idx] *= self.cfg.angle_mult[k];
                    }
                    l.update(dt, &inp, tracking, physics);
                } else {
                    l.update(dt, neutral, false, physics);
                }
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
        let (alpha, censor, fps) = (self.alpha, self.cfg.censor, self.cfg.fps);
        match &mut self.body {
            Body::Sprite(s) => {
                s.set_fps(fps);
                let t = s.tint();
                s.set_look(gpu, t, alpha, if censor { 1.0 / 14.0 } else { 0.0 });
                s.set_affine(gpu, &self.xf.affine(vw, vh, false));
            }
            Body::Live2D(l) => {
                l.xf = rxf;
                l.alpha = alpha;
                l.mosaic = if censor { 1.0 / 48.0 } else { 0.0 };
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

    fn draw_part(&self, p: &mut Painter, part: Option<std::ops::Range<usize>>) {
        match (&self.body, part) {
            (Body::Live2D(l), Some(r)) => l.draw_range(p, r),
            _ => self.draw(p),
        }
    }

    pub fn texture_bytes(&self) -> u64 {
        match &self.body {
            Body::Sprite(s) => s.texture_bytes,
            Body::Live2D(l) => l.texture_bytes,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Place {
    Back,
    Front,
    /// Above the avatar drawable at this render-order position.
    Above(usize),
}

fn place_of(layer: &str, order: i32, avatar: Option<&Live2D>) -> Place {
    match layer {
        LAYER_FRONT => Place::Front,
        LAYER_BACK => Place::Back,
        "" => {
            if order > 0 {
                Place::Front
            } else {
                Place::Back
            }
        }
        id => match avatar.and_then(|a| a.order_pos(id)) {
            Some(p) => Place::Above(p),
            // Layer missing in this model: fall back to the normal item order (VTS).
            None => {
                if order > 0 {
                    Place::Front
                } else {
                    Place::Back
                }
            }
        },
    }
}

/// What gets drawn, back to front: background, items behind the model, the model with items
/// inserted between its layers, items in front of the model (VTS item order + layer pinning).
pub fn draw_scene(p: &mut Painter, bg: Option<&Sprite>, items: &[Item], avatar: Option<&Live2D>) {
    if let Some(b) = bg {
        b.draw(p);
    }
    // Pieces: (item, optional drawable range of a split Live2D item, placement).
    let mut pieces: Vec<(&Item, Option<std::ops::Range<usize>>, Place)> = Vec::new();
    for it in items {
        let split = match &it.body {
            Body::Live2D(l) if !it.cfg.split.is_empty() && (!it.cfg.split_front.is_empty() || !it.cfg.split_back.is_empty()) => l.order_pos(&it.cfg.split).map(|s| (s, l.order_len())),
            _ => None,
        };
        match split {
            Some((s, n)) => {
                let back = if it.cfg.split_back.is_empty() { &it.cfg.layer } else { &it.cfg.split_back };
                let front = if it.cfg.split_front.is_empty() { &it.cfg.layer } else { &it.cfg.split_front };
                let pb = place_of(back, it.cfg.order, avatar);
                let mut pf = place_of(front, it.cfg.order, avatar);
                // The front part never goes behind the back part.
                pf = match (pb, pf) {
                    (Place::Front, Place::Back) | (Place::Front, Place::Above(_)) => Place::Front,
                    (Place::Above(a), Place::Above(b)) if b < a => Place::Above(a),
                    (Place::Above(a), Place::Back) => Place::Above(a),
                    _ => pf,
                };
                pieces.push((it, Some(0..s + 1), pb));
                pieces.push((it, Some(s + 1..n), pf));
            }
            None => pieces.push((it, None, place_of(&it.cfg.layer, it.cfg.order, avatar))),
        }
    }
    let by_order = |v: &mut Vec<&(&Item, Option<std::ops::Range<usize>>, Place)>| v.sort_by_key(|x| x.0.cfg.order);
    let mut back: Vec<_> = pieces.iter().filter(|x| x.2 == Place::Back).collect();
    by_order(&mut back);
    for (it, r, _) in back {
        it.draw_part(p, r.clone());
    }
    if let Some(a) = avatar {
        let mut inside: Vec<_> = pieces.iter().filter_map(|x| if let Place::Above(k) = x.2 { Some((k, x)) } else { None }).collect();
        inside.sort_by_key(|(k, x)| (*k, x.0.cfg.order));
        let mut start = 0;
        for (k, (it, r, _)) in inside {
            a.draw_range(p, start..k + 1);
            start = k + 1;
            it.draw_part(p, r.clone());
        }
        a.draw_range(p, start..a.order_len());
    }
    let mut front: Vec<_> = pieces.iter().filter(|x| x.2 == Place::Front).collect();
    by_order(&mut front);
    for (it, r, _) in front {
        it.draw_part(p, r.clone());
    }
}
