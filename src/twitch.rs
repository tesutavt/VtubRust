//! Twitch connection.
//!
//! * Chat: IRC (irc.chat.twitch.tv:6667). Works anonymously with just a channel name —
//!   gives chat messages, emotes, bits, subs, gift subs, raids and Power-Ups.
//! * Optional login (OAuth Device Code flow with your own Twitch application Client ID):
//!   EventSub WebSocket for follows, channel point redeems, Power-Ups, shoutouts and ad
//!   breaks, plus the list of channel point rewards.
//! * Third-party emotes: BTTV, 7TV and FFZ (channel + global).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hotkeys::TwitchEvent;

pub const SCOPES: &str = "bits:read channel:read:subscriptions channel:read:redemptions moderator:read:followers moderator:read:shoutouts channel:read:ads user:read:chat";

// ------------------------------------------------------------------ settings

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DropperCfg {
    pub enabled: bool,
    pub filter: Vec<String>,
    pub subs_only: bool,
    pub privileged_only: bool,
    pub smooth_borders: bool,
    pub max_per_message: f32,
    pub max_on_screen: f32,
    pub spawn_chance: f32,
    pub bttv_channel: bool,
    pub bttv_global: bool,
    pub seventv_channel: bool,
    pub seventv_global: bool,
    pub ffz_channel: bool,
    pub ffz_global: bool,
    pub lifetime: f32,
    pub anim_speed: f32,
    pub fall_speed: f32,
    pub gravity: f32,
    pub opacity: f32,
    pub bounciness: f32,
    pub size: f32,
    pub rotation: f32,
    pub bounce_off_model: bool,
    pub bounce_bottom: f32,
    pub bounce_top: f32,
    pub bounce_left: f32,
    pub bounce_right: f32,
    pub notify: bool,
}

impl Default for DropperCfg {
    fn default() -> Self {
        DropperCfg {
            enabled: false,
            filter: Vec::new(),
            subs_only: false,
            privileged_only: false,
            smooth_borders: true,
            max_per_message: 3.0,
            max_on_screen: 50.0,
            spawn_chance: 100.0,
            bttv_channel: true,
            bttv_global: true,
            seventv_channel: true,
            seventv_global: true,
            ffz_channel: true,
            ffz_global: true,
            lifetime: 9.0,
            anim_speed: 1.0,
            fall_speed: 0.3,
            gravity: 1.0,
            opacity: 1.0,
            bounciness: 0.5,
            size: 1.0,
            rotation: 0.5,
            bounce_off_model: true,
            bounce_bottom: 2.0,
            bounce_top: 0.0,
            bounce_left: 20.0,
            bounce_right: 20.0,
            notify: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ChatCfg {
    pub enabled: bool,
    pub foreground: bool,
    pub opacity: f32,
    pub grayscale: bool,
    pub fly_in: bool,
    pub speed: f32,
    pub speed_random: f32,
    pub size: f32,
    pub size_random: f32,
    pub shadow_opacity: f32,
    pub shadow_offset: f32,
    pub zone_width: f32,
    pub zone_center: f32,
    pub angle: f32,
    pub angle_random: f32,
    pub max_len: f32,
    pub sync_emote_anim: bool,
    pub emote_anim_speed: f32,
    pub max_on_screen: f32,
    pub remove_old: bool,
    pub per_user_limit: bool,
    pub subs_only: bool,
    pub privileged_only: bool,
    pub blocked_words: String,
    pub block_scam_bots: bool,
    pub block_obscene: bool,
    pub block_profanity: bool,
    pub block_urls: bool,
    pub exclude_you: bool,
    pub bttv: bool,
    pub seventv: bool,
    pub ffz: bool,
    pub special_privileged: f32,
    pub special_others: f32,
    pub notify: bool,
}

impl Default for ChatCfg {
    fn default() -> Self {
        ChatCfg {
            enabled: false,
            foreground: true,
            opacity: 1.0,
            grayscale: false,
            fly_in: true,
            speed: 1.0,
            speed_random: 0.3,
            size: 1.0,
            size_random: 0.2,
            shadow_opacity: 0.6,
            shadow_offset: 0.3,
            zone_width: 0.8,
            zone_center: 0.5,
            angle: 0.0,
            angle_random: 0.0,
            max_len: 200.0,
            sync_emote_anim: true,
            emote_anim_speed: 1.0,
            max_on_screen: 130.0,
            remove_old: true,
            per_user_limit: true,
            subs_only: false,
            privileged_only: false,
            blocked_words: String::new(),
            block_scam_bots: true,
            block_obscene: true,
            block_profanity: false,
            block_urls: true,
            exclude_you: true,
            bttv: true,
            seventv: true,
            ffz: true,
            special_privileged: 15.0,
            special_others: 0.0,
            notify: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TwitchCfg {
    /// Connect on start.
    pub enabled: bool,
    pub channel: String,
    /// Client ID of the user's own Twitch application (for login).
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub login: String,
    pub user_id: String,
    pub triggers_on: bool,
    pub trigger_log: bool,
    pub trigger_notify: bool,
    pub dropper: DropperCfg,
    pub chat: ChatCfg,
}

impl Default for TwitchCfg {
    fn default() -> Self {
        TwitchCfg {
            enabled: false,
            channel: String::new(),
            client_id: String::new(),
            access_token: String::new(),
            refresh_token: String::new(),
            login: String::new(),
            user_id: String::new(),
            triggers_on: true,
            trigger_log: false,
            trigger_notify: true,
            dropper: DropperCfg::default(),
            chat: ChatCfg::default(),
        }
    }
}

// ------------------------------------------------------------------ messages

#[derive(Clone, Debug)]
pub struct Emote {
    pub code: String,
    pub url: String,
    /// Byte range in the message text (Twitch emotes) or None (third-party, matched by word).
    pub range: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub struct ChatMsg {
    pub user: String,
    pub user_id: String,
    pub color: Option<[u8; 3]>,
    pub text: String,
    pub emotes: Vec<Emote>,
    pub broadcaster: bool,
    pub sub: bool,
    pub mod_: bool,
    pub vip: bool,
    pub artist: bool,
    pub first_msg: bool,
}

impl ChatMsg {
    pub fn privileged(&self) -> bool {
        self.broadcaster || self.mod_ || self.vip
    }
}

#[derive(Clone, Debug)]
pub struct Reward {
    pub id: String,
    pub title: String,
    pub cost: u32,
}

pub enum Msg {
    Status(Status),
    Chat(ChatMsg),
    Event(TwitchEvent),
    RoomId(String),
    ThirdParty(HashMap<String, (String, u8)>),
    DeviceCode { user_code: String, uri: String },
    Authorized { access: String, refresh: String, login: String, user_id: String },
    AuthFailed(String),
    Rewards(Vec<Reward>),
    EventSub(Result<usize, String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    Connected,
    Error(String),
}

// ------------------------------------------------------------------ client

pub struct Twitch {
    rx: Receiver<Msg>,
    tx: Sender<Msg>,
    stop: Arc<AtomicBool>,
    pub status: Status,
    pub eventsub: Option<Result<usize, String>>,
    pub channel: String,
    pub room_id: String,
    /// Third-party emotes: code → (url, source: 0 BTTV, 1 7TV, 2 FFZ; +10 = global).
    pub third_party: HashMap<String, (String, u8)>,
    pub rewards: Vec<Reward>,
    pub device: Option<(String, String)>,
    pub recent: std::collections::VecDeque<String>,
    pub started: Instant,
}

fn tag_map(tags: &str) -> HashMap<&str, String> {
    tags.split(';')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k, v.replace("\\s", " ").replace("\\:", ";").replace("\\\\", "\\").replace("\\r", "").replace("\\n", "")))
        .collect()
}

/// Parses one IRC line into (tags, command, channel/params, trailing).
fn parse_irc(line: &str) -> Option<(HashMap<&str, String>, &str, &str, &str, &str)> {
    let mut rest = line;
    let mut tags = HashMap::new();
    if let Some(r) = rest.strip_prefix('@') {
        let (t, r) = r.split_once(' ')?;
        tags = tag_map(t);
        rest = r;
    }
    let mut prefix = "";
    if let Some(r) = rest.strip_prefix(':') {
        let (p, r) = r.split_once(' ')?;
        prefix = p;
        rest = r;
    }
    let (head, trailing) = match rest.split_once(" :") {
        Some((h, t)) => (h, t),
        None => (rest, ""),
    };
    let mut parts = head.splitn(2, ' ');
    let cmd = parts.next()?;
    let params = parts.next().unwrap_or("");
    Some((tags, cmd, params, trailing, prefix))
}

fn color(hex: &str) -> Option<[u8; 3]> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn has_badge(badges: &str, b: &str) -> bool {
    badges.split(',').any(|x| x.split('/').next() == Some(b))
}

fn chat_from_tags(tags: &HashMap<&str, String>, prefix: &str, text: &str) -> ChatMsg {
    let badges = tags.get("badges").cloned().unwrap_or_default();
    let mut emotes = Vec::new();
    // emotes=25:0-4,12-16/1902:6-10  (ranges are in characters)
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if let Some(e) = tags.get("emotes") {
        for group in e.split('/').filter(|g| !g.is_empty()) {
            let Some((id, ranges)) = group.split_once(':') else { continue };
            for r in ranges.split(',') {
                let Some((a, b)) = r.split_once('-') else { continue };
                let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else { continue };
                let (Some(&(sa, _)), end) = (chars.get(a), chars.get(b + 1).map(|c| c.0).unwrap_or(text.len())) else { continue };
                emotes.push(Emote { code: text[sa..end].to_string(), url: format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0"), range: Some((sa, end)) });
            }
        }
    }
    emotes.sort_by_key(|e| e.range.map(|r| r.0).unwrap_or(0));
    let login = prefix.split('!').next().unwrap_or("").to_string();
    ChatMsg {
        user: tags.get("display-name").filter(|s| !s.is_empty()).cloned().unwrap_or(login),
        user_id: tags.get("user-id").cloned().unwrap_or_default(),
        color: tags.get("color").and_then(|c| color(c)),
        text: text.to_string(),
        emotes,
        broadcaster: has_badge(&badges, "broadcaster"),
        sub: tags.get("subscriber").map(|s| s == "1").unwrap_or(false) || has_badge(&badges, "subscriber") || has_badge(&badges, "founder"),
        mod_: tags.get("mod").map(|s| s == "1").unwrap_or(false),
        vip: tags.contains_key("vip") || has_badge(&badges, "vip"),
        artist: has_badge(&badges, "artist-badge"),
        first_msg: tags.get("first-msg").map(|s| s == "1").unwrap_or(false),
    }
}

fn irc_loop(channel_name: String, auth: Option<(String, String)>, tx: Sender<Msg>, stop: Arc<AtomicBool>, eventsub_on: Arc<AtomicBool>) {
    let mut backoff = 1u64;
    while !stop.load(Ordering::Relaxed) {
        let _ = tx.send(Msg::Status(Status::Connecting));
        match irc_session(&channel_name, auth.as_ref(), &tx, &stop, &eventsub_on) {
            Ok(()) => backoff = 1,
            Err(e) => {
                let _ = tx.send(Msg::Status(Status::Error(e)));
            }
        }
        for _ in 0..backoff * 10 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        backoff = (backoff * 2).min(30);
    }
}

fn irc_session(channel_name: &str, auth: Option<&(String, String)>, tx: &Sender<Msg>, stop: &AtomicBool, eventsub_on: &AtomicBool) -> Result<(), String> {
    let mut sock = TcpStream::connect("irc.chat.twitch.tv:6667").map_err(|e| format!("нет соединения с чатом Twitch: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(500))).ok();
    let (pass, nick) = match auth {
        Some((token, login)) => (format!("oauth:{token}"), login.clone()),
        None => ("SCHMOOPIIE".to_string(), format!("justinfan{}", 10000 + std::process::id() % 80000)),
    };
    let hello = format!("CAP REQ :twitch.tv/tags twitch.tv/commands\r\nPASS {pass}\r\nNICK {nick}\r\nJOIN #{}\r\n", channel_name.to_lowercase());
    sock.write_all(hello.as_bytes()).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(sock.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    let mut last_rx = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = sock.write_all(b"QUIT\r\n");
            return Ok(());
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Err("чат Twitch закрыл соединение".into()),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if last_rx.elapsed() > Duration::from_secs(360) {
                    return Err("нет данных от чата Twitch".into());
                }
                continue;
            }
            Err(e) => return Err(e.to_string()),
        }
        last_rx = Instant::now();
        let l = line.trim_end();
        let Some((tags, cmd, _params, trailing, prefix)) = parse_irc(l) else { continue };
        let es = eventsub_on.load(Ordering::Relaxed);
        match cmd {
            "PING" => {
                let _ = sock.write_all(format!("PONG :{trailing}\r\n").as_bytes());
            }
            "001" => {}
            "ROOMSTATE" => {
                if let Some(id) = tags.get("room-id") {
                    let _ = tx.send(Msg::RoomId(id.clone()));
                }
            }
            "JOIN" => {
                let _ = tx.send(Msg::Status(Status::Connected));
            }
            "NOTICE" if trailing.contains("Login authentication failed") || trailing.contains("Improperly formatted auth") => {
                return Err("неверный токен Twitch".into());
            }
            "RECONNECT" => return Err("Twitch попросил переподключиться".into()),
            "PRIVMSG" => {
                let msg = chat_from_tags(&tags, prefix, trailing);
                if !es {
                    if let Some(b) = tags.get("bits").and_then(|b| b.parse::<u32>().ok()) {
                        let _ = tx.send(Msg::Event(TwitchEvent::Bits { user: msg.user.clone(), amount: b }));
                    }
                    match tags.get("msg-id").map(|s| s.as_str()) {
                        Some("gigantified-emote-message") => {
                            let _ = tx.send(Msg::Event(TwitchEvent::PowerUp { user: msg.user.clone(), kind: "gigantify_an_emote".into() }));
                        }
                        Some("animated-message") => {
                            let _ = tx.send(Msg::Event(TwitchEvent::PowerUp { user: msg.user.clone(), kind: "message_effect".into() }));
                        }
                        _ => {}
                    }
                    if let Some(rid) = tags.get("custom-reward-id") {
                        let _ = tx.send(Msg::Event(TwitchEvent::Redeem { user: msg.user.clone(), reward_id: rid.clone(), reward_title: String::new() }));
                    }
                }
                let _ = tx.send(Msg::Event(TwitchEvent::Chat {
                    user: msg.user.clone(),
                    user_id: msg.user_id.clone(),
                    text: msg.text.clone(),
                    broadcaster: msg.broadcaster,
                    sub: msg.sub,
                    mod_: msg.mod_,
                    vip: msg.vip,
                    artist: msg.artist,
                }));
                let _ = tx.send(Msg::Chat(msg));
            }
            "USERNOTICE" if !es => {
                let user = tags.get("display-name").cloned().unwrap_or_default();
                let p = |k: &str| tags.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                let ev = match tags.get("msg-id").map(|s| s.as_str()) {
                    Some("sub") | Some("resub") | Some("primepaidupgrade") | Some("giftpaidupgrade") => Some(TwitchEvent::Sub { user }),
                    Some("submysterygift") => Some(TwitchEvent::GiftSub { user, count: p("msg-param-mass-gift-count").max(1) }),
                    // Single gift that is part of a mystery gift was already counted above.
                    Some("subgift") if !tags.contains_key("msg-param-community-gift-id") => Some(TwitchEvent::GiftSub { user, count: 1 }),
                    Some("raid") => Some(TwitchEvent::Raid { from: tags.get("msg-param-displayName").cloned().unwrap_or(user), viewers: p("msg-param-viewerCount") }),
                    _ => None,
                };
                if let Some(ev) = ev {
                    let _ = tx.send(Msg::Event(ev));
                }
                if !trailing.is_empty() {
                    let msg = chat_from_tags(&tags, prefix, trailing);
                    let _ = tx.send(Msg::Chat(msg));
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------ HTTP helpers

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).user_agent("rigrs").build()
}

fn get_json(url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let mut r = agent().get(url);
    for (k, v) in headers {
        r = r.set(k, v);
    }
    r.call().map_err(|e| e.to_string())?.into_json::<Value>().map_err(|e| e.to_string())
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<Value, (u16, Value)> {
    match agent().post(url).send_form(form) {
        Ok(r) => r.into_json::<Value>().map_err(|e| (0, Value::String(e.to_string()))),
        Err(ureq::Error::Status(code, r)) => Err((code, r.into_json::<Value>().unwrap_or(Value::Null))),
        Err(e) => Err((0, Value::String(e.to_string()))),
    }
}

/// Fetches BTTV / 7TV / FFZ emotes for a channel (by Twitch user id) and the global sets.
fn third_party(room: &str) -> HashMap<String, (String, u8)> {
    let mut m = HashMap::new();
    let bttv = |v: &Value, src: u8, m: &mut HashMap<String, (String, u8)>| {
        for e in v.as_array().into_iter().flatten() {
            if let (Some(id), Some(code)) = (e["id"].as_str(), e["code"].as_str()) {
                m.insert(code.to_string(), (format!("https://cdn.betterttv.net/emote/{id}/2x"), src));
            }
        }
    };
    if let Ok(v) = get_json("https://api.betterttv.net/3/cached/emotes/global", &[]) {
        bttv(&v, 10, &mut m);
    }
    if let Ok(v) = get_json(&format!("https://api.betterttv.net/3/cached/users/twitch/{room}"), &[]) {
        bttv(&v["channelEmotes"], 0, &mut m);
        bttv(&v["sharedEmotes"], 0, &mut m);
    }
    let ffz = |v: &Value, src: u8, m: &mut HashMap<String, (String, u8)>| {
        if let Some(sets) = v["sets"].as_object() {
            for s in sets.values() {
                for e in s["emoticons"].as_array().into_iter().flatten() {
                    let url = e["urls"]["2"].as_str().or(e["urls"]["1"].as_str()).unwrap_or("");
                    if let Some(name) = e["name"].as_str() {
                        if !url.is_empty() {
                            let url = if url.starts_with("//") { format!("https:{url}") } else { url.to_string() };
                            m.insert(name.to_string(), (url, src));
                        }
                    }
                }
            }
        }
    };
    if let Ok(v) = get_json("https://api.frankerfacez.com/v1/set/global", &[]) {
        ffz(&v, 12, &mut m);
    }
    if let Ok(v) = get_json(&format!("https://api.frankerfacez.com/v1/room/id/{room}"), &[]) {
        ffz(&v, 2, &mut m);
    }
    let stv = |v: &Value, src: u8, m: &mut HashMap<String, (String, u8)>| {
        for e in v["emotes"].as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (e["id"].as_str(), e["name"].as_str()) {
                // GIF variant: decodable by the image crate (webp support is not compiled in).
                m.insert(name.to_string(), (format!("https://cdn.7tv.app/emote/{id}/2x.gif"), src));
            }
        }
    };
    if let Ok(v) = get_json("https://7tv.io/v3/emote-sets/global", &[]) {
        stv(&v, 11, &mut m);
    }
    if let Ok(v) = get_json(&format!("https://7tv.io/v3/users/twitch/{room}"), &[]) {
        stv(&v["emote_set"], 1, &mut m);
    }
    m
}

// ------------------------------------------------------------------ EventSub

fn eventsub_loop(client_id: String, token: String, user_id: String, tx: Sender<Msg>, stop: Arc<AtomicBool>, on: Arc<AtomicBool>) {
    let mut url = "wss://eventsub.wss.twitch.tv/ws".to_string();
    let mut backoff = 2u64;
    while !stop.load(Ordering::Relaxed) {
        match eventsub_session(&url, &client_id, &token, &user_id, &tx, &stop, &on) {
            Ok(Some(next)) => {
                url = next;
                continue;
            }
            Ok(None) => return,
            Err(e) => {
                on.store(false, Ordering::Relaxed);
                let _ = tx.send(Msg::EventSub(Err(e)));
            }
        }
        for _ in 0..backoff * 10 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        backoff = (backoff * 2).min(60);
        url = "wss://eventsub.wss.twitch.tv/ws".to_string();
    }
}

fn eventsub_session(url: &str, client_id: &str, token: &str, uid: &str, tx: &Sender<Msg>, stop: &AtomicBool, on: &AtomicBool) -> Result<Option<String>, String> {
    let (mut ws, _) = tungstenite::connect(url).map_err(|e| format!("EventSub: {e}"))?;
    if let tungstenite::stream::MaybeTlsStream::Rustls(s) = ws.get_mut() {
        let _ = s.get_mut().set_read_timeout(Some(Duration::from_millis(500)));
    }
    let mut last = Instant::now();
    let mut keepalive = 30u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = ws.close(None);
            return Ok(None);
        }
        let msg = match ws.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if last.elapsed() > Duration::from_secs(keepalive + 10) {
                    return Err("EventSub: нет keepalive".into());
                }
                continue;
            }
            Err(e) => return Err(format!("EventSub: {e}")),
        };
        last = Instant::now();
        let tungstenite::Message::Text(t) = msg else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
        let kind = v["metadata"]["message_type"].as_str().unwrap_or("");
        match kind {
            "session_welcome" => {
                keepalive = v["payload"]["session"]["keepalive_timeout_seconds"].as_u64().unwrap_or(30);
                let sid = v["payload"]["session"]["id"].as_str().unwrap_or("").to_string();
                if url.contains("eventsub.wss.twitch.tv/ws") && !url.contains('?') {
                    let n = subscribe_all(client_id, token, uid, &sid);
                    on.store(n.as_ref().map(|n| *n > 0).unwrap_or(false), Ordering::Relaxed);
                    let _ = tx.send(Msg::EventSub(n));
                }
            }
            "session_reconnect" => {
                if let Some(u) = v["payload"]["session"]["reconnect_url"].as_str() {
                    return Ok(Some(u.to_string()));
                }
            }
            "notification" => {
                let ty = v["metadata"]["subscription_type"].as_str().unwrap_or("");
                let e = &v["payload"]["event"];
                let s = |k: &str| e[k].as_str().unwrap_or("").to_string();
                let ev = match ty {
                    "channel.follow" => Some(TwitchEvent::Follow { user: s("user_name"), user_id: s("user_id") }),
                    "channel.subscribe" if e["is_gift"].as_bool() != Some(true) => Some(TwitchEvent::Sub { user: s("user_name") }),
                    "channel.subscription.message" => Some(TwitchEvent::Sub { user: s("user_name") }),
                    "channel.subscription.gift" => Some(TwitchEvent::GiftSub { user: s("user_name"), count: e["total"].as_u64().unwrap_or(1) as u32 }),
                    "channel.cheer" => Some(TwitchEvent::Bits { user: s("user_name"), amount: e["bits"].as_u64().unwrap_or(0) as u32 }),
                    "channel.raid" => Some(TwitchEvent::Raid { from: s("from_broadcaster_user_name"), viewers: e["viewers"].as_u64().unwrap_or(0) as u32 }),
                    "channel.channel_points_custom_reward_redemption.add" => Some(TwitchEvent::Redeem {
                        user: s("user_name"),
                        reward_id: e["reward"]["id"].as_str().unwrap_or("").into(),
                        reward_title: e["reward"]["title"].as_str().unwrap_or("").into(),
                    }),
                    "channel.channel_points_automatic_reward_redemption.add" => Some(TwitchEvent::PowerUp { user: s("user_name"), kind: e["reward"]["type"].as_str().unwrap_or("").into() }),
                    "channel.shoutout.create" => Some(TwitchEvent::Shoutout { to: s("to_broadcaster_user_name") }),
                    "channel.ad_break.begin" => Some(TwitchEvent::AdBreak { seconds: e["duration_seconds"].as_u64().unwrap_or(0) as u32 }),
                    _ => None,
                };
                if let Some(ev) = ev {
                    let _ = tx.send(Msg::Event(ev));
                }
            }
            _ => {}
        }
    }
}

fn subscribe_all(client_id: &str, token: &str, uid: &str, session: &str) -> Result<usize, String> {
    let b = serde_json::json!({ "broadcaster_user_id": uid });
    let bm = serde_json::json!({ "broadcaster_user_id": uid, "moderator_user_id": uid });
    let subs: Vec<(&str, &str, Value)> = vec![
        ("channel.follow", "2", bm.clone()),
        ("channel.subscribe", "1", b.clone()),
        ("channel.subscription.message", "1", b.clone()),
        ("channel.subscription.gift", "1", b.clone()),
        ("channel.cheer", "1", b.clone()),
        ("channel.raid", "1", serde_json::json!({ "to_broadcaster_user_id": uid })),
        ("channel.channel_points_custom_reward_redemption.add", "1", b.clone()),
        ("channel.channel_points_automatic_reward_redemption.add", "2", b.clone()),
        ("channel.shoutout.create", "1", bm),
        ("channel.ad_break.begin", "1", b),
    ];
    let mut ok = 0;
    let mut last_err = String::new();
    for (ty, ver, cond) in subs {
        let body = serde_json::json!({ "type": ty, "version": ver, "condition": cond, "transport": { "method": "websocket", "session_id": session } });
        match agent()
            .post("https://api.twitch.tv/helix/eventsub/subscriptions")
            .set("Client-Id", client_id)
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(body)
        {
            Ok(_) => ok += 1,
            Err(ureq::Error::Status(code, r)) => {
                last_err = format!("{ty}: {code} {}", r.into_string().unwrap_or_default());
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    if ok == 0 {
        Err(last_err)
    } else {
        Ok(ok)
    }
}

// ------------------------------------------------------------------ public API

impl Twitch {
    pub fn start(cfg: &TwitchCfg) -> Twitch {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let es_on = Arc::new(AtomicBool::new(false));
        let logged_in = !cfg.access_token.is_empty() && !cfg.user_id.is_empty() && !cfg.client_id.is_empty();
        let channel_name = if cfg.channel.trim().is_empty() { cfg.login.clone() } else { cfg.channel.trim().trim_start_matches('#').to_lowercase() };
        {
            let (tx, stop, es) = (tx.clone(), stop.clone(), es_on.clone());
            let auth = logged_in.then(|| (cfg.access_token.clone(), cfg.login.clone()));
            let ch = channel_name.clone();
            std::thread::Builder::new().name("twitch-irc".into()).spawn(move || irc_loop(ch, auth, tx, stop, es)).ok();
        }
        if logged_in && channel_name.eq_ignore_ascii_case(&cfg.login) {
            let (tx, stop, es) = (tx.clone(), stop.clone(), es_on.clone());
            let (cid, tok, uid) = (cfg.client_id.clone(), cfg.access_token.clone(), cfg.user_id.clone());
            std::thread::Builder::new().name("twitch-eventsub".into()).spawn(move || eventsub_loop(cid, tok, uid, tx, stop, es)).ok();
        }
        Twitch {
            rx,
            tx,
            stop,
            status: Status::Connecting,
            eventsub: None,
            channel: channel_name,
            room_id: String::new(),
            third_party: HashMap::new(),
            rewards: Vec::new(),
            device: None,
            recent: Default::default(),
            started: Instant::now(),
        }
    }

    pub fn drain(&mut self) -> Vec<Msg> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Status(s) => self.status = s,
                Msg::RoomId(id) => {
                    if id != self.room_id {
                        self.room_id = id.clone();
                        let tx = self.tx.clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(Msg::ThirdParty(third_party(&id)));
                        });
                    }
                }
                Msg::ThirdParty(m) => self.third_party = m,
                Msg::Rewards(r) => self.rewards = r,
                Msg::EventSub(r) => self.eventsub = Some(r),
                other => out.push(other),
            }
        }
        out
    }

    pub fn sender(&self) -> Sender<Msg> {
        self.tx.clone()
    }

    pub fn fetch_rewards(&self, cfg: &TwitchCfg) {
        let tx = self.tx.clone();
        let (cid, tok, uid) = (cfg.client_id.clone(), cfg.access_token.clone(), cfg.user_id.clone());
        std::thread::spawn(move || {
            let url = format!("https://api.twitch.tv/helix/channel_points/custom_rewards?broadcaster_id={uid}");
            let auth = format!("Bearer {tok}");
            match get_json(&url, &[("Client-Id", &cid), ("Authorization", &auth)]) {
                Ok(v) => {
                    let list = v["data"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|r| Reward { id: r["id"].as_str().unwrap_or("").into(), title: r["title"].as_str().unwrap_or("").into(), cost: r["cost"].as_u64().unwrap_or(0) as u32 })
                        .collect();
                    let _ = tx.send(Msg::Rewards(list));
                }
                Err(e) => {
                    let _ = tx.send(Msg::AuthFailed(format!("Не удалось получить награды (нужен статус компаньона/партнёра): {e}")));
                }
            }
        });
    }
}

impl Drop for Twitch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// OAuth Device Code flow. Results arrive as Msg::DeviceCode / Authorized / AuthFailed.
pub fn login(client_id: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let r = post_form("https://id.twitch.tv/oauth2/device", &[("client_id", &client_id), ("scopes", SCOPES)]);
        let v = match r {
            Ok(v) => v,
            Err((code, v)) => {
                let _ = tx.send(Msg::AuthFailed(format!("Twitch: {code} {}", v["message"].as_str().unwrap_or(&v.to_string()))));
                return;
            }
        };
        let device = v["device_code"].as_str().unwrap_or("").to_string();
        let interval = v["interval"].as_u64().unwrap_or(5).max(1);
        let expires = Instant::now() + Duration::from_secs(v["expires_in"].as_u64().unwrap_or(1800));
        let uri = v["verification_uri"].as_str().unwrap_or("https://www.twitch.tv/activate").to_string();
        let _ = tx.send(Msg::DeviceCode { user_code: v["user_code"].as_str().unwrap_or("").into(), uri });
        while Instant::now() < expires {
            std::thread::sleep(Duration::from_secs(interval));
            match post_form(
                "https://id.twitch.tv/oauth2/token",
                &[("client_id", &client_id), ("scopes", SCOPES), ("device_code", &device), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")],
            ) {
                Ok(t) => {
                    let access = t["access_token"].as_str().unwrap_or("").to_string();
                    let refresh = t["refresh_token"].as_str().unwrap_or("").to_string();
                    match get_json("https://id.twitch.tv/oauth2/validate", &[("Authorization", &format!("OAuth {access}"))]) {
                        Ok(info) => {
                            let _ = tx.send(Msg::Authorized { access, refresh, login: info["login"].as_str().unwrap_or("").into(), user_id: info["user_id"].as_str().unwrap_or("").into() });
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::AuthFailed(e));
                        }
                    }
                    return;
                }
                Err((_, v)) if v["message"].as_str() == Some("authorization_pending") => continue,
                Err((code, v)) => {
                    let _ = tx.send(Msg::AuthFailed(format!("Twitch: {code} {}", v["message"].as_str().unwrap_or(""))));
                    return;
                }
            }
        }
        let _ = tx.send(Msg::AuthFailed("Время на вход истекло".into()));
    });
}

/// Refreshes an expired token (public clients need no secret). Blocking; call from a thread.
pub fn refresh(client_id: &str, refresh_token: &str) -> Option<(String, String)> {
    let t = post_form("https://id.twitch.tv/oauth2/token", &[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", refresh_token)]).ok()?;
    Some((t["access_token"].as_str()?.to_string(), t["refresh_token"].as_str().unwrap_or(refresh_token).to_string()))
}

/// Validates the stored token; returns false when it is no longer valid.
pub fn validate(token: &str) -> bool {
    get_json("https://id.twitch.tv/oauth2/validate", &[("Authorization", &format!("OAuth {token}"))]).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_privmsg_with_emotes_and_bits() {
        let line = "@badge-info=;badges=moderator/1,subscriber/12;bits=150;color=#FF0000;display-name=Tester;emotes=25:6-10;mod=1;subscriber=1;user-id=42 :tester!tester@tester.tmi.twitch.tv PRIVMSG #chan :hello Kappa cheer150";
        let (tags, cmd, params, trailing, prefix) = parse_irc(line).unwrap();
        assert_eq!(cmd, "PRIVMSG");
        assert_eq!(params, "#chan");
        assert_eq!(tags.get("bits").map(|s| s.as_str()), Some("150"));
        let m = chat_from_tags(&tags, prefix, trailing);
        assert_eq!(m.user, "Tester");
        assert!(m.mod_ && m.sub && !m.broadcaster);
        assert_eq!(m.color, Some([255, 0, 0]));
        assert_eq!(m.emotes.len(), 1);
        assert_eq!(m.emotes[0].code, "Kappa");
    }

    #[test]
    fn emote_ranges_are_char_based() {
        let line = "@display-name=Ю;emotes=25:7-11 :u!u@u PRIVMSG #c :Привет Kappa";
        let (tags, _, _, trailing, prefix) = parse_irc(line).unwrap();
        let m = chat_from_tags(&tags, prefix, trailing);
        assert_eq!(m.emotes[0].code, "Kappa");
    }

    #[test]
    fn parses_usernotice_tags() {
        let line = "@msg-id=raid;msg-param-displayName=Raider;msg-param-viewerCount=77;display-name=Raider :tmi.twitch.tv USERNOTICE #chan";
        let (tags, cmd, _, _, _) = parse_irc(line).unwrap();
        assert_eq!(cmd, "USERNOTICE");
        assert_eq!(tags.get("msg-param-viewerCount").map(|s| s.as_str()), Some("77"));
    }
}
