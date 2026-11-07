// Premultiplied -> straight alpha (+ optional horizontal mirror), for video outputs and screenshots.
@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> opts: vec4<u32>;

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_unpremul(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    var px = vec2<i32>(p.xy);
    if (opts.x == 1u) {
        px.x = i32(textureDimensions(src_tex).x) - 1 - px.x;
    }
    let c = textureLoad(src_tex, px, 0);
    if (c.a <= 0.0001) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(c.rgb / c.a, c.a);
}
