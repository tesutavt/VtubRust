// Live2D drawables and sprites share one shader. Textures are premultiplied on upload.

struct Affine {
    m: vec4<f32>, // 2x2 matrix columns: (m.x, m.y) and (m.z, m.w)
    t: vec4<f32>, // translation xy, z > 0.5 => flip V
};

struct DrawData {
    multiply: vec4<f32>,
    screen: vec4<f32>,
    opacity: vec4<f32>,
    // x = mask set index (-1 = none / stencil), y = 1 if inverted
    mask: vec4<f32>,
};

// One smooth clipping mask: NDC bounds of the clipped area, its atlas cell (uv rect)
// and the RGBA channel it lives in.
struct MaskSet {
    bbox: vec4<f32>,
    cell: vec4<f32>,
    chan: vec4<f32>,
};

@group(0) @binding(0) var<uniform> xf: Affine;
@group(0) @binding(1) var<storage, read> draws: array<DrawData>;
@group(0) @binding(2) var<storage, read> mask_sets: array<MaskSet>;
@group(0) @binding(3) var mask_atlas: texture_2d<f32>;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) inst: u32,
    @location(2) ndc: vec2<f32>,
};

@vertex
fn vs_main(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @builtin(instance_index) inst: u32) -> VOut {
    var o: VOut;
    let x = xf.m.x * p.x + xf.m.z * p.y + xf.t.x;
    let y = xf.m.y * p.x + xf.m.w * p.y + xf.t.y;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = select(uv, vec2<f32>(uv.x, 1.0 - uv.y), xf.t.z > 0.5);
    o.inst = inst;
    o.ndc = vec2<f32>(x, y);
    return o;
}

fn mask_value(set_f: f32, inverted: f32, ndc: vec2<f32>) -> f32 {
    let s = mask_sets[u32(max(set_f, 0.0))];
    let l = (ndc - s.bbox.xy) / s.bbox.zw;
    let uv = s.cell.xy + vec2<f32>(l.x, 1.0 - l.y) * s.cell.zw;
    let m4 = textureSampleLevel(mask_atlas, samp, uv, 0.0);
    let ch = u32(s.chan.x);
    var m = select(select(select(m4.a, m4.b, ch == 2u), m4.g, ch == 1u), m4.r, ch == 0u);
    let inside = all(l >= vec2<f32>(0.0)) && all(l <= vec2<f32>(1.0));
    m = select(0.0, clamp(m, 0.0, 1.0), inside);
    return select(m, 1.0 - m, inverted > 0.5);
}

@fragment
fn fs_draw(i: VOut) -> @location(0) vec4<f32> {
    let d = draws[i.inst];
    let c = textureSample(tex, samp, i.uv);
    var rgb = c.rgb * d.multiply.rgb;
    rgb = rgb + d.screen.rgb * c.a - rgb * d.screen.rgb;
    let m = mask_value(d.mask.x, d.mask.y, i.ndc);
    return vec4<f32>(rgb, c.a) * d.opacity.x * select(1.0, m, d.mask.x >= 0.0);
}

// Renders mask drawables into their atlas cell (viewport = cell).
@vertex
fn vs_mask_atlas(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @builtin(instance_index) inst: u32) -> VOut {
    var o: VOut;
    let s = mask_sets[inst];
    let x = xf.m.x * p.x + xf.m.z * p.y + xf.t.x;
    let y = xf.m.y * p.x + xf.m.w * p.y + xf.t.y;
    let l = (vec2<f32>(x, y) - s.bbox.xy) / s.bbox.zw;
    o.pos = vec4<f32>(l * 2.0 - 1.0, 0.0, 1.0);
    o.uv = select(uv, vec2<f32>(uv.x, 1.0 - uv.y), xf.t.z > 0.5);
    o.inst = inst;
    o.ndc = vec2<f32>(x, y);
    return o;
}

@fragment
fn fs_mask_atlas(i: VOut) -> @location(0) vec4<f32> {
    let a = textureSample(tex, samp, i.uv).a;
    return vec4<f32>(a);
}

@fragment
fn fs_mask(i: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv);
    if (c.a < 0.5) {
        discard;
    }
    return vec4<f32>(0.0);
}

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_none() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
