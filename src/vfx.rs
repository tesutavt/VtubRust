//! Post-processing visual effects with the VTube Studio effect set (Files/Effects.cs) and
//! config IDs (Files/EffectConfigs.cs), see `vfx_schema.rs` (generated).
//!
//! The model (items + avatar) and the background are rendered into separate textures; a
//! compose pass (which also does "Background shift") combines them, then every active effect
//! runs as one full-screen pass (ping-pong). Effects that need the previous frame (low FPS,
//! datamosh, motion blur, VHS afterimage, line scanner, old film, foggy window) keep a history
//! texture. Nothing runs while no effect is active.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;
pub use crate::vfx_schema::DEFS;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Float,
    Int,
    Bool,
    Color,
    Text,
    Item,
}

pub struct ConfDef {
    pub id: &'static str,
    pub internal: &'static str,
    pub kind: Kind,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Default color (RRGGBB) for Color configs.
    pub color: &'static str,
    pub alpha: bool,
    /// A value > 0 makes the effect active (VTS "activationConfig").
    pub active: bool,
    pub en: &'static str,
    pub ru: &'static str,
    /// Offset of the value in the effect's uniform floats.
    pub offset: usize,
}

pub struct Def {
    pub id: &'static str,
    pub internal: &'static str,
    pub name_ru: &'static str,
    pub name_en: &'static str,
    pub distorts: bool,
    pub restricted: bool,
    pub configs: &'static [ConfDef],
}

impl Def {
    pub fn name(&self) -> &'static str {
        if crate::i18n::is_en() { self.name_en } else { self.name_ru }
    }
}

impl ConfDef {
    pub fn label(&self) -> &'static str {
        if crate::i18n::is_en() { self.en } else { self.ru }
    }
}

/// A config value: number (Float/Int/Bool as 0/1) or text (color "RRGGBBAA", file name).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Val {
    Num(f32),
    Text(String),
}

/// All effect values by config enum ID (missing = default).
pub type Values = BTreeMap<String, Val>;

/// VTS accepts IDs case-insensitively and ignores `_` / `-`.
pub fn norm_id(s: &str) -> String {
    s.chars().filter(|c| *c != '_' && *c != '-').collect::<String>().to_lowercase()
}

pub fn find_effect(id: &str) -> Option<usize> {
    let n = norm_id(id);
    DEFS.iter().position(|d| norm_id(d.id) == n || norm_id(d.internal) == n)
}

pub fn find_config(id: &str) -> Option<(usize, usize)> {
    let n = norm_id(id);
    for (e, d) in DEFS.iter().enumerate() {
        if let Some(c) = d.configs.iter().position(|c| norm_id(c.id) == n || norm_id(c.internal) == n) {
            return Some((e, c));
        }
    }
    None
}

fn hex_color(s: &str) -> Option<[f32; 4]> {
    let s = s.trim().trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|b| b as f32 / 255.0);
    Some([v(0)?, v(2)?, v(4)?, if s.len() >= 8 { v(6)? } else { 1.0 }])
}

pub fn color_hex(c: [f32; 4]) -> String {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{:02X}{:02X}{:02X}{:02X}", b(c[0]), b(c[1]), b(c[2]), b(c[3]))
}

pub fn get_f(v: &Values, c: &ConfDef) -> f32 {
    match v.get(c.id) {
        Some(Val::Num(x)) => *x,
        Some(Val::Text(t)) => match c.kind {
            Kind::Bool => if t.eq_ignore_ascii_case("true") { 1.0 } else { 0.0 },
            _ => t.parse().unwrap_or(c.default),
        },
        None => c.default,
    }
}

pub fn get_color(v: &Values, c: &ConfDef) -> [f32; 4] {
    match v.get(c.id) {
        Some(Val::Text(t)) => hex_color(t),
        _ => None,
    }
    .or_else(|| hex_color(c.color))
    .unwrap_or([1.0; 4])
}

pub fn get_text(v: &Values, c: &ConfDef) -> String {
    match v.get(c.id) {
        Some(Val::Text(t)) => t.clone(),
        _ => String::new(),
    }
}

/// Sets a config from a string (API / presets). Returns false when the value is invalid.
pub fn set_from_str(v: &mut Values, c: &ConfDef, s: &str) -> bool {
    match c.kind {
        Kind::Float | Kind::Int => match s.trim().parse::<f32>() {
            Ok(x) => {
                let x = x.clamp(c.min, c.max);
                v.insert(c.id.into(), Val::Num(if c.kind == Kind::Int { x.round() } else { x }));
                true
            }
            Err(_) => false,
        },
        Kind::Bool => match s.trim().to_lowercase().as_str() {
            "true" | "1" => {
                v.insert(c.id.into(), Val::Num(1.0));
                true
            }
            "false" | "0" => {
                v.insert(c.id.into(), Val::Num(0.0));
                true
            }
            _ => false,
        },
        Kind::Color => match hex_color(s) {
            Some(col) => {
                v.insert(c.id.into(), Val::Text(color_hex(if c.alpha { col } else { [col[0], col[1], col[2], 1.0] })));
                true
            }
            None => false,
        },
        Kind::Text | Kind::Item => {
            v.insert(c.id.into(), Val::Text(s.to_string()));
            true
        }
    }
}

/// VTS: an effect is active when one of its activation configs is > 0.
pub fn effect_active(v: &Values, d: &Def) -> bool {
    d.configs.iter().any(|c| c.active && get_f(v, c) > 0.0)
}

pub fn active_effects(v: &Values) -> Vec<usize> {
    DEFS.iter().enumerate().filter(|(_, d)| effect_active(v, d)).map(|(i, _)| i).collect()
}

/// Removes every value of an effect (back to defaults).
pub fn reset_effect(v: &mut Values, d: &Def) {
    for c in d.configs {
        v.remove(c.id);
    }
}

/// Effect uniform floats in schema order (colors as 4 floats, straight alpha).
pub fn pack(v: &Values, d: &Def) -> Vec<f32> {
    let mut out = vec![0.0f32; 120];
    for c in d.configs {
        match c.kind {
            Kind::Float | Kind::Int | Kind::Bool => {
                if c.offset < 120 {
                    out[c.offset] = get_f(v, c);
                }
            }
            Kind::Color => {
                let col = get_color(v, c);
                for k in 0..4 {
                    if c.offset + k < 120 {
                        out[c.offset + k] = col[k];
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Image files used by an effect (Text/Item configs): up to 3.
pub fn textures(v: &Values, d: &Def) -> Vec<String> {
    d.configs.iter().filter(|c| matches!(c.kind, Kind::Text | Kind::Item)).map(|c| get_text(v, c)).take(3).collect()
}

/// ModelGlitch strengths (explode, wiggle, pulse) — applied to the model vertices, not as a pass.
pub fn model_glitch(v: &Values) -> [f32; 3] {
    let d = &DEFS[find_effect("ModelGlitch").unwrap()];
    [get_f(v, &d.configs[0]), get_f(v, &d.configs[1]), get_f(v, &d.configs[2])]
}

fn has_history(id: &str) -> bool {
    matches!(id, "Lowfps" | "Datamosh" | "BlurEffects" | "Vhs" | "LineScanner" | "OldFilm" | "FoggyWindow")
}

// ------------------------------------------------------------------ renderer

/// Per-frame inputs that are not effect configs.
#[derive(Clone, Copy, Default)]
pub struct FrameInputs {
    pub time: f32,
    pub dt: f32,
    pub mic: f32,
    pub head: [f32; 2],
    /// Mouse position in UV (0..1) and button down (foggy window wiping).
    pub mouse: [f32; 3],
}

const SLOT: u64 = 768;
const MAX_PASSES: u64 = 40;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    v: [[f32; 4]; 30],
    /// x = time, y = mic, z/w = head X/Y
    misc: [f32; 4],
    /// x/y = mouse uv, z = mouse down, w = dt
    misc2: [f32; 4],
    /// x = refresh history this frame (low FPS), y = effect frame counter, z = aspect, w = random
    misc3: [f32; 4],
}

struct Chain {
    w: u32,
    h: u32,
    model_view: wgpu::TextureView,
    bg_view: wgpu::TextureView,
    a_view: wgpu::TextureView,
    b_view: wgpu::TextureView,
    /// Bind groups (group 0) sampling a / b.
    bgs: [wgpu::BindGroup; 2],
    /// Bind group sampling the model layer (input of the compose pass).
    model_bg: wgpu::BindGroup,
    /// History textures per effect index: (texture view, group-0 bind group sampling it).
    hist: HashMap<usize, (wgpu::TextureView, wgpu::BindGroup)>,
}

pub struct Renderer {
    layout0: wgpu::BindGroupLayout,
    layout1: wgpu::BindGroupLayout,
    pipes: Vec<wgpu::RenderPipeline>,
    compose: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    blank: wgpu::TextureView,
    images: HashMap<String, Option<wgpu::TextureView>>,
    chains: [Option<Chain>; 4],
    /// Low-FPS / old-film frame timing per effect.
    last_refresh: HashMap<usize, f32>,
    frame: u64,
}

fn target(gpu: &Gpu, w: u32, h: u32, label: &str) -> wgpu::TextureView {
    gpu.device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gpu.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn image_view(gpu: &Gpu, rgba: Vec<u8>, w: u32, h: u32) -> wgpu::TextureView {
    use wgpu::util::DeviceExt;
    let mut rgba = rgba;
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    gpu.device
        .create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("vfx-image"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &rgba,
        )
        .create_view(&Default::default())
}

/// One active pass of the frame.
pub struct Pass {
    pub effect: usize,
    pub textures: Vec<String>,
}

impl Renderer {
    pub fn new(gpu: &Gpu) -> Renderer {
        let d = &gpu.device;
        let src = format!("{}\n{}", include_str!("gpu/vfx_consts.wgsl"), include_str!("gpu/vfx.wgsl"));
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("vfx"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout0 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vfx0"),
            entries: &[
                tex(0),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Uniform>() as u64) },
                    count: None,
                },
            ],
        });
        // Group 1: background, model layer, history, three images.
        let layout1 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("vfx1"), entries: &[tex(0), tex(1), tex(2), tex(3), tex(4), tex(5)] });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&layout0, &layout1], push_constant_ranges: &[] });
        let mk = |entry: &str| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: "vs", compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: entry,
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: gpu.format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let pipes = DEFS.iter().map(|def| mk(&format!("fs_{}", def.internal))).collect();
        let compose = mk("fs_compose");
        let copy = mk("fs_copy");
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniform = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vfx-params"),
            size: SLOT * MAX_PASSES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Renderer {
            layout0,
            layout1,
            pipes,
            compose,
            copy,
            sampler,
            uniform,
            blank: image_view(gpu, vec![0; 4], 1, 1),
            images: HashMap::new(),
            chains: [None, None, None, None],
            last_refresh: HashMap::new(),
            frame: 0,
        }
    }

    fn bind0(&self, gpu: &Gpu, src: &wgpu::TextureView) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout0,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.uniform, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<Uniform>() as u64) }),
                },
            ],
        })
    }

    /// Loads (and caches) an effect image: from the Items folder or an absolute path.
    pub fn load_image(&mut self, gpu: &Gpu, items_dirs: &[std::path::PathBuf], name: &str) {
        if name.is_empty() || self.images.contains_key(name) {
            return;
        }
        let path = std::path::Path::new(name);
        let found = if path.is_absolute() { Some(path.to_path_buf()) } else { items_dirs.iter().map(|d| d.join(name)).find(|p| p.exists()) };
        let view = found.and_then(|p| image::open(p).ok()).map(|img| {
            let img = img.thumbnail(512, 512).to_rgba8();
            let (w, h) = img.dimensions();
            image_view(gpu, img.into_raw(), w, h)
        });
        self.images.insert(name.to_string(), view);
    }

    fn image(&self, name: &str) -> &wgpu::TextureView {
        self.images.get(name).and_then(|v| v.as_ref()).unwrap_or(&self.blank)
    }

    /// Makes sure the textures for `slot` match the size.
    pub fn ensure(&mut self, gpu: &Gpu, slot: usize, w: u32, h: u32) {
        if self.chains[slot].as_ref().map(|c| c.w == w && c.h == h).unwrap_or(false) || w == 0 || h == 0 {
            return;
        }
        let model_view = target(gpu, w, h, "vfx-model");
        let bg_view = target(gpu, w, h, "vfx-bg");
        let a_view = target(gpu, w, h, "vfx-a");
        let b_view = target(gpu, w, h, "vfx-b");
        let bgs = [self.bind0(gpu, &a_view), self.bind0(gpu, &b_view)];
        let model_bg = self.bind0(gpu, &model_view);
        self.chains[slot] = Some(Chain { w, h, model_view, bg_view, a_view, b_view, bgs, model_bg, hist: HashMap::new() });
    }

    /// Model layer (items + avatar, transparent background) of `slot`.
    pub fn scene_view(&self, slot: usize) -> Option<&wgpu::TextureView> {
        self.chains[slot].as_ref().map(|c| &c.model_view)
    }

    /// Background layer of `slot`.
    pub fn bg_view(&self, slot: usize) -> Option<&wgpu::TextureView> {
        self.chains[slot].as_ref().map(|c| &c.bg_view)
    }

    /// Runs compose + effects: model/bg → … → `out`.
    pub fn run(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, slot: usize, out: &wgpu::TextureView, values: &Values, passes: &[usize], inp: FrameInputs) {
        self.frame += 1;
        // History textures for this chain.
        let needs: Vec<usize> = passes.iter().copied().filter(|e| has_history(DEFS[*e].id)).collect();
        if let Some(c) = self.chains[slot].as_ref() {
            let (w, h) = (c.w, c.h);
            let missing: Vec<usize> = needs.iter().copied().filter(|e| !c.hist.contains_key(e)).collect();
            for e in missing {
                let v = target(gpu, w, h, "vfx-hist");
                let bg = self.bind0(gpu, &v);
                if let Some(c) = self.chains[slot].as_mut() {
                    c.hist.insert(e, (v, bg));
                }
            }
        }
        let Some(c) = self.chains[slot].as_ref() else { return };
        let aspect = c.w as f32 / c.h.max(1) as f32;
        // Uniforms: 0 = compose (BackgroundShift values), then one per pass.
        let bs = find_effect("BackgroundShift").unwrap();
        let mut list: Vec<(Option<usize>, Vec<f32>)> = vec![(None, pack(values, &DEFS[bs]))];
        for &e in passes {
            list.push((Some(e), pack(values, &DEFS[e])));
        }
        let mut refresh = HashMap::new();
        for (k, (e, v)) in list.iter().enumerate() {
            let mut r = 1.0;
            if let Some(e) = e {
                // Frame-hold timing for low FPS / old film.
                let fps = match DEFS[*e].id {
                    "Lowfps" => v[1].max(1.0) * (1.0 + (hash(self.frame as f32) - 0.5) * v[2]),
                    "OldFilm" => v[1].max(1.0),
                    _ => 0.0,
                };
                if fps > 0.0 {
                    let last = self.last_refresh.get(e).copied().unwrap_or(-1.0);
                    if inp.time - last >= 1.0 / fps || last > inp.time {
                        self.last_refresh.insert(*e, inp.time);
                    } else {
                        r = 0.0;
                    }
                }
                refresh.insert(*e, r);
            }
            let mut u = Uniform { v: [[0.0; 4]; 30], misc: [inp.time, inp.mic, inp.head[0], inp.head[1]], misc2: [inp.mouse[0], inp.mouse[1], inp.mouse[2], inp.dt], misc3: [r, self.frame as f32, aspect, hash(self.frame as f32 * 1.37)] };
            for (k, x) in v.iter().take(120).enumerate() {
                u.v[k / 4][k % 4] = *x;
            }
            gpu.queue.write_buffer(&self.uniform, k as u64 * SLOT, bytemuck::bytes_of(&u));
        }
        let group1 = |hist: Option<&wgpu::TextureView>, texs: &[String]| {
            let t = |i: usize| texs.get(i).map(|n| self.image(n)).unwrap_or(&self.blank);
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout1,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&c.bg_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&c.model_view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(hist.unwrap_or(&self.blank)) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(t(0)) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(t(1)) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(t(2)) },
                ],
            })
        };
        let pass = |enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, dst: &wgpu::TextureView, g0: &wgpu::BindGroup, slot_k: u64, g1: &wgpu::BindGroup| {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vfx"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            p.set_pipeline(pipe);
            p.set_bind_group(0, g0, &[(slot_k * SLOT) as u32]);
            p.set_bind_group(1, g1, &[]);
            p.draw(0..3, 0..1);
        };
        let empty = group1(None, &[]);
        // History effects need their output in a chain texture, so a final copy pass is added.
        let final_copy = !needs.is_empty();
        let n = passes.len();
        // compose → a (or straight to out when there is nothing else)
        let first_dst = if n == 0 && !final_copy { out } else { &c.a_view };
        pass(enc, &self.compose, first_dst, &c.model_bg, 0, &empty);
        if n == 0 {
            return;
        }
        let mut src = 0usize; // 0 = a, 1 = b
        for (k, &e) in passes.iter().enumerate() {
            let last = k + 1 == n && !final_copy;
            let dst = if last { out } else if src == 0 { &c.b_view } else { &c.a_view };
            let hist = c.hist.get(&e).map(|h| &h.0);
            let texs = textures(values, &DEFS[e]);
            let g1 = group1(hist, &texs);
            pass(enc, &self.pipes[e], dst, &c.bgs[src], k as u64 + 1, &g1);
            // Store this pass' output as the effect's history for the next frame.
            if let Some((hv, _)) = c.hist.get(&e) {
                if !last {
                    let from = if src == 0 { &c.bgs[1] } else { &c.bgs[0] };
                    pass(enc, &self.copy, hv, from, 0, &empty);
                }
            }
            if !last {
                src = 1 - src;
            }
        }
        if final_copy {
            pass(enc, &self.copy, out, &c.bgs[src], 0, &empty);
        }
        let _ = refresh;
    }
}

fn hash(x: f32) -> f32 {
    ((x * 12.9898).sin() * 43758.547).fract().abs()
}

/// Random effect configuration ("randomizeAll"): chaos 0 = one or two mild effects, 1 = chaos.
pub fn randomize(chaos: f32, restricted: bool) -> Values {
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut v = Values::new();
    let count = 1 + (chaos * 8.0 * rnd()).round() as usize;
    let pool: Vec<&Def> = DEFS.iter().filter(|d| (restricted || !d.restricted) && d.id != "SimpleOverlay" && d.id != "ParticleShower").collect();
    for _ in 0..count {
        let d = pool[(rnd() * pool.len() as f32) as usize % pool.len()];
        for c in d.configs {
            match c.kind {
                Kind::Float | Kind::Int => {
                    let x = if c.active { 0.3 + 0.7 * rnd() * (0.4 + chaos * 0.6) } else { c.default + (c.min + rnd() * (c.max - c.min) - c.default) * chaos };
                    v.insert(c.id.into(), Val::Num(if c.kind == Kind::Int { x.round() } else { x }));
                }
                Kind::Bool => {
                    v.insert(c.id.into(), Val::Num(if rnd() < 0.5 { 1.0 } else { 0.0 }));
                }
                Kind::Color => {
                    v.insert(c.id.into(), Val::Text(color_hex([rnd(), rnd(), rnd(), 1.0])));
                }
                _ => {}
            }
        }
    }
    v
}

/// Linear blend of two value sets (numbers and colors), for smooth fades.
pub fn blend(a: &Values, b: &Values, t: f32) -> Values {
    let mut out = b.clone();
    for d in DEFS {
        for c in d.configs {
            match c.kind {
                Kind::Float => {
                    let (x, y) = (get_f(a, c), get_f(b, c));
                    out.insert(c.id.into(), Val::Num(x + (y - x) * t));
                }
                Kind::Color => {
                    let (x, y) = (get_color(a, c), get_color(b, c));
                    let m = [0, 1, 2, 3].map(|k| x[k] + (y[k] - x[k]) * t);
                    out.insert(c.id.into(), Val::Text(color_hex(m)));
                }
                _ => {}
            }
        }
    }
    out
}

/// Packs the values a client may want to save as a preset.
pub fn preset_json(v: &Values) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or_default()
}

pub fn presets_dir(data: &std::path::Path) -> std::path::PathBuf {
    data.join("vfx_presets")
}

pub fn list_presets(data: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(presets_dir(data))
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Loads a preset file (VTS-style config map). Old rigrs presets are ignored.
pub fn load_preset(data: &std::path::Path, name: &str) -> Option<Values> {
    let v: serde_json::Value = crate::formats::read_json(&presets_dir(data).join(format!("{name}.json"))).ok()?;
    let obj = v.as_object()?;
    let mut out = Values::new();
    for (k, val) in obj {
        let Some((e, c)) = find_config(k) else { continue };
        let cd = &DEFS[e].configs[c];
        let s = match val {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Bool(b) => b.to_string(),
            other => other.to_string(),
        };
        set_from_str(&mut out, cd, &s);
    }
    Some(out)
}

pub fn save_preset(data: &std::path::Path, name: &str, v: &Values) -> std::io::Result<()> {
    std::fs::create_dir_all(presets_dir(data))?;
    std::fs::write(presets_dir(data).join(format!("{name}.json")), serde_json::to_vec_pretty(&preset_json(v)).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_vts() {
        assert_eq!(DEFS.len(), 29);
        assert_eq!(DEFS.iter().map(|d| d.configs.len()).sum::<usize>(), 257);
        assert_eq!(find_config("color_grading-strength"), find_config("ColorGrading_Strength"));
        assert_eq!(find_config("COLORGRADING_STRENGTH"), find_config("ColorGrading_Strength"));
        let mut v = Values::new();
        let (e, c) = find_config("Bloom_Strength").unwrap();
        assert!(!effect_active(&v, &DEFS[e]));
        assert!(set_from_str(&mut v, &DEFS[e].configs[c], "0.7"));
        assert!(effect_active(&v, &DEFS[e]));
        assert!(!set_from_str(&mut v, &DEFS[e].configs[c], "abc"));
        assert!(DEFS.iter().all(|d| d.configs.iter().map(|c| c.offset + 4).max().unwrap_or(0) <= 120));
    }
}
