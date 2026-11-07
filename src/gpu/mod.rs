//! wgpu renderer: Live2D drawables (3 blend modes, stencil clipping masks) and sprites.

use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use winit::window::Window;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct Affine {
    pub m: [f32; 4],
    pub t: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct DrawData {
    pub multiply: [f32; 4],
    pub screen: [f32; 4],
    pub opacity: [f32; 4],
    /// x = smooth-mask set index (-1 = none), y = inverted
    pub mask: [f32; 4],
}

impl DrawData {
    pub fn plain(opacity: f32) -> DrawData {
        DrawData { multiply: [1.0; 4], screen: [0.0; 4], opacity: [opacity, 0.0, 0.0, 0.0], mask: [-1.0, 0.0, 0.0, 0.0] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct MaskSet {
    pub bbox: [f32; 4],
    pub cell: [f32; 4],
    pub chan: [f32; 4],
}

pub const MAX_MASK_SETS: usize = 1024;

/// Per-frame allocator of smooth-mask atlas cells (4 masks per cell, one per RGBA channel).
pub struct MaskAlloc {
    pub size: u32,
    grid: u32,
    max: u32,
    pub sets: Vec<MaskSet>,
    /// Pixel rectangles of the cells, parallel to `sets`.
    pub rects: Vec<[f32; 4]>,
}

impl MaskAlloc {
    pub fn new(size: u32, max: u32) -> MaskAlloc {
        let max = max.clamp(4, MAX_MASK_SETS as u32);
        let grid = ((max / 4) as f32).sqrt().ceil().max(1.0) as u32;
        MaskAlloc { size, grid, max, sets: Vec::new(), rects: Vec::new() }
    }

    /// Reserves a mask for the given NDC bounds [min_x, min_y, max_x, max_y].
    pub fn alloc(&mut self, b: [f32; 4]) -> Option<u32> {
        let i = self.sets.len() as u32;
        if i >= self.max {
            return None;
        }
        let (w, h) = ((b[2] - b[0]).max(1e-4), (b[3] - b[1]).max(1e-4));
        let pad = [w * 0.02, h * 0.02];
        let bbox = [b[0] - pad[0], b[1] - pad[1], w + pad[0] * 2.0, h + pad[1] * 2.0];
        let cell = i / 4;
        let (cx, cy) = (cell % self.grid, cell / self.grid);
        let g = self.grid as f32;
        let cpx = self.size as f32 / g;
        self.sets.push(MaskSet {
            bbox,
            cell: [cx as f32 / g, cy as f32 / g, 1.0 / g, 1.0 / g],
            chan: [(i % 4) as f32, 0.0, 0.0, 0.0],
        });
        self.rects.push([cx as f32 * cpx, cy as f32 * cpx, cpx, cpx]);
        Some(i)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blend {
    Normal = 0,
    Additive = 1,
    Multiply = 2,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Clip {
    None = 0,
    Inside = 1,
    Outside = 2,
}

pub struct Gpu {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub format: wgpu::TextureFormat,
    pub adapter_name: String,
    pub depth: wgpu::TextureView,
    obj_layout: wgpu::BindGroupLayout,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    draw_pipes: Vec<wgpu::RenderPipeline>,
    mask_pipe: wgpu::RenderPipeline,
    clear_pipe: wgpu::RenderPipeline,
    unpremul_layout: wgpu::BindGroupLayout,
    unpremul_pipe: wgpu::RenderPipeline,
    atlas_pipes: Vec<wgpu::RenderPipeline>,
    pub atlas_size: u32,
    atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    /// 1x1 stand-in for the atlas while rendering *into* the atlas.
    dummy_view: wgpu::TextureView,
    mask_sets: wgpu::Buffer,
}

fn atlas_texture(device: &wgpu::Device, size: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mask-atlas"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let v = t.create_view(&Default::default());
    (t, v)
}

fn blend_state(b: Blend) -> wgpu::BlendState {
    use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
    let keep_alpha = C { src_factor: F::Zero, dst_factor: F::One, operation: O::Add };
    match b {
        Blend::Normal => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        Blend::Additive => wgpu::BlendState {
            color: C { src_factor: F::One, dst_factor: F::One, operation: O::Add },
            alpha: keep_alpha,
        },
        Blend::Multiply => wgpu::BlendState {
            color: C { src_factor: F::Dst, dst_factor: F::OneMinusSrcAlpha, operation: O::Add },
            alpha: keep_alpha,
        },
    }
}

fn stencil_state(compare: wgpu::CompareFunction, pass: wgpu::StencilOperation, write: u32) -> wgpu::StencilState {
    let face = wgpu::StencilFaceState {
        compare,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op: pass,
    };
    wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask: write }
}

fn depth_texture(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth-stencil"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl Gpu {
    pub async fn new(window: Arc<Window>, vsync: bool, low_power: bool, atlas_size: u32) -> Result<Gpu> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: if low_power {
                    wgpu::PowerPreference::LowPower
                } else {
                    wgpu::PowerPreference::HighPerformance
                },
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no compatible GPU adapter")?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("rigrs"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                },
                None,
            )
            .await?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync },
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let obj_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&obj_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let buffers = [
            wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2],
            },
            wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![1 => Float32x2],
            },
        ];
        let make = |vs: &str, fs: &str, bufs: &[wgpu::VertexBufferLayout], blend: Option<wgpu::BlendState>, writes: wgpu::ColorWrites, stencil: wgpu::StencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: vs,
                    compilation_options: Default::default(),
                    buffers: bufs,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil,
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: fs,
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: writes })],
                }),
                multiview: None,
                cache: None,
            })
        };
        use wgpu::{CompareFunction as Cmp, StencilOperation as Op};
        let mut draw_pipes = Vec::new();
        for b in [Blend::Normal, Blend::Additive, Blend::Multiply] {
            for (cmp, _) in [(Cmp::Always, Clip::None), (Cmp::Equal, Clip::Inside), (Cmp::NotEqual, Clip::Outside)] {
                draw_pipes.push(make(
                    "vs_main",
                    "fs_draw",
                    &buffers,
                    Some(blend_state(b)),
                    wgpu::ColorWrites::ALL,
                    stencil_state(cmp, Op::Keep, 0),
                ));
            }
        }
        let mask_pipe = make("vs_main", "fs_mask", &buffers, None, wgpu::ColorWrites::empty(), stencil_state(Cmp::Always, Op::Replace, 0xff));
        let clear_pipe = make("vs_full", "fs_none", &[], None, wgpu::ColorWrites::empty(), stencil_state(Cmp::Always, Op::Replace, 0xff));

        // Smooth-mask atlas: additive union of mask alpha into one RGBA channel per mask.
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let atlas_pipes = [wgpu::ColorWrites::RED, wgpu::ColorWrites::GREEN, wgpu::ColorWrites::BLUE, wgpu::ColorWrites::ALPHA]
            .into_iter()
            .map(|writes| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("mask-atlas"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState { module: &shader, entry_point: "vs_mask_atlas", compilation_options: Default::default(), buffers: &buffers },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: "fs_mask_atlas",
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: Some(add), write_mask: writes })],
                    }),
                    multiview: None,
                    cache: None,
                })
            })
            .collect();
        let (atlas, atlas_view) = atlas_texture(&device, atlas_size);
        let (_dummy, dummy_view) = atlas_texture(&device, 1);
        let mask_sets = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mask-sets"),
            size: (MAX_MASK_SETS * std::mem::size_of::<MaskSet>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let unpremul_shader = device.create_shader_module(wgpu::include_wgsl!("unpremul.wgsl"));
        let unpremul_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("unpremul"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let up_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&unpremul_layout],
            push_constant_ranges: &[],
        });
        let unpremul_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("unpremul"),
            layout: Some(&up_layout),
            vertex: wgpu::VertexState {
                module: &unpremul_shader,
                entry_point: "vs_full",
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &unpremul_shader,
                entry_point: "fs_unpremul",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let depth = depth_texture(&device, config.width, config.height);
        Ok(Gpu {
            adapter_name: adapter.get_info().name,
            surface,
            device,
            queue,
            config,
            format,
            depth,
            obj_layout,
            tex_layout,
            sampler,
            draw_pipes,
            mask_pipe,
            clear_pipe,
            unpremul_layout,
            unpremul_pipe,
            atlas_pipes,
            atlas_size,
            atlas,
            atlas_view,
            dummy_view,
            mask_sets,
        })
    }

    /// Recreates the mask atlas; existing batches must be `rebind`-ed afterwards.
    pub fn set_atlas_size(&mut self, size: u32) {
        if size != self.atlas_size {
            let (t, v) = atlas_texture(&self.device, size);
            self.atlas = t;
            self.atlas_view = v;
            self.atlas_size = size;
        }
    }

    pub fn write_mask_sets(&self, sets: &[MaskSet]) {
        if !sets.is_empty() {
            self.queue.write_buffer(&self.mask_sets, 0, bytemuck::cast_slice(&sets[..sets.len().min(MAX_MASK_SETS)]));
        }
    }

    /// Starts the pass that renders smooth masks into the atlas.
    pub fn mask_painter<'a>(&'a self, enc: &'a mut wgpu::CommandEncoder) -> MaskPainter<'a> {
        let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mask-atlas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.atlas_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        MaskPainter { pass, gpu: self, chan: usize::MAX, draws: 0 }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth = depth_texture(&self.device, w, h);
    }

    pub fn set_vsync(&mut self, vsync: bool) {
        let mode = if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        if self.config.present_mode != mode {
            self.config.present_mode = mode;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Uploads straight-alpha RGBA8 pixels (premultiplying them) as a sampled texture.
    pub fn texture(&self, mut rgba: Vec<u8>, w: u32, h: u32) -> wgpu::BindGroup {
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a != 255 {
                px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
                px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
                px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
            }
        }
        let tex = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: None,
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
        );
        let view = tex.create_view(&Default::default());
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    pub fn offscreen(&self, w: u32, h: u32) -> Offscreen {
        let mk = |usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("offscreen"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage,
                view_formats: &[],
            })
        };
        let scene = mk(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let out = mk(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let scene_view = scene.create_view(&Default::default());
        let opts = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unpremul-opts"),
            contents: bytemuck::cast_slice(&[0u32; 4]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.unpremul_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene_view) },
                wgpu::BindGroupEntry { binding: 1, resource: opts.as_entire_binding() },
            ],
        });
        Offscreen {
            w,
            h,
            out_view: out.create_view(&Default::default()),
            out,
            scene_view,
            depth: depth_texture(&self.device, w, h),
            bind,
            opts,
        }
    }

    /// scene (premultiplied) -> out (straight alpha)
    pub fn unpremultiply(&self, enc: &mut wgpu::CommandEncoder, off: &Offscreen) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("unpremul"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &off.out_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.unpremul_pipe);
        pass.set_bind_group(0, &off.bind, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn is_bgra(&self) -> bool {
        matches!(self.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb)
    }
}

pub struct Offscreen {
    pub w: u32,
    pub h: u32,
    pub scene_view: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    pub out: wgpu::Texture,
    out_view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    opts: wgpu::Buffer,
}

impl Offscreen {
    /// Horizontal mirror applied in the straight-alpha conversion pass.
    pub fn set_mirror(&self, q: &wgpu::Queue, mirror: bool) {
        q.write_buffer(&self.opts, 0, bytemuck::cast_slice(&[mirror as u32, 0, 0, 0]));
    }
}

pub fn padded_row(w: u32) -> u32 {
    let row = w * 4;
    let a = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    (row + a - 1) / a * a
}

pub fn copy_to_buffer(enc: &mut wgpu::CommandEncoder, tex: &wgpu::Texture, buf: &wgpu::Buffer, w: u32, h: u32) {
    enc.copy_texture_to_buffer(
        wgpu::ImageCopyTexture { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::ImageCopyBuffer {
            buffer: buf,
            layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(padded_row(w)), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

/// GPU geometry of one Live2D model or sprite.
pub struct Batch {
    pos: wgpu::Buffer,
    uv: wgpu::Buffer,
    idx: wgpu::Buffer,
    draws: wgpu::Buffer,
    xf: wgpu::Buffer,
    obj: wgpu::BindGroup,
    /// Same as `obj` but without the atlas (used in the atlas pass itself).
    obj_mask: wgpu::BindGroup,
    pub textures: Vec<wgpu::BindGroup>,
}

fn obj_bind_group(gpu: &Gpu, xf: &wgpu::Buffer, draws: &wgpu::Buffer, with_atlas: bool) -> wgpu::BindGroup {
    let atlas = if with_atlas { &gpu.atlas_view } else { &gpu.dummy_view };
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &gpu.obj_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: xf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: draws.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: gpu.mask_sets.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(atlas) },
        ],
    })
}

/// Draws mask drawables into the smooth-mask atlas.
pub struct MaskPainter<'a> {
    pass: wgpu::RenderPass<'a>,
    gpu: &'a Gpu,
    chan: usize,
    pub draws: u32,
}

impl<'a> MaskPainter<'a> {
    pub fn bind(&mut self, b: &Batch) {
        self.pass.set_bind_group(0, &b.obj_mask, &[]);
        self.pass.set_vertex_buffer(0, b.pos.slice(..));
        self.pass.set_vertex_buffer(1, b.uv.slice(..));
        self.pass.set_index_buffer(b.idx.slice(..), wgpu::IndexFormat::Uint16);
    }

    /// Selects the atlas cell of mask `set` (viewport + channel).
    pub fn cell(&mut self, alloc: &MaskAlloc, set: u32) {
        let r = alloc.rects[set as usize];
        self.pass.set_viewport(r[0], r[1], r[2], r[3], 0.0, 1.0);
        self.pass.set_scissor_rect(r[0] as u32, r[1] as u32, r[2] as u32, r[3] as u32);
        let ch = (set % 4) as usize;
        if ch != self.chan {
            self.pass.set_pipeline(&self.gpu.atlas_pipes[ch]);
            self.chan = ch;
        }
    }

    pub fn draw(&mut self, b: &Batch, tex: usize, range: Range<u32>, base: i32, set: u32) {
        if let Some(t) = b.textures.get(tex) {
            self.pass.set_bind_group(1, t, &[]);
            self.pass.draw_indexed(range, base, set..set + 1);
            self.draws += 1;
        }
    }
}

fn bytes_or_min<T: Pod>(v: &[T]) -> Vec<u8> {
    let mut b = bytemuck::cast_slice(v).to_vec();
    while b.len() < 8 || b.len() % 4 != 0 {
        b.push(0);
    }
    b
}

impl Batch {
    pub fn new(gpu: &Gpu, pos: &[[f32; 2]], uv: &[[f32; 2]], idx: &[u16], draw_count: usize, textures: Vec<wgpu::BindGroup>) -> Batch {
        let d = &gpu.device;
        let init = |label, contents: &[u8], usage| d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage });
        use wgpu::BufferUsages as U;
        let pos = init("pos", &bytes_or_min(pos), U::VERTEX | U::COPY_DST);
        let uv = init("uv", &bytes_or_min(uv), U::VERTEX);
        let idx = init("idx", &bytes_or_min(idx), U::INDEX);
        let draws = init("draws", bytemuck::cast_slice(&vec![DrawData::plain(1.0); draw_count.max(1)]), U::STORAGE | U::COPY_DST);
        let xf = init("xf", bytemuck::bytes_of(&Affine::default()), U::UNIFORM | U::COPY_DST);
        let obj = obj_bind_group(gpu, &xf, &draws, true);
        let obj_mask = obj_bind_group(gpu, &xf, &draws, false);
        Batch { pos, uv, idx, draws, xf, obj, obj_mask, textures }
    }

    /// Re-creates the bind group after the mask atlas changed.
    pub fn rebind(&mut self, gpu: &Gpu) {
        self.obj = obj_bind_group(gpu, &self.xf, &self.draws, true);
        self.obj_mask = obj_bind_group(gpu, &self.xf, &self.draws, false);
    }

    pub fn set_xf(&self, q: &wgpu::Queue, a: &Affine) {
        q.write_buffer(&self.xf, 0, bytemuck::bytes_of(a));
    }
    pub fn set_draws(&self, q: &wgpu::Queue, d: &[DrawData]) {
        if !d.is_empty() {
            q.write_buffer(&self.draws, 0, bytemuck::cast_slice(d));
        }
    }
    pub fn set_positions(&self, q: &wgpu::Queue, p: &[[f32; 2]]) {
        if !p.is_empty() {
            q.write_buffer(&self.pos, 0, bytemuck::cast_slice(p));
        }
    }
}

/// Records draw calls into a render pass, tracking stencil state for clipping masks.
pub struct Painter<'a> {
    pass: wgpu::RenderPass<'a>,
    gpu: &'a Gpu,
    stencil: u32,
    pipe: usize,
    mask_key: Option<usize>,
    pub draw_calls: u32,
}

const PIPE_MASK: usize = 100;
const PIPE_NONE: usize = usize::MAX;

impl<'a> Painter<'a> {
    pub fn new(gpu: &'a Gpu, enc: &'a mut wgpu::CommandEncoder, color: &'a wgpu::TextureView, depth: &'a wgpu::TextureView, clear: wgpu::Color) -> Painter<'a> {
        let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        Painter { pass, gpu, stencil: 0, pipe: PIPE_NONE, mask_key: None, draw_calls: 0 }
    }

    pub fn bind(&mut self, b: &Batch) {
        self.pass.set_bind_group(0, &b.obj, &[]);
        self.pass.set_vertex_buffer(0, b.pos.slice(..));
        self.pass.set_vertex_buffer(1, b.uv.slice(..));
        self.pass.set_index_buffer(b.idx.slice(..), wgpu::IndexFormat::Uint16);
        self.mask_key = None;
    }

    fn use_pipe(&mut self, id: usize) {
        if self.pipe != id {
            let p = if id == PIPE_MASK { &self.gpu.mask_pipe } else { &self.gpu.draw_pipes[id] };
            self.pass.set_pipeline(p);
            self.pipe = id;
        }
    }

    /// Starts a new mask set. Returns false if the previous mask (same key) can be reused.
    pub fn begin_mask(&mut self, key: usize) -> bool {
        if self.mask_key == Some(key) {
            return false;
        }
        if self.stencil >= 255 {
            self.pass.set_pipeline(&self.gpu.clear_pipe);
            self.pass.set_stencil_reference(0);
            self.pass.draw(0..3, 0..1);
            self.pipe = PIPE_NONE;
            self.stencil = 0;
        }
        self.stencil += 1;
        self.use_pipe(PIPE_MASK);
        self.pass.set_stencil_reference(self.stencil);
        self.mask_key = Some(key);
        true
    }

    pub fn mask(&mut self, b: &Batch, tex: usize, range: Range<u32>, base: i32, inst: u32) {
        self.use_pipe(PIPE_MASK);
        if let Some(t) = b.textures.get(tex) {
            self.pass.set_bind_group(1, t, &[]);
            self.pass.draw_indexed(range, base, inst..inst + 1);
            self.draw_calls += 1;
        }
    }

    pub fn draw(&mut self, b: &Batch, tex: usize, range: Range<u32>, base: i32, inst: u32, blend: Blend, clip: Clip) {
        self.use_pipe(blend as usize * 3 + clip as usize);
        if clip != Clip::None {
            self.pass.set_stencil_reference(self.stencil);
        }
        if let Some(t) = b.textures.get(tex) {
            self.pass.set_bind_group(1, t, &[]);
            self.pass.draw_indexed(range, base, inst..inst + 1);
            self.draw_calls += 1;
        }
    }
}
