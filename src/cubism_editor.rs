//! Live2D Cubism Editor "External Application Integration" client
//! (docs.live2d.com: external-application-integration-api). WebSocket JSON on port 22033.
//! Sends the current model's parameter values into the Editor (physics preview / animation
//! recording) and picks up exported models, like VTS does.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub enum Msg {
    State { connected: bool, approved: bool },
    Token(String),
    EditMode(String),
    Exported(String),
    Log(String),
}

pub struct Editor {
    rx: Receiver<Msg>,
    stop: Arc<AtomicBool>,
    /// Latest parameter values to send (replaced every frame; the thread sends the newest).
    params: Arc<Mutex<Option<Vec<(String, f32)>>>>,
    pub connected: bool,
    pub approved: bool,
    pub edit_mode: String,
    pub status: String,
}

fn msg(method: &str, data: Value, id: u64) -> String {
    json!({
        "Version": "1.0.0",
        "Timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
        "RequestId": id.to_string(),
        "Type": "Request",
        "Method": method,
        "Data": data,
    })
    .to_string()
}

impl Editor {
    pub fn start(host: String, port: u16, token: String) -> Editor {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let params = Arc::new(Mutex::new(None));
        {
            let (stop, params) = (stop.clone(), params.clone());
            std::thread::Builder::new().name("cubism-editor".into()).spawn(move || run(host, port, token, tx, stop, params)).ok();
        }
        Editor { rx, stop, params, connected: false, approved: false, edit_mode: String::new(), status: "Подключение…".into() }
    }

    /// Queues parameter values for the Editor (only the newest set is sent).
    pub fn send_params(&self, p: Vec<(String, f32)>) {
        if let Ok(mut g) = self.params.lock() {
            *g = Some(p);
        }
    }

    pub fn drain(&mut self) -> Vec<Msg> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            match &m {
                Msg::State { connected, approved } => {
                    self.connected = *connected;
                    self.approved = *approved;
                    self.status = match (connected, approved) {
                        (false, _) => "Нет связи с Cubism Editor (включите API в «Настройки внешнего приложения»)".into(),
                        (true, false) => "Подключено — разрешите доступ (галочка «Разрешить») в Cubism Editor".into(),
                        (true, true) => "Подключено и разрешено".into(),
                    };
                }
                Msg::EditMode(m) => self.edit_mode = m.clone(),
                Msg::Log(s) => self.status = s.clone(),
                _ => {}
            }
            out.push(m);
        }
        out
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(host: String, port: u16, mut token: String, tx: Sender<Msg>, stop: Arc<AtomicBool>, params: Arc<Mutex<Option<Vec<(String, f32)>>>>) {
    while !stop.load(Ordering::Relaxed) {
        let url = format!("ws://{host}:{port}");
        match tungstenite::connect(url.as_str()) {
            Ok((mut ws, _)) => {
                if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
                    let _ = s.set_read_timeout(Some(Duration::from_millis(20)));
                }
                let _ = tx.send(Msg::State { connected: true, approved: false });
                session(&mut ws, &mut token, &tx, &stop, &params);
                let _ = ws.close(None);
                let _ = tx.send(Msg::State { connected: false, approved: false });
            }
            Err(_) => {
                let _ = tx.send(Msg::State { connected: false, approved: false });
            }
        }
        for _ in 0..30 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

fn session(ws: &mut Ws, token: &mut String, tx: &Sender<Msg>, stop: &AtomicBool, params: &Mutex<Option<Vec<(String, f32)>>>) {
    let mut id = 1u64;
    let mut send = |ws: &mut Ws, m: &str, d: Value, id: &mut u64| -> bool {
        *id += 1;
        ws.send(tungstenite::Message::Text(msg(m, d, *id).into())).is_ok()
    };
    let mut reg = json!({ "Name": "rigrs", "Path": std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default() });
    if !token.is_empty() {
        reg["Token"] = json!(token.clone());
    }
    if !send(ws, "RegisterPlugin", reg, &mut id) {
        return;
    }
    let mut approved = false;
    let mut model_uid = String::new();
    let mut last_poll = Instant::now() - Duration::from_secs(5);
    let mut last_uid = Instant::now() - Duration::from_secs(5);
    let mut notified = false;
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        // Read everything available.
        loop {
            match ws.read() {
                Ok(tungstenite::Message::Text(t)) => {
                    let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                    let method = v["Method"].as_str().unwrap_or("");
                    let ty = v["Type"].as_str().unwrap_or("");
                    let d = &v["Data"];
                    if ty == "Error" {
                        if d["ErrorType"].as_str() == Some("PluginNotRegistered") || d["ErrorType"].as_str() == Some("NotApproved") {
                            approved = false;
                        }
                        if method == "GetCurrentModelUID" {
                            model_uid.clear();
                        }
                        continue;
                    }
                    match method {
                        "RegisterPlugin" => {
                            if let Some(t) = d["Token"].as_str() {
                                if *t != *token {
                                    *token = t.to_string();
                                    let _ = tx.send(Msg::Token(t.to_string()));
                                }
                            }
                        }
                        "GetIsApproval" => {
                            let a = d["Result"].as_bool().unwrap_or(false);
                            if a != approved {
                                approved = a;
                                let _ = tx.send(Msg::State { connected: true, approved });
                            }
                        }
                        "GetCurrentModelUID" => model_uid = d["ModelUID"].as_str().unwrap_or("").to_string(),
                        "GetCurrentEditMode" | "NotifyChangeEditMode" => {
                            if let Some(m) = d["EditMode"].as_str() {
                                let _ = tx.send(Msg::EditMode(m.to_string()));
                            }
                        }
                        "NotifyMocFileExported" => {
                            if let Some(p) = d["Path"].as_str() {
                                let _ = tx.send(Msg::Exported(p.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
                Ok(tungstenite::Message::Close(_)) => return,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
                Err(_) => return,
            }
        }
        if last_poll.elapsed() > Duration::from_secs(1) {
            last_poll = Instant::now();
            if !send(ws, "GetIsApproval", json!({}), &mut id) {
                return;
            }
        }
        if approved {
            if !notified {
                notified = true;
                send(ws, "NotifyMocFileExported", json!({ "Enabled": true }), &mut id);
                send(ws, "NotifyChangeEditMode", json!({ "Enabled": true }), &mut id);
                send(ws, "GetCurrentEditMode", json!({}), &mut id);
            }
            if last_uid.elapsed() > Duration::from_secs(2) {
                last_uid = Instant::now();
                send(ws, "GetCurrentModelUID", json!({}), &mut id);
            }
            let p = params.lock().ok().and_then(|mut g| g.take());
            if let (Some(p), false) = (p, model_uid.is_empty()) {
                let list: Vec<Value> = p.iter().map(|(i, v)| json!({ "Id": i, "Value": v })).collect();
                if !send(ws, "SetParameterValues", json!({ "ModelUID": model_uid, "Parameters": list }), &mut id) {
                    return;
                }
            }
        } else {
            notified = false;
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

/// Copies an exported model (folder of the exported .moc3) into rigrs' model folder.
pub fn import_export(moc: &std::path::Path, models_dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let src = moc.parent().ok_or_else(|| anyhow::anyhow!("bad path"))?;
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "model".into());
    let dst = models_dir.join(&name);
    let mut stack = vec![src.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)?.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(src)?;
            let t = dst.join(rel);
            if p.is_dir() {
                stack.push(p);
            } else {
                if let Some(parent) = t.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&p, &t)?;
            }
        }
    }
    Ok(dst)
}
