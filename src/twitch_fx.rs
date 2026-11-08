//! Twitch on-screen effects: Emote Dropper (emotes falling and bouncing off the model and
//! window edges) and flying chat messages. Drawn as an overlay above the scene.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};

use egui::{Align2, Color32, Context, FontId, Pos2, Rect, Vec2};

use crate::app::App;
use crate::twitch::{ChatCfg, ChatMsg, DropperCfg};

struct Frames {
    frames: Vec<(egui::TextureHandle, f32)>,
    total: f32,
    size: [f32; 2],
}

struct Drop {
    url: String,
    pos: Pos2,
    vel: Vec2,
    rot: f32,
    spin: f32,
    size: f32,
    age: f32,
    life: f32,
    bounces: [i32; 4],
    phase: f32,
}

enum Piece {
    Text(String),
    Emote(String),
}

struct Flying {
    user: String,
    color: Color32,
    pieces: Vec<Piece>,
    pos: Pos2,
    speed: f32,
    size: f32,
    angle: f32,
    age: f32,
    special: bool,
    width: f32,
}

pub struct Fx {
    tex: HashMap<String, Frames>,
    pending: HashSet<String>,
    failed: HashSet<String>,
    rx: Receiver<(String, Option<Vec<(egui::ColorImage, f32)>>)>,
    tx: Sender<(String, Option<Vec<(egui::ColorImage, f32)>>)>,
    drops: Vec<Drop>,
    flying: VecDeque<Flying>,
    queue: Vec<ChatMsg>,
    seed: u64,
    time: f32,
}

impl Default for Fx {
    fn default() -> Self {
        let (tx, rx) = channel();
        Fx {
            tex: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            rx,
            tx,
            drops: Vec::new(),
            flying: VecDeque::new(),
            queue: Vec::new(),
            seed: 0x9E3779B97F4A7C15 ^ std::process::id() as u64,
            time: 0.0,
        }
    }
}

fn decode(bytes: &[u8]) -> Option<Vec<(egui::ColorImage, f32)>> {
    use image::AnimationDecoder;
    let to_ci = |img: &image::RgbaImage| egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    if bytes.starts_with(b"GIF8") {
        let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
        let frames: Vec<_> = dec.into_frames().take(200).filter_map(|f| f.ok()).collect();
        if !frames.is_empty() {
            return Some(
                frames
                    .iter()
                    .map(|f| {
                        let (n, d) = f.delay().numer_denom_ms();
                        (to_ci(f.buffer()), ((n as f32 / d.max(1) as f32) / 1000.0).max(0.02))
                    })
                    .collect(),
            );
        }
    }
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(vec![(to_ci(&img), 1.0)])
}

impl Fx {
    fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    fn request(&mut self, url: &str) {
        if self.tex.contains_key(url) || self.pending.contains(url) || self.failed.contains(url) || self.pending.len() > 24 {
            return;
        }
        self.pending.insert(url.to_string());
        let (tx, url) = (self.tx.clone(), url.to_string());
        std::thread::spawn(move || {
            let data = ureq::get(&url).call().ok().and_then(|r| {
                let mut v = Vec::new();
                use std::io::Read;
                r.into_reader().take(8 << 20).read_to_end(&mut v).ok().map(|_| v)
            });
            let _ = tx.send((url, data.and_then(|d| decode(&d))));
        });
    }

    pub fn clear(&mut self) {
        self.drops.clear();
        self.flying.clear();
    }

    pub fn on_chat(&mut self, m: ChatMsg) {
        self.queue.push(m);
    }

    pub fn active(&self) -> bool {
        !self.drops.is_empty() || !self.flying.is_empty()
    }
}

/// Emotes of a message: Twitch ones from tags plus BTTV/7TV/FFZ by word.
fn emotes_of(m: &ChatMsg, third: &HashMap<String, (String, u8)>, allow: impl Fn(u8) -> bool) -> Vec<String> {
    let mut v: Vec<String> = m.emotes.iter().map(|e| e.url.clone()).collect();
    for w in m.text.split_whitespace() {
        if let Some((url, src)) = third.get(w) {
            if allow(*src) {
                v.push(url.clone());
            }
        }
    }
    v
}

fn split_pieces(m: &ChatMsg, third: &HashMap<String, (String, u8)>, cfg: &ChatCfg, max_len: usize) -> Vec<Piece> {
    let allow = |src: u8| match src % 10 {
        0 => cfg.bttv,
        1 => cfg.seventv,
        _ => cfg.ffz,
    };
    let mut text: String = m.text.chars().take(max_len).collect();
    if m.text.chars().count() > max_len {
        text.push('…');
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut pos = 0usize;
    for word in text.split(' ') {
        let start = pos;
        pos += word.len() + 1;
        let tw = m.emotes.iter().find(|e| e.range.map(|r| r.0 == start).unwrap_or(false)).map(|e| e.url.clone());
        let tp = third.get(word).filter(|(_, s)| allow(*s)).map(|(u, _)| u.clone());
        match tw.or(tp) {
            Some(url) => {
                if !buf.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut buf)));
                }
                out.push(Piece::Emote(url));
            }
            None => {
                buf.push_str(word);
                buf.push(' ');
            }
        }
    }
    if !buf.trim().is_empty() {
        out.push(Piece::Text(buf.trim_end().to_string()));
    }
    out
}

const SCAM: [&str; 8] = ["best viewers", "cheap viewers", "buy followers", "followers, primes", "dogehype", "streamboo", "bigfollows", "remove the space"];
const OBSCENE: [&str; 6] = ["porn", "nude", "onlyfans", "порно", "нюдс", "секс"];
const PROFANITY: [&str; 10] = ["fuck", "shit", "bitch", "cunt", "хуй", "пизд", "ебат", "бля", "сука", "мудак"];

fn chat_allowed(m: &ChatMsg, cfg: &ChatCfg) -> bool {
    if cfg.subs_only && !m.sub && !m.broadcaster {
        return false;
    }
    if cfg.privileged_only && !m.privileged() {
        return false;
    }
    if cfg.exclude_you && m.broadcaster {
        return true;
    }
    let low = m.text.to_lowercase();
    let blocked = cfg.blocked_words.split([',', '\n', ';']).map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).any(|w| low.contains(&w));
    if blocked {
        return false;
    }
    if cfg.block_scam_bots && SCAM.iter().any(|s| low.contains(s)) {
        return false;
    }
    if cfg.block_obscene && OBSCENE.iter().any(|s| low.contains(s)) {
        return false;
    }
    if cfg.block_profanity && PROFANITY.iter().any(|s| low.contains(s)) {
        return false;
    }
    if cfg.block_urls && (low.contains("http://") || low.contains("https://") || low.contains("www.") || low.split_whitespace().any(|w| w.contains('.') && w.len() > 4 && w.rsplit('.').next().map(|t| (2..=4).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphabetic())).unwrap_or(false))) {
        return false;
    }
    true
}

/// Spawns test content (the "Test" button).
pub fn test(app: &mut App, dropper: bool) {
    let msg = ChatMsg {
        user: "rigrs".into(),
        user_id: "0".into(),
        color: Some([145, 70, 255]),
        text: "Привет из rigrs! Kappa PogChamp LUL".into(),
        emotes: vec![
            crate::twitch::Emote { code: "Kappa".into(), url: "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0".into(), range: Some((22, 27)) },
            crate::twitch::Emote { code: "PogChamp".into(), url: "https://static-cdn.jtvnw.net/emoticons/v2/305954156/default/dark/2.0".into(), range: Some((28, 36)) },
            crate::twitch::Emote { code: "LUL".into(), url: "https://static-cdn.jtvnw.net/emoticons/v2/425618/default/dark/2.0".into(), range: Some((37, 40)) },
        ],
        broadcaster: true,
        sub: true,
        mod_: true,
        vip: false,
        artist: false,
        first_msg: false,
    };
    let (d, c) = (app.settings.twitch.dropper.enabled, app.settings.twitch.chat.enabled);
    app.settings.twitch.dropper.enabled = dropper;
    app.settings.twitch.chat.enabled = !dropper;
    let third = app.twitch.as_ref().map(|t| t.third_party.clone()).unwrap_or_default();
    let (vw, vh) = app.viewport();
    let ppp = app.egui_ctx.pixels_per_point();
    let cfg = app.settings.twitch.clone();
    for _ in 0..if dropper { 4 } else { 1 } {
        ingest(&mut app.fx, msg.clone(), &cfg, &third, Vec2::new(vw / ppp, vh / ppp), true);
    }
    app.settings.twitch.dropper.enabled = d;
    app.settings.twitch.chat.enabled = c;
}

fn ingest(fx: &mut Fx, m: ChatMsg, cfg: &crate::twitch::TwitchCfg, third: &HashMap<String, (String, u8)>, screen: Vec2, force: bool) {
    // ---- Emote Dropper
    let d: &DropperCfg = &cfg.dropper;
    if d.enabled {
        let ok_user = (!d.subs_only || m.sub || m.broadcaster) && (!d.privileged_only || m.privileged());
        if ok_user && (force || fx.rand() * 100.0 < d.spawn_chance) {
            let allow = |src: u8| match (src % 10, src >= 10) {
                (0, false) => d.bttv_channel,
                (0, true) => d.bttv_global,
                (1, false) => d.seventv_channel,
                (1, true) => d.seventv_global,
                (_, false) => d.ffz_channel,
                (_, true) => d.ffz_global,
            };
            let filter: Vec<String> = d.filter.iter().map(|f| f.to_lowercase()).collect();
            let mut list = emotes_of(&m, third, allow);
            list.retain(|u| {
                let code = m.emotes.iter().find(|e| &e.url == u).map(|e| e.code.to_lowercase()).or_else(|| third.iter().find(|(_, v)| &v.0 == u).map(|(k, _)| k.to_lowercase())).unwrap_or_default();
                !filter.contains(&code)
            });
            list.truncate(d.max_per_message.max(1.0) as usize);
            for url in list {
                if fx.drops.len() >= d.max_on_screen as usize {
                    fx.drops.remove(0);
                }
                fx.request(&url);
                let x = screen.x * (0.1 + 0.8 * fx.rand());
                let r = fx.rand();
                let r2 = fx.rand();
                let r3 = fx.rand();
                fx.drops.push(Drop {
                    url,
                    pos: Pos2::new(x, -40.0),
                    vel: Vec2::new((r - 0.5) * 300.0, 200.0 + d.fall_speed * 900.0),
                    rot: 0.0,
                    spin: (r2 - 0.5) * 10.0 * d.rotation,
                    size: 56.0 * d.size * (0.85 + 0.3 * r3),
                    age: 0.0,
                    life: d.lifetime,
                    bounces: [d.bounce_bottom as i32, d.bounce_top as i32, d.bounce_left as i32, d.bounce_right as i32],
                    phase: r3 * 10.0,
                });
            }
        }
    }
    // ---- Flying chat
    let c: &ChatCfg = &cfg.chat;
    if c.enabled && (force || chat_allowed(&m, c)) {
        if c.per_user_limit && fx.flying.iter().any(|f| f.user == m.user) && !force {
            return;
        }
        while fx.flying.len() >= c.max_on_screen as usize {
            if c.remove_old {
                fx.flying.pop_front();
            } else {
                return;
            }
        }
        let pieces = split_pieces(&m, third, c, c.max_len as usize);
        for p in &pieces {
            if let Piece::Emote(u) = p {
                fx.request(u);
            }
        }
        let zone_h = screen.y * c.zone_width;
        let y = screen.y * c.zone_center - zone_h / 2.0 + fx.rand() * zone_h;
        let special_chance = if m.privileged() { c.special_privileged } else { c.special_others };
        let rs = fx.rand();
        let rz = fx.rand();
        let ra = fx.rand();
        let col = m.color.map(|c| Color32::from_rgb(c[0], c[1], c[2])).unwrap_or(Color32::from_rgb(145, 70, 255));
        let special = fx.rand() * 100.0 < special_chance;
        fx.flying.push_back(Flying {
            user: m.user.clone(),
            color: col,
            pieces,
            pos: Pos2::new(screen.x + 20.0, y),
            speed: 160.0 * c.speed * (1.0 + (rs - 0.5) * 2.0 * c.speed_random).max(0.2),
            size: 24.0 * c.size * (1.0 + (rz - 0.5) * 2.0 * c.size_random).max(0.3),
            angle: (c.angle + (ra - 0.5) * 2.0 * c.angle_random * 45.0).to_radians(),
            age: 0.0,
            special,
            width: 0.0,
        });
    }
}

fn frame_of(f: &Frames, t: f32) -> &egui::TextureHandle {
    if f.frames.len() == 1 || f.total <= 0.0 {
        return &f.frames[0].0;
    }
    let mut x = t.rem_euclid(f.total);
    for (h, d) in &f.frames {
        if x < *d {
            return h;
        }
        x -= d;
    }
    &f.frames[0].0
}

/// Per-frame update + drawing. Called from the UI pass.
pub fn run(app: &mut App, ctx: &Context, dt: f32) {
    // Finished downloads → textures.
    while let Ok((url, data)) = app.fx.rx.try_recv() {
        app.fx.pending.remove(&url);
        match data {
            Some(frames) if !frames.is_empty() => {
                let size = [frames[0].0.size[0] as f32, frames[0].0.size[1] as f32];
                let total = frames.iter().map(|f| f.1).sum();
                let frames = frames.into_iter().enumerate().map(|(i, (img, d))| (ctx.load_texture(format!("emote-{url}-{i}"), img, egui::TextureOptions::LINEAR), d)).collect();
                app.fx.tex.insert(url, Frames { frames, total, size });
            }
            _ => {
                app.fx.failed.insert(url);
            }
        }
    }
    let screen = ctx.screen_rect().size();
    let queue = std::mem::take(&mut app.fx.queue);
    if !queue.is_empty() {
        let third = app.twitch.as_ref().map(|t| t.third_party.clone()).unwrap_or_default();
        let cfg = app.settings.twitch.clone();
        for m in queue {
            ingest(&mut app.fx, m, &cfg, &third, screen, false);
        }
    }
    if !app.fx.active() {
        return;
    }
    app.fx.time += dt;
    let d = app.settings.twitch.dropper.clone();
    let c = app.settings.twitch.chat.clone();
    let ppp = ctx.pixels_per_point();
    let (vw, vh) = app.viewport();
    let to_world = |p: Pos2| [p.x * ppp - vw / 2.0, vh / 2.0 - p.y * ppp];

    // ---- dropper physics
    let g = 1400.0 * d.gravity;
    let bnc = d.bounciness.clamp(0.0, 1.0);
    let avatar = app.avatar.as_ref();
    for e in &mut app.fx.drops {
        e.age += dt;
        e.vel.y += g * dt;
        let next = e.pos + e.vel * dt;
        let r = e.size / 2.0;
        // Model collision: bounce up when the emote's bottom touches the model.
        if d.bounce_off_model {
            if let Some(a) = avatar {
                let foot = Pos2::new(next.x, next.y + r * 0.8);
                if e.vel.y > 0.0 && a.pick(to_world(foot)).is_some() {
                    e.vel.y = -e.vel.y.abs() * (0.35 + bnc * 0.6);
                    e.vel.x += if next.x < screen.x / 2.0 { -60.0 } else { 60.0 };
                    e.spin = -e.spin;
                }
            }
        }
        e.pos += e.vel * dt;
        if e.pos.y + r > screen.y && e.vel.y > 0.0 && e.bounces[0] > 0 {
            e.bounces[0] -= 1;
            e.pos.y = screen.y - r;
            e.vel.y = -e.vel.y * bnc;
            e.vel.x *= 0.9;
        }
        if e.pos.y - r < 0.0 && e.vel.y < 0.0 && e.age > 0.5 && e.bounces[1] > 0 {
            e.bounces[1] -= 1;
            e.vel.y = -e.vel.y * bnc;
        }
        if e.pos.x - r < 0.0 && e.vel.x < 0.0 && e.bounces[2] > 0 {
            e.bounces[2] -= 1;
            e.vel.x = -e.vel.x * bnc.max(0.3);
        }
        if e.pos.x + r > screen.x && e.vel.x > 0.0 && e.bounces[3] > 0 {
            e.bounces[3] -= 1;
            e.vel.x = -e.vel.x * bnc.max(0.3);
        }
        e.rot += e.spin * dt;
    }
    app.fx.drops.retain(|e| e.age < e.life && e.pos.y < screen.y + 200.0 && e.pos.x > -200.0 && e.pos.x < screen.x + 200.0);

    // ---- flying chat
    for f in &mut app.fx.flying {
        f.age += dt;
        let dir = Vec2::new(-f.angle.cos(), -f.angle.sin());
        f.pos += dir * f.speed * dt;
    }
    app.fx.flying.retain(|f| f.width == 0.0 || f.pos.x + f.width > -50.0);

    // ---- draw
    // Above the scene (the scene is rendered before egui), below every UI panel.
    let _ = c.foreground;
    let layer = egui::Order::Background;
    let painter = ctx.layer_painter(egui::LayerId::new(layer, egui::Id::new("twitch-fx")));
    let t = app.fx.time;
    for e in &app.fx.drops {
        let Some(fr) = app.fx.tex.get(&e.url) else { continue };
        let fade = ((e.life - e.age) / 0.6).clamp(0.0, 1.0) * if d.smooth_borders { (e.age / 0.2).min(1.0) } else { 1.0 };
        let alpha = (d.opacity * fade).clamp(0.0, 1.0);
        let k = e.size / fr.size[0].max(fr.size[1]).max(1.0);
        let half = Vec2::new(fr.size[0] * k, fr.size[1] * k) / 2.0;
        let tex = frame_of(fr, (t + e.phase) * d.anim_speed);
        let mut mesh = egui::Mesh::with_texture(tex.id());
        let col = Color32::from_white_alpha((alpha * 255.0) as u8);
        let (s, cs) = e.rot.sin_cos();
        let rot = |v: Vec2| Vec2::new(v.x * cs - v.y * s, v.x * s + v.y * cs);
        let corners = [Vec2::new(-half.x, -half.y), Vec2::new(half.x, -half.y), Vec2::new(half.x, half.y), Vec2::new(-half.x, half.y)];
        let uvs = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
        for i in 0..4 {
            mesh.vertices.push(egui::epaint::Vertex { pos: e.pos + rot(corners[i]), uv: uvs[i], color: col });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(egui::Shape::mesh(mesh));
    }
    let fx = &mut app.fx;
    let tex = &fx.tex;
    for f in fx.flying.iter_mut() {
        let a = (c.opacity * if c.fly_in { (f.age / 0.3).min(1.0) } else { 1.0 }).clamp(0.0, 1.0);
        let fid = FontId::proportional(f.size);
        let gray = |col: Color32| {
            if c.grayscale {
                let l = (col.r() as f32 * 0.3 + col.g() as f32 * 0.59 + col.b() as f32 * 0.11) as u8;
                Color32::from_rgb(l, l, l)
            } else {
                col
            }
        };
        let mut x = f.pos.x;
        let base_y = f.pos.y;
        let wave = |x: f32| if f.special { (x * 0.03 + f.age * 6.0).sin() * f.size * 0.25 } else { 0.0 };
        let sh_off = Vec2::splat(f.size * 0.12 * c.shadow_offset * 2.0);
        let sh_col = Color32::from_black_alpha((c.shadow_opacity * a * 255.0) as u8);
        let name = format!("{}: ", f.user);
        let ucol = gray(f.color).gamma_multiply(a);
        let g = painter.layout_no_wrap(name.clone(), FontId::proportional(f.size).clone(), ucol);
        let w = g.size().x;
        painter.text(Pos2::new(x, base_y + wave(x)) + sh_off, Align2::LEFT_CENTER, &name, fid.clone(), sh_col);
        painter.text(Pos2::new(x, base_y + wave(x)), Align2::LEFT_CENTER, &name, fid.clone(), ucol);
        x += w;
        for (pi, p) in f.pieces.iter().enumerate() {
            match p {
                Piece::Text(s) => {
                    let col = if f.special { Color32::from(egui::ecolor::Hsva::new((f.age * 0.5 + pi as f32 * 0.1).fract(), 0.7, 1.0, 1.0)) } else { Color32::WHITE };
                    let col = gray(col).gamma_multiply(a);
                    let g = painter.layout_no_wrap(s.clone(), fid.clone(), col);
                    let gw = g.size().x;
                    painter.text(Pos2::new(x, base_y + wave(x)) + sh_off, Align2::LEFT_CENTER, s, fid.clone(), sh_col);
                    painter.text(Pos2::new(x, base_y + wave(x)), Align2::LEFT_CENTER, s, fid.clone(), col);
                    x += gw + f.size * 0.25;
                }
                Piece::Emote(u) => {
                    let h = f.size * 1.4;
                    if let Some(fr) = tex.get(u) {
                        let k = h / fr.size[1].max(1.0);
                        let wdt = fr.size[0] * k;
                        let ts = if c.sync_emote_anim { t } else { f.age };
                        let tx = frame_of(fr, ts * c.emote_anim_speed);
                        let r = Rect::from_min_size(Pos2::new(x, base_y + wave(x) - h / 2.0), Vec2::new(wdt, h));
                        let tint = if c.grayscale { Color32::from_gray(200) } else { Color32::WHITE };
                        painter.image(tx.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint.gamma_multiply(a));
                        x += wdt + f.size * 0.25;
                    } else {
                        x += h;
                    }
                }
            }
        }
        f.width = x - f.pos.x;
    }
    ctx.request_repaint();
}
