//! Thin dynamic binding to the official Live2D Cubism Core C API (Live2DCubismCore.dll).
//!
//! The Core library is proprietary and NOT shipped with this project. Point `--core`
//! at a `Live2DCubismCore.dll` from the official Cubism SDK for Native
//! (https://www.live2d.com/en/sdk/download/native/), subject to the Live2D license.

use std::ffi::{c_char, c_void};
use std::path::Path;

use anyhow::{Context, Result};

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CsmVec2 {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CsmVec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

pub enum CsmMoc {}
pub enum CsmModel {}

pub const ALIGN_MOC: usize = 64;
pub const ALIGN_MODEL: usize = 16;

// Constant drawable flags
pub const BLEND_ADDITIVE: u8 = 1 << 0;
pub const BLEND_MULTIPLICATIVE: u8 = 1 << 1;
#[allow(dead_code)]
pub const IS_DOUBLE_SIDED: u8 = 1 << 2;
pub const IS_INVERTED_MASK: u8 = 1 << 3;

// Dynamic drawable flags
pub const IS_VISIBLE: u8 = 1 << 0;
pub const VERTEX_POSITIONS_DID_CHANGE: u8 = 1 << 5;

macro_rules! core_api {
    ($( $field:ident : $sym:literal => fn($($arg:ty),*) $(-> $ret:ty)? ;)*) => {
        pub struct Core {
            _lib: libloading::Library,
            $( pub $field: unsafe extern "C" fn($($arg),*) $(-> $ret)?, )*
        }

        impl Core {
            pub fn load(path: &Path) -> Result<Core> {
                unsafe {
                    let lib = libloading::Library::new(path)
                        .with_context(|| format!("cannot load Cubism Core from {}", path.display()))?;
                    $(
                        let $field = *lib
                            .get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(concat!($sym, "\0").as_bytes())
                            .with_context(|| format!("missing symbol {}", $sym))?;
                    )*
                    Ok(Core { _lib: lib, $($field,)* })
                }
            }
        }
    };
}

core_api! {
    get_version: "csmGetVersion" => fn() -> u32;
    get_latest_moc_version: "csmGetLatestMocVersion" => fn() -> u32;
    get_moc_version: "csmGetMocVersion" => fn(*const c_void, u32) -> u32;
    revive_moc_in_place: "csmReviveMocInPlace" => fn(*mut c_void, u32) -> *mut CsmMoc;
    get_sizeof_model: "csmGetSizeofModel" => fn(*const CsmMoc) -> u32;
    initialize_model_in_place: "csmInitializeModelInPlace" => fn(*const CsmMoc, *mut c_void, u32) -> *mut CsmModel;
    update_model: "csmUpdateModel" => fn(*mut CsmModel);
    read_canvas_info: "csmReadCanvasInfo" => fn(*const CsmModel, *mut CsmVec2, *mut CsmVec2, *mut f32);

    get_parameter_count: "csmGetParameterCount" => fn(*const CsmModel) -> i32;
    get_parameter_ids: "csmGetParameterIds" => fn(*const CsmModel) -> *const *const c_char;
    get_parameter_minimum_values: "csmGetParameterMinimumValues" => fn(*const CsmModel) -> *const f32;
    get_parameter_maximum_values: "csmGetParameterMaximumValues" => fn(*const CsmModel) -> *const f32;
    get_parameter_default_values: "csmGetParameterDefaultValues" => fn(*const CsmModel) -> *const f32;
    get_parameter_values: "csmGetParameterValues" => fn(*mut CsmModel) -> *mut f32;

    get_part_count: "csmGetPartCount" => fn(*const CsmModel) -> i32;
    get_part_ids: "csmGetPartIds" => fn(*const CsmModel) -> *const *const c_char;
    get_part_opacities: "csmGetPartOpacities" => fn(*mut CsmModel) -> *mut f32;

    get_drawable_count: "csmGetDrawableCount" => fn(*const CsmModel) -> i32;
    get_drawable_ids: "csmGetDrawableIds" => fn(*const CsmModel) -> *const *const c_char;
    get_drawable_constant_flags: "csmGetDrawableConstantFlags" => fn(*const CsmModel) -> *const u8;
    get_drawable_dynamic_flags: "csmGetDrawableDynamicFlags" => fn(*const CsmModel) -> *const u8;
    get_drawable_texture_indices: "csmGetDrawableTextureIndices" => fn(*const CsmModel) -> *const i32;
    get_drawable_render_orders: "csmGetDrawableRenderOrders" => fn(*const CsmModel) -> *const i32;
    get_drawable_opacities: "csmGetDrawableOpacities" => fn(*const CsmModel) -> *const f32;
    get_drawable_mask_counts: "csmGetDrawableMaskCounts" => fn(*const CsmModel) -> *const i32;
    get_drawable_masks: "csmGetDrawableMasks" => fn(*const CsmModel) -> *const *const i32;
    get_drawable_vertex_counts: "csmGetDrawableVertexCounts" => fn(*const CsmModel) -> *const i32;
    get_drawable_vertex_positions: "csmGetDrawableVertexPositions" => fn(*const CsmModel) -> *const *const CsmVec2;
    get_drawable_vertex_uvs: "csmGetDrawableVertexUvs" => fn(*const CsmModel) -> *const *const CsmVec2;
    get_drawable_index_counts: "csmGetDrawableIndexCounts" => fn(*const CsmModel) -> *const i32;
    get_drawable_indices: "csmGetDrawableIndices" => fn(*const CsmModel) -> *const *const u16;
    get_drawable_multiply_colors: "csmGetDrawableMultiplyColors" => fn(*const CsmModel) -> *const CsmVec4;
    get_drawable_screen_colors: "csmGetDrawableScreenColors" => fn(*const CsmModel) -> *const CsmVec4;
    reset_drawable_dynamic_flags: "csmResetDrawableDynamicFlags" => fn(*mut CsmModel);
}

impl Core {
    pub fn version_string(&self) -> String {
        let v = unsafe { (self.get_version)() };
        format!("{}.{}.{}", v >> 24, (v >> 16) & 0xff, v & 0xffff)
    }
}
