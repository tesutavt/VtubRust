//! Plugin API compatible with the VTube Studio Public API ("VTubeStudioPublicAPI" 1.0):
//! WebSocket JSON on port 8001 (configurable) plus the UDP discovery broadcast on 47779.
//! Existing VTS plugins and bots (Streamer.bot, Mix It Up, VNyan bridges, Python/JS
//! libraries …) can connect to rigrs the same way.

use std::collections::{HashMap, VecDeque};
use std::net::{TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::{Action, App};
use crate::mapping::INPUT_NAMES;
use crate::scene::live2d::Xform;
use crate::scene::Body;

pub const API_NAME: &str = "VTubeStudioPublicAPI";
pub const VERSION: &str = "rigrs-1.0 (VTS API 1.0)";

// ------------------------------------------------------------------ persistent state

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ApiToken {
    pub plugin: String,
    pub developer: String,
    pub token: String,
}

#[derive(Clone, Debug)]
pub struct CustomParam {
    pub name: String,
    pub added_by: String,
    pub explanation: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub value: f32,
}

pub enum PromptKind {
    Token,
    Permission(String),
}

/// A plugin waiting for the user to allow or deny something.
pub struct ApiPrompt {
    pub session: u64,
    pub request_id: String,
    pub plugin: String,
    pub developer: String,
    pub icon_png: Option<Vec<u8>>,
    pub kind: PromptKind,
}

// ------------------------------------------------------------------ network side

enum Incoming {
    Connected(u64, Sender<String>),
    Message(u64, String),
    Disconnected(u64),
}

pub struct Session {
    tx: Sender<String>,
    pub plugin: Option<(String, String)>,
    pub events: HashMap<String, Value>,
    pending_prompt: bool,
    pub ev: crate::api_events::EvState,
}

pub struct ApiServer {
    rx: Receiver<Incoming>,
    pub sessions: HashMap<u64, Session>,
    stop: Arc<AtomicBool>,
    pub port: u16,
    pub requests: u64,
    pub instance_id: String,
    pub started: Instant,
    last_test: Instant,
    last_moved: Option<(f32, f32, f32, f32)>,
    last_moved_sent: Instant,
    last_tracking: Option<(bool, bool, bool)>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn random_hex(n: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut s = String::new();
    while s.len() < n {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_usize(s.len());
        s.push_str(&format!("{:016x}", h.finish()));
    }
    s.truncate(n);
    s
}

fn envelope(request_id: &str, message_type: &str, data: Value) -> String {
    json!({
        "apiName": API_NAME,
        "apiVersion": "1.0",
        "timestamp": now_ms(),
        "messageType": message_type,
        "requestID": request_id,
        "data": data,
    })
    .to_string()
}

fn api_error(request_id: &str, id: i32, msg: &str) -> String {
    envelope(request_id, "APIError", json!({ "errorID": id, "message": msg }))
}

impl ApiServer {
    pub fn start(port: u16, lan: bool) -> anyhow::Result<ApiServer> {
        let addr = if lan { "0.0.0.0" } else { "127.0.0.1" };
        let listener = TcpListener::bind((addr, port)).map_err(|e| anyhow::anyhow!("порт {port} занят ({e})"))?;
        listener.set_nonblocking(true)?;
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let instance_id = random_hex(32);

        // Accept loop: one thread per plugin connection.
        {
            let (tx, stop) = (tx.clone(), stop.clone());
            std::thread::Builder::new().name("api-accept".into()).spawn(move || {
                let mut next = 1u64;
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let id = next;
                            next += 1;
                            let (tx, stop) = (tx.clone(), stop.clone());
                            let _ = std::thread::Builder::new().name(format!("api-{id}")).spawn(move || serve(id, stream, tx, stop));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(50)),
                    }
                }
            })?;
        }
        // UDP discovery broadcast, every 2 seconds.
        {
            let (stop, inst) = (stop.clone(), instance_id.clone());
            std::thread::Builder::new().name("api-discovery".into()).spawn(move || {
                let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)) else { return };
                let _ = sock.set_broadcast(true);
                while !stop.load(Ordering::Relaxed) {
                    let msg = json!({
                        "apiName": API_NAME, "apiVersion": "1.0", "timestamp": now_ms(),
                        "messageType": "VTubeStudioAPIStateBroadcast", "requestID": "VTubeStudioAPIStateBroadcast",
                        "data": { "active": true, "port": port, "instanceID": inst, "windowTitle": "rigrs" }
                    })
                    .to_string();
                    let _ = sock.send_to(msg.as_bytes(), ("255.255.255.255", 47779));
                    for _ in 0..20 {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            })?;
        }
        Ok(ApiServer {
            rx,
            sessions: HashMap::new(),
            stop,
            port,
            requests: 0,
            instance_id,
            started: Instant::now(),
            last_test: Instant::now(),
            last_moved: None,
            last_moved_sent: Instant::now(),
            last_tracking: None,
        })
    }

    pub fn authenticated_count(&self) -> usize {
        self.sessions.values().filter(|s| s.plugin.is_some()).count()
    }

    fn emit(&mut self, name: &str, data: &Value) {
        for s in self.sessions.values() {
            let Some(cfg) = s.events.get(name) else { continue };
            if !event_passes(name, cfg, data) {
                continue;
            }
            let _ = s.tx.send(envelope(&random_hex(16), name, data.clone()));
        }
    }

    fn subscribed(&self, name: &str) -> bool {
        self.sessions.values().any(|s| s.events.contains_key(name))
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn event_passes(name: &str, cfg: &Value, data: &Value) -> bool {
    match name {
        "ModelLoadedEvent" => match cfg.get("modelID").and_then(|v| v.as_array()) {
            Some(ids) if !ids.is_empty() => ids.iter().any(|i| i.as_str() == data["modelID"].as_str()),
            _ => true,
        },
        "HotkeyTriggeredEvent" => {
            if cfg["ignoreHotkeysTriggeredByAPI"].as_bool() == Some(true) && data["hotkeyTriggeredByAPI"].as_bool() == Some(true) {
                return false;
            }
            match cfg["onlyForAction"].as_str() {
                Some(a) if !a.is_empty() => data["hotkeyAction"].as_str() == Some(a),
                _ => true,
            }
        }
        "ModelClickedEvent" => !(cfg["onlyClicksOnModel"].as_bool() == Some(true) && data["modelWasClicked"].as_bool() != Some(true)),
        "ModelAnimationEvent" => {
            !(cfg["ignoreLive2DItems"].as_bool() == Some(true) && data["isLive2DItem"].as_bool() == Some(true))
                && !(cfg["ignoreIdleAnimations"].as_bool() == Some(true) && data["isIdleAnimation"].as_bool() == Some(true))
        }
        "ItemEvent" => {
            let by_id = cfg["itemInstanceIDs"].as_array().filter(|a| !a.is_empty());
            let by_file = cfg["itemFileNames"].as_array().filter(|a| !a.is_empty());
            by_id.map(|a| a.iter().any(|i| i.as_str() == data["itemInstanceID"].as_str())).unwrap_or(true)
                && by_file
                    .map(|a| a.iter().any(|f| f.as_str().map(|f| data["itemFileName"].as_str().unwrap_or("").contains(f)).unwrap_or(false)))
                    .unwrap_or(true)
        }
        _ => true,
    }
}

/// One plugin connection: WebSocket in, text frames both ways.
fn serve(id: u64, stream: std::net::TcpStream, tx: Sender<Incoming>, stop: Arc<AtomicBool>) {
    let _ = stream.set_nonblocking(false);
    let Ok(mut ws) = tungstenite::accept(stream) else { return };
    let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(15)));
    let (out_tx, out_rx) = channel::<String>();
    if tx.send(Incoming::Connected(id, out_tx)).is_err() {
        return;
    }
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = ws.close(None);
            break;
        }
        match ws.read() {
            Ok(tungstenite::Message::Text(t)) => {
                let _ = tx.send(Incoming::Message(id, t.to_string()));
            }
            Ok(tungstenite::Message::Binary(b)) => {
                if let Ok(t) = String::from_utf8(b.to_vec()) {
                    let _ = tx.send(Incoming::Message(id, t));
                }
            }
            Ok(tungstenite::Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        while let Ok(msg) = out_rx.try_recv() {
            if ws.send(tungstenite::Message::Text(msg.into())).is_err() {
                let _ = tx.send(Incoming::Disconnected(id));
                return;
            }
        }
    }
    let _ = tx.send(Incoming::Disconnected(id));
}

// ------------------------------------------------------------------ main-thread side

/// Called every frame: handles plugin messages, animations started by plugins and events.
pub fn pump(app: &mut App, dt: f32) {
    let Some(mut srv) = app.api.take() else { return };
    while let Ok(m) = srv.rx.try_recv() {
        match m {
            Incoming::Connected(id, tx) => {
                srv.sessions.insert(id, Session { tx, plugin: None, events: HashMap::new(), pending_prompt: false, ev: Default::default() });
            }
            Incoming::Disconnected(id) => {
                srv.sessions.remove(&id);
                app.api_prompts.retain(|p| p.session != id);
                // Unload items that asked to disappear with their plugin.
                let gone: Vec<u64> = app.api_item_owner.iter().filter(|(_, o)| o.0 == id && o.1).map(|(k, _)| *k).collect();
                for g in gone {
                    app.items.retain(|i| i.id != g);
                    app.api_item_owner.remove(&g);
                }
            }
            Incoming::Message(id, text) => {
                srv.requests += 1;
                let reply = handle_message(app, &mut srv, id, &text);
                if let (Some(r), Some(s)) = (reply, srv.sessions.get(&id)) {
                    let _ = s.tx.send(r);
                }
            }
        }
    }
    tick_animations(app, dt);
    // Events queued by the rest of the app.
    let queued: Vec<(String, Value)> = std::mem::take(&mut app.api_events);
    for (name, data) in queued {
        srv.emit(&name, &data);
    }
    // Periodic events.
    if srv.last_test.elapsed() >= Duration::from_secs(1) {
        srv.last_test = Instant::now();
        let counter = srv.started.elapsed().as_secs();
        for s in srv.sessions.values() {
            if let Some(cfg) = s.events.get("TestEvent") {
                let msg = cfg["testMessageForEvent"].as_str().unwrap_or("Hello from rigrs!");
                let _ = s.tx.send(envelope(&random_hex(16), "TestEvent", json!({ "yourTestMessage": msg, "counter": counter })));
            }
        }
    }
    let ids: Vec<u64> = srv.sessions.keys().copied().collect();
    for id in ids {
        let Some(s) = srv.sessions.get_mut(&id) else { continue };
        let evs = s.events.clone();
        if evs.is_empty() {
            continue;
        }
        let out = crate::api_events::tick(app, &evs, &mut s.ev);
        for (name, data) in out {
            let _ = s.tx.send(envelope(&random_hex(16), name, data));
        }
    }
    if srv.subscribed("ModelMovedEvent") {
        if let Some(pos) = model_position(app) {
            let key = (pos["positionX"].as_f64().unwrap_or(0.0) as f32, pos["positionY"].as_f64().unwrap_or(0.0) as f32, pos["size"].as_f64().unwrap_or(0.0) as f32, pos["rotation"].as_f64().unwrap_or(0.0) as f32);
            if srv.last_moved != Some(key) && srv.last_moved_sent.elapsed() >= Duration::from_millis(100) {
                srv.last_moved = Some(key);
                srv.last_moved_sent = Instant::now();
                let a = app.avatar.as_ref().unwrap();
                srv.emit("ModelMovedEvent", &json!({ "modelID": a.model_id, "modelName": a.name, "modelPosition": pos }));
            }
        }
    }
    let hands = app.last_hands.map(|h| (h.hands[0].is_some(), h.hands[1].is_some())).unwrap_or((false, false));
    let st = (app.tracking, hands.0, hands.1);
    if srv.last_tracking != Some(st) {
        srv.last_tracking = Some(st);
        srv.emit("TrackingStatusChangedEvent", &json!({ "faceFound": st.0, "leftHandFound": st.1, "rightHandFound": st.2 }));
    }
    app.api = Some(srv);
}

/// Sends a response to one plugin session (deferred replies).
pub fn reply(app: &App, session: u64, request_id: &str, message_type: &str, data: Value) {
    if let Some(s) = app.api.as_ref().and_then(|srv| srv.sessions.get(&session)) {
        let _ = s.tx.send(envelope(request_id, message_type, data));
    }
}

/// Answers a prompt shown to the user (token or permission).
pub fn resolve_prompt(app: &mut App, allow: bool) {
    let Some(p) = app.api_prompts.pop_front() else { return };
    if app.api.as_ref().map(|s| !s.sessions.contains_key(&p.session)).unwrap_or(true) {
        return;
    }
    let reply = match p.kind {
        PromptKind::Token => {
            if allow {
                let token = app
                    .settings
                    .api_tokens
                    .iter()
                    .find(|t| t.plugin == p.plugin && t.developer == p.developer)
                    .map(|t| t.token.clone())
                    .unwrap_or_else(|| {
                        let t = random_hex(64);
                        app.settings.api_tokens.push(ApiToken { plugin: p.plugin.clone(), developer: p.developer.clone(), token: t.clone() });
                        t
                    });
                envelope(&p.request_id, "AuthenticationTokenResponse", json!({ "authenticationToken": token }))
            } else {
                api_error(&p.request_id, 50, "User has denied API access for your plugin.")
            }
        }
        PromptKind::Permission(perm) => {
            let key = format!("{}|{}|{}", p.plugin, p.developer, perm);
            if allow && !app.settings.api_permissions.contains(&key) {
                app.settings.api_permissions.push(key);
            }
            envelope(
                &p.request_id,
                "PermissionResponse",
                json!({ "grantSuccess": allow, "requestedPermission": perm, "permissions": permissions_of(app, &p.plugin, &p.developer) }),
            )
        }
    };
    if let Some(s) = app.api.as_mut().and_then(|srv| srv.sessions.get_mut(&p.session)) {
        s.pending_prompt = false;
        let _ = s.tx.send(reply);
    }
    app.save();
}

const PERMISSIONS: [&str; 1] = ["LoadCustomImagesAsItems"];

fn permissions_of(app: &App, plugin: &str, dev: &str) -> Value {
    Value::Array(
        PERMISSIONS
            .iter()
            .map(|p| json!({ "name": p, "granted": app.settings.api_permissions.contains(&format!("{plugin}|{dev}|{p}")) }))
            .collect(),
    )
}

type Res = Result<Value, (i32, String)>;

fn err<T>(id: i32, msg: impl Into<String>) -> Result<T, (i32, String)> {
    Err((id, msg.into()))
}

fn handle_message(app: &mut App, srv: &mut ApiServer, sid: u64, text: &str) -> Option<String> {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Some(api_error("", 2, "JSON is invalid.")),
    };
    let rid = v["requestID"].as_str().map(str::to_string).unwrap_or_else(|| random_hex(32));
    if v["apiName"].as_str() != Some(API_NAME) {
        return Some(api_error(&rid, 3, "API name is invalid."));
    }
    if v["apiVersion"].as_str() != Some("1.0") {
        return Some(api_error(&rid, 4, "API version is invalid."));
    }
    if rid.is_empty() || rid.len() > 64 || !rid.is_ascii() {
        return Some(api_error(&rid, 5, "Request ID is invalid."));
    }
    let mt = v["messageType"].as_str().unwrap_or("").to_string();
    if mt.is_empty() {
        return Some(api_error(&rid, 6, "Request type is missing or empty."));
    }
    let data = if v["data"].is_object() { v["data"].clone() } else { json!({}) };
    let authed = srv.sessions.get(&sid).map(|s| s.plugin.is_some()).unwrap_or(false);

    match mt.as_str() {
        "APIStateRequest" => {
            return Some(envelope(&rid, "APIStateResponse", json!({ "active": true, "vTubeStudioVersion": VERSION, "currentSessionAuthenticated": authed })));
        }
        "AuthenticationTokenRequest" => {
            let plugin = data["pluginName"].as_str().unwrap_or("").to_string();
            let dev = data["pluginDeveloper"].as_str().unwrap_or("").to_string();
            if !(3..=32).contains(&plugin.chars().count()) {
                return Some(api_error(&rid, 52, "Plugin name is invalid (3-32 characters)."));
            }
            if !(3..=32).contains(&dev.chars().count()) {
                return Some(api_error(&rid, 53, "Plugin developer name is invalid (3-32 characters)."));
            }
            let s = srv.sessions.get_mut(&sid)?;
            if s.pending_prompt {
                return Some(api_error(&rid, 51, "A token request is currently ongoing."));
            }
            s.pending_prompt = true;
            let icon_png = data["pluginIcon"].as_str().and_then(|b| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(b).ok()
            });
            app.api_prompts.push_back(ApiPrompt { session: sid, request_id: rid, plugin, developer: dev, icon_png, kind: PromptKind::Token });
            return None; // answered when the user decides
        }
        "AuthenticationRequest" => {
            let token = data["authenticationToken"].as_str().unwrap_or("");
            let plugin = data["pluginName"].as_str().unwrap_or("");
            let dev = data["pluginDeveloper"].as_str().unwrap_or("");
            if token.is_empty() {
                return Some(api_error(&rid, 100, "Authentication token is missing."));
            }
            if plugin.is_empty() {
                return Some(api_error(&rid, 101, "Plugin name is missing."));
            }
            if dev.is_empty() {
                return Some(api_error(&rid, 102, "Plugin developer is missing."));
            }
            let ok = app.settings.api_tokens.iter().any(|t| t.token == token && t.plugin == plugin && t.developer == dev);
            if ok {
                if let Some(s) = srv.sessions.get_mut(&sid) {
                    s.plugin = Some((plugin.to_string(), dev.to_string()));
                }
            }
            let reason = if ok { "Token valid. The plugin is authenticated for the duration of this session." } else { "Token invalid. The plugin is not authenticated." };
            return Some(envelope(&rid, "AuthenticationResponse", json!({ "authenticated": ok, "reason": reason })));
        }
        _ => {}
    }
    if !authed {
        return Some(api_error(&rid, 8, "This request requires authentication."));
    }
    let (plugin, dev) = srv.sessions[&sid].plugin.clone().unwrap();

    let result: Res = match mt.as_str() {
        "StatisticsRequest" => Ok(json!({
            "uptime": srv.started.elapsed().as_millis() as u64,
            "framerate": app.stats.fps.round() as i64,
            "vTubeStudioVersion": VERSION,
            "allowedPlugins": app.settings.api_tokens.len(),
            "connectedPlugins": srv.authenticated_count(),
            "startedWithSteam": false,
            "windowWidth": app.gpu.config.width,
            "windowHeight": app.gpu.config.height,
            "windowIsFullscreen": app.window.fullscreen().is_some(),
        })),
        "VTSFolderInfoRequest" => Ok(json!({
            "models": "Live2DModels", "backgrounds": "Backgrounds", "items": "Items",
            "config": "Config", "logs": "Logs", "backup": "Backup",
        })),
        "CurrentModelRequest" => Ok(current_model(app)),
        "AvailableModelsRequest" => {
            let cur = app.avatar.as_ref().map(|a| a.model3.clone());
            let list: Vec<Value> = app
                .library
                .models
                .iter()
                .map(|m| {
                    json!({
                        "modelLoaded": cur.as_ref() == Some(&m.model3),
                        "modelName": m.name,
                        "modelID": model_id_of(&m.model3, m.vtube.as_deref()),
                        "vtsModelName": m.vtube.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        "vtsModelIconName": m.icon.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    })
                })
                .collect();
            Ok(json!({ "numberOfModels": list.len(), "availableModels": list }))
        }
        "ModelLoadRequest" => {
            let id = data["modelID"].as_str().unwrap_or("");
            if id.is_empty() {
                app.actions.push(Action::UnloadModel);
                Ok(json!({ "modelID": "" }))
            } else {
                match app.library.models.iter().find(|m| model_id_of(&m.model3, m.vtube.as_deref()) == id).cloned() {
                    Some(m) => {
                        app.load_model(&m);
                        Ok(json!({ "modelID": id }))
                    }
                    None => err(152, "Model with this ID was not found."),
                }
            }
        }
        "MoveModelRequest" => move_model(app, &data),
        "HotkeysInCurrentModelRequest" => Ok(hotkey_list(app)),
        "HotkeyTriggerRequest" => {
            let id = data["hotkeyID"].as_str().unwrap_or("").to_string();
            trigger_hotkey(app, &id).map(|_| json!({ "hotkeyID": id }))
        }
        "ExpressionStateRequest" => expression_state(app, &data),
        "ExpressionActivationRequest" => {
            let file = data["expressionFile"].as_str().unwrap_or("");
            let active = data["active"].as_bool().unwrap_or(true);
            if !file.ends_with(".exp3.json") {
                err(650, "Expression file name is invalid.")
            } else if let Some(a) = app.avatar.as_mut() {
                match a.expressions.iter_mut().find(|e| e.file.ends_with(file)) {
                    Some(e) => {
                        e.active = active;
                        let ev = json!({ "modelID": a.model_id, "modelName": a.name, "isLive2DItem": false, "itemInstanceID": "", "justLoaded": false, "expressionFile": e.file, "expressionName": e.name, "active": active });
                        app.api_events.push(("ExpressionToggledEvent".into(), ev));
                        Ok(json!({}))
                    }
                    None => err(651, "Expression file was not found in the model."),
                }
            } else {
                err(652, "No model loaded.")
            }
        }
        "ArtMeshListRequest" => Ok(match app.avatar.as_ref() {
            Some(a) => {
                let names: Vec<&str> = a.model.drawables.iter().map(|d| d.id.as_str()).collect();
                json!({ "modelLoaded": true, "numberOfArtMeshNames": names.len(), "numberOfArtMeshTags": 0, "artMeshNames": names, "artMeshTags": [], "numberOfArtMeshGroups": 0, "artMeshGroups": [] })
            }
            None => json!({ "modelLoaded": false, "numberOfArtMeshNames": 0, "numberOfArtMeshTags": 0, "artMeshNames": [], "artMeshTags": [], "numberOfArtMeshGroups": 0, "artMeshGroups": [] }),
        }),
        "ColorTintRequest" => color_tint(app, &data),
        "ArtMeshAtPositionRequest" => {
            let (x, y) = (data["x"].as_f64().unwrap_or(0.0) as f32, data["y"].as_f64().unwrap_or(0.0) as f32);
            Ok(art_mesh_hits(app, x, y))
        }
        "SceneColorOverlayInfoRequest" => {
            let st = &app.settings;
            let c = app.light_tint;
            let to = |v: f32| (v * 255.0).round() as i64;
            Ok(json!({
                "active": st.light_mode != 0, "itemsIncluded": st.light_items, "isWindowCapture": st.light_mode == 2,
                "baseBrightness": st.light_brightness as i64, "colorBoost": st.light_overlay as i64, "smoothing": st.light_smoothing as i64,
                "colorOverlayR": to(c[0]), "colorOverlayG": to(c[1]), "colorOverlayB": to(c[2]),
                "colorAvgR": to(app.light_sample.color[0]), "colorAvgG": to(app.light_sample.color[1]), "colorAvgB": to(app.light_sample.color[2]),
                "leftCapturePart": { "active": false, "colorR": 0, "colorG": 0, "colorB": 0 },
                "middleCapturePart": { "active": st.light_mode != 0, "colorR": to(app.light_sample.color[0]), "colorG": to(app.light_sample.color[1]), "colorB": to(app.light_sample.color[2]) },
                "rightCapturePart": { "active": false, "colorR": 0, "colorG": 0, "colorB": 0 },
            }))
        }
        "FaceFoundRequest" => Ok(json!({ "found": app.tracking })),
        "InputParameterListRequest" => Ok(input_parameters(app)),
        "ParameterValueRequest" => {
            let name = data["name"].as_str().unwrap_or("");
            match find_input(app, name) {
                Some(p) => Ok(p),
                None => err(500, "Parameter not found."),
            }
        }
        "Live2DParameterListRequest" => Ok(match app.avatar.as_ref() {
            Some(a) => {
                let vals = a.model.params();
                let list: Vec<Value> = (0..a.model.param_ids.len())
                    .map(|i| json!({ "name": a.model.param_ids[i], "value": vals[i], "min": a.model.param_min[i], "max": a.model.param_max[i], "defaultValue": a.model.param_default[i] }))
                    .collect();
                json!({ "modelLoaded": true, "modelName": a.name, "modelID": a.model_id, "parameters": list })
            }
            None => json!({ "modelLoaded": false, "modelName": "", "modelID": "", "parameters": [] }),
        }),
        "ParameterCreationRequest" => create_parameter(app, &plugin, &data),
        "ParameterDeletionRequest" => {
            let name = data["parameterName"].as_str().unwrap_or("");
            if INPUT_NAMES.contains(&name) {
                err(403, "Default parameters cannot be deleted.")
            } else {
                match app.api_custom.get(name) {
                    None => err(401, "Parameter not found."),
                    Some(p) if p.added_by != plugin => err(402, "Parameter was created by another plugin."),
                    Some(_) => {
                        app.api_custom.remove(name);
                        crate::mapping::remove_custom_input(name);
                        Ok(json!({ "parameterName": name }))
                    }
                }
            }
        }
        "InjectParameterDataRequest" => inject(app, &plugin, &data),
        "GetCurrentModelPhysicsRequest" => Ok(match app.avatar.as_ref() {
            Some(a) => json!({
                "modelLoaded": true, "modelName": a.name, "modelID": a.model_id,
                "modelHasPhysics": a.has_physics(), "physicsSwitchedOn": a.cfg.physics_on && app.settings.physics,
                "usingLegacyPhysics": false,
                "physicsFPSSetting": match a.cfg.physics_fps { 30 => 30, 60 => 60, 120 => 120, _ => -1 },
                "baseStrength": a.cfg.physics_strength as i64, "baseWind": 0,
                "apiPhysicsOverrideActive": app.api_physics.is_some(),
                "apiPhysicsOverridePluginName": app.api_physics.as_ref().map(|p| p.2.clone()).unwrap_or_default(),
                "physicsGroups": (0..a.physics_chains()).map(|i| json!({ "groupID": format!("PhysicsSetting{}", i + 1), "groupName": format!("Physics {}", i + 1), "strengthMultiplier": 1.0, "windMultiplier": 1.0 })).collect::<Vec<_>>(),
            }),
            None => json!({ "modelLoaded": false, "modelName": "", "modelID": "", "modelHasPhysics": false, "physicsSwitchedOn": false, "usingLegacyPhysics": false, "physicsFPSSetting": -1, "baseStrength": 50, "baseWind": 0, "apiPhysicsOverrideActive": false, "apiPhysicsOverridePluginName": "", "physicsGroups": [] }),
        }),
        "SetCurrentModelPhysicsRequest" => {
            if app.avatar.is_none() {
                err(700, "No model loaded.")
            } else if app.api_physics.as_ref().map(|p| p.2 != plugin && p.1 > Instant::now()).unwrap_or(false) {
                err(702, "Physics are controlled by another plugin.")
            } else {
                let o = data["strengthOverrides"].as_array().and_then(|a| a.first()).cloned();
                match o {
                    None => err(703, "No overrides provided."),
                    Some(o) => {
                        let v = o["value"].as_f64().unwrap_or(1.0) as f32;
                        let secs = (o["overrideSeconds"].as_f64().unwrap_or(1.0) as f32).clamp(0.5, 5.0);
                        // setBaseValue: 0..100 strength, otherwise a multiplier of the current strength.
                        let base = app.avatar.as_ref().map(|a| a.cfg.physics_strength).unwrap_or(50.0);
                        let strength = if o["setBaseValue"].as_bool() == Some(true) { v } else { base * v };
                        app.api_physics = Some((strength.clamp(0.0, 100.0), Instant::now() + Duration::from_secs_f32(secs), plugin.clone()));
                        Ok(json!({}))
                    }
                }
            }
        }
        "NDIConfigRequest" => {
            if data["setNewConfig"].as_bool() == Some(true) {
                app.settings.ndi_custom_size = data["useCustomResolution"].as_bool().unwrap_or(false);
                let w = data["customWidthNDI"].as_i64().unwrap_or(-1);
                let h = data["customHeightNDI"].as_i64().unwrap_or(-1);
                if w > 0 {
                    app.settings.ndi_size[0] = w as u32;
                }
                if h > 0 {
                    app.settings.ndi_size[1] = h as u32;
                }
                app.actions.push(Action::SetNdi(data["ndiActive"].as_bool().unwrap_or(false)));
            }
            let s = &app.settings;
            Ok(json!({ "setNewConfig": false, "ndiActive": s.ndi, "useNDI5": true, "useCustomResolution": s.ndi_custom_size, "customWidthNDI": s.ndi_size[0], "customHeightNDI": s.ndi_size[1] }))
        }
        "ItemListRequest" => Ok(item_list(app, &data)),
        "ItemLoadRequest" => item_load(app, sid, &plugin, &dev, &data),
        "ItemUnloadRequest" => Ok(item_unload(app, sid, &data)),
        "ItemMoveRequest" => Ok(item_move(app, &data)),
        "ItemAnimationControlRequest" => {
            let id = data["itemInstanceID"].as_str().unwrap_or("");
            match find_item(app, id) {
                None => err(850, "Item instance not found."),
                Some(k) => {
                    let b = data["brightness"].as_f64().unwrap_or(-1.0) as f32;
                    if b >= 0.0 {
                        let it = &mut app.items[k];
                        it.set_tint(&app.gpu, [b, b, b]);
                    }
                    Ok(json!({ "frame": 0, "animationPlaying": true }))
                }
            }
        }
        "ItemPinRequest" => item_pin(app, &data),
        "ItemSortRequest" => {
            let id = data["itemInstanceID"].as_str().unwrap_or("");
            match find_item(app, id) {
                None => err(1250, "Item instance not found."),
                Some(k) => item_sort(app, k, id, &data),
            }
        }
        "PostProcessingListRequest" => post_processing_list(app, &data),
        "PostProcessingUpdateRequest" => post_processing_update(app, &data),
        "ArtMeshSelectionRequest" => {
            let active: Vec<String> = data["activeArtMeshes"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
            match app.avatar.as_ref() {
                None => err(1000, "No model loaded."),
                Some(_) if app.mesh_select.is_some() || !app.api_prompts.is_empty() || app.ui.modal.is_some() => err(1001, "Other windows are open."),
                Some(a) if active.len() > a.model.drawables.len() => err(1003, "ArtMesh ID list too long."),
                Some(a) if active.iter().any(|id| a.drawable_index(id).is_none()) => err(1002, "Model does not have this ArtMesh."),
                Some(_) => {
                    let txt = |k: &str, d: &str| data[k].as_str().filter(|s| (4..=1024).contains(&s.chars().count())).unwrap_or(d).to_string();
                    app.mesh_select = Some(crate::api_events::MeshSelect {
                        session: sid,
                        request_id: rid.clone(),
                        text: txt("textOverride", "Плагин «{plugin}» просит выбрать части модели (ArtMesh)."),
                        help: txt("helpOverride", "Отметьте нужные части модели и нажмите OK. Клик по модели фильтрует список по частям под курсором."),
                        count: data["requestedArtMeshCount"].as_i64().unwrap_or(0),
                        active: active.into_iter().collect(),
                        filter: String::new(),
                        at_click: None,
                        show_help: false,
                    });
                    if let Some(m) = app.mesh_select.as_mut() {
                        m.text = m.text.replace("{plugin}", &plugin);
                    }
                    return None;
                }
            }
        }
        "PermissionRequest" => {
            let perm = data["requestedPermission"].as_str().unwrap_or("").to_string();
            if !PERMISSIONS.contains(&perm.as_str()) {
                err(1100, "Unknown permission.")
            } else if app.settings.api_permissions.contains(&format!("{plugin}|{dev}|{perm}")) {
                Ok(json!({ "grantSuccess": true, "requestedPermission": perm, "permissions": permissions_of(app, &plugin, &dev) }))
            } else {
                app.api_prompts.push_back(ApiPrompt { session: sid, request_id: rid.clone(), plugin, developer: dev, icon_png: None, kind: PromptKind::Permission(perm) });
                return None;
            }
        }
        "EventSubscriptionRequest" => {
            const EVENTS: [&str; 16] = [
                "TestEvent", "ModelLoadedEvent", "TrackingStatusChangedEvent", "BackgroundChangedEvent", "ModelConfigChangedEvent", "ModelMovedEvent",
                "HotkeyTriggeredEvent", "ExpressionToggledEvent", "ItemEvent", "ModelClickedEvent", "PostProcessingEvent", "ModelAnimationEvent",
                "ModelOutlineEvent", "Live2DCubismEditorConnectedEvent", "ArtMeshTrackingEvent", "ArtMeshOutlineEvent",
            ];
            let name = data["eventName"].as_str().unwrap_or("");
            let sub = data["subscribe"].as_bool().unwrap_or(true);
            let s = srv.sessions.get_mut(&sid).unwrap();
            if !sub && name.is_empty() {
                s.events.clear();
                Ok(json!({ "subscribedEventCount": 0, "subscribedEvents": [] }))
            } else if !EVENTS.contains(&name) {
                err(950, "Unknown event type.")
            } else if name == "TestEvent" && data["config"]["testMessageForEvent"].as_str().map(|m| m.chars().count() > 32).unwrap_or(false) {
                err(100000, "Test message too long.")
            } else if let (true, Err((code, msg))) = (sub, crate::api_events::validate(name, &data["config"])) {
                Err((code, msg))
            } else {
                if sub {
                    s.events.insert(name.to_string(), data["config"].clone());
                    // Re-subscribing resets the event counter (VTS).
                    s.ev.counter.remove(name);
                    s.ev.sent_unloaded.remove(name);
                    if name == "Live2DCubismEditorConnectedEvent" {
                        // Sent once right after subscribing with the initial state.
                        let st = app.editor_state();
                        let _ = s.tx.send(envelope(&random_hex(16), name, st));
                    }
                } else {
                    s.events.remove(name);
                }
                let list: Vec<&String> = s.events.keys().collect();
                let reply = json!({ "subscribedEventCount": list.len(), "subscribedEvents": list });
                if sub && name == "ModelMovedEvent" {
                    srv.last_moved = None; // send the current position right away
                }
                Ok(reply)
            }
        }
        _ => err(7, "Request type unknown or not supported by rigrs."),
    };
    Some(match result {
        Ok(d) => envelope(&rid, &mt.replace("Request", "Response"), d),
        Err((id, msg)) => api_error(&rid, id, &msg),
    })
}

// ------------------------------------------------------------------ helpers

/// ModelConfigChangedEvent payload.
pub fn model_config_event(app: &App) -> Value {
    let a = app.avatar.as_ref();
    json!({ "modelID": a.map(|a| a.model_id.clone()).unwrap_or_default(), "modelName": a.map(|a| a.name.clone()).unwrap_or_default(), "hotkeyConfigChanged": false })
}

pub fn model_id_of(model3: &std::path::Path, vtube: Option<&std::path::Path>) -> String {
    if let Some(v) = vtube.and_then(|p| crate::formats::read_json::<crate::formats::VtubeJson>(p).ok()) {
        if v.model_id.len() == 32 {
            return v.model_id;
        }
    }
    use std::hash::{Hash, Hasher};
    let mut h1 = std::collections::hash_map::DefaultHasher::new();
    model3.hash(&mut h1);
    let mut h2 = std::collections::hash_map::DefaultHasher::new();
    (model3, 7u8).hash(&mut h2);
    format!("{:016x}{:016x}", h1.finish(), h2.finish())
}

/// Scale at which the model fills the window height (VTS size 0).
fn base_scale(app: &App) -> f32 {
    let (_, vh) = app.viewport();
    app.avatar
        .as_ref()
        .map(|a| {
            let c = &a.model.canvas;
            vh / (c.size.y / c.pixels_per_unit).max(0.01)
        })
        .unwrap_or(1.0)
}

fn model_position(app: &App) -> Option<Value> {
    let a = app.avatar.as_ref()?;
    let (vw, vh) = app.viewport();
    let size = 50.0 * (a.xf.scale / base_scale(app)).max(1e-4).log2();
    Some(json!({
        "positionX": a.xf.pos[0] / (vw / 2.0),
        "positionY": a.xf.pos[1] / (vh / 2.0),
        "rotation": -a.xf.rot,
        "size": size.clamp(-100.0, 100.0),
    }))
}

fn current_model(app: &App) -> Value {
    match app.avatar.as_ref() {
        Some(a) => json!({
            "modelLoaded": true,
            "modelName": a.name,
            "modelID": a.model_id,
            "vtsModelName": a.model3.with_extension("").with_extension("vtube.json").file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            "vtsModelIconName": "",
            "live2DModelName": a.model3.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            "modelLoadTime": a.load_ms,
            "timeSinceModelLoaded": a.load_time.elapsed().as_millis() as u64,
            "numberOfLive2DParameters": a.model.param_ids.len(),
            "numberOfLive2DArtmeshes": a.model.drawables.len(),
            "hasPhysicsFile": a.has_physics(),
            "numberOfTextures": a.texture_count,
            "textureResolution": a.texture_size,
            "modelPosition": model_position(app),
        }),
        None => json!({
            "modelLoaded": false, "modelName": "", "modelID": "", "vtsModelName": "", "vtsModelIconName": "", "live2DModelName": "",
            "modelLoadTime": 0, "timeSinceModelLoaded": 0, "numberOfLive2DParameters": 0, "numberOfLive2DArtmeshes": 0,
            "hasPhysicsFile": false, "numberOfTextures": 0, "textureResolution": 0,
            "modelPosition": { "positionX": 0.0, "positionY": 0.0, "rotation": 0.0, "size": 0.0 },
        }),
    }
}

fn move_model(app: &mut App, d: &Value) -> Res {
    let Some(a) = app.avatar.as_ref() else { return err(300, "No model loaded.") };
    let Some(t) = d["timeInSeconds"].as_f64() else { return err(301, "timeInSeconds is missing.") };
    let Some(rel) = d["valuesAreRelativeToModel"].as_bool() else { return err(301, "valuesAreRelativeToModel is missing.") };
    let (vw, vh) = app.viewport();
    let base = base_scale(app);
    let from = a.xf;
    let mut to = from;
    let f = |k: &str| d[k].as_f64().map(|v| v as f32);
    if let Some(x) = f("positionX") {
        if !(-1000.0..=1000.0).contains(&x) {
            return err(302, "positionX out of range.");
        }
        to.pos[0] = if rel { from.pos[0] + x * vw / 2.0 } else { x * vw / 2.0 };
    }
    if let Some(y) = f("positionY") {
        to.pos[1] = if rel { from.pos[1] + y * vh / 2.0 } else { y * vh / 2.0 };
    }
    if let Some(r) = f("rotation") {
        to.rot = if rel { from.rot - r } else { -r };
    }
    if let Some(s) = f("size") {
        let cur = 50.0 * (from.scale / base).max(1e-4).log2();
        let target = if rel { cur + s } else { s };
        to.scale = base * 2f32.powf(target.clamp(-100.0, 100.0) / 50.0);
    }
    let t = (t as f32).clamp(0.0, 2.0);
    if t <= 0.0 {
        app.avatar.as_mut().unwrap().xf = to;
    } else {
        app.api_model_move = Some((from, to, 0.0, t));
    }
    Ok(json!({}))
}

fn hotkey_list(app: &App) -> Value {
    let list: Vec<Value> = app
        .hotkeys()
        .iter()
        .map(|h| {
            json!({
                "name": h.name, "type": h.action, "description": crate::hotkeys::action_label(&h.action), "file": h.file, "hotkeyID": h.id,
                "keyCombination": h.triggers.list(), "onScreenButtonID": h.triggers.screen_button,
            })
        })
        .collect();
    let a = app.avatar.as_ref();
    json!({
        "modelLoaded": a.is_some(),
        "modelName": a.map(|a| a.name.clone()).unwrap_or_default(),
        "modelID": a.map(|a| a.model_id.clone()).unwrap_or_default(),
        "availableHotkeys": list,
    })
}

/// Runs a hotkey of the current model by ID or name.
pub fn trigger_hotkey(app: &mut App, id_or_name: &str) -> Result<(), (i32, String)> {
    if app.avatar.is_none() {
        return err(201, "No model loaded.");
    }
    if !app.hotkeys().iter().any(|h| h.id == id_or_name || (!h.name.is_empty() && h.name.eq_ignore_ascii_case(id_or_name))) {
        return err(202, "Hotkey not found in model.");
    }
    if crate::hotkeys::trigger(app, id_or_name, crate::hotkeys::Source::Api) {
        Ok(())
    } else {
        err(205, "Hotkey action could not be executed.")
    }
}

fn expression_state(app: &App, d: &Value) -> Res {
    let Some(a) = app.avatar.as_ref() else {
        return Ok(json!({ "modelLoaded": false, "modelName": "", "modelID": "", "expressions": [] }));
    };
    let filter = d["expressionFile"].as_str().unwrap_or("");
    if !filter.is_empty() && !filter.ends_with(".exp3.json") {
        return err(600, "Expression file name is invalid.");
    }
    let list: Vec<Value> = a
        .expressions
        .iter()
        .filter(|e| filter.is_empty() || e.file.ends_with(filter))
        .map(|e| json!({ "name": e.name, "file": e.file, "active": e.active, "deactivateWhenKeyIsLetGo": false, "autoDeactivateAfterSeconds": false, "secondsRemaining": 0, "usedInHotkeys": [], "parameters": [] }))
        .collect();
    if !filter.is_empty() && list.is_empty() {
        return err(601, "Expression file not found.");
    }
    Ok(json!({ "modelLoaded": true, "modelName": a.name, "modelID": a.model_id, "expressions": list }))
}

fn color_tint(app: &mut App, d: &Value) -> Res {
    let Some(a) = app.avatar.as_mut() else { return err(250, "No model loaded.") };
    let c = &d["colorTint"];
    let ch = |k: &str| c[k].as_i64();
    let (Some(r), Some(g), Some(b), Some(al)) = (ch("colorR"), ch("colorG"), ch("colorB"), ch("colorA")) else {
        return err(251, "Color or matcher missing.");
    };
    if [r, g, b, al].iter().any(|v| !(0..=255).contains(v)) {
        return err(252, "Color values must be 0-255.");
    }
    let m = &d["artMeshMatcher"];
    let strs = |k: &str| m[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>()).unwrap_or_default();
    let nums: Vec<i64> = m["artMeshNumber"].as_array().map(|a| a.iter().filter_map(|v| v.as_i64()).collect()).unwrap_or_default();
    let (exact, contains) = (strs("nameExact"), strs("nameContains"));
    let all = m["tintAll"].as_bool().unwrap_or(false);
    let col = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, al as f32 / 255.0];
    let mut n = 0;
    for (i, dr) in a.model.drawables.iter().enumerate() {
        let hit = all || nums.contains(&(i as i64)) || exact.iter().any(|e| *e == dr.id) || contains.iter().any(|c| dr.id.contains(c.as_str()));
        if hit {
            a.art_tint[i] = col;
            n += 1;
        }
    }
    Ok(json!({ "matchedArtMeshes": n }))
}

fn to_world(app: &App, x: f32, y: f32) -> [f32; 2] {
    let (vw, vh) = app.viewport();
    [x * vw / 2.0, y * vh / 2.0]
}

fn art_mesh_hits(app: &App, x: f32, y: f32) -> Value {
    let (vw, vh) = app.viewport();
    let Some(a) = app.avatar.as_ref() else {
        return json!({ "modelLoaded": false, "loadedModelID": "", "loadedModelName": "", "modelWasHit": false, "checkedPosition": { "x": x, "y": y }, "windowSize": { "x": vw, "y": vh }, "artMeshHitCount": 0, "artMeshHits": [] });
    };
    let hits: Vec<Value> = a
        .pick(to_world(app, x, y))
        .into_iter()
        .map(|p| {
            let d = &a.model.drawables[p.drawable];
            json!({ "artMeshOrder": 0, "isMasked": !d.masks.is_empty(), "hitInfo": {
                "modelID": a.model_id, "artMeshID": d.id, "angle": 0.0, "size": 0.33,
                "vertexID1": p.tri[0], "vertexID2": p.tri[1], "vertexID3": p.tri[2],
                "vertexWeight1": p.bary[0], "vertexWeight2": p.bary[1], "vertexWeight3": p.bary[2] } })
        })
        .collect();
    json!({ "modelLoaded": true, "loadedModelID": a.model_id, "loadedModelName": a.name, "modelWasHit": !hits.is_empty(), "checkedPosition": { "x": x, "y": y }, "windowSize": { "x": vw, "y": vh }, "artMeshHitCount": hits.len(), "artMeshHits": hits })
}

/// Rough ranges of the built-in inputs (as VTS reports them).
fn default_range(name: &str) -> (f32, f32, f32) {
    match name {
        n if n.starts_with("FaceAngle") || n.starts_with("HandLeftAngle") || n.starts_with("HandRightAngle") => (-30.0, 30.0, 0.0),
        n if n.starts_with("FacePosition") => (-15.0, 15.0, 0.0),
        "MouthSmile" | "Brows" | "BrowLeftY" | "BrowRightY" => (0.0, 1.0, 0.5),
        "EyeOpenLeft" | "EyeOpenRight" => (0.0, 1.0, 1.0),
        n if n.starts_with("Eye") || n.starts_with("Mouse") || n == "MouthX" || n.contains("Position") => (-1.0, 1.0, 0.0),
        _ => (0.0, 1.0, 0.0),
    }
}

fn input_parameters(app: &App) -> Value {
    let defaults: Vec<Value> = INPUT_NAMES
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (mn, mx, df) = default_range(n);
            json!({ "name": n, "addedBy": "VTube Studio", "value": app.inputs[i], "min": mn, "max": mx, "defaultValue": df })
        })
        .collect();
    let custom: Vec<Value> = app
        .api_custom
        .values()
        .map(|p| json!({ "name": p.name, "addedBy": p.added_by, "value": p.value, "min": p.min, "max": p.max, "defaultValue": p.default }))
        .collect();
    let a = app.avatar.as_ref();
    json!({
        "modelLoaded": a.is_some(), "modelName": a.map(|a| a.name.clone()).unwrap_or_default(), "modelID": a.map(|a| a.model_id.clone()).unwrap_or_default(),
        "customParameters": custom, "defaultParameters": defaults,
    })
}

fn find_input(app: &App, name: &str) -> Option<Value> {
    if let Some(i) = INPUT_NAMES.iter().position(|n| *n == name) {
        let (mn, mx, df) = default_range(name);
        return Some(json!({ "name": name, "addedBy": "VTube Studio", "value": app.inputs[i], "min": mn, "max": mx, "defaultValue": df }));
    }
    app.api_custom.get(name).map(|p| json!({ "name": p.name, "addedBy": p.added_by, "value": p.value, "min": p.min, "max": p.max, "defaultValue": p.default }))
}

fn create_parameter(app: &mut App, plugin: &str, d: &Value) -> Res {
    let name = d["parameterName"].as_str().unwrap_or("");
    if !(4..=32).contains(&name.len()) || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return err(350, "Parameter name must be 4-32 alphanumeric characters.");
    }
    if INPUT_NAMES.contains(&name) {
        return err(354, "This is a default parameter name.");
    }
    let expl = d["explanation"].as_str().unwrap_or("");
    if expl.len() >= 256 {
        return err(353, "Explanation too long.");
    }
    let (mn, mx, df) = (d["min"].as_f64().unwrap_or(0.0) as f32, d["max"].as_f64().unwrap_or(1.0) as f32, d["defaultValue"].as_f64().unwrap_or(0.0) as f32);
    if [mn, mx, df].iter().any(|v| v.abs() > 1_000_000.0) || mn >= mx {
        return err(351, "Parameter values invalid.");
    }
    if let Some(p) = app.api_custom.get(name) {
        if p.added_by != plugin {
            return err(352, "Parameter already created by another plugin.");
        }
    }
    if app.api_custom.values().filter(|p| p.added_by == plugin).count() >= 100 && !app.api_custom.contains_key(name) {
        return err(355, "Too many parameters for this plugin.");
    }
    app.api_custom.insert(name.to_string(), CustomParam { name: name.into(), added_by: plugin.into(), explanation: expl.into(), min: mn, max: mx, default: df, value: df });
    crate::mapping::set_custom_input(name, df);
    Ok(json!({ "parameterName": name }))
}

fn inject(app: &mut App, plugin: &str, d: &Value) -> Res {
    let Some(vals) = d["parameterValues"].as_array().filter(|a| !a.is_empty()) else { return err(450, "No data provided.") };
    let add = match d["mode"].as_str().unwrap_or("set") {
        "set" => false,
        "add" => true,
        _ => return err(455, "Unknown mode."),
    };
    for v in vals {
        let id = v["id"].as_str().unwrap_or("");
        let Some(val) = v["value"].as_f64().map(|x| x as f32) else { return err(451, "Value invalid.") };
        let w = v["weight"].as_f64().map(|x| x as f32).unwrap_or(1.0);
        if !(0.0..=1.0).contains(&w) {
            return err(452, "Weight must be 0-1.");
        }
        if let Some(i) = INPUT_NAMES.iter().position(|n| *n == id) {
            if let Some(o) = app.api_inject.get(&i) {
                if o.3 != plugin && o.2.elapsed() < Duration::from_secs(1) {
                    return err(454, "Parameter is controlled by another plugin.");
                }
            }
            app.api_inject.insert(i, (val, w, Instant::now(), plugin.to_string(), add));
        } else if let Some(p) = app.api_custom.get_mut(id) {
            p.value = if add { p.value + val } else { p.value * (1.0 - w) + val * w };
            crate::mapping::set_custom_input(id, p.value);
        } else {
            return err(453, format!("Parameter {id} not found."));
        }
    }
    if let Some(f) = d["faceFound"].as_bool() {
        app.api_face_found = Some((f, Instant::now()));
    }
    Ok(json!({}))
}

/// Applies values injected by plugins (they stay active for 1 s after the last update).
pub fn apply_injections(app: &mut App) {
    app.api_inject.retain(|_, o| o.2.elapsed() < Duration::from_secs(1));
    for (&i, o) in &app.api_inject {
        let cur = app.inputs[i];
        app.inputs[i] = if o.4 { cur + o.0 * o.1 } else { cur * (1.0 - o.1) + o.0 * o.1 };
    }
    if let Some((f, t)) = app.api_face_found {
        if t.elapsed() < Duration::from_secs(1) {
            if f {
                app.tracking = true;
            }
        } else {
            app.api_face_found = None;
        }
    }
    if let Some((s, until, _)) = &app.api_physics {
        if Instant::now() > *until {
            app.api_physics = None;
        } else if let Some(a) = app.avatar.as_mut() {
            a.api_strength = Some(*s);
        }
    }
    if app.api_physics.is_none() {
        if let Some(a) = app.avatar.as_mut() {
            a.api_strength = None;
        }
    }
}

fn instance_id(id: u64) -> String {
    format!("{:032x}", id)
}

fn find_item(app: &App, iid: &str) -> Option<usize> {
    let id = u64::from_str_radix(iid.trim_start_matches('0').trim(), 16).ok().or_else(|| (iid.chars().all(|c| c == '0') && !iid.is_empty()).then_some(0))?;
    app.items.iter().position(|i| i.id == id)
}

fn item_file_name(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    if s.ends_with(".model3.json") {
        path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

fn item_type(path: &std::path::Path) -> &'static str {
    let s = path.to_string_lossy().to_lowercase();
    if s.ends_with(".model3.json") {
        "Live2D"
    } else if s.ends_with(".png") {
        "PNG"
    } else if s.ends_with(".jpg") || s.ends_with(".jpeg") {
        "JPG"
    } else if s.ends_with(".gif") {
        "GIF"
    } else {
        "Unknown"
    }
}

fn item_list(app: &App, d: &Value) -> Value {
    let only_file = d["onlyItemsWithFileName"].as_str().unwrap_or("");
    let only_id = d["onlyItemsWithInstanceID"].as_str().unwrap_or("");
    let (vw, vh) = app.viewport();
    let _ = (vw, vh);
    let in_scene: Vec<Value> = if d["includeItemInstancesInScene"].as_bool().unwrap_or(false) {
        app.items
            .iter()
            .enumerate()
            .filter(|(_, i)| only_id.is_empty() || instance_id(i.id) == only_id)
            .filter(|(_, i)| only_file.is_empty() || item_file_name(&i.source).contains(only_file))
            .map(|(_, i)| {
                json!({
                    "fileName": item_file_name(&i.source), "instanceID": instance_id(i.id), "order": i.cfg.order,
                    "type": item_type(&i.source), "censored": i.cfg.censor, "flipped": i.xf.flip, "locked": i.locked, "smoothing": i.cfg.smoothing,
                    "framerate": frames_of(i).0, "frameCount": frames_of(i).1, "currentFrame": frames_of(i).2, "pinnedToModel": i.pin.is_some(),
                    "pinnedModelID": if i.pin.is_some() { app.avatar.as_ref().map(|a| a.model_id.clone()).unwrap_or_default() } else { String::new() },
                    "pinnedArtMeshID": i.pin.as_ref().and_then(|p| app.avatar.as_ref().and_then(|a| a.model.drawables.get(p.drawable)).map(|d| d.id.clone())).unwrap_or_default(),
                    "groupName": "", "sceneName": "", "fromWorkshop": false,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let files: Vec<Value> = if d["includeAvailableItemFiles"].as_bool().unwrap_or(false) {
        app.library
            .items
            .iter()
            .filter(|e| only_file.is_empty() || item_file_name(e.path()).contains(only_file))
            .map(|e| {
                let n = app.items.iter().filter(|i| i.source == e.path()).count();
                json!({ "fileName": item_file_name(e.path()), "type": item_type(e.path()), "loadedCount": n })
            })
            .collect()
    } else {
        Vec::new()
    };
    let spots: Vec<i64> = if d["includeAvailableSpots"].as_bool().unwrap_or(false) { (-30..=30).filter(|o| *o != 0).collect() } else { Vec::new() };
    json!({
        "itemsInSceneCount": app.items.len(), "totalItemsAllowedCount": 120, "canLoadItemsRightNow": app.items.len() < 120,
        "availableSpots": spots, "itemInstancesInScene": in_scene, "availableItemFiles": files,
    })
}

fn item_height_px(it: &crate::scene::Item) -> f32 {
    match &it.body {
        Body::Sprite(s) => s.size[1] * it.xf.scale,
        Body::Live2D(l) => {
            let b = l.local_bounds();
            (b[3] - b[1]) * it.xf.scale
        }
    }
}

fn item_load(app: &mut App, sid: u64, plugin: &str, dev: &str, d: &Value) -> Res {
    let file = d["fileName"].as_str().unwrap_or("").to_string();
    if file.is_empty() {
        return err(750, "fileName missing.");
    }
    if app.items.len() >= 120 {
        return err(754, "Scene is full.");
    }
    let path = if let Some(b64) = d["customDataBase64"].as_str().filter(|s| !s.is_empty()) {
        if !app.settings.api_permissions.contains(&format!("{plugin}|{dev}|LoadCustomImagesAsItems")) {
            return err(9, "This request requires the LoadCustomImagesAsItems permission.");
        }
        use base64::Engine;
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { return err(758, "Custom data invalid.") };
        let dir = app.paths.data.join("Items").join("_api");
        let _ = std::fs::create_dir_all(&dir);
        let safe: String = file.chars().map(|c| if c.is_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }).collect();
        let p = dir.join(safe);
        if std::fs::write(&p, bytes).is_err() {
            return err(758, "Custom data could not be stored.");
        }
        p
    } else {
        match app.library.items.iter().find(|e| item_file_name(e.path()).eq_ignore_ascii_case(&file)) {
            Some(e) => e.path().to_path_buf(),
            None => return err(751, "Item file not found."),
        }
    };
    let Some(id) = app.add_item(&path) else { return err(753, "Item could not be loaded.") };
    let (vw, vh) = app.viewport();
    let k = app.items.iter().position(|i| i.id == id).unwrap();
    let f = |key: &str| d[key].as_f64().map(|v| v as f32);
    let it = &mut app.items[k];
    if let (Some(x), Some(y)) = (f("positionX"), f("positionY")) {
        it.xf.pos = [x * vw / 2.0, y * vh / 2.0];
    }
    if let Some(s) = f("size").filter(|s| *s > 0.0) {
        let h = item_height_px(it).max(1.0);
        it.xf.scale *= s.clamp(0.0, 1.0) * vh / h;
    }
    if let Some(r) = f("rotation") {
        it.xf.rot = -r;
    }
    it.xf.flip = d["flipped"].as_bool().unwrap_or(false);
    it.locked = d["locked"].as_bool().unwrap_or(false);
    if let Some(o) = d["order"].as_i64() {
        it.cfg.order = if o == 0 { 1 } else { o as i32 };
    }
    it.cfg.smoothing = d["smoothing"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    it.cfg.censor = d["censored"].as_bool().unwrap_or(false);
    if let Some(ft) = d["fadeTime"].as_f64().filter(|t| *t > 0.0) {
        it.alpha = 0.0;
        it.fade_speed = 1.0 / (ft.min(2.0) as f32);
    }
    app.api_item_owner.insert(id, (sid, d["unloadWhenPluginDisconnects"].as_bool().unwrap_or(false)));
    let pos = app.items[k].xf.pos;
    app.api_events.push(("ItemEvent".into(), json!({ "itemEventType": "Added", "itemInstanceID": instance_id(id), "itemFileName": item_file_name(&path), "itemPosition": { "x": pos[0] / (vw / 2.0), "y": pos[1] / (vh / 2.0) } })));
    Ok(json!({ "instanceID": instance_id(id), "fileName": item_file_name(&path) }))
}

/// (fps, frame count, current frame) of an animated item.
fn frames_of(i: &crate::scene::Item) -> (f32, usize, usize) {
    match &i.body {
        crate::scene::Body::Sprite(s) if s.frame_count() > 1 => (if i.cfg.fps > 0.0 { i.cfg.fps } else { 12.0 }, s.frame_count(), s.current_frame()),
        _ => (0.0, 1, 0),
    }
}

fn item_sort(app: &mut App, k: usize, iid: &str, d: &Value) -> Res {
    let mode = |key: &str| d[key].as_str().unwrap_or("Unchanged").to_string();
    let (sp, fo, bo) = (mode("setSplitPoint"), mode("setFrontOrder"), mode("setBackOrder"));
    for m in [&sp, &fo, &bo] {
        if !["Unchanged", "UseArtMeshID", "UseSpecialID"].contains(&m.as_str()) {
            return err(1251, "Invalid value set type.");
        }
    }
    let special = |v: &str| match v {
        "FullyInFront" => Some(crate::scene::LAYER_FRONT.to_string()),
        "FullyInBack" => Some(crate::scene::LAYER_BACK.to_string()),
        _ => None,
    };
    let s = |key: &str| d[key].as_str().unwrap_or("").to_string();
    let front_on = d["frontOn"].as_bool().unwrap_or(false);
    let back_on = d["backOn"].as_bool().unwrap_or(false);
    let is_l2d = matches!(app.items[k].body, crate::scene::Body::Live2D(_));
    let av = app.avatar.as_ref();
    let mut had_front = false;
    let mut had_back = false;
    // Validate before changing anything.
    let front = match fo.as_str() {
        "UseArtMeshID" => {
            had_front = av.map(|a| a.drawable_index(&s("withinModelOrderFront")).is_some()).unwrap_or(false);
            Some(s("withinModelOrderFront"))
        }
        "UseSpecialID" => match special(&s("withinModelOrderFront")) {
            Some(v) => Some(v),
            None => return err(1252, "Invalid front order."),
        },
        _ => None,
    };
    let back = if !is_l2d {
        None
    } else {
        match bo.as_str() {
            "UseArtMeshID" => {
                had_back = av.map(|a| a.drawable_index(&s("withinModelOrderBack")).is_some()).unwrap_or(false);
                Some(s("withinModelOrderBack"))
            }
            "UseSpecialID" => match s("withinModelOrderBack").as_str() {
                "FullyInBack" => Some(crate::scene::LAYER_BACK.to_string()),
                _ => return err(1253, "Invalid back order."),
            },
            _ => None,
        }
    };
    if is_l2d && sp == "UseArtMeshID" {
        let id = s("splitAt");
        let ok = match &app.items[k].body {
            crate::scene::Body::Live2D(l) => l.drawable_index(&id).is_some(),
            _ => false,
        };
        if !ok {
            return err(1254, "Invalid split point.");
        }
        app.items[k].cfg.split = id;
    }
    let it = &mut app.items[k];
    if let Some(f) = front {
        it.cfg.split_front = f;
    }
    if let Some(b) = back {
        it.cfg.split_back = b;
    }
    it.cfg.layer = if front_on { it.cfg.split_front.clone() } else { String::new() };
    if !back_on {
        it.cfg.split_back.clear();
    }
    if !front_on {
        it.cfg.split_front.clear();
    }
    let a = app.avatar.as_ref();
    Ok(json!({
        "itemInstanceID": iid, "modelLoaded": a.is_some(), "modelID": a.map(|a| a.model_id.clone()).unwrap_or_default(),
        "modelName": a.map(|a| a.name.clone()).unwrap_or_default(), "loadedModelHadRequestedFrontLayer": had_front, "loadedModelHadRequestedBackLayer": had_back,
    }))
}

fn item_unload(app: &mut App, sid: u64, d: &Value) -> Value {
    let all = d["unloadAllInScene"].as_bool().unwrap_or(false);
    let mine = d["unloadAllLoadedByThisPlugin"].as_bool().unwrap_or(false);
    let others = d["allowUnloadingItemsLoadedByUserOrOtherPlugins"].as_bool().unwrap_or(true);
    let ids: Vec<String> = d["instanceIDs"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let files: Vec<String> = d["fileNames"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let mut removed = Vec::new();
    let owner = app.api_item_owner.clone();
    app.items.retain(|i| {
        let is_mine = owner.get(&i.id).map(|o| o.0 == sid).unwrap_or(false);
        let target = all || (mine && is_mine) || ids.contains(&instance_id(i.id)) || files.iter().any(|f| item_file_name(&i.source) == *f);
        let allowed = is_mine || others;
        if target && allowed {
            removed.push(json!({ "instanceID": instance_id(i.id), "fileName": item_file_name(&i.source) }));
            false
        } else {
            true
        }
    });
    for r in &removed {
        app.api_events.push(("ItemEvent".into(), json!({ "itemEventType": "Removed", "itemInstanceID": r["instanceID"], "itemFileName": r["fileName"], "itemPosition": { "x": 0.0, "y": 0.0 } })));
    }
    json!({ "unloadedItems": removed })
}

fn item_move(app: &mut App, d: &Value) -> Value {
    let (vw, vh) = app.viewport();
    let mut moved = Vec::new();
    for m in d["itemsToMove"].as_array().cloned().unwrap_or_default().iter().take(64) {
        let iid = m["itemInstanceID"].as_str().unwrap_or("").to_string();
        let Some(k) = find_item(app, &iid) else {
            moved.push(json!({ "itemInstanceID": iid, "success": false, "errorID": 900 }));
            continue;
        };
        let fade = m["fadeMode"].as_str().unwrap_or("linear").to_string();
        if !["linear", "easeIn", "easeOut", "easeBoth", "overshoot", "zip"].contains(&fade.as_str()) {
            moved.push(json!({ "itemInstanceID": iid, "success": false, "errorID": 901 }));
            continue;
        }
        let it = &mut app.items[k];
        let from = it.xf;
        let mut to = from;
        let g = |key: &str| m[key].as_f64().map(|v| v as f32).filter(|v| *v > -1000.0);
        if let Some(x) = g("positionX") {
            to.pos[0] = x * vw / 2.0;
        }
        if let Some(y) = g("positionY") {
            to.pos[1] = y * vh / 2.0;
        }
        if let Some(r) = g("rotation") {
            to.rot = -r;
        }
        if let Some(s) = g("size").filter(|s| *s > 0.0) {
            let h = item_height_px(it).max(1.0);
            to.scale = from.scale * s * vh / h;
        }
        if m["setFlip"].as_bool() == Some(true) {
            to.flip = m["flip"].as_bool().unwrap_or(false);
        }
        if let Some(o) = m["order"].as_i64().filter(|o| *o > -1000) {
            it.cfg.order = if o == 0 { 1 } else { o as i32 };
        }
        let t = (m["timeInSeconds"].as_f64().unwrap_or(0.0) as f32).clamp(0.0, 30.0);
        it.pin = None;
        if t <= 0.0 {
            it.xf = to;
        } else {
            let id = it.id;
            app.api_item_moves.retain(|mv| mv.0 != id);
            app.api_item_moves.push((id, from, to, 0.0, t, fade));
        }
        moved.push(json!({ "itemInstanceID": iid, "success": true, "errorID": -1 }));
    }
    json!({ "movedItems": moved })
}

fn ease(mode: &str, t: f32) -> f32 {
    match mode {
        "easeIn" => t * t,
        "easeOut" => 1.0 - (1.0 - t) * (1.0 - t),
        "easeBoth" => t * t * (3.0 - 2.0 * t),
        "overshoot" => {
            let c = 1.70158 + 1.0;
            1.0 + c * (t - 1.0).powi(3) + 1.70158 * (t - 1.0).powi(2)
        }
        "zip" => 1.0 - (1.0 - t).powi(5),
        _ => t,
    }
}

fn lerp_xf(a: &Xform, b: &Xform, t: f32) -> Xform {
    Xform {
        pos: [a.pos[0] + (b.pos[0] - a.pos[0]) * t, a.pos[1] + (b.pos[1] - a.pos[1]) * t],
        scale: a.scale + (b.scale - a.scale) * t,
        rot: a.rot + (b.rot - a.rot) * t,
        flip: if t >= 0.5 { b.flip } else { a.flip },
    }
}

fn tick_animations(app: &mut App, dt: f32) {
    if let Some((from, to, t, dur)) = app.api_model_move.as_mut() {
        *t += dt;
        let k = (*t / *dur).min(1.0);
        let x = lerp_xf(from, to, ease("easeBoth", k));
        let done = k >= 1.0;
        if let Some(a) = app.avatar.as_mut() {
            a.xf = x;
        }
        if done {
            app.api_model_move = None;
        }
    }
    let mut done = Vec::new();
    for (i, mv) in app.api_item_moves.iter_mut().enumerate() {
        mv.3 += dt;
        let k = (mv.3 / mv.4).min(1.0);
        let x = lerp_xf(&mv.1, &mv.2, ease(&mv.5, k));
        if let Some(it) = app.items.iter_mut().find(|it| it.id == mv.0) {
            it.xf = x;
        }
        if k >= 1.0 {
            done.push(i);
        }
    }
    for i in done.into_iter().rev() {
        app.api_item_moves.remove(i);
    }
}

fn item_pin(app: &mut App, d: &Value) -> Res {
    let iid = d["itemInstanceID"].as_str().unwrap_or("");
    let Some(k) = find_item(app, iid) else { return err(1050, "Item not loaded.") };
    if d["pin"].as_bool() == Some(false) {
        app.items[k].pin = None;
        return Ok(json!({ "isPinned": false, "itemInstanceID": iid, "itemFileName": item_file_name(&app.items[k].source) }));
    }
    let Some(av) = app.avatar.as_ref() else { return err(1052, "Model not found.") };
    let info = &d["pinInfo"];
    let ok = match d["vertexPinType"].as_str().unwrap_or("Provided") {
        "Provided" => {
            let Some(di) = info["artMeshID"].as_str().and_then(|m| av.drawable_index(m)) else { return err(1053, "ArtMesh not found.") };
            let tri = [info["vertexID1"].as_u64().unwrap_or(0) as u16, info["vertexID2"].as_u64().unwrap_or(0) as u16, info["vertexID3"].as_u64().unwrap_or(0) as u16];
            let bary = [info["vertexWeight1"].as_f64().unwrap_or(1.0) as f32, info["vertexWeight2"].as_f64().unwrap_or(0.0) as f32, info["vertexWeight3"].as_f64().unwrap_or(0.0) as f32];
            if tri.iter().any(|&t| t as usize >= av.model.drawables[di].vertex_count) {
                return err(1054, "Pin position invalid.");
            }
            app.items[k].pin_exact(av, di, tri, bary)
        }
        "Center" | "Random" => {
            let di = info["artMeshID"].as_str().and_then(|m| av.drawable_index(m));
            match di {
                Some(di) => {
                    let idx = av.model.indices(di);
                    let n = idx.len() / 3;
                    if n == 0 {
                        return err(1054, "ArtMesh has no triangles.");
                    }
                    let t = if d["vertexPinType"].as_str() == Some("Random") { (now_ms() as usize) % n } else { n / 2 };
                    app.items[k].pin_exact(av, di, [idx[t * 3], idx[t * 3 + 1], idx[t * 3 + 2]], [1.0 / 3.0; 3])
                }
                None => app.items[k].pin_to(av),
            }
        }
        _ => return err(1051, "Invalid pin type."),
    };
    Ok(json!({ "isPinned": ok, "itemInstanceID": iid, "itemFileName": item_file_name(&app.items[k].source) }))
}

fn post_processing_list(app: &App, d: &Value) -> Res {
    use crate::vfx::{Kind, DEFS};
    let filter: Vec<String> = d["effectIDFilter"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if filter.len() > 512 {
        return err(1150, "Filter array too long.");
    }
    let want: Vec<usize> = filter.iter().filter_map(|f| crate::vfx::find_effect(f)).collect();
    let vals = &app.settings.vfx;
    let picked: Vec<usize> = (0..DEFS.len()).filter(|i| filter.is_empty() || want.contains(i)).collect();
    let effects: Vec<Value> = if d["fillPostProcessingEffectsArray"].as_bool().unwrap_or(false) {
        picked
            .iter()
            .map(|&i| {
                let def = &DEFS[i];
                let cfg: Vec<Value> = def
                    .configs
                    .iter()
                    .map(|c| {
                        let f = crate::vfx::get_f(vals, c);
                        let (ty, is) = match c.kind {
                            Kind::Float => ("Float", 0),
                            Kind::Int => ("Int", 1),
                            Kind::Bool => ("Bool", 2),
                            Kind::Color => ("Color", 3),
                            Kind::Text => ("String", 4),
                            Kind::Item => ("SceneItem", 5),
                        };
                        let col = if is == 3 { crate::vfx::color_hex(crate::vfx::get_color(vals, c)) } else { String::new() };
                        let col_def = if is == 3 { format!("{}FF", c.color) } else { String::new() };
                        let txt = if is >= 4 { crate::vfx::get_text(vals, c) } else { String::new() };
                        json!({
                            "internalID": c.internal, "enumID": c.id, "explanation": c.en, "type": ty, "activationConfig": c.active,
                            "floatValue": if is == 0 { f } else { 0.0 }, "floatMin": if is == 0 { c.min } else { 0.0 }, "floatMax": if is == 0 { c.max } else { 0.0 }, "floatDefault": if is == 0 { c.default } else { 0.0 },
                            "intValue": if is == 1 { f as i64 } else { 0 }, "intMin": if is == 1 { c.min as i64 } else { 0 }, "intMax": if is == 1 { c.max as i64 } else { 0 }, "intDefault": if is == 1 { c.default as i64 } else { 0 },
                            "colorValue": col, "colorDefault": col_def, "colorHasAlpha": c.alpha,
                            "boolValue": is == 2 && f > 0.5, "boolDefault": is == 2 && c.default > 0.5,
                            "stringValue": if is == 4 { txt.clone() } else { String::new() }, "stringDefault": "",
                            "sceneItemValue": if is == 5 { txt } else { String::new() }, "sceneItemDefault": "",
                        })
                    })
                    .collect();
                json!({ "internalID": def.internal, "enumID": def.id, "explanation": def.name_en, "effectIsActive": crate::vfx::effect_active(vals, def), "effectIsRestricted": def.restricted, "configEntries": cfg })
            })
            .collect()
    } else {
        Vec::new()
    };
    let presets: Vec<String> = if d["fillPostProcessingPresetsArray"].as_bool().unwrap_or(false) { crate::vfx::list_presets(&app.paths.data) } else { Vec::new() };
    let active = crate::vfx::active_effects(vals).len();
    let total_cfg: usize = DEFS.iter().map(|d| d.configs.len()).sum();
    let after_cfg: usize = picked.iter().map(|&i| DEFS[i].configs.len()).sum();
    Ok(json!({
        "postProcessingSupported": true, "postProcessingActive": app.settings.vfx_on, "canSendPostProcessingUpdateRequestRightNow": !vfx_window_open(app),
        "restrictedEffectsAllowed": app.settings.vfx_restricted, "presetIsActive": !app.vfx_preset.is_empty(), "activePreset": app.vfx_preset,
        "presetCount": presets.len(), "activeEffectCount": active,
        "effectCountBeforeFilter": DEFS.len(), "configCountBeforeFilter": total_cfg, "effectCountAfterFilter": picked.len(), "configCountAfterFilter": after_cfg,
        "postProcessingEffects": effects, "postProcessingPresets": presets,
    }))
}

fn vfx_window_open(app: &App) -> bool {
    matches!(app.ui.modal, Some(crate::ui::Modal::VfxEdit(_)) | Some(crate::ui::Modal::VfxList))
}

fn post_processing_update(app: &mut App, d: &Value) -> Res {
    use crate::vfx::DEFS;
    if vfx_window_open(app) {
        return err(1200, "Cannot update post-processing while the effect windows are open.");
    }
    let fade = d["postProcessingFadeTime"].as_f64().unwrap_or(0.0);
    if !(0.0..=2.0).contains(&fade) {
        return err(1201, "Fade time must be between 0 and 2 seconds.");
    }
    let set_preset = d["setPostProcessingPreset"].as_bool().unwrap_or(false);
    let set_values = d["setPostProcessingValues"].as_bool().unwrap_or(false);
    let restricted_ok = d["usingRestrictedEffects"].as_bool().unwrap_or(false) && app.settings.vfx_restricted;
    let mut target = app.settings.vfx.clone();
    let mut preset_name = app.vfx_preset.clone();
    if d["randomizeAll"].as_bool().unwrap_or(false) {
        let chaos = d["randomizeAllChaosLevel"].as_f64().unwrap_or(0.4).clamp(0.0, 1.0) as f32;
        target = crate::vfx::randomize(chaos, restricted_ok);
        preset_name.clear();
    } else if set_preset && set_values {
        return err(1202, "Cannot load a preset and individual values in one request.");
    } else if set_preset {
        let name = d["presetToSet"].as_str().unwrap_or("");
        match crate::vfx::load_preset(&app.paths.data, name) {
            Some(mut v) => {
                if !app.settings.vfx_restricted {
                    for def in DEFS.iter().filter(|d| d.restricted) {
                        crate::vfx::reset_effect(&mut v, def);
                    }
                }
                target = v;
                preset_name = name.to_string();
            }
            None => return err(1203, format!("Preset \"{name}\" could not be loaded.")),
        }
    } else if set_values {
        let list = d["postProcessingValues"].as_array().cloned().unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        let mut parsed = Vec::new();
        for v in &list {
            let id = v["configID"].as_str().unwrap_or("");
            let val = match &v["configValue"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let Some((e, c)) = crate::vfx::find_config(id) else { return err(1204, format!("Unknown configID {id}.")) };
            if !seen.insert((e, c)) {
                return err(1205, "The value list contains duplicates.");
            }
            if DEFS[e].restricted && !restricted_ok {
                return err(1206, "Restricted effects are not allowed (set usingRestrictedEffects and allow them in the VFX settings).");
            }
            parsed.push((e, c, val));
        }
        if d["setAllOtherValuesToDefault"].as_bool().unwrap_or(false) {
            target.clear();
        }
        for (e, c, val) in parsed {
            if !crate::vfx::set_from_str(&mut target, &DEFS[e].configs[c], &val) {
                return err(1204, format!("Invalid value for {}.", DEFS[e].configs[c].id));
            }
        }
        preset_name.clear();
    }
    if let Some(on) = d["postProcessingOn"].as_bool() {
        app.settings.vfx_on = on;
    }
    app.vfx_fade_to(target, fade as f32);
    app.vfx_preset = if app.settings.vfx_on { preset_name } else { String::new() };
    let active = crate::vfx::active_effects(&app.settings.vfx).len();
    app.api_events.push(("PostProcessingEvent".into(), json!({ "currentOnState": app.settings.vfx_on, "currentPreset": app.vfx_preset })));
    Ok(json!({ "postProcessingActive": app.settings.vfx_on, "presetIsActive": !app.vfx_preset.is_empty(), "activePreset": app.vfx_preset, "activeEffectCount": active }))
}

/// Model click → ModelClickedEvent (+ ItemEvent "Clicked" for items).
pub fn on_click(app: &mut App, world: [f32; 2], px: [f32; 2], button: u8, item: Option<u64>) {
    if app.api.is_none() {
        return;
    }
    let (vw, vh) = app.viewport();
    if let Some(id) = item {
        if let Some(it) = app.items.iter().find(|i| i.id == id) {
            let ev = json!({ "itemEventType": "Clicked", "itemInstanceID": instance_id(id), "itemFileName": item_file_name(&it.source), "itemPosition": { "x": it.xf.pos[0] / (vw / 2.0), "y": it.xf.pos[1] / (vh / 2.0) } });
            app.api_events.push(("ItemEvent".into(), ev));
        }
    }
    let x = world[0] / (vw / 2.0);
    let y = world[1] / (vh / 2.0);
    let hits = art_mesh_hits(app, x, y);
    let a = app.avatar.as_ref();
    let ev = json!({
        "modelLoaded": a.is_some(), "loadedModelID": a.map(|a| a.model_id.clone()).unwrap_or_default(), "loadedModelName": a.map(|a| a.name.clone()).unwrap_or_default(),
        "modelWasClicked": hits["modelWasHit"], "mouseButtonID": button, "clickPosition": { "x": px[0], "y": px[1] }, "windowSize": { "x": vw, "y": vh },
        "clickedArtMeshCount": hits["artMeshHitCount"], "artMeshHits": hits["artMeshHits"],
    });
    app.api_events.push(("ModelClickedEvent".into(), ev));
}

pub type PromptQueue = VecDeque<ApiPrompt>;
pub type InjectMap = HashMap<usize, (f32, f32, Instant, String, bool)>;
