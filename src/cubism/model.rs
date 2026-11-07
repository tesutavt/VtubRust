//! Safe-ish wrapper over a Cubism moc + model instance.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::collections::HashMap;
use std::ffi::{c_void, CStr};

use anyhow::{bail, Result};

use super::core::*;

struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
}

impl AlignedBuf {
    fn new(size: usize, align: usize) -> AlignedBuf {
        let layout = Layout::from_size_align(size.max(1), align).expect("layout");
        let ptr = unsafe { alloc_zeroed(layout) };
        assert!(!ptr.is_null(), "out of memory");
        AlignedBuf { ptr, layout }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, self.layout) }
    }
}

pub struct Drawable {
    pub id: String,
    pub texture: usize,
    pub constant_flags: u8,
    pub vertex_count: usize,
    pub index_count: usize,
    pub masks: Vec<usize>,
    /// Offset into the renderer's concatenated vertex/index buffers.
    pub vertex_offset: usize,
    pub index_offset: usize,
}

pub struct Canvas {
    pub size: CsmVec2,
    pub origin: CsmVec2,
    pub pixels_per_unit: f32,
}

pub struct Model {
    core: &'static Core,
    _moc_mem: AlignedBuf,
    _model_mem: AlignedBuf,
    raw: *mut CsmModel,

    pub param_ids: Vec<String>,
    pub param_min: Vec<f32>,
    pub param_max: Vec<f32>,
    pub param_default: Vec<f32>,
    param_index: HashMap<String, usize>,
    pub part_ids: Vec<String>,
    part_index: HashMap<String, usize>,
    pub drawables: Vec<Drawable>,
    pub canvas: Canvas,

    params: *mut f32,
    parts: *mut f32,
    dyn_flags: *const u8,
    opacities: *const f32,
    render_orders: *const i32,
    positions: *const *const CsmVec2,
    multiply: *const CsmVec4,
    screen: *const CsmVec4,
}

unsafe fn ids(ptr: *const *const std::ffi::c_char, n: usize) -> Vec<String> {
    (0..n)
        .map(|i| CStr::from_ptr(*ptr.add(i)).to_string_lossy().into_owned())
        .collect()
}

unsafe fn slice<'a, T>(ptr: *const T, n: usize) -> &'a [T] {
    if n == 0 || ptr.is_null() {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, n)
    }
}

impl Model {
    pub fn load(core: &'static Core, moc_bytes: &[u8]) -> Result<Model> {
        unsafe {
            let moc_mem = AlignedBuf::new(moc_bytes.len(), ALIGN_MOC);
            std::ptr::copy_nonoverlapping(moc_bytes.as_ptr(), moc_mem.ptr, moc_bytes.len());
            let size = moc_bytes.len() as u32;

            let ver = (core.get_moc_version)(moc_mem.ptr as *const c_void, size);
            let latest = (core.get_latest_moc_version)();
            if ver > latest {
                bail!("moc3 version {ver} is newer than this Cubism Core supports ({latest})");
            }
            let moc = (core.revive_moc_in_place)(moc_mem.ptr as *mut c_void, size);
            if moc.is_null() {
                bail!("csmReviveMocInPlace failed (corrupt or unsupported .moc3)");
            }
            let model_size = (core.get_sizeof_model)(moc) as usize;
            let model_mem = AlignedBuf::new(model_size, ALIGN_MODEL);
            let raw = (core.initialize_model_in_place)(moc, model_mem.ptr as *mut c_void, model_size as u32);
            if raw.is_null() {
                bail!("csmInitializeModelInPlace failed");
            }

            let pc = (core.get_parameter_count)(raw) as usize;
            let param_ids = ids((core.get_parameter_ids)(raw), pc);
            let param_min = slice((core.get_parameter_minimum_values)(raw), pc).to_vec();
            let param_max = slice((core.get_parameter_maximum_values)(raw), pc).to_vec();
            let param_default = slice((core.get_parameter_default_values)(raw), pc).to_vec();
            let param_index = param_ids.iter().enumerate().map(|(i, s)| (s.clone(), i)).collect();

            let partc = (core.get_part_count)(raw) as usize;
            let part_ids = ids((core.get_part_ids)(raw), partc);
            let part_index = part_ids.iter().enumerate().map(|(i, s)| (s.clone(), i)).collect();

            let dc = (core.get_drawable_count)(raw) as usize;
            let d_ids = ids((core.get_drawable_ids)(raw), dc);
            let cflags = slice((core.get_drawable_constant_flags)(raw), dc);
            let tex = slice((core.get_drawable_texture_indices)(raw), dc);
            let vcount = slice((core.get_drawable_vertex_counts)(raw), dc);
            let icount = slice((core.get_drawable_index_counts)(raw), dc);
            let mcount = slice((core.get_drawable_mask_counts)(raw), dc);
            let masks = slice((core.get_drawable_masks)(raw), dc);

            let mut drawables = Vec::with_capacity(dc);
            let (mut vo, mut io) = (0usize, 0usize);
            for i in 0..dc {
                let m = slice(masks[i], mcount[i].max(0) as usize)
                    .iter()
                    .filter(|&&x| x >= 0)
                    .map(|&x| x as usize)
                    .collect();
                drawables.push(Drawable {
                    id: d_ids[i].clone(),
                    texture: tex[i].max(0) as usize,
                    constant_flags: cflags[i],
                    vertex_count: vcount[i] as usize,
                    index_count: icount[i] as usize,
                    masks: m,
                    vertex_offset: vo,
                    index_offset: io,
                });
                vo += vcount[i] as usize;
                io += icount[i] as usize;
            }

            let mut canvas = Canvas { size: CsmVec2::default(), origin: CsmVec2::default(), pixels_per_unit: 1.0 };
            (core.read_canvas_info)(raw, &mut canvas.size, &mut canvas.origin, &mut canvas.pixels_per_unit);

            let model = Model {
                core,
                params: (core.get_parameter_values)(raw),
                parts: (core.get_part_opacities)(raw),
                dyn_flags: (core.get_drawable_dynamic_flags)(raw),
                opacities: (core.get_drawable_opacities)(raw),
                render_orders: (core.get_drawable_render_orders)(raw),
                positions: (core.get_drawable_vertex_positions)(raw),
                multiply: (core.get_drawable_multiply_colors)(raw),
                screen: (core.get_drawable_screen_colors)(raw),
                _moc_mem: moc_mem,
                _model_mem: model_mem,
                raw,
                param_ids,
                param_min,
                param_max,
                param_default,
                param_index,
                part_ids,
                part_index,
                drawables,
                canvas,
            };
            Ok(model)
        }
    }

    pub fn param(&self, id: &str) -> Option<usize> {
        self.param_index.get(id).copied()
    }

    pub fn part(&self, id: &str) -> Option<usize> {
        self.part_index.get(id).copied()
    }

    pub fn params(&self) -> &[f32] {
        unsafe { slice(self.params, self.param_ids.len()) }
    }

    pub fn params_mut(&mut self) -> &mut [f32] {
        unsafe { std::slice::from_raw_parts_mut(self.params, self.param_ids.len()) }
    }

    pub fn parts_mut(&mut self) -> &mut [f32] {
        if self.part_ids.is_empty() {
            return &mut [];
        }
        unsafe { std::slice::from_raw_parts_mut(self.parts, self.part_ids.len()) }
    }

    pub fn reset_pose(&mut self) {
        let n = self.param_ids.len();
        let defaults = self.param_default.as_ptr();
        unsafe { std::ptr::copy_nonoverlapping(defaults, self.params, n) };
        self.parts_mut().fill(1.0);
    }

    pub fn set_param(&mut self, idx: usize, v: f32) {
        let (lo, hi) = (self.param_min[idx], self.param_max[idx]);
        self.params_mut()[idx] = v.clamp(lo.min(hi), hi.max(lo));
    }

    pub fn update(&mut self) {
        unsafe {
            (self.core.reset_drawable_dynamic_flags)(self.raw);
            (self.core.update_model)(self.raw);
        }
    }

    pub fn dynamic_flags(&self) -> &[u8] {
        unsafe { slice(self.dyn_flags, self.drawables.len()) }
    }
    pub fn opacities(&self) -> &[f32] {
        unsafe { slice(self.opacities, self.drawables.len()) }
    }
    pub fn render_orders(&self) -> &[i32] {
        unsafe { slice(self.render_orders, self.drawables.len()) }
    }
    pub fn multiply_colors(&self) -> &[CsmVec4] {
        unsafe { slice(self.multiply, self.drawables.len()) }
    }
    pub fn screen_colors(&self) -> &[CsmVec4] {
        unsafe { slice(self.screen, self.drawables.len()) }
    }
    pub fn positions(&self, i: usize) -> &[CsmVec2] {
        unsafe { slice(*self.positions.add(i), self.drawables[i].vertex_count) }
    }
    pub fn uvs(&self, i: usize) -> &[CsmVec2] {
        unsafe { slice(*(self.core.get_drawable_vertex_uvs)(self.raw).add(i), self.drawables[i].vertex_count) }
    }
    pub fn indices(&self, i: usize) -> &[u16] {
        unsafe { slice(*(self.core.get_drawable_indices)(self.raw).add(i), self.drawables[i].index_count) }
    }
}
