// Post-processing effects. Input/output colours are premultiplied alpha.

struct Params {
    a: vec4<f32>,     // effect parameters 0..3
    b: vec4<f32>,     // effect parameters 4..7
    color: vec4<f32>, // effect colour (straight)
    misc: vec4<f32>,  // x = time (s), y = aux image present
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> P: Params;
@group(0) @binding(3) var aux: texture_2d<f32>;

struct V {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> V {
    var o: V;
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return o;
}

fn S(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src, smp, uv, 0.0);
}
fn res() -> vec2<f32> {
    return vec2<f32>(textureDimensions(src));
}
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}
fn hash2(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(hash(p), hash(p + vec2<f32>(17.31, 4.7)));
}
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}
fn unpremul(c: vec4<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0), c.rgb / c.a, c.a > 0.0001);
}
// "src over dst" with a straight colour and coverage k.
fn over(dst: vec4<f32>, rgb: vec3<f32>, k: f32) -> vec4<f32> {
    let kk = clamp(k, 0.0, 1.0);
    return vec4<f32>(rgb * kk, kk) + dst * (1.0 - kk);
}

// ---------------------------------------------------------------- colour grading
@fragment
fn fs_color(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    var rgb = unpremul(c);
    rgb = rgb + vec3<f32>(P.a.x);
    rgb = (rgb - 0.5) * P.a.y + 0.5;
    let l = luma(rgb);
    rgb = mix(vec3<f32>(l), rgb, P.a.z);
    // Hue rotation in YIQ space.
    let h = radians(P.a.w);
    let yiq = mat3x3<f32>(0.299, 0.596, 0.211, 0.587, -0.274, -0.523, 0.114, -0.322, 0.312) * rgb;
    let cs = cos(h);
    let sn = sin(h);
    let rot = vec3<f32>(yiq.x, yiq.y * cs - yiq.z * sn, yiq.y * sn + yiq.z * cs);
    rgb = mat3x3<f32>(1.0, 1.0, 1.0, 0.956, -0.272, -1.106, 0.621, -0.647, 1.703) * rot;
    rgb = rgb + vec3<f32>(P.b.x * 0.12, 0.0, -P.b.x * 0.12);
    rgb = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / max(P.b.y, 0.05)));
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * c.a, c.a);
}

// ---------------------------------------------------------------- weather (rain / snow)
@fragment
fn fs_weather(i: V) -> @location(0) vec4<f32> {
    let base = S(i.uv);
    let r = res();
    let aspect = r.x / r.y;
    let t = P.misc.x * P.a.z;
    var k = 0.0;
    for (var layer = 0; layer < 3; layer = layer + 1) {
        let lf = f32(layer);
        if (P.a.x < 0.5) {
            // Rain: thin slanted streaks.
            let sc = vec2<f32>(60.0 + lf * 30.0, 4.0) / P.b.x;
            var st = vec2<f32>(i.uv.x * aspect * sc.x + P.a.w * i.uv.y * 8.0, i.uv.y * sc.y - t * (2.5 + lf));
            let id = floor(st);
            let f = fract(st);
            if (hash(id + lf * 13.0) < P.a.y) {
                let x0 = 0.3 + 0.4 * hash(id * 1.7);
                let w = abs(f.x - x0);
                k = max(k, smoothstep(0.08, 0.0, w) * smoothstep(0.0, 0.6, f.y) * (0.5 - lf * 0.12));
            }
        } else {
            // Snow: soft drifting flakes.
            let sc = (12.0 + lf * 10.0) / P.b.x;
            var st = vec2<f32>(i.uv.x * aspect, i.uv.y) * sc + vec2<f32>(P.a.w * t * 0.3 + sin(t * 0.7 + lf) * 0.3, -t * (0.4 + lf * 0.15));
            let id = floor(st);
            if (hash(id + lf * 7.0) < P.a.y) {
                let c = hash2(id) * 0.7 + 0.15;
                let d = length(fract(st) - c);
                k = max(k, smoothstep(0.12, 0.04, d) * (0.9 - lf * 0.2));
            }
        }
    }
    return over(base, P.color.rgb, k * P.color.a);
}

// ---------------------------------------------------------------- glow / bloom
@fragment
fn fs_glow(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    let r = res();
    var acc = vec3<f32>(0.0);
    for (var n = 0; n < 24; n = n + 1) {
        let fn_ = f32(n);
        let ang = fn_ * 2.39996;
        let rad = P.a.z * sqrt((fn_ + 0.5) / 24.0);
        let s = S(i.uv + vec2<f32>(cos(ang), sin(ang)) * rad / r);
        let b = max(luma(unpremul(s)) - P.a.x, 0.0) / max(1.0 - P.a.x, 0.01);
        acc = acc + s.rgb * b;
    }
    acc = acc / 24.0 * P.a.y * P.color.rgb;
    return vec4<f32>(c.rgb + acc, min(1.0, c.a + luma(acc)));
}

// ---------------------------------------------------------------- backlight + outline
@fragment
fn fs_rim(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    let r = res();
    var amax = 0.0;
    for (var n = 0; n < 16; n = n + 1) {
        let ang = f32(n) * 0.3927;
        amax = max(amax, S(i.uv + vec2<f32>(cos(ang), sin(ang)) * P.a.z / r).a);
    }
    let edge = clamp(amax - c.a, 0.0, 1.0) * step(0.01, P.a.z);
    var ab = 0.0;
    for (var n = 0; n < 24; n = n + 1) {
        let fn_ = f32(n);
        let ang = fn_ * 2.39996;
        let rad = P.a.y * sqrt((fn_ + 0.5) / 24.0);
        ab = ab + S(i.uv + vec2<f32>(cos(ang), sin(ang)) * rad / r).a;
    }
    let back = clamp(ab / 24.0 * P.a.x, 0.0, 1.0) * (1.0 - c.a);
    let k = clamp(edge + back, 0.0, 1.0) * P.color.a;
    // Light sits *behind* the model.
    return c + vec4<f32>(P.color.rgb * k, k) * (1.0 - c.a);
}

// ---------------------------------------------------------------- particle helpers
fn star(p: vec2<f32>) -> f32 {
    let a = atan2(p.y, p.x);
    let r = length(p);
    let k = 0.25 + 0.12 * cos(a * 5.0);
    return smoothstep(k, k - 0.05, r);
}
fn heart(p0: vec2<f32>) -> f32 {
    let p = vec2<f32>(p0.x, -p0.y + 0.05) * 2.6;
    let q = p.x * p.x + p.y * p.y - 1.0;
    let v = q * q * q - p.x * p.x * p.y * p.y * p.y;
    return smoothstep(0.02, -0.02, v);
}
fn rot2(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

// ---------------------------------------------------------------- falling shapes
@fragment
fn fs_falling(i: V) -> @location(0) vec4<f32> {
    let base = S(i.uv);
    let r = res();
    let t = P.misc.x * P.a.w;
    var k = 0.0;
    for (var layer = 0; layer < 2; layer = layer + 1) {
        let lf = f32(layer);
        let sc = (6.0 + lf * 4.0) / P.a.z;
        var st = vec2<f32>(i.uv.x * r.x / r.y, i.uv.y) * sc + vec2<f32>(sin(t * 0.5 + lf) * 0.2, -t * (0.3 + lf * 0.1));
        let id = floor(st);
        if (hash(id + lf * 3.1) < P.a.y) {
            let c = hash2(id) * 0.5 + 0.25;
            let p = rot2(fract(st) - c, t * (hash(id) - 0.5) * 2.0);
            var v = 0.0;
            if (P.a.x < 0.5) {
                v = smoothstep(0.2, 0.15, length(p));
            } else if (P.a.x < 1.5) {
                v = star(p);
            } else {
                v = heart(p);
            }
            k = max(k, v);
        }
    }
    return over(base, P.color.rgb, k * P.color.a);
}

// ---------------------------------------------------------------- custom image particles
@fragment
fn fs_custom(i: V) -> @location(0) vec4<f32> {
    let base = S(i.uv);
    if (P.misc.y < 0.5) {
        return base;
    }
    let r = res();
    let t = P.misc.x * P.a.z;
    var outc = base;
    for (var layer = 0; layer < 2; layer = layer + 1) {
        let lf = f32(layer);
        let sc = (5.0 + lf * 3.0) / P.a.y;
        var st = vec2<f32>(i.uv.x * r.x / r.y, i.uv.y) * sc + vec2<f32>(sin(t * 0.4 + lf) * 0.2, -t * (0.25 + lf * 0.1));
        let id = floor(st);
        if (hash(id + lf * 5.3) < P.a.x) {
            let c = hash2(id) * 0.4 + 0.3;
            let p = rot2(fract(st) - c, t * (hash(id) - 0.5) * 4.0 * P.a.w) / 0.5 + 0.5;
            if (all(p >= vec2<f32>(0.0)) && all(p <= vec2<f32>(1.0))) {
                let s = textureSampleLevel(aux, smp, p, 0.0);
                outc = s + outc * (1.0 - s.a);
            }
        }
    }
    return outc;
}

// ---------------------------------------------------------------- broken video file
@fragment
fn fs_broken(i: V) -> @location(0) vec4<f32> {
    let r = res();
    let blk = floor(i.uv * r / max(P.a.z, 2.0));
    let tt = floor(P.misc.x * 8.0);
    var uv = i.uv;
    let h = hash(blk + vec2<f32>(tt, tt * 0.37));
    if (h < P.a.y * P.a.x) {
        uv = uv + (hash2(blk + tt) - 0.5) * 0.15 * P.a.x;
    }
    var c = S(uv);
    if (hash(blk.yy + tt) < P.a.y * 0.5) {
        c = vec4<f32>(c.g, c.b, c.r, c.a);
    }
    return c;
}

// ---------------------------------------------------------------- line scan
@fragment
fn fs_scan(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    let y = fract(P.misc.x * P.a.x * 0.25);
    let d = abs(i.uv.y - y);
    let line = smoothstep(P.a.y, 0.0, d);
    let lines = 0.06 * sin(i.uv.y * res().y * 1.5);
    let add = P.color.rgb * line * P.a.z * c.a;
    return vec4<f32>(c.rgb * (1.0 - lines) + add, c.a);
}

// ---------------------------------------------------------------- analog noise (VHS)
@fragment
fn fs_analog(i: V) -> @location(0) vec4<f32> {
    let t = P.misc.x;
    var uv = i.uv;
    uv.x = uv.x + sin(uv.y * 40.0 + t * 6.0) * 0.003 * P.a.y + (hash(vec2<f32>(floor(uv.y * 120.0), floor(t * 20.0))) - 0.5) * 0.004 * P.a.x;
    let sh = 0.004 * P.a.x;
    let cr = S(uv + vec2<f32>(sh, 0.0));
    let cg = S(uv);
    let cb = S(uv - vec2<f32>(sh, 0.0));
    var c = vec4<f32>(cr.r, cg.g, cb.b, max(max(cr.a, cg.a), cb.a));
    let n = (hash(uv * res() + t) - 0.5) * P.a.z * 0.3;
    c = vec4<f32>(c.rgb + n * c.a, c.a);
    return c;
}

// ---------------------------------------------------------------- digital glitch
@fragment
fn fs_digital(i: V) -> @location(0) vec4<f32> {
    let t = floor(P.misc.x * 12.0);
    let row = floor(i.uv.y * 24.0);
    var uv = i.uv;
    if (hash(vec2<f32>(row, t)) < P.a.y) {
        uv.x = uv.x + (hash(vec2<f32>(t, row)) - 0.5) * 0.2 * P.a.x;
    }
    let sh = P.a.z * 0.01;
    let cr = S(uv + vec2<f32>(sh, 0.0));
    let cg = S(uv);
    let cb = S(uv - vec2<f32>(sh, 0.0));
    return vec4<f32>(cr.r, cg.g, cb.b, max(max(cr.a, cg.a), cb.a));
}

// ---------------------------------------------------------------- cinematic frame
@fragment
fn fs_cinema(i: V) -> @location(0) vec4<f32> {
    var c = S(i.uv);
    let d = length(i.uv - 0.5) * 1.414;
    c = vec4<f32>(c.rgb * (1.0 - P.a.y * d * d), c.a);
    if (i.uv.y < P.a.x || i.uv.y > 1.0 - P.a.x) {
        return vec4<f32>(P.color.rgb * P.color.a, P.color.a) + c * (1.0 - P.color.a);
    }
    return c;
}

// ---------------------------------------------------------------- blur
@fragment
fn fs_blur(i: V) -> @location(0) vec4<f32> {
    let r = res();
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var n = 0; n < 32; n = n + 1) {
        let fn_ = f32(n);
        let ang = fn_ * 2.39996;
        let q = sqrt((fn_ + 0.5) / 32.0);
        let w = exp(-q * q * 2.0);
        acc = acc + S(i.uv + vec2<f32>(cos(ang), sin(ang)) * q * P.a.x / r) * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}

// ---------------------------------------------------------------- TV grain
@fragment
fn fs_grain(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    let n = hash(floor(i.uv * res() / max(P.a.y, 1.0)) + floor(P.misc.x * 24.0) * 1.37) - 0.5;
    return vec4<f32>(max(c.rgb + n * P.a.x * c.a, vec3<f32>(0.0)), c.a);
}

// ---------------------------------------------------------------- old TV / film
@fragment
fn fs_oldtv(i: V) -> @location(0) vec4<f32> {
    let t = P.misc.x;
    let c = S(i.uv);
    var rgb = unpremul(c);
    let sep = vec3<f32>(dot(rgb, vec3<f32>(0.393, 0.769, 0.189)), dot(rgb, vec3<f32>(0.349, 0.686, 0.168)), dot(rgb, vec3<f32>(0.272, 0.534, 0.131)));
    rgb = mix(rgb, sep, P.a.w);
    rgb = rgb * (1.0 + (hash(vec2<f32>(floor(t * 24.0), 1.0)) - 0.5) * 0.25 * P.a.z);
    let d = length(i.uv - 0.5) * 1.414;
    rgb = rgb * (1.0 - P.a.x * d * d);
    var a = c.a;
    // Vertical scratches.
    let sx = hash(vec2<f32>(floor(t * 6.0), 3.0));
    if (P.a.y > 0.0 && abs(i.uv.x - sx) < 0.0015 && hash(vec2<f32>(floor(t * 6.0), 9.0)) < P.a.y) {
        rgb = vec3<f32>(0.9);
        a = max(a, 0.6);
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * a, a);
}

// ---------------------------------------------------------------- contour outline (edges)
@fragment
fn fs_edge(i: V) -> @location(0) vec4<f32> {
    let px = P.a.x / res();
    var gx = 0.0;
    var gy = 0.0;
    var ax = 0.0;
    var ay = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let s = S(i.uv + vec2<f32>(f32(x), f32(y)) * px);
            let l = luma(s.rgb) + s.a;
            let wx = f32(x) * select(1.0, 2.0, y == 0);
            let wy = f32(y) * select(1.0, 2.0, x == 0);
            gx = gx + l * wx;
            gy = gy + l * wy;
            ax = ax + s.a * wx;
            ay = ay + s.a * wy;
        }
    }
    let e = smoothstep(P.a.y, P.a.y + 0.2, length(vec2<f32>(gx, gy)) * 0.25);
    let c = S(i.uv);
    let k = e * P.a.z * P.color.a;
    return over(c, P.color.rgb, k);
}

// ---------------------------------------------------------------- posterize
@fragment
fn fs_poster(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    let lv = max(P.a.x, 2.0);
    var rgb = pow(unpremul(c), vec3<f32>(P.a.y));
    rgb = floor(rgb * lv) / (lv - 1.0);
    rgb = pow(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / P.a.y));
    return vec4<f32>(rgb * c.a, c.a);
}

// ---------------------------------------------------------------- ASCII art
fn glyph(i: u32) -> u32 {
    var g = array<u32, 10>(
    0u,                // ' '
    134217728u,        // '.'
    4194432u,          // ':'
    458752u,           // '-'
    32537600u,         // '='
    139344004u,        // '+'
    22442496u,         // '*'
    368389098u,        // '#'
    588422434u,        // '%'
    474909246u         // '@'
    );
    return g[i];
}

@fragment
fn fs_ascii(i: V) -> @location(0) vec4<f32> {
    let r = res();
    let cs = max(P.a.x, 4.0);
    let cell = floor(i.uv * r / cs);
    let center = (cell + 0.5) * cs / r;
    let s = S(center);
    let l = luma(unpremul(s)) * s.a;
    let gi = u32(clamp(l * 9.99, 0.0, 9.0));
    let f = fract(i.uv * r / cs);
    let cx = u32(clamp(f.x * 5.0, 0.0, 4.0));
    let cy = u32(clamp(f.y * 6.0, 0.0, 5.0));
    let bit = f32((glyph(gi) >> (cy * 5u + cx)) & 1u);
    let col = mix(P.color.rgb, unpremul(s), P.a.y);
    let a = bit * max(s.a, 0.0);
    return vec4<f32>(col * a, a);
}

// ---------------------------------------------------------------- fish-eye
@fragment
fn fs_fisheye(i: V) -> @location(0) vec4<f32> {
    let p = i.uv - 0.5;
    let rr = dot(p, p);
    let uv = 0.5 + p * (1.0 + P.a.x * rr * 2.0) / (1.0 + P.a.x * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    return S(uv);
}
