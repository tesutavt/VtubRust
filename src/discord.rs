//! Discord Rich Presence over the local Discord IPC socket (`discord-ipc-0`): shows
//! "rigrs · <model>" with the session time in your Discord profile while the app runs.
//!
//! Needs the Application ID of a Discord application (free, created at
//! discord.com/developers/applications); its name is what Discord shows as the activity.
//! Frames are `[u32 opcode][u32 length][JSON]`: opcode 0 = handshake, 1 = command.

use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct Activity {
    pub details: String,
    pub state: String,
    pub start: u64,
}

pub struct Presence {
    tx: Sender<Option<Activity>>,
    status: Receiver<String>,
    pub last_status: String,
    last_sent: Option<(String, String)>,
    last_time: Instant,
}

#[cfg(windows)]
type Pipe = std::fs::File;
#[cfg(unix)]
type Pipe = std::os::unix::net::UnixStream;

fn open_pipe() -> Option<Pipe> {
    for i in 0..10 {
        #[cfg(windows)]
        {
            if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) {
                return Some(f);
            }
        }
        #[cfg(unix)]
        {
            let dirs = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(|k| std::env::var(k).ok()).chain(std::iter::once("/tmp".to_string()));
            for d in dirs {
                if let Ok(s) = std::os::unix::net::UnixStream::connect(format!("{d}/discord-ipc-{i}")) {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn send(p: &mut Pipe, op: u32, v: &Value) -> std::io::Result<()> {
    let body = v.to_string();
    let mut f = Vec::with_capacity(body.len() + 8);
    f.extend_from_slice(&op.to_le_bytes());
    f.extend_from_slice(&(body.len() as u32).to_le_bytes());
    f.extend_from_slice(body.as_bytes());
    p.write_all(&f)?;
    p.flush()
}

fn recv(p: &mut Pipe) -> std::io::Result<Value> {
    let mut h = [0u8; 8];
    p.read_exact(&mut h)?;
    let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
    if len > 1 << 20 {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut b = vec![0u8; len];
    p.read_exact(&mut b)?;
    Ok(serde_json::from_slice(&b).unwrap_or(Value::Null))
}

fn set_activity(p: &mut Pipe, a: &Option<Activity>, nonce: u64) -> std::io::Result<Value> {
    let activity = a.as_ref().map(|a| {
        json!({
            "details": a.details.chars().take(120).collect::<String>(),
            "state": a.state.chars().take(120).collect::<String>(),
            "timestamps": { "start": a.start },
        })
    });
    send(p, 1, &json!({ "cmd": "SET_ACTIVITY", "args": { "pid": std::process::id(), "activity": activity }, "nonce": nonce.to_string() }))?;
    recv(p)
}

impl Presence {
    pub fn start(client_id: &str) -> Presence {
        let (tx, rx) = channel::<Option<Activity>>();
        let (stx, status) = channel::<String>();
        let id = client_id.trim().to_string();
        let _ = std::thread::Builder::new().name("discord-rpc".into()).spawn(move || {
            let mut pipe: Option<Pipe> = None;
            let mut want: Option<Activity> = None;
            let mut dirty = true;
            let mut nonce = 0u64;
            loop {
                match rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(a) => {
                        want = a;
                        dirty = true;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => {
                        // App closed the presence: clear the activity.
                        if let Some(p) = pipe.as_mut() {
                            let _ = set_activity(p, &None, nonce + 1);
                        }
                        return;
                    }
                }
                if pipe.is_none() {
                    match open_pipe() {
                        Some(mut p) => {
                            let ok = send(&mut p, 0, &json!({ "v": 1, "client_id": id })).is_ok() && recv(&mut p).map(|v| v["evt"] == "READY").unwrap_or(false);
                            if ok {
                                pipe = Some(p);
                                dirty = true;
                                let _ = stx.send("connected".into());
                            } else {
                                let _ = stx.send("Discord rejected the Application ID".into());
                            }
                        }
                        None => {
                            let _ = stx.send("Discord is not running".into());
                        }
                    }
                }
                if dirty {
                    if let Some(p) = pipe.as_mut() {
                        nonce += 1;
                        match set_activity(p, &want, nonce) {
                            Ok(v) if v["evt"] == "ERROR" => {
                                let _ = stx.send(format!("Discord: {}", v["data"]["message"].as_str().unwrap_or("error")));
                            }
                            Ok(_) => {
                                dirty = false;
                                let _ = stx.send("connected".into());
                            }
                            Err(_) => pipe = None,
                        }
                    }
                }
            }
        });
        Presence { tx, status, last_status: "connecting…".into(), last_sent: None, last_time: Instant::now() - Duration::from_secs(60) }
    }

    /// Updates the activity (rate limited; Discord allows one update per 15 s).
    pub fn update(&mut self, details: &str, state: &str, start: u64) {
        while let Ok(s) = self.status.try_recv() {
            self.last_status = s;
        }
        let key = (details.to_string(), state.to_string());
        if self.last_sent.as_ref() == Some(&key) || self.last_time.elapsed() < Duration::from_secs(15) {
            return;
        }
        self.last_time = Instant::now();
        self.last_sent = Some(key);
        let _ = self.tx.send(Some(Activity { details: details.into(), state: state.into(), start }));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Fake Discord client: handshake, then a SET_ACTIVITY with our model name.
    #[test]
    fn presence_handshake_and_activity() {
        let dir = std::env::temp_dir().join(format!("rigrs-discord-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        let path = dir.join("discord-ipc-0");
        let _ = std::fs::remove_file(&path);
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let hello = recv(&mut s).unwrap();
            assert_eq!(hello["client_id"], "42");
            send(&mut s, 1, &json!({ "cmd": "DISPATCH", "evt": "READY" })).unwrap();
            let cmd = recv(&mut s).unwrap();
            send(&mut s, 1, &json!({ "cmd": "SET_ACTIVITY", "evt": null })).unwrap();
            cmd
        });
        let mut p = Presence::start("42");
        p.update("Model: Akari", "In rigrs", 1);
        let cmd = t.join().unwrap();
        assert_eq!(cmd["cmd"], "SET_ACTIVITY");
        assert_eq!(cmd["args"]["activity"]["details"], "Model: Akari");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
