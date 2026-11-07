//! Post-processing visual effects (VFX). The scene is rendered into an off-screen texture,
//! then each enabled effect runs as one full-screen pass (ping-pong), the last pass writing
//! straight into the final target. Nothing of this runs while no effect is enabled.

use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

pub struct Param {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub def: f32,
    pub step: f32,
}

const fn p(name: &'static str, min: f32, max: f32, def: f32, step: f32) -> Param {
    Param { name, min, max, def, step }
}

pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub entry: &'static str,
    pub params: &'static [Param],
    pub color: Option<[f32; 4]>,
    pub image: bool,
    /// Moves / distorts pixels (breaks item pinning visually).
    pub distorts: bool,
    /// Optional labels for an enum-like first parameter.
    pub choices: &'static [&'static str],
}

pub const DEFS: &[Def] = &[
    Def { id: "color", name: "Градация цвета", entry: "fs_color", params: &[p("Яркость", -0.5, 0.5, 0.0, 0.01), p("Контраст", 0.0, 2.0, 1.0, 0.01), p("Насыщенность", 0.0, 2.0, 1.0, 0.01), p("Оттенок", -180.0, 180.0, 0.0, 1.0), p("Температура", -1.0, 1.0, 0.0, 0.01), p("Гамма", 0.3, 3.0, 1.0, 0.01)], color: None, image: false, distorts: false, choices: &[] },
    Def { id: "weather", name: "Погодные эффекты", entry: "fs_weather", params: &[p("Тип", 0.0, 1.0, 0.0, 1.0), p("Плотность", 0.0, 1.0, 0.5, 0.01), p("Скорость", 0.0, 3.0, 1.0, 0.01), p("Ветер", -1.0, 1.0, 0.2, 0.01), p("Размер", 0.3, 3.0, 1.0, 0.01)], color: Some([0.85, 0.9, 1.0, 0.8]), image: false, distorts: false, choices: &["Дождь", "Снег"] },
    Def { id: "glow", name: "Свечение и световые эффекты", entry: "fs_glow", params: &[p("Порог яркости", 0.0, 0.95, 0.75, 0.01), p("Сила", 0.0, 3.0, 0.6, 0.01), p("Радиус", 1.0, 40.0, 10.0, 0.5)], color: Some([1.0, 1.0, 1.0, 1.0]), image: false, distorts: false, choices: &[] },
    Def { id: "rim", name: "Задняя подсветка и обводка", entry: "fs_rim", params: &[p("Подсветка", 0.0, 3.0, 1.0, 0.01), p("Радиус подсветки", 1.0, 60.0, 16.0, 0.5), p("Толщина обводки", 0.0, 12.0, 2.0, 0.5)], color: Some([1.0, 0.85, 0.4, 1.0]), image: false, distorts: false, choices: &[] },
    Def { id: "custom", name: "Пользовательские эффекты частиц", entry: "fs_custom", params: &[p("Количество", 0.0, 1.0, 0.35, 0.01), p("Размер", 0.3, 4.0, 1.0, 0.01), p("Скорость", 0.0, 3.0, 1.0, 0.01), p("Вращение", 0.0, 2.0, 0.5, 0.01)], color: None, image: true, distorts: false, choices: &[] },
    Def { id: "broken", name: "Имитация битого видеофайла", entry: "fs_broken", params: &[p("Сила", 0.0, 1.0, 0.5, 0.01), p("Частота", 0.0, 1.0, 0.3, 0.01), p("Размер блоков", 2.0, 64.0, 16.0, 1.0)], color: None, image: false, distorts: true, choices: &[] },
    Def { id: "scan", name: "Линейное сканирование", entry: "fs_scan", params: &[p("Скорость", 0.0, 3.0, 1.0, 0.01), p("Ширина", 0.005, 0.3, 0.05, 0.005), p("Яркость", 0.0, 2.0, 0.8, 0.01)], color: Some([0.3, 0.9, 1.0, 1.0]), image: false, distorts: false, choices: &[] },
    Def { id: "falling", name: "Падающие частицы", entry: "fs_falling", params: &[p("Форма", 0.0, 2.0, 2.0, 1.0), p("Количество", 0.0, 1.0, 0.3, 0.01), p("Размер", 0.3, 4.0, 1.0, 0.01), p("Скорость", 0.0, 3.0, 1.0, 0.01)], color: Some([1.0, 0.5, 0.7, 0.9]), image: false, distorts: false, choices: &["Круги", "Звёзды", "Сердечки"] },
    Def { id: "analog", name: "Аналоговые помехи", entry: "fs_analog", params: &[p("Сила", 0.0, 2.0, 0.6, 0.01), p("Волнистость", 0.0, 2.0, 0.5, 0.01), p("Шум", 0.0, 1.0, 0.3, 0.01)], color: None, image: false, distorts: true, choices: &[] },
    Def { id: "digital", name: "Цифровые помехи", entry: "fs_digital", params: &[p("Сила", 0.0, 1.0, 0.4, 0.01), p("Частота", 0.0, 1.0, 0.15, 0.01), p("Сдвиг RGB", 0.0, 3.0, 0.6, 0.01)], color: None, image: false, distorts: true, choices: &[] },
    Def { id: "cinema", name: "Кинематографическая рамка", entry: "fs_cinema", params: &[p("Высота полос", 0.0, 0.3, 0.1, 0.005), p("Виньетка", 0.0, 1.0, 0.35, 0.01)], color: Some([0.0, 0.0, 0.0, 1.0]), image: false, distorts: false, choices: &[] },
    Def { id: "blur", name: "Размытие", entry: "fs_blur", params: &[p("Радиус", 0.0, 30.0, 4.0, 0.5)], color: None, image: false, distorts: false, choices: &[] },
    Def { id: "grain", name: "Зернистость ТВ-помех", entry: "fs_grain", params: &[p("Сила", 0.0, 1.0, 0.25, 0.01), p("Размер зерна", 1.0, 4.0, 1.5, 0.1)], color: None, image: false, distorts: false, choices: &[] },
    Def { id: "oldtv", name: "Артефакты видеоплёнки / старый телевизор", entry: "fs_oldtv", params: &[p("Виньетка", 0.0, 1.0, 0.5, 0.01), p("Царапины", 0.0, 1.0, 0.4, 0.01), p("Мерцание", 0.0, 1.0, 0.4, 0.01), p("Сепия", 0.0, 1.0, 0.6, 0.01)], color: None, image: false, distorts: false, choices: &[] },
    Def { id: "edge", name: "Контурная обводка", entry: "fs_edge", params: &[p("Толщина", 0.5, 4.0, 1.0, 0.1), p("Порог", 0.0, 1.0, 0.2, 0.01), p("Сила", 0.0, 1.0, 1.0, 0.01)], color: Some([0.1, 0.1, 0.12, 1.0]), image: false, distorts: false, choices: &[] },
    Def { id: "poster", name: "Постеризация", entry: "fs_poster", params: &[p("Уровни", 2.0, 16.0, 5.0, 1.0), p("Гамма", 0.5, 2.0, 1.0, 0.01)], color: None, image: false, distorts: false, choices: &[] },
    Def { id: "ascii", name: "ASCII-графика", entry: "fs_ascii", params: &[p("Размер символа", 4.0, 24.0, 8.0, 1.0), p("Цвета модели", 0.0, 1.0, 1.0, 0.01)], color: Some([0.3, 1.0, 0.4, 1.0]), image: false, distorts: false, choices: &[] },
    Def { id: "fisheye", name: "Рыбий глаз", entry: "fs_fisheye", params: &[p("Сила", -1.0, 1.0, 0.4, 0.01)], color: None, image: false, distorts: true, choices: &[] },
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Cfg {
    pub id: String,
    pub on: bool,
    pub p: Vec<f32>,
    pub color: [f32; 4],
    pub image: String,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { id: String::new(), on: false, p: Vec::new(), color: [1.0; 4], image: String::new() }
    }
}

pub fn default_cfg(d: &Def) -> Cfg {
    Cfg { id: d.id.into(), on: false, p: d.params.iter().map(|x| x.def).collect(), color: d.color.unwrap_or([1.0; 4]), image: String::new() }
}

/// Makes sure the list has one entry per effect, in DEFS order.
pub fn normalize(list: &mut Vec<Cfg>) {
    let mut out = Vec::with_capacity(DEFS.len());
    for d in DEFS {
        let mut c = list.iter().find(|c| c.id == d.id).cloned().unwrap_or_else(|| default_cfg(d));
        c.p.resize(d.params.len(), 0.0);
        out.push(c);
    }
    *list = out;
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    a: [f32; 4],
    b: [f32; 4],
    color: [f32; 4],
    misc: [f32; 4],
    _pad: [f32; 48],
}

const SLOT: u64 = 256;

struct Chain {
    w: u32,
    h: u32,
    scene_view: wgpu::TextureView,
    a_view: wgpu::TextureView,
    b_view: wgpu::TextureView,
    /// Bind groups sampling scene / a / b.
    bgs: [wgpu::BindGroup; 3],
}

pub struct Renderer {
    layout: wgpu::BindGroupLayout,
    pipes: Vec<wgpu::RenderPipeline>,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    aux_view: wgpu::TextureView,
    aux_path: String,
    aux_ok: bool,
    /// 0 = window, 1 = Spout2, 2 = NDI
    chains: [Option<Chain>; 4],
    format: wgpu::TextureFormat,
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
                label: Some("vfx-aux"),
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

impl Renderer {
    pub fn new(gpu: &Gpu) -> Renderer {
        let d = &gpu.device;
        let shader = d.create_shader_module(wgpu::include_wgsl!("gpu/vfx.wgsl"));
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vfx"),
            entries: &[
                tex(0),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(64) },
                    count: None,
                },
                tex(3),
            ],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipes = DEFS
            .iter()
            .map(|def| {
                d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(def.id),
                    layout: Some(&pl),
                    vertex: wgpu::VertexState { module: &shader, entry_point: "vs", compilation_options: Default::default(), buffers: &[] },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: def.entry,
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState { format: gpu.format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    }),
                    multiview: None,
                    cache: None,
                })
            })
            .collect();
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniform = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vfx-params"),
            size: SLOT * DEFS.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Renderer {
            layout,
            pipes,
            sampler,
            uniform,
            aux_view: image_view(gpu, vec![0; 4], 1, 1),
            aux_path: String::new(),
            aux_ok: false,
            chains: [None, None, None, None],
            format: gpu.format,
        }
    }

    fn bind(&self, gpu: &Gpu, src: &wgpu::TextureView) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.uniform, offset: 0, size: wgpu::BufferSize::new(64) }),
                },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.aux_view) },
            ],
        })
    }

    /// Loads the image used by "custom particles" (rebinds all chains when it changes).
    pub fn set_aux(&mut self, gpu: &Gpu, path: &str) {
        if path == self.aux_path {
            return;
        }
        self.aux_path = path.to_string();
        self.aux_ok = false;
        self.aux_view = match image::open(path) {
            Ok(img) => {
                let img = img.thumbnail(256, 256).to_rgba8();
                let (w, h) = img.dimensions();
                self.aux_ok = true;
                image_view(gpu, img.into_raw(), w, h)
            }
            Err(_) => image_view(gpu, vec![0; 4], 1, 1),
        };
        for slot in 0..4 {
            if let Some(c) = self.chains[slot].take() {
                self.ensure(gpu, slot, c.w, c.h);
            }
        }
    }

    /// Makes sure the ping-pong textures for `slot` match the size.
    pub fn ensure(&mut self, gpu: &Gpu, slot: usize, w: u32, h: u32) {
        if self.chains[slot].as_ref().map(|c| c.w == w && c.h == h).unwrap_or(false) || w == 0 || h == 0 {
            return;
        }
        let _ = self.format;
        let scene_view = target(gpu, w, h, "vfx-scene");
        let a_view = target(gpu, w, h, "vfx-a");
        let b_view = target(gpu, w, h, "vfx-b");
        let bgs = [self.bind(gpu, &scene_view), self.bind(gpu, &a_view), self.bind(gpu, &b_view)];
        self.chains[slot] = Some(Chain { w, h, scene_view, a_view, b_view, bgs });
    }

    pub fn scene_view(&self, slot: usize) -> Option<&wgpu::TextureView> {
        self.chains[slot].as_ref().map(|c| &c.scene_view)
    }

    /// Writes the parameters of the active effects (call once per frame).
    pub fn upload(&self, gpu: &Gpu, active: &[(usize, &Cfg)], time: f32) {
        for (k, (_, c)) in active.iter().enumerate() {
            let g = |i: usize| c.p.get(i).copied().unwrap_or(0.0);
            let u = Uniform {
                a: [g(0), g(1), g(2), g(3)],
                b: [g(4), g(5), g(6), g(7)],
                color: c.color,
                misc: [time, if self.aux_ok { 1.0 } else { 0.0 }, 0.0, 0.0],
                _pad: [0.0; 48],
            };
            gpu.queue.write_buffer(&self.uniform, k as u64 * SLOT, bytemuck::bytes_of(&u));
        }
    }

    /// Runs the effect chain: chain scene → … → `out`.
    pub fn run(&self, enc: &mut wgpu::CommandEncoder, slot: usize, out: &wgpu::TextureView, active: &[(usize, &Cfg)]) {
        let Some(c) = &self.chains[slot] else { return };
        let mut src = 0usize; // 0 = scene, 1 = a, 2 = b
        for (k, (di, _)) in active.iter().enumerate() {
            let last = k + 1 == active.len();
            let dst_idx = if src == 1 { 2 } else { 1 };
            let dst = if last { out } else if dst_idx == 1 { &c.a_view } else { &c.b_view };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            pass.set_pipeline(&self.pipes[*di]);
            pass.set_bind_group(0, &c.bgs[src], &[(k as u64 * SLOT) as u32]);
            pass.draw(0..3, 0..1);
            src = dst_idx;
        }
    }
}
