// Post-processing effects with VTube Studio's effect set. Colours are premultiplied alpha.
// Parameter offsets (P.v) come from vfx_consts.wgsl, generated from VTS EffectConfigs.cs.

struct Params {
    v: array<vec4<f32>, 30>,
    misc: vec4<f32>,  // x = time, y = mic volume 0..1, z/w = head X/Y (-1..1)
    misc2: vec4<f32>, // x/y = mouse uv, z = mouse down, w = dt
    misc3: vec4<f32>, // x = refresh (frame-hold effects), y = frame, z = aspect, w = random
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> P: Params;
@group(1) @binding(0) var bgt: texture_2d<f32>;
@group(1) @binding(1) var mdl: texture_2d<f32>;
@group(1) @binding(2) var hst: texture_2d<f32>;
@group(1) @binding(3) var img0: texture_2d<f32>;
@group(1) @binding(4) var img1: texture_2d<f32>;
@group(1) @binding(5) var img2: texture_2d<f32>;

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

fn p(i: u32) -> f32 {
    return P.v[i >> 2u][i & 3u];
}
fn pc(i: u32) -> vec4<f32> {
    return vec4<f32>(p(i), p(i + 1u), p(i + 2u), p(i + 3u));
}
fn T() -> f32 {
    return P.misc.x;
}
fn S(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src, smp, uv, 0.0);
}
fn M(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(mdl, smp, uv, 0.0).a;
}
fn H(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(hst, smp, uv, 0.0);
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
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), u.x), mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
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
// Additive light (keeps alpha so it shows on transparent output too).
fn add_light(dst: vec4<f32>, rgb: vec3<f32>) -> vec4<f32> {
    let a = clamp(max(dst.a, luma(rgb)), 0.0, 1.0);
    return vec4<f32>(dst.rgb + rgb, a);
}
fn rot2(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}
fn blur_src(uv: vec2<f32>, r: f32) -> vec4<f32> {
    let px = r / res();
    var acc = vec4<f32>(0.0);
    for (var k = 0; k < 12; k = k + 1) {
        let a = f32(k) * 2.39996;
        let d = sqrt(f32(k) / 12.0 + 0.04);
        acc = acc + S(uv + vec2<f32>(cos(a), sin(a)) * d * px);
    }
    return acc / 12.0;
}
fn blur_tex_bg(uv: vec2<f32>, r: f32) -> vec4<f32> {
    let px = r / res();
    var acc = vec4<f32>(0.0);
    for (var k = 0; k < 12; k = k + 1) {
        let a = f32(k) * 2.39996;
        let d = sqrt(f32(k) / 12.0 + 0.04);
        acc = acc + textureSampleLevel(bgt, smp, uv + vec2<f32>(cos(a), sin(a)) * d * px, 0.0);
    }
    return acc / 12.0;
}
fn blur_alpha(uv: vec2<f32>, r: f32) -> f32 {
    let px = r / res();
    var acc = 0.0;
    for (var k = 0; k < 12; k = k + 1) {
        let a = f32(k) * 2.39996;
        let d = sqrt(f32(k) / 12.0 + 0.04);
        acc = acc + M(uv + vec2<f32>(cos(a), sin(a)) * d * px);
    }
    return acc / 12.0;
}
fn star(p: vec2<f32>) -> f32 {
    let a = atan2(p.y, p.x);
    let r = length(p);
    let k = 0.25 + 0.12 * cos(a * 4.0);
    return smoothstep(k, k - 0.05, r);
}
fn heart(p0: vec2<f32>) -> f32 {
    let p = vec2<f32>(p0.x, -p0.y + 0.05) * 2.6;
    let q = p.x * p.x + p.y * p.y - 1.0;
    let v = q * q * q - p.x * p.x * p.y * p.y * p.y;
    return smoothstep(0.02, -0.02, v);
}
fn rgb2hsv(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let pp = mix(vec4<f32>(c.bg, K.wz), vec4<f32>(c.gb, K.xy), step(c.b, c.g));
    let q = mix(vec4<f32>(pp.xyw, c.r), vec4<f32>(c.r, pp.yzx), step(pp.x, c.r));
    let d = q.x - min(q.w, q.y);
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + 1e-10)), d / (q.x + 1e-10), q.x);
}
fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let pp = abs(fract(c.xxx + vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
    return c.z * mix(vec3<f32>(1.0), clamp(pp - 1.0, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}
fn mixc(a: vec4<f32>, b: vec4<f32>, k: f32) -> vec4<f32> {
    return mix(a, b, clamp(k, 0.0, 1.0));
}

// ---------------------------------------------------------------- compose (+ Background shift)
@fragment
fn fs_compose(i: V) -> @location(0) vec4<f32> {
    // src = model layer; bgt = background. P = BackgroundShift values.
    let s = p(BACKGROUND_SHIFT_STRENGTH);
    let m = S(i.uv);
    var buv = i.uv;
    if (s > 0.0) {
        let spd = p(BACKGROUND_SHIFT_RANDOM_MOVEMENT_SPEED);
        let t = T() * spd;
        let zoom = 1.0 / (1.0 + (p(BACKGROUND_SHIFT_ZOOM_IN) + P.misc.y * p(BACKGROUND_SHIFT_MIC_ZOOM_IN)) * 0.5 * s);
        let rnd = vec2<f32>(noise(vec2<f32>(t, 1.3)) - 0.5, noise(vec2<f32>(4.1, t)) - 0.5) * vec2<f32>(p(BACKGROUND_SHIFT_RANDOM_MOVEMENT_X), p(BACKGROUND_SHIFT_RANDOM_MOVEMENT_Y)) * 0.08;
        let trk = vec2<f32>(P.misc.z * p(BACKGROUND_SHIFT_TRACKING_X), -P.misc.w * p(BACKGROUND_SHIFT_TRACKING_Y)) * 0.04;
        let ang = (noise(vec2<f32>(t * 0.7, 9.0)) - 0.5) * p(BACKGROUND_SHIFT_RANDOM_MOVEMENT_ROTATION) * 0.2 * s;
        var q = (i.uv - 0.5) * vec2<f32>(P.misc3.z, 1.0);
        q = rot2(q, ang) / vec2<f32>(P.misc3.z, 1.0);
        buv = q * zoom + 0.5 + (rnd + trk) * s;
    }
    var b = textureSampleLevel(bgt, smp, buv, 0.0);
    if (s > 0.0 && p(BACKGROUND_SHIFT_BLUR_MIX_BACK) > 0.0) {
        let bb = blur_tex_bg(buv, 2.0 + p(BACKGROUND_SHIFT_BLUR_MAIN_BACK) * 40.0) * p(BACKGROUND_SHIFT_BLUR_BRIGHTNESS_BACK);
        b = mixc(b, bb, p(BACKGROUND_SHIFT_BLUR_MIX_BACK) * s);
    }
    var o = m + b * (1.0 - m.a);
    if (s > 0.0 && p(BACKGROUND_SHIFT_BLUR_MIX_FRONT) > 0.0) {
        let fb = blur_tex_bg(buv, 2.0 + p(BACKGROUND_SHIFT_BLUR_MAIN_FRONT) * 40.0) * p(BACKGROUND_SHIFT_BLUR_BRIGHTNESS_FRONT);
        o = mixc(o, vec4<f32>(fb.rgb, max(o.a, fb.a)), p(BACKGROUND_SHIFT_BLUR_MIX_FRONT) * s * 0.6);
    }
    return o;
}

@fragment
fn fs_copy(i: V) -> @location(0) vec4<f32> {
    return S(i.uv);
}

@fragment
fn fs_background_shift(i: V) -> @location(0) vec4<f32> {
    return S(i.uv); // done in the compose pass
}

@fragment
fn fs_model_glitch(i: V) -> @location(0) vec4<f32> {
    return S(i.uv); // applied to the model vertices on the CPU
}

// ---------------------------------------------------------------- Color grading
@fragment
fn fs_color_grading(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    var rgb = unpremul(c);
    var hsv = rgb2hsv(rgb);
    hsv.x = fract(hsv.x + p(COLOR_GRADING_HUE_SHIFT) / 360.0);
    hsv.y = clamp(hsv.y * (1.0 + p(COLOR_GRADING_SATURATION) / 100.0), 0.0, 1.0);
    rgb = hsv2rgb(hsv);
    rgb = rgb + vec3<f32>(p(COLOR_GRADING_BRIGHTNESS) / 200.0);
    rgb = (rgb - 0.5) * (1.0 + p(COLOR_GRADING_CONTRAST) / 100.0) + 0.5;
    rgb = rgb * pc(COLOR_GRADING_COLOR_FILTER).rgb;
    let tmp = p(COLOR_GRADING_WHITEBALANCE_TEMPERATURE) / 100.0;
    let tnt = p(COLOR_GRADING_WHITEBALANCE_TINT) / 100.0;
    rgb = rgb * vec3<f32>(1.0 + tmp * 0.25, 1.0 - tnt * 0.2, 1.0 - tmp * 0.25);
    rgb = mix(rgb, 1.0 - rgb, p(COLOR_GRADING_INVERT));
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return mixc(c, vec4<f32>(rgb * c.a, c.a), p(COLOR_GRADING_STRENGTH));
}

// ---------------------------------------------------------------- Weather
@fragment
fn fs_weather_effects(i: V) -> @location(0) vec4<f32> {
    var o = S(i.uv);
    let r = res();
    let aspect = r.x / r.y;
    let t = T();
    let ma = M(i.uv);
    var rain = 0.0;
    var snow = 0.0;
    for (var layer = 0; layer < 3; layer = layer + 1) {
        let lf = f32(layer);
        let sc = vec2<f32>(60.0 + lf * 30.0, 4.0);
        let st = vec2<f32>(i.uv.x * aspect * sc.x + 0.2 * i.uv.y * 8.0, i.uv.y * sc.y - t * (2.5 + lf));
        let id = floor(st);
        let f = fract(st);
        if (hash(id + lf * 13.0) < p(WEATHER_EFFECTS_RAIN_STRENGTH)) {
            let x0 = 0.3 + 0.4 * hash(id * 1.7);
            rain = max(rain, smoothstep(0.08, 0.0, abs(f.x - x0)) * smoothstep(0.0, 0.6, f.y) * (0.55 - lf * 0.12));
        }
        let ss = (12.0 + lf * 10.0);
        let st2 = vec2<f32>(i.uv.x * aspect, i.uv.y) * ss + vec2<f32>(0.2 * t * 0.3 + sin(t * 0.7 + lf) * 0.3, -t * (0.4 + lf * 0.15));
        let id2 = floor(st2);
        if (hash(id2 + lf * 7.0) < p(WEATHER_EFFECTS_SNOW_STRENGTH)) {
            let c = hash2(id2) * 0.7 + 0.15;
            snow = max(snow, smoothstep(0.12, 0.04, length(fract(st2) - c)) * (0.9 - lf * 0.2));
        }
    }
    if (p(WEATHER_EFFECTS_RAIN_IN_FRONT) < 0.5) {
        rain = rain * (1.0 - ma);
    }
    if (p(WEATHER_EFFECTS_SNOW_IN_FRONT) < 0.5) {
        snow = snow * (1.0 - ma);
    }
    o = over(o, vec3<f32>(0.75, 0.82, 0.95), rain * 0.8);
    o = over(o, vec3<f32>(1.0), snow);
    return o;
}

// ---------------------------------------------------------------- Bloom
@fragment
fn fs_bloom(i: V) -> @location(0) vec4<f32> {
    let s = clamp(p(BLOOM_STRENGTH) * (1.0 + P.misc.y * p(BLOOM_MIC_INCREASES_BLOOM) * 2.0), 0.0, 2.0);
    var c = S(i.uv);
    let ma = M(i.uv);
    c = vec4<f32>(c.rgb * (1.0 - ma * p(BLOOM_MODEL_COLOR_DARKEN) * 0.6 * s) * (1.0 - (1.0 - ma) * p(BLOOM_BACKGROUND_COLOR_DARKEN) * 0.6 * s), c.a);
    let q = 4.0 + p(BLOOM_QUALITY) * 2.0;
    let px = 1.0 / res();
    var acc = vec3<f32>(0.0);
    let th = p(BLOOM_MAIN_THRESHOLD);
    for (var k = 0; k < 24; k = k + 1) {
        let a = f32(k) * 2.39996;
        let d = sqrt(f32(k) / 24.0) * q * 6.0;
        let sm = S(i.uv + vec2<f32>(cos(a), sin(a)) * d * px);
        let l = luma(sm.rgb);
        acc = acc + sm.rgb * smoothstep(th, th + 0.2, l);
    }
    var glow = acc / 24.0 * pc(BLOOM_MAIN_COLOR_TINT).rgb * 2.0 * p(BLOOM_MAIN_INTENSITY) * 3.0;
    // Streaks (anamorphic light lines).
    let dir = select(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), p(BLOOM_STREAK_VERTICAL) > 0.5);
    var st = vec3<f32>(0.0);
    let sth = p(BLOOM_STREAK_THRESHOLD);
    for (var k = -12; k <= 12; k = k + 1) {
        let sm = S(i.uv + dir * f32(k) * 6.0 * px);
        st = st + sm.rgb * smoothstep(sth, sth + 0.2, luma(sm.rgb)) * (1.0 - abs(f32(k)) / 13.0);
    }
    glow = glow + st / 12.0 * pc(BLOOM_STREAK_COLOR_TINT).rgb * 2.0 * p(BLOOM_STREAK_INTENSITY) * 3.0;
    return add_light(c, glow * s);
}

// ---------------------------------------------------------------- Backlight
@fragment
fn fs_backlight(i: V) -> @location(0) vec4<f32> {
    let s = p(BACKLIGHT_STRENGTH);
    var c = S(i.uv);
    let ma = M(i.uv);
    let px = 1.0 / res();
    // Darkening and blurred background overlays.
    let bgb = blur_tex_bg(i.uv, 24.0);
    c = vec4<f32>(c.rgb * (1.0 - ma * p(BACKLIGHT_DARKEN_MODEL) * s) * (1.0 - (1.0 - ma) * p(BACKLIGHT_DARKEN_BG) * s), c.a);
    c = mixc(c, vec4<f32>(bgb.rgb * c.a, c.a), ma * p(BACKLIGHT_BG_BLUR_OVER_MODEL) * s * 0.2);
    c = mixc(c, vec4<f32>(bgb.rgb, max(c.a, bgb.a)), (1.0 - ma) * p(BACKLIGHT_BG_BLUR_OVER_BG) * s);
    // Shadow behind the model.
    let so = vec2<f32>(p(BACKLIGHT_SHADOW_OFFSET_X), -p(BACKLIGHT_SHADOW_OFFSET_Y)) * 4.0 * px;
    let sh = pc(BACKLIGHT_SHADOW_MAIN_COLOR);
    if (length(so) > 0.0) {
        let sa = blur_alpha(i.uv - so, 6.0) * (1.0 - ma) * sh.a * s;
        c = over(c, sh.rgb, sa);
    }
    // Rim light: model pixels near the silhouette edge, stronger toward the light direction.
    let soft = 2.0 + p(BACKLIGHT_BACKLIGHT_SOFTNESS) * 10.0;
    let inner = blur_alpha(i.uv, soft);
    let edge = clamp(ma - inner, 0.0, 1.0) * 1.5;
    let ang = radians(p(BACKLIGHT_BACKLIGHT_DIRECTION));
    let ld = vec2<f32>(cos(ang), -sin(ang)) * soft * px;
    let dirA = clamp(ma - M(i.uv + ld), 0.0, 1.0);
    let dirB = clamp(ma - M(i.uv - ld), 0.0, 1.0) * p(BACKLIGHT_BACKLIGHT_BOTH_DIRECTIONS);
    let rim = edge * p(BACKLIGHT_STRENGTH_NORMAL) + (dirA + dirB) * p(BACKLIGHT_STRENGTH_DIRECTIONAL);
    let bgcol = blur_tex_bg(i.uv, 40.0).rgb;
    let tint = mix(pc(BACKLIGHT_BACKLIGHT_COLOR_TINT).rgb * 2.0, bgcol * 2.0, p(BACKLIGHT_BACKLIGHT_COLOR_FROM_BG));
    c = add_light(c, min(tint * rim * s, vec3<f32>(p(BACKLIGHT_BRIGHTNESS_LIMIT) * 2.0)) * ma);
    // Outline with animated stripes outside the silhouette.
    let osz = p(BACKLIGHT_OUTLINE_SIZE);
    if (osz > 0.0) {
        let grow = blur_alpha(i.uv, 2.0 + osz * 24.0);
        let o = clamp(grow * 3.0, 0.0, 1.0) * (1.0 - ma);
        let cnt = 4.0 + p(BACKLIGHT_OUTLINE_STRIPE_COUNT) * 60.0;
        let ph = (i.uv.x + i.uv.y * (1.0 + p(BACKLIGHT_OUTLINE_STRIPE_CURVE) * 4.0)) * cnt + T() * p(BACKLIGHT_OUTLINE_STRIPE_SPEED) * 6.0;
        let stripe = step(0.5, fract(ph));
        let col = mix(pc(BACKLIGHT_OUTLINE_COLOR_MAIN), pc(BACKLIGHT_OUTLINE_COLOR_STRIPES), stripe);
        c = over(c, col.rgb, o * col.a * s);
    }
    return c;
}

// ---------------------------------------------------------------- Custom particles
fn particle_layer(uv: vec2<f32>, t: f32, amount: f32, size: f32, speed: vec2<f32>, seed: f32, kind: i32) -> vec2<f32> {
    // Returns (coverage, colour mix) for a grid of particles.
    let aspect = P.misc3.z;
    let cells = mix(14.0, 3.0, size);
    let q = vec2<f32>(uv.x * aspect, uv.y) * cells + speed * t;
    let id = floor(q);
    let f = fract(q) - 0.5;
    let h = hash(id + seed);
    if (h > amount) {
        return vec2<f32>(0.0);
    }
    let o = (hash2(id * 1.3 + seed) - 0.5) * 0.5;
    let d = f - o;
    let tw = 0.5 + 0.5 * sin(t * (2.0 + h * 4.0) + h * 30.0);
    var k = 0.0;
    if (kind == 0) {
        k = star(rot2(d * 2.2, t * 0.5 + h * 6.0)) * tw;
    } else if (kind == 1) {
        k = smoothstep(0.18, 0.0, length(d)) * (0.6 + 0.4 * tw);
    } else if (kind == 2) {
        k = smoothstep(0.5, 0.0, length(d)) * 0.35 * (0.6 + 0.4 * noise(id + t * 0.2));
    } else if (kind == 3) {
        k = smoothstep(0.45, 0.25, length(d)) * 0.5;
    } else {
        k = heart(rot2(d * 1.4, sin(t + h * 6.0) * 0.3));
    }
    return vec2<f32>(k, hash(id + seed + 3.0));
}

fn sprite_layer(tex: i32, uv: vec2<f32>, t: f32, amount: f32, size: f32, rot0: f32, rspeed: f32, fill: f32, seed: f32) -> vec4<f32> {
    let aspect = P.misc3.z;
    let cells = mix(12.0, 2.5, size);
    var q = vec2<f32>(uv.x * aspect, uv.y) * cells + vec2<f32>(0.0, -t * 0.3);
    let id = floor(q);
    let f = fract(q) - 0.5;
    let h = hash(id + seed);
    // "Fill to center": fewer particles toward the screen centre when low.
    let centre = 1.0 - length(uv - 0.5) * 2.0;
    if (h > amount * mix(1.0 - centre * 0.8, 1.0, fill)) {
        return vec4<f32>(0.0);
    }
    let d = rot2(f, radians(rot0) * (h - 0.5) * 2.0 + t * rspeed * 3.0 * (h - 0.5)) * 1.3 + 0.5;
    if (any(d < vec2<f32>(0.0)) || any(d > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    if (tex == 0) {
        return textureSampleLevel(img0, smp, d, 0.0);
    }
    return textureSampleLevel(img1, smp, d, 0.0);
}

@fragment
fn fs_custom_particles(i: V) -> @location(0) vec4<f32> {
    var o = S(i.uv);
    let s = p(CUSTOM_PARTICLES_STRENGTH);
    let t = T();
    let ma = M(i.uv);
    let head = vec2<f32>(P.misc.z, -P.misc.w) * 0.05;
    let uv = i.uv + head * p(CUSTOM_PARTICLES_BASE_MOVE_WITH_HEAD);
    // Built-in kinds: sparkle, floaty, cloud, sphere, hearts.
    for (var k = 0; k < 5; k = k + 1) {
        let base = u32(2 + k * 10);
        let amount = p(base);
        if (amount <= 0.0) {
            continue;
        }
        let size = p(base + 1u);
        let spd = select(vec2<f32>(0.05, -0.2), vec2<f32>(0.02, -0.05), k == 2 || k == 3);
        let r = particle_layer(uv, t, amount * 0.5, size, spd, f32(k) * 11.0, k);
        let col = mix(pc(base + 2u), pc(base + 6u), r.y);
        o = over(o, col.rgb, r.x * col.a * s);
    }
    // Custom textures 1 / 2.
    for (var k = 0; k < 2; k = k + 1) {
        let b = select(CUSTOM_PARTICLES_CUSTOM1_COLOR_A, CUSTOM_PARTICLES_CUSTOM2_COLOR_A, k == 1);
        let amount = p(b + 12u);
        if (amount <= 0.0) {
            continue;
        }
        let mv = vec2<f32>(P.misc.z, -P.misc.w) * 0.05 * p(b + 10u);
        let mic = 1.0 + P.misc.y * p(b + 16u) * 3.0;
        let sp = sprite_layer(k, i.uv + mv, t * mic, amount * 0.5, p(b + 11u), p(b + 14u), p(b + 15u), p(b + 13u), f32(k) * 7.0 + 50.0);
        let col = mix(pc(b), pc(b + 4u), hash(floor(i.uv * 9.0)));
        var a = sp.a * col.a * s;
        if (p(b + 9u) > 0.5) {
            a = a * (1.0 - ma);
        }
        if (p(b + 8u) > 0.5) {
            o = add_light(o, sp.rgb * col.rgb * a);
        } else {
            o = vec4<f32>(sp.rgb * col.rgb * col.a * s, a) + o * (1.0 - a);
        }
    }
    return o;
}

// ---------------------------------------------------------------- Simple overlay
@fragment
fn fs_simple_overlay(i: V) -> @location(0) vec4<f32> {
    let o = S(i.uv);
    let s = p(SIMPLE_OVERLAY_STRENGTH);
    let t = T() * p(SIMPLE_OVERLAY_RANDOM_MOVEMENT_SPEED);
    let zoom = 1.0 / (1.0 + p(SIMPLE_OVERLAY_ZOOM_IN));
    let rnd = vec2<f32>(noise(vec2<f32>(t, 2.0)) - 0.5, noise(vec2<f32>(5.0, t)) - 0.5) * vec2<f32>(p(SIMPLE_OVERLAY_RANDOM_MOVEMENT_X), p(SIMPLE_OVERLAY_RANDOM_MOVEMENT_Y)) * 0.1;
    let trk = vec2<f32>(P.misc.z * p(SIMPLE_OVERLAY_TRACKING_X), -P.misc.w * p(SIMPLE_OVERLAY_TRACKING_Y)) * 0.05;
    let ang = (noise(vec2<f32>(t, 7.0)) - 0.5) * p(SIMPLE_OVERLAY_RANDOM_MOVEMENT_ROTATION) * 0.3;
    var q = (i.uv - 0.5) * vec2<f32>(P.misc3.z, 1.0);
    q = rot2(q, ang) / vec2<f32>(P.misc3.z, 1.0);
    let uv = q * zoom + 0.5 + rnd + trk;
    let ov = textureSampleLevel(img0, smp, uv, 0.0) * pc(SIMPLE_OVERLAY_TINT_COLOR);
    let a = ov.a * s;
    return vec4<f32>(ov.rgb * s, a) + o * (1.0 - a);
}

// ---------------------------------------------------------------- Vignette
@fragment
fn fs_vignette(i: V) -> @location(0) vec4<f32> {
    let o = S(i.uv);
    let c = vec2<f32>(0.5 + p(VIGNETTE_CENTER_X) * 0.5, 0.5 - p(VIGNETTE_CENTER_Y) * 0.5);
    var d = (i.uv - c) * 2.0;
    if (p(VIGNETTE_CIRCULAR) > 0.5) {
        d.x = d.x * P.misc3.z;
    }
    let rnd = p(VIGNETTE_ROUNDNESS);
    let r = mix(max(abs(d.x), abs(d.y)), length(d), rnd);
    let k = smoothstep(1.0 - p(VIGNETTE_SMOOTHNESS) * 0.9 - 0.05, 1.3, r) * p(VIGNETTE_STRENGTH);
    let col = pc(VIGNETTE_COLOR);
    return over(o, col.rgb, k * col.a);
}

// ---------------------------------------------------------------- Chromatic aberration
@fragment
fn fs_chromatic_aberration(i: V) -> @location(0) vec4<f32> {
    let s = p(CHROMATIC_ABERRATION_STRENGTH);
    let d = (i.uv - 0.5) * s * 0.03;
    var r = S(i.uv + d);
    var g = S(i.uv);
    var b = S(i.uv - d);
    if (p(CHROMATIC_ABERRATION_BLUR_EDGES) > 0.5) {
        let e = length(i.uv - 0.5) * s * 8.0;
        r = mixc(r, blur_src(i.uv + d, e), 0.5);
        b = mixc(b, blur_src(i.uv - d, e), 0.5);
    }
    return vec4<f32>(r.r, g.g, b.b, max(max(r.a, g.a), b.a));
}

// ---------------------------------------------------------------- Old film
@fragment
fn fs_old_film(i: V) -> @location(0) vec4<f32> {
    let s = p(OLD_FILM_STRENGTH);
    var c = select(H(i.uv), S(i.uv), P.misc3.x > 0.5);
    if (P.misc3.y < 3.0) {
        c = S(i.uv);
    }
    let rgb = unpremul(c);
    var g = luma(rgb);
    g = (g - 0.5) * p(OLD_FILM_FILM_CONTRAST) + 0.5;
    var sep = vec3<f32>(g * 1.07, g * 0.93, g * 0.75);
    let fr = floor(T() * max(p(OLD_FILM_FILM_FPS), 1.0));
    // Burn: flickering vignette.
    let v = length(i.uv - 0.5);
    sep = sep * (1.0 - smoothstep(0.3, 0.8, v) * p(OLD_FILM_FILM_BURN) * 0.4 * (0.8 + 0.2 * hash(vec2<f32>(fr, 1.0))));
    // Dirt specks and scratches.
    let ds = max(p(OLD_FILM_FILM_SCENE_CUT), 0.05);
    let cell = floor(i.uv * res() / (6.0 * ds));
    if (hash(cell + fr) > 0.9985) {
        sep = sep * 0.2;
    }
    let sx = hash(vec2<f32>(fr, 3.0));
    if (abs(i.uv.x - sx) < 0.0015 && hash(vec2<f32>(fr, 5.0)) > 0.5) {
        sep = sep * 0.6;
    }
    let out = vec4<f32>(clamp(sep, vec3<f32>(0.0), vec3<f32>(1.0)) * c.a, c.a);
    return mixc(S(i.uv), out, s);
}

// ---------------------------------------------------------------- Low FPS
@fragment
fn fs_lowfps(i: V) -> @location(0) vec4<f32> {
    let s = p(LOWFPS_STRENGTH);
    let cur = S(i.uv);
    var held = select(H(i.uv), cur, P.misc3.x > 0.5 || P.misc3.y < 3.0);
    // Screen tearing: a band that already shows the new frame.
    let tear = p(LOWFPS_SCREEN_TEARING);
    if (tear > 0.0) {
        let y0 = hash(vec2<f32>(floor(T() * 7.0), 2.0));
        if (abs(i.uv.y - y0) < tear * 0.15) {
            held = S(i.uv + vec2<f32>(tear * 0.01, 0.0));
        }
    }
    return mixc(cur, held, s);
}

// ---------------------------------------------------------------- Datamosh
@fragment
fn fs_datamosh(i: V) -> @location(0) vec4<f32> {
    let s = p(DATAMOSH_STRENGTH);
    let cur = S(i.uv);
    let reset = p(DATAMOSH_RESET_AFTER_SECS);
    if (P.misc3.y < 3.0 || (reset > 0.0 && fract(T() / reset) < P.misc2.w / reset)) {
        return cur;
    }
    let bs = max(p(DATAMOSH_SIZE), 1.0) / res();
    let blk = floor(i.uv / bs);
    let n = hash(blk + floor(T() * 3.0));
    // Pseudo motion vectors from the luminance difference between frames.
    let prev = H(i.uv);
    let diff = luma(cur.rgb) - luma(prev.rgb);
    let vel = (hash2(blk) - 0.5) * p(DATAMOSH_VELOCITY_SCALE) * 0.02 + vec2<f32>(diff) * 0.01;
    var moshed = H(i.uv - vel);
    if (n < p(DATAMOSH_ENTROPY) * 0.2) {
        moshed = cur;
    }
    let noise_c = (hash(i.uv * res() + T()) - 0.5) * p(DATAMOSH_NOISE_CONTRAST) * 0.1;
    moshed = vec4<f32>(moshed.rgb + vec3<f32>(noise_c) * moshed.a, moshed.a);
    let out = mixc(moshed, cur, (1.0 - p(DATAMOSH_DIFFUSION)) * 0.15);
    return mixc(cur, out, s);
}

// ---------------------------------------------------------------- Line scanner
@fragment
fn fs_line_scanner(i: V) -> @location(0) vec4<f32> {
    let s = p(LINE_SCANNER_STRENGTH);
    let cur = S(i.uv);
    if (P.misc3.y < 3.0) {
        return cur;
    }
    let dirn = i32(p(LINE_SCANNER_DIRECTION));
    let total = max(p(LINE_SCANNER_SCAN_STEP_TOTAL), 16.0);
    let step_px = max(p(LINE_SCANNER_SCAN_STEP_SIZE), 1.0);
    let dur = total / (step_px * 60.0);
    let cyc = dur + p(LINE_SCANNER_SCAN_LINE_WAIT_BETWEEN_SCANS_SECS);
    let prog = fract(T() / max(cyc, 0.01)) * cyc / max(dur, 0.001);
    var x = i.uv.y;
    if (dirn == 2) { x = 1.0 - i.uv.y; }
    if (dirn == 3) { x = i.uv.x; }
    if (dirn == 4) { x = 1.0 - i.uv.x; }
    // Behind the scan line the old image stays frozen; at the line the live image is copied.
    let w = step_px / select(res().y, res().x, dirn >= 3) * 2.0;
    var o = H(i.uv);
    if (prog <= 1.0 && abs(x - prog) < w) {
        o = cur;
    }
    if (prog > 1.0) {
        o = H(i.uv);
    }
    let lc = pc(LINE_SCANNER_SCAN_LINE_COLOR);
    let lw = p(LINE_SCANNER_SCAN_LINE_SIZE) * 0.02;
    if (prog <= 1.0 && abs(x - prog) < lw) {
        o = over(o, lc.rgb, lc.a);
    }
    return mixc(cur, o, s);
}

// ---------------------------------------------------------------- Particle shower
@fragment
fn fs_particle_shower(i: V) -> @location(0) vec4<f32> {
    var o = S(i.uv);
    let s = p(PARTICLE_SHOWER_STRENGTH);
    let ma = M(i.uv);
    let aspect = P.misc3.z;
    for (var k = 0; k < 3; k = k + 1) {
        let spd = p(PARTICLE_SHOWER_SPEED1 + u32(k));
        let cells = 7.0 + f32(k) * 2.0;
        let q = vec2<f32>(i.uv.x * aspect, i.uv.y) * cells + vec2<f32>(0.0, -T() * (0.3 + spd * 3.0));
        let id = floor(q);
        let f = fract(q);
        if (hash(id + f32(k) * 9.0) > 0.35) {
            continue;
        }
        let d = rot2(f - 0.5, (hash(id) - 0.5) * 2.0 + T() * (hash(id + 1.0) - 0.5)) * 1.4 + 0.5;
        if (any(d < vec2<f32>(0.0)) || any(d > vec2<f32>(1.0))) {
            continue;
        }
        var c = vec4<f32>(0.0);
        if (k == 0) { c = textureSampleLevel(img0, smp, d, 0.0); }
        if (k == 1) { c = textureSampleLevel(img1, smp, d, 0.0); }
        if (k == 2) { c = textureSampleLevel(img2, smp, d, 0.0); }
        var a = c.a * s;
        if (p(PARTICLE_SHOWER_IN_BACK1 + u32(k)) > 0.5) {
            a = a * (1.0 - ma);
        }
        o = vec4<f32>(c.rgb * s, a) * select(1.0, 1.0 - ma, p(PARTICLE_SHOWER_IN_BACK1 + u32(k)) > 0.5) + o * (1.0 - a);
    }
    return o;
}

// ---------------------------------------------------------------- Analog glitch
@fragment
fn fs_analog_glitch(i: V) -> @location(0) vec4<f32> {
    let s = p(ANALOG_GLITCH_STRENGTH) * (1.0 + P.misc.y * p(ANALOG_GLITCH_MIC_EFFECT) * 3.0);
    let t = T();
    var uv = i.uv;
    let jit = (hash(vec2<f32>(uv.y * 400.0, t)) - 0.5) * p(ANALOG_GLITCH_SCANLINE_JITTER) * 0.04 * step(1.0 - p(ANALOG_GLITCH_SCANLINE_JITTER), hash(vec2<f32>(uv.y * 60.0, t * 3.0)));
    uv.x = uv.x + jit * s;
    uv.y = fract(uv.y + fract(t * p(ANALOG_GLITCH_VERTICAL_JUMP) * 2.0) * s * p(ANALOG_GLITCH_VERTICAL_JUMP));
    uv.x = uv.x + (hash(vec2<f32>(t, 2.0)) - 0.5) * p(ANALOG_GLITCH_HORIZONTAL_SHAKE) * 0.05 * s;
    let drift = sin(uv.y * 8.0 + t * 5.0) * p(ANALOG_GLITCH_COLOR_DRIFT) * 0.02 * s;
    let a = S(uv);
    let b = S(uv + vec2<f32>(drift, 0.0));
    return mixc(S(i.uv), vec4<f32>(a.r, b.g, a.b, max(a.a, b.a)), clamp(s, 0.0, 1.0) + 0.0001);
}

// ---------------------------------------------------------------- Digital glitch
@fragment
fn fs_digital_glitch(i: V) -> @location(0) vec4<f32> {
    let s = clamp(p(DIGITAL_GLITCH_STRENGTH) * (1.0 + P.misc.y * p(DIGITAL_GLITCH_MIC_EFFECT) * 3.0), 0.0, 1.0);
    let t = floor(T() * 12.0);
    let blk = floor(i.uv * vec2<f32>(16.0, 32.0));
    var uv = i.uv;
    if (hash(blk + t) < s * 0.25) {
        uv = uv + (hash2(blk * 1.7 + t) - 0.5) * 0.1 * s;
    }
    let cs = p(DIGITAL_GLITCH_COLORSHIFT) * 0.02 * s;
    let r = S(uv + vec2<f32>(cs, 0.0));
    let g = S(uv);
    let b = S(uv - vec2<f32>(cs, 0.0));
    var c = vec4<f32>(r.r, g.g, b.b, max(max(r.a, g.a), b.a));
    if (hash(vec2<f32>(blk.y, t)) < s * 0.05) {
        c = vec4<f32>(c.gbr, c.a);
    }
    return c;
}

// ---------------------------------------------------------------- Letterbox
@fragment
fn fs_letterbox(i: V) -> @location(0) vec4<f32> {
    let s = p(LETTERBOX_STRENGTH);
    let z = 1.0 / (1.0 + p(LETTERBOX_ZOOM) * 0.5 * s);
    let o = S((i.uv - 0.5) * z + 0.5);
    let by = p(LETTERBOX_PROGRESS_Y) * 0.5 * s;
    let bx = p(LETTERBOX_PROGRESS_X) * 0.5 * s;
    let inside = i.uv.y > by && i.uv.y < 1.0 - by && i.uv.x > bx && i.uv.x < 1.0 - bx;
    let col = pc(LETTERBOX_COLOR);
    return select(over(o, col.rgb, col.a), o, inside);
}

// ---------------------------------------------------------------- Foggy window
@fragment
fn fs_foggy_window(i: V) -> @location(0) vec4<f32> {
    let s = p(FOGGY_WINDOW_STRENGTH);
    let cur = S(i.uv);
    let fogged = blur_src(i.uv, 18.0);
    let tint = pc(FOGGY_WINDOW_FOG_TINT);
    var fog = p(FOGGY_WINDOW_FOG_STRENGTH);
    // Raindrops clear the fog.
    let aspect = P.misc3.z;
    let q = vec2<f32>(i.uv.x * aspect, i.uv.y) * mix(30.0, 8.0, p(FOGGY_WINDOW_RAINDROP_SIZE)) + vec2<f32>(0.0, -T() * p(FOGGY_WINDOW_RAINDROP_SPEED) * 2.0);
    let id = floor(q);
    let f = fract(q) - 0.5 - (hash2(id) - 0.5) * 0.5;
    var drop = 0.0;
    if (hash(id) < p(FOGGY_WINDOW_RAINDROP_VISIBILITY) * 0.5) {
        drop = smoothstep(0.25, 0.1, length(f * vec2<f32>(1.0, 0.7)));
    }
    // Wiping with the mouse.
    let mw = P.misc2.z * smoothstep(p(FOGGY_WINDOW_FOG_WIPE_SIZE) * 0.25 + 0.01, 0.0, length((i.uv - P.misc2.xy) * vec2<f32>(aspect, 1.0)));
    fog = fog * (1.0 - drop) * (1.0 - mw);
    let foggy = vec4<f32>((fogged.rgb + tint.rgb * (0.2 + p(FOGGY_WINDOW_FOG_BOOST) * 0.5) * max(fogged.a, 0.4)), max(fogged.a, 0.4 * tint.a));
    let refr = S(i.uv + f * drop * 0.03);
    let o = mixc(mixc(cur, refr, drop), foggy, fog);
    return mixc(cur, o, s);
}

// ---------------------------------------------------------------- Speedlines
@fragment
fn fs_speedlines(i: V) -> @location(0) vec4<f32> {
    let o = S(i.uv);
    let s = clamp(p(SPEEDLINES_STRENGTH) * (1.0 + P.misc.y * p(SPEEDLINES_MIC_EFFECT) * 2.0), 0.0, 1.5);
    let c = vec2<f32>(0.5 + p(SPEEDLINES_XCENTER) * 0.5, 0.5 - p(SPEEDLINES_YCENTER) * 0.5);
    let d = (i.uv - c) * vec2<f32>(P.misc3.z, 1.0);
    let a = atan2(d.y, d.x);
    let r = length(d);
    let n = noise(vec2<f32>(a * 40.0, floor(T() * 12.0)));
    let line = smoothstep(0.55, 0.8, n) * smoothstep(0.15, 0.6, r);
    let col = mix(pc(SPEEDLINES_COLOR_A), pc(SPEEDLINES_COLOR_B), step(0.5, hash(vec2<f32>(floor(a * 40.0), 1.0))));
    return over(o, col.rgb, line * col.a * s);
}

// ---------------------------------------------------------------- Pixelation
@fragment
fn fs_pixelation(i: V) -> @location(0) vec4<f32> {
    let s = p(PIXELATION_STRENGTH);
    let rows = max(p(PIXELATION_RESOLUTION), 10.0);
    let grid = vec2<f32>(rows * P.misc3.z, rows);
    let uv = (floor(i.uv * grid) + 0.5) / grid;
    let c = S(uv);
    var rgb = unpremul(c);
    let fry = p(PIXELATION_FRY);
    rgb = clamp((rgb - 0.5) * (1.0 + fry * 2.0) + 0.5 + vec3<f32>((hash(uv * 99.0) - 0.5) * fry * 0.3), vec3<f32>(0.0), vec3<f32>(1.0));
    // Palette: 8 colours, pick by luminance.
    let l = luma(rgb);
    let idx = u32(clamp(l * 8.0, 0.0, 7.0));
    let pal = pc(PIXELATION_C1 + idx * 4u).rgb;
    rgb = mix(rgb, pal, p(PIXELATION_COLORIZE));
    return mixc(S(i.uv), vec4<f32>(rgb * c.a, c.a), s);
}

// ---------------------------------------------------------------- Lens distortion
@fragment
fn fs_lens_distortion(i: V) -> @location(0) vec4<f32> {
    let s = p(LENS_DISTORTION_STRENGTH);
    var d = i.uv - 0.5;
    d.x = d.x * (1.0 + p(LENS_DISTORTION_SQUISH) * 0.5 * s);
    d.y = d.y * (1.0 - p(LENS_DISTORTION_SQUISH) * 0.5 * s);
    let r2 = dot(d, d);
    d = d * (1.0 + p(LENS_DISTORTION_LENS_STRENGTH) * r2 * 2.0 * s);
    d = d / (1.0 + p(LENS_DISTORTION_ZOOM_IN) * 0.5 * s);
    let uv = d + 0.5;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    return S(uv);
}

// ---------------------------------------------------------------- Wave distortion
@fragment
fn fs_wave_distortion(i: V) -> @location(0) vec4<f32> {
    let s = p(WAVE_DISTORTION_STRENGTH);
    let t = T();
    var d = (i.uv - 0.5) * vec2<f32>(P.misc3.z, 1.0);
    d = rot2(d, radians(p(WAVE_DISTORTION_ROTATION_BASE)) * s) / vec2<f32>(P.misc3.z, 1.0);
    d = d / (1.0 + p(WAVE_DISTORTION_ZOOM_IN) * 0.5 * s);
    var uv = d + 0.5;
    uv.x = uv.x + sin(uv.y * (5.0 + p(WAVE_DISTORTION_WAVE_XFREQUENCY) * 60.0) + t * p(WAVE_DISTORTION_WAVE_XSCROLL) * 5.0) * p(WAVE_DISTORTION_WAVE_XSTRENGTH) * 0.03 * s;
    uv.y = uv.y + sin(uv.x * (5.0 + p(WAVE_DISTORTION_WAVE_YFREQUENCY) * 60.0) + t * p(WAVE_DISTORTION_WAVE_YSCROLL) * 5.0) * p(WAVE_DISTORTION_WAVE_YSTRENGTH) * 0.03 * s;
    // Heat waves.
    let hn = noise(vec2<f32>(uv.x * 20.0, uv.y * 30.0 - t * 3.0)) - 0.5;
    uv = uv + vec2<f32>(hn) * p(WAVE_DISTORTION_HEAT_STRENGTH) * 0.02 * s;
    // Raindrop ripples (one every N seconds).
    let per = max(p(WAVE_DISTORTION_RAINDROP_FREQUENCY), 1.0);
    let k = floor(t / per);
    let c = hash2(vec2<f32>(k, 4.0));
    let age = fract(t / per) * per;
    let r = length((uv - c) * vec2<f32>(P.misc3.z, 1.0));
    let ring = sin((r - age * 0.4) * 80.0) * smoothstep(0.08, 0.0, abs(r - age * 0.4)) * exp(-age * 1.5);
    uv = uv + normalize(uv - c + vec2<f32>(1e-5)) * ring * p(WAVE_DISTORTION_RAINDROP_STRENGTH) * 0.02 * s;
    return S(uv);
}

// ---------------------------------------------------------------- Blur effects
@fragment
fn fs_blur_effects(i: V) -> @location(0) vec4<f32> {
    let s = p(BLUR_EFFECTS_STRENGTH);
    var o = S(i.uv);
    let pb = p(BLUR_EFFECTS_PIXELATION_BLUR);
    if (pb > 0.0) {
        let g = res() / (2.0 + pb * 30.0);
        o = S((floor(i.uv * g) + 0.5) / g);
    }
    let bv = p(BLUR_EFFECTS_BASIC_BLUR_VISIBILITY);
    if (bv > 0.0) {
        o = mixc(o, blur_src(i.uv, 2.0 + p(BLUR_EFFECTS_BASIC_BLUR_STRENGTH) * 40.0), bv);
    }
    let mb = p(BLUR_EFFECTS_MOTION_BLUR);
    if (mb > 0.0 && P.misc3.y > 3.0) {
        o = mixc(o, H(i.uv), mb * 0.85);
    }
    return mixc(S(i.uv), o, s);
}

// ---------------------------------------------------------------- Grain
@fragment
fn fs_grain(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    let s = p(GRAIN_STRENGTH);
    let g = res() / max(p(GRAIN_SIZE), 0.1);
    let cell = floor(i.uv * g) + floor(T() * 24.0) * 13.0;
    var n = vec3<f32>(hash(cell) - 0.5);
    if (p(GRAIN_COLORED) > 0.5) {
        n = vec3<f32>(hash(cell) - 0.5, hash(cell + 3.1) - 0.5, hash(cell + 7.7) - 0.5);
    }
    let l = mix(1.0, 1.0 - luma(unpremul(c)), p(GRAIN_LUMINOSITY));
    return vec4<f32>(clamp(c.rgb + n * 0.35 * s * l * c.a, vec3<f32>(0.0), vec3<f32>(c.a)), c.a);
}

// ---------------------------------------------------------------- VHS
@fragment
fn fs_vhs(i: V) -> @location(0) vec4<f32> {
    let s = p(VHS_STRENGTH);
    let t = T();
    var d = i.uv - 0.5;
    d = d * (1.0 + dot(d, d) * p(VHS_FISHEYE) * 0.4 * s);
    var uv = d + 0.5;
    uv.y = uv.y + (hash(vec2<f32>(floor(t * 4.0), 1.0)) - 0.5) * 0.004 * p(VHS_TWITCH_VERTICAL) * s * step(0.8, hash(vec2<f32>(floor(t * 4.0), 2.0)));
    uv.x = uv.x + (hash(vec2<f32>(floor(uv.y * 120.0), floor(t * 10.0))) - 0.5) * 0.004 * p(VHS_TWITCH_HORIZONTAL) * s;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    let px = 1.0 / res();
    var c = S(uv);
    // Sideways chroma bleed.
    let bl = p(VHS_SCREEN_BLEED);
    let cb = (S(uv + vec2<f32>(px.x * 3.0 * bl, 0.0)) + S(uv - vec2<f32>(px.x * 3.0 * bl, 0.0))) * 0.5;
    c = vec4<f32>(c.r, mix(c.g, cb.g, 0.5), cb.b, max(c.a, cb.a));
    var rgb = unpremul(c);
    rgb = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(1.0 - p(VHS_GAMMA_CORRECTION) * 0.5));
    rgb = mix(rgb, vec3<f32>(luma(rgb)), p(VHS_PALE_COLOR) * 0.6);
    rgb = rgb + vec3<f32>((hash(uv * res() + t) - 0.5) * p(VHS_NOISE_GRAIN) * 0.25);
    let ln = step(0.995 - p(VHS_NOISE_LINES) * 0.02, hash(vec2<f32>(floor(uv.y * 240.0), floor(t * 30.0))));
    rgb = rgb + vec3<f32>(ln * 0.6);
    rgb = rgb * (1.0 - p(VHS_INTERLACING) * 0.4 * step(0.5, fract(uv.y * res().y * 0.5)));
    rgb = rgb * (1.0 - smoothstep(0.35, 0.75, length(d)) * p(VHS_VIGNETTE));
    var o = vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * c.a, c.a);
    // Afterimage (previous frame, tinted).
    let ai = p(VHS_AFTER_IMAGE_AMOUNT);
    if (ai > 0.0 && P.misc3.y > 3.0) {
        let prev = H(i.uv + vec2<f32>(px.x * 6.0, 0.0));
        o = add_light(o, prev.rgb * pc(VHS_AFTER_IMAGE_COLOR).rgb * ai * 0.6);
    }
    return mixc(S(i.uv), o, s);
}

// ---------------------------------------------------------------- Outline
@fragment
fn fs_outline(i: V) -> @location(0) vec4<f32> {
    let s = p(OUTLINE_STRENGTH);
    let c = S(i.uv);
    let px = 1.0 / res();
    let n = S(i.uv + vec2<f32>(0.0, px.y));
    let so = S(i.uv - vec2<f32>(0.0, px.y));
    let e = S(i.uv + vec2<f32>(px.x, 0.0));
    let w = S(i.uv - vec2<f32>(px.x, 0.0));
    // Sharpen (unsharp mask).
    var o = c + (c * 4.0 - n - so - e - w) * p(OUTLINE_SHARPEN) * 0.5;
    o = vec4<f32>(clamp(o.rgb, vec3<f32>(0.0), vec3<f32>(o.a)), c.a);
    // Edges from luminance + alpha gradients.
    let gx = luma(e.rgb) - luma(w.rgb) + (e.a - w.a);
    let gy = luma(n.rgb) - luma(so.rgb) + (n.a - so.a);
    let g = length(vec2<f32>(gx, gy));
    let edge = smoothstep(1.0 - p(OUTLINE_THRESHOLD), 1.0 - p(OUTLINE_THRESHOLD) + 0.05 + p(OUTLINE_CONTRAST) * 0.5, g);
    let col = pc(OUTLINE_COLOR);
    o = over(o, col.rgb, edge * col.a * p(OUTLINE_VISIBILITY));
    return mixc(c, o, s);
}

// ---------------------------------------------------------------- Posterize
@fragment
fn fs_posterize(i: V) -> @location(0) vec4<f32> {
    let c = S(i.uv);
    let s = p(POSTERIZE_STRENGTH);
    let lv = mix(32.0, 3.0, s);
    let rgb = floor(unpremul(c) * lv + 0.5) / lv;
    return mixc(c, vec4<f32>(rgb * c.a, c.a), clamp(s * 4.0, 0.0, 1.0));
}

// ---------------------------------------------------------------- ASCII
fn glyph(i: u32) -> u32 {
    var g = array<u32, 10>(0u, 134217728u, 4194432u, 458752u, 32537600u, 139344004u, 22442496u, 368389098u, 588422434u, 474909246u);
    return g[i];
}

@fragment
fn fs_ascii(i: V) -> @location(0) vec4<f32> {
    let s = p(ASCII_STRENGTH);
    let r = res();
    let cs = 4.0 + p(ASCII_SIZE) * 2.0;
    let cell = floor(i.uv * r / cs);
    let center = (cell + 0.5) * cs / r;
    let sm = S(center);
    let l = luma(unpremul(sm)) * sm.a;
    let gi = u32(clamp(l * 9.99, 0.0, 9.0));
    let f = fract(i.uv * r / cs);
    let cx = u32(clamp(f.x * 5.0, 0.0, 4.0));
    let cy = u32(clamp(f.y * 6.0, 0.0, 5.0));
    let bit = f32((glyph(gi) >> (cy * 5u + cx)) & 1u);
    let col = mix(unpremul(sm), pc(ASCII_CHARACTER_COLOR).rgb, p(ASCII_CHARACTER_COLOR_STRENGTH));
    let a = mix(sm.a, bit * sm.a, p(ASCII_CHARACTER_VISIBILITY));
    return mixc(S(i.uv), vec4<f32>(col * a, a), s);
}
