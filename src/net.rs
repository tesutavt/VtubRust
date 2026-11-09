//! VNet-style multiplayer collab (modelled on VTube Studio's VNet, see the VTS wiki
//! "Multiplayer" page).
//!
//! * One participant hosts, up to 5 others join (6 in total). Joining needs the host's
//!   address, the same protocol version and the session password.
//! * Every participant's model and items are shared automatically: the files are zipped,
//!   uploaded once to the host and handed to everyone who needs them (in a temporary cache
//!   that is wiped when the session ends). Limits as in VTS: 150 files and 300 MB per model
//!   or item, 95 MB per file.
//! * Movement data (the Live2D parameter values of the model after tracking, expressions
//!   and animations, plus model and item placement) is sent at the host's "Data FPS".
//!   Receivers run physics themselves and smooth the values with a small buffer.
//!
//! Unlike VTS there is no Steam P2P / relay: the host must be reachable over TCP (same LAN,
//! port forwarding, or a virtual LAN such as Tailscale / ZeroTier / Radmin VPN).
//!
//! Wire format: frames of `[u32 length][u8 kind][payload]`, little endian.
//! kind 1 = JSON control message, 2 = binary movement state, 3 = file chunk.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::mapping::NEUTRAL;
use crate::scene::live2d::Xform;
use crate::scene::{Body, Item};

pub const PROTOCOL: u32 = 1;
pub const MAX_PARTICIPANTS: usize = 6;
pub const MAX_ASSET_BYTES: usize = 300 << 20;
pub const MAX_FILE_BYTES: u64 = 95 << 20;
pub const MAX_ASSET_FILES: usize = 150;
/// VTS: Live2D parameter values are limited to -9000..9000.
const PARAM_LIMIT: f32 = 9000.0;
const MAX_FRAME: usize = 1 << 20;
const CHUNK: usize = 48 * 1024;
const K_JSON: u8 = 1;
const K_STATE: u8 = 2;
const K_CHUNK: u8 = 3;
const HOST_ID: u16 = 0;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NetSettings {
    /// true = host, false = join.
    pub host: bool,
    pub nickname: String,
    pub password: String,
    /// Port the host listens on.
    pub port: u16,
    /// Host address to join (`ip` or `ip:port`).
    pub address: String,
    /// Movement data rate (host setting).
    pub data_fps: u32,
    /// Upload limit for model/item files in KB/s (0 = unlimited).
    pub upload_kbps: u32,
    /// Share items you drag in (VTS collab setting).
    pub share_items: bool,
    /// Movement buffer in milliseconds (smoothing of received data).
    pub buffer_ms: u32,
    /// Smooth received parameters (turn off for frame-by-frame rigs).
    pub smoothing: bool,
}

impl Default for NetSettings {
    fn default() -> Self {
        NetSettings {
            host: true,
            nickname: String::new(),
            password: String::new(),
            port: 25580,
            address: String::new(),
            data_fps: 30,
            upload_kbps: 0,
            share_items: true,
            buffer_ms: 80,
            smoothing: true,
        }
    }
}

// ------------------------------------------------------------------ helpers

fn sha1_bytes(parts: &[&[u8]]) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn proof(nonce: &str, password: &str) -> String {
    hex(&sha1_bytes(&[nonce.as_bytes(), b":", password.as_bytes()]))
}

fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    format!("{:016x}{:016x}", h.finish(), std::collections::hash_map::RandomState::new().build_hasher().finish())
}

fn asset_key(id: u64) -> String {
    format!("{id:016x}")
}

fn parse_key(s: &str) -> Option<u64> {
    u64::from_str_radix(s, 16).ok()
}

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 5);
    v.extend_from_slice(&((payload.len() + 1) as u32).to_le_bytes());
    v.push(kind);
    v.extend_from_slice(payload);
    v
}

fn json_frame(v: &Value) -> Vec<u8> {
    frame(K_JSON, v.to_string().as_bytes())
}

struct Rd<'a>(&'a [u8], usize);

impl Rd<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.0.get(self.1..self.1 + n)?;
        self.1 += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
    fn f32(&mut self) -> Option<f32> {
        self.u32().map(f32::from_bits).map(|v| if v.is_finite() { v } else { 0.0 })
    }
}

// ------------------------------------------------------------------ connection

struct Outbox {
    hi: VecDeque<Vec<u8>>,
    lo: VecDeque<Vec<u8>>,
    closed: bool,
}

/// Writer side of a TCP connection. Control and movement frames go before file chunks,
/// so a big upload never delays movement data.
#[derive(Clone)]
struct Conn {
    q: Arc<(Mutex<Outbox>, Condvar)>,
    bulk_bytes: Arc<AtomicU64>,
}

impl Conn {
    fn new(mut stream: TcpStream, kbps: u32, sent: Arc<AtomicU64>) -> Conn {
        let q = Arc::new((Mutex::new(Outbox { hi: VecDeque::new(), lo: VecDeque::new(), closed: false }), Condvar::new()));
        let bulk_bytes = Arc::new(AtomicU64::new(0));
        let (q2, bb) = (q.clone(), bulk_bytes.clone());
        let _ = std::thread::Builder::new().name("net-writer".into()).spawn(move || loop {
            let (data, bulk) = {
                let (m, cv) = &*q2;
                let mut g = m.lock().unwrap();
                while g.hi.is_empty() && g.lo.is_empty() && !g.closed {
                    g = cv.wait(g).unwrap();
                }
                if g.closed && g.hi.is_empty() {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    return;
                }
                match g.hi.pop_front() {
                    Some(d) => (d, false),
                    None => (g.lo.pop_front().unwrap(), true),
                }
            };
            sent.fetch_add(data.len() as u64, Ordering::Relaxed);
            if stream.write_all(&data).is_err() {
                let (m, _) = &*q2;
                let mut g = m.lock().unwrap();
                g.closed = true;
                g.hi.clear();
                g.lo.clear();
                bb.store(0, Ordering::Relaxed);
                return;
            }
            if bulk {
                bb.fetch_sub(data.len() as u64, Ordering::Relaxed);
                if kbps > 0 {
                    std::thread::sleep(Duration::from_secs_f64(data.len() as f64 / (kbps as f64 * 1024.0)));
                }
            }
        });
        Conn { q, bulk_bytes }
    }

    fn push(&self, data: Vec<u8>, bulk: bool) {
        let (m, cv) = &*self.q;
        let mut g = m.lock().unwrap();
        if g.closed {
            return;
        }
        if bulk {
            self.bulk_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
            g.lo.push_back(data);
        } else {
            // Movement frames are only useful while fresh: keep at most 4 queued.
            if data.get(4) == Some(&K_STATE) {
                let n = g.hi.iter().filter(|f| f.get(4) == Some(&K_STATE)).count();
                if n >= 4 {
                    if let Some(p) = g.hi.iter().position(|f| f.get(4) == Some(&K_STATE)) {
                        g.hi.remove(p);
                    }
                }
            }
            g.hi.push_back(data);
        }
        cv.notify_one();
    }

    fn json(&self, v: &Value) {
        self.push(json_frame(v), false);
    }

    fn close(&self) {
        let (m, cv) = &*self.q;
        m.lock().unwrap().closed = true;
        cv.notify_all();
    }

    fn uploading(&self) -> u64 {
        self.bulk_bytes.load(Ordering::Relaxed)
    }
}

enum Event {
    /// Host: new TCP connection (peer id assigned by the listener).
    Accepted(u16, Conn),
    /// Client: connected to the host.
    Connected(Conn),
    ConnectFailed(String),
    Frame(u16, u8, Vec<u8>),
    Closed(u16),
}

fn spawn_reader(id: u16, mut stream: TcpStream, tx: Sender<Event>, stop: Arc<AtomicBool>, recv: Arc<AtomicU64>) {
    let _ = std::thread::Builder::new().name("net-reader".into()).spawn(move || {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let mut hdr = [0u8; 4];
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match read_full(&mut stream, &mut hdr, &stop) {
                Ok(true) => {}
                _ => break,
            }
            let len = u32::from_le_bytes(hdr) as usize;
            if len == 0 || len > MAX_FRAME {
                break;
            }
            let mut buf = vec![0u8; len];
            if !matches!(read_full(&mut stream, &mut buf, &stop), Ok(true)) {
                break;
            }
            recv.fetch_add(len as u64 + 4, Ordering::Relaxed);
            let kind = buf[0];
            buf.remove(0);
            if tx.send(Event::Frame(id, kind, buf)).is_err() {
                break;
            }
        }
        let _ = tx.send(Event::Closed(id));
    });
}

/// Reads exactly `buf.len()` bytes, tolerating read timeouts (used to check `stop`).
fn read_full(s: &mut TcpStream, buf: &mut [u8], stop: &AtomicBool) -> std::io::Result<bool> {
    let mut got = 0;
    while got < buf.len() {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        match s.read(&mut buf[got..]) {
            Ok(0) => return Ok(false),
            Ok(n) => got += n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

// ------------------------------------------------------------------ assets

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetMeta {
    pub id: String,
    /// "model" or "item".
    pub kind: String,
    pub name: String,
    pub size: u64,
    /// Path inside the zip of the file to load (`.model3.json` or the image).
    pub entry: String,
}

struct Asset {
    meta: Option<AssetMeta>,
    /// Zip bytes (complete, or being received).
    data: Vec<u8>,
    filled: usize,
    complete: bool,
    /// Participant that has the files (host: who to ask for an upload).
    owner: Option<u16>,
    requested: Option<Instant>,
    /// Host: participants waiting for this asset.
    waiting: HashSet<u16>,
    /// Extracted folder (receivers only).
    dir: Option<PathBuf>,
    extract: Option<Arc<Mutex<Option<Result<PathBuf, String>>>>>,
    failed: Option<String>,
}

impl Asset {
    fn new() -> Asset {
        Asset { meta: None, data: Vec::new(), filled: 0, complete: false, owner: None, requested: None, waiting: HashSet::new(), dir: None, extract: None, failed: None }
    }

    fn entry_path(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(&self.meta.as_ref()?.entry))
    }
}

type PackResult = Result<(u64, Vec<u8>, AssetMeta), String>;

/// Zips a model folder (or a single image) in memory. Returns the content hash id.
fn pack(source: &Path, kind: &str) -> PackResult {
    let is_model = source.to_string_lossy().ends_with(".model3.json");
    let root = if is_model { source.parent().ok_or("bad path")?.to_path_buf() } else { source.parent().unwrap_or(Path::new(".")).to_path_buf() };
    let mut files: Vec<PathBuf> = Vec::new();
    if is_model {
        fn walk(d: &Path, out: &mut Vec<PathBuf>, depth: u32) -> Result<(), String> {
            let mut rd: Vec<_> = std::fs::read_dir(d).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
            rd.sort();
            for p in rd {
                if p.is_dir() {
                    if depth < 6 {
                        walk(&p, out, depth + 1)?;
                    }
                } else {
                    out.push(p);
                }
                if out.len() > MAX_ASSET_FILES {
                    return Err(format!("more than {MAX_ASSET_FILES} files"));
                }
            }
            Ok(())
        }
        walk(&root, &mut files, 0)?;
        // Skip things that are never needed to display the model.
        files.retain(|p| {
            let n = p.to_string_lossy().to_lowercase();
            !(n.ends_with(".cmo3") || n.ends_with(".can3") || n.ends_with(".psd") || n.ends_with(".zip") || n.ends_with(".mp4"))
        });
    } else {
        files.push(source.to_path_buf());
    }
    let mut hasher_parts: Vec<Vec<u8>> = Vec::new();
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for f in &files {
        let meta = std::fs::metadata(f).map_err(|e| e.to_string())?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!("{} is larger than 95 MB", f.display()));
        }
        let rel = f.strip_prefix(&root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        let bytes = std::fs::read(f).map_err(|e| e.to_string())?;
        hasher_parts.push(rel.as_bytes().to_vec());
        hasher_parts.push(sha1_bytes(&[&bytes]).to_vec());
        zw.start_file(rel, opts).map_err(|e| e.to_string())?;
        zw.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    let data = zw.finish().map_err(|e| e.to_string())?.into_inner();
    if data.len() > MAX_ASSET_BYTES {
        return Err("larger than 300 MB after compression".into());
    }
    let parts: Vec<&[u8]> = hasher_parts.iter().map(|v| v.as_slice()).collect();
    let h = sha1_bytes(&parts);
    let id = u64::from_le_bytes(h[..8].try_into().unwrap());
    let entry = source.strip_prefix(&root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
    let name = source.file_name().unwrap_or_default().to_string_lossy().trim_end_matches(".model3.json").to_string();
    Ok((id, data.clone(), AssetMeta { id: asset_key(id), kind: kind.into(), name, size: data.len() as u64, entry }))
}

/// Extracts a received zip into `dir` (path traversal and size checked).
fn extract(data: &[u8], dir: &Path) -> Result<PathBuf, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
    if z.len() > MAX_ASSET_FILES {
        return Err("too many files".into());
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else { return Err("bad file name".into()) };
        let out = dir.join(rel);
        if f.is_dir() {
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::new();
        (&mut f).take(MAX_FILE_BYTES + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        if buf.len() as u64 > MAX_FILE_BYTES {
            return Err("file too large".into());
        }
        total += buf.len() as u64;
        if total > (MAX_ASSET_BYTES as u64) * 3 {
            return Err("too large".into());
        }
        std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
    }
    Ok(dir.to_path_buf())
}

// ------------------------------------------------------------------ remote participants

#[derive(Clone, Default)]
struct RemoteState {
    avatar: Option<(u64, Xform, Vec<f32>)>,
    items: Vec<(u64, Xform, i16)>,
}

pub struct RemoteItem {
    asset: u64,
    pub item: Option<Item>,
}

pub struct Peer {
    pub id: u16,
    pub nick: String,
    pub ping_ms: f32,
    /// Local display options (Session Control screen).
    pub show_model: bool,
    pub show_items: bool,
    /// Movement buffer in seconds (VTS: 0..1, default 0.05).
    pub buffer: f32,
    /// Position in the fixed order list (higher = further in front).
    pub rank: i32,
    pub front: bool,
    /// "You control position": incoming placement is ignored, `offset` is used instead.
    pub local_control: bool,
    /// x, y (fraction of window height) and scale multiplier applied locally.
    pub offset: [f32; 3],
    pub avatar_asset: Option<u64>,
    pub avatar: Option<Item>,
    pub items: Vec<RemoteItem>,
    target: RemoteState,
    params: Vec<f32>,
    xf: Option<Xform>,
    last_state: Option<Instant>,
    joined: bool,
    /// Bytes still queued for upload to this participant (host only).
    pub uploading: u64,
}

impl Peer {
    fn new(id: u16, nick: String) -> Peer {
        Peer {
            id,
            nick,
            ping_ms: 0.0,
            show_model: true,
            show_items: true,
            buffer: 0.05,
            rank: id as i32,
            front: false,
            local_control: false,
            offset: [0.0, 0.0, 1.0],
            avatar_asset: None,
            avatar: None,
            items: Vec::new(),
            target: RemoteState::default(),
            params: Vec::new(),
            xf: None,
            last_state: None,
            joined: true,
            uploading: 0,
        }
    }

    pub fn has_data(&self) -> bool {
        self.last_state.map(|t| t.elapsed() < Duration::from_secs(3)).unwrap_or(false)
    }
}

struct HostConn {
    conn: Conn,
    authed: bool,
    nonce: String,
    since: Instant,
    ping_sent: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Role {
    Host,
    Client,
}

pub struct Session {
    pub role: Role,
    pub my_id: u16,
    pub nick: String,
    pub host_nick: String,
    pub connected: bool,
    pub status: String,
    pub data_fps: u32,
    pub started: Instant,
    pub peers: BTreeMap<u16, Peer>,
    password: String,
    kbps: u32,
    rx: Receiver<Event>,
    tx: Sender<Event>,
    stop: Arc<AtomicBool>,
    /// Host: every participant connection. Client: the host under `HOST_ID`.
    conns: HashMap<u16, HostConn>,
    assets: HashMap<u64, Asset>,
    /// Local file → packed asset id (or pending job / error).
    packed: HashMap<PathBuf, Result<u64, String>>,
    pack_jobs: Vec<(PathBuf, String, Arc<Mutex<Option<PackResult>>>)>,
    cache: PathBuf,
    last_state: Instant,
    last_ping: Instant,
    ended: Option<String>,
    sent: Arc<AtomicU64>,
    recv: Arc<AtomicU64>,
    rate_mark: (Instant, u64, u64),
    /// Bandwidth in kbit/s (up, down).
    pub rate: [f32; 2],
    /// "Use fixed orders": layer participants by `Peer::rank` instead of join order.
    pub fixed_order: bool,
    /// Client: round trip to the host in ms.
    pub my_ping: f32,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for c in self.conns.values() {
            c.conn.json(&json!({ "t": "bye" }));
            c.conn.close();
        }
        let _ = std::fs::remove_dir_all(&self.cache);
    }
}

impl Session {
    fn base(role: Role, s: &NetSettings, cache: PathBuf) -> Session {
        let (tx, rx) = channel();
        let _ = std::fs::remove_dir_all(&cache);
        let nick = if s.nickname.trim().is_empty() { "Player".to_string() } else { s.nickname.trim().chars().take(24).collect() };
        Session {
            role,
            my_id: HOST_ID,
            host_nick: if role == Role::Host { nick.clone() } else { String::new() },
            nick,
            connected: false,
            status: String::new(),
            data_fps: s.data_fps.clamp(10, 60),
            started: Instant::now(),
            peers: BTreeMap::new(),
            password: s.password.clone(),
            kbps: s.upload_kbps,
            rx,
            tx,
            stop: Arc::new(AtomicBool::new(false)),
            conns: HashMap::new(),
            assets: HashMap::new(),
            packed: HashMap::new(),
            pack_jobs: Vec::new(),
            cache,
            last_state: Instant::now(),
            last_ping: Instant::now(),
            ended: None,
            sent: Arc::new(AtomicU64::new(0)),
            recv: Arc::new(AtomicU64::new(0)),
            rate_mark: (Instant::now(), 0, 0),
            rate: [0.0; 2],
            fixed_order: false,
            my_ping: 0.0,
        }
    }

    pub fn host(s: &NetSettings, cache: PathBuf) -> anyhow::Result<Session> {
        let listener = TcpListener::bind(("0.0.0.0", s.port)).map_err(|e| anyhow::anyhow!("port {}: {e}", s.port))?;
        listener.set_nonblocking(true)?;
        let mut me = Session::base(Role::Host, s, cache);
        me.connected = true;
        me.status = format!("hosting on port {}", s.port);
        let (tx, stop, kbps, sent, recv) = (me.tx.clone(), me.stop.clone(), me.kbps, me.sent.clone(), me.recv.clone());
        std::thread::Builder::new().name("net-listen".into()).spawn(move || {
            let mut next: u16 = 1;
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_nodelay(true);
                        let Ok(rs) = stream.try_clone() else { continue };
                        let id = next;
                        next = next.wrapping_add(1).max(1);
                        let conn = Conn::new(stream, kbps, sent.clone());
                        spawn_reader(id, rs, tx.clone(), stop.clone(), recv.clone());
                        if tx.send(Event::Accepted(id, conn)).is_err() {
                            break;
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        })?;
        Ok(me)
    }

    pub fn join(s: &NetSettings, cache: PathBuf) -> anyhow::Result<Session> {
        let addr = s.address.trim().to_string();
        if addr.is_empty() {
            anyhow::bail!("enter the host address");
        }
        let target = if addr.contains(':') && !addr.starts_with('[') && addr.matches(':').count() == 1 || addr.ends_with(']') || addr.contains("]:") {
            addr.clone()
        } else {
            format!("{addr}:{}", s.port)
        };
        let me = Session::base(Role::Client, s, cache);
        let (tx, stop, kbps, sent, recv) = (me.tx.clone(), me.stop.clone(), me.kbps, me.sent.clone(), me.recv.clone());
        std::thread::Builder::new().name("net-connect".into()).spawn(move || {
            let res = (|| -> Result<TcpStream, String> {
                let sa = target.to_socket_addrs().map_err(|e| format!("{target}: {e}"))?.next().ok_or("address not found")?;
                let s = TcpStream::connect_timeout(&sa, Duration::from_secs(8)).map_err(|e| format!("{target}: {e}"))?;
                let _ = s.set_nodelay(true);
                Ok(s)
            })();
            match res.and_then(|s| s.try_clone().map(|r| (s, r)).map_err(|e| e.to_string())) {
                Ok((s, r)) => {
                    let conn = Conn::new(s, kbps, sent);
                    spawn_reader(HOST_ID, r, tx.clone(), stop, recv);
                    let _ = tx.send(Event::Connected(conn));
                }
                Err(e) => {
                    let _ = tx.send(Event::ConnectFailed(e));
                }
            }
        })?;
        let mut me = me;
        me.status = "connecting…".into();
        Ok(me)
    }

    /// Bytes still queued for upload (client: to the host; host: to everyone).
    pub fn uploading(&self) -> u64 {
        self.conns.values().map(|c| c.conn.uploading()).sum()
    }

    /// Download progress 0..1 of an asset (1 when ready, None when unknown).
    pub fn asset_progress(&self, id: Option<u64>) -> Option<f32> {
        let a = self.assets.get(&id?)?;
        if a.complete {
            return Some(1.0);
        }
        Some(a.filled as f32 / a.data.len().max(1) as f32)
    }

    fn update_rate(&mut self) {
        let el = self.rate_mark.0.elapsed().as_secs_f32();
        if el >= 1.0 {
            let (s, r) = (self.sent.load(Ordering::Relaxed), self.recv.load(Ordering::Relaxed));
            self.rate = [(s - self.rate_mark.1) as f32 * 8.0 / 1000.0 / el, (r - self.rate_mark.2) as f32 * 8.0 / 1000.0 / el];
            self.rate_mark = (Instant::now(), s, r);
        }
    }

    pub fn ended(&self) -> Option<&str> {
        self.ended.as_deref()
    }

    pub fn participant_count(&self) -> usize {
        1 + self.peers.len()
    }

    /// Host: change the movement data rate for everyone.
    pub fn set_data_fps(&mut self, fps: u32) {
        self.data_fps = fps.clamp(10, 60);
        if self.role == Role::Host {
            self.broadcast(&json!({ "t": "fps", "fps": self.data_fps }), None);
        }
    }

    /// Download progress of an asset (0..1), for the UI.
    pub fn transfers(&self) -> Vec<(String, f32)> {
        self.assets
            .values()
            .filter(|a| !a.complete && a.data.len() > 0)
            .map(|a| (a.meta.as_ref().map(|m| m.name.clone()).unwrap_or_default(), a.filled as f32 / a.data.len().max(1) as f32))
            .collect()
    }

    pub fn pack_errors(&self) -> Vec<(String, String)> {
        self.packed.iter().filter_map(|(p, r)| r.as_ref().err().map(|e| (p.file_name().unwrap_or_default().to_string_lossy().into_owned(), e.clone()))).collect()
    }

    pub fn packing(&self) -> usize {
        self.pack_jobs.len()
    }

    /// Host: kick a participant.
    pub fn kick(&mut self, id: u16) {
        if self.role != Role::Host {
            return;
        }
        if let Some(c) = self.conns.remove(&id) {
            c.conn.json(&json!({ "t": "reject", "reason": "removed by host" }));
            c.conn.close();
        }
        self.peer_left(id);
    }

    fn send_to(&self, id: u16, v: &Value) {
        if let Some(c) = self.conns.get(&id) {
            c.conn.json(v);
        }
    }

    fn broadcast(&self, v: &Value, except: Option<u16>) {
        let f = json_frame(v);
        self.broadcast_raw(&f, except);
    }

    fn broadcast_raw(&self, f: &[u8], except: Option<u16>) {
        for (id, c) in &self.conns {
            if c.authed && Some(*id) != except {
                c.conn.push(f.to_vec(), false);
            }
        }
    }

    fn peer_left(&mut self, id: u16) {
        if self.peers.remove(&id).is_some() && self.role == Role::Host {
            self.broadcast(&json!({ "t": "leave", "id": id }), None);
        }
        for a in self.assets.values_mut() {
            a.waiting.remove(&id);
            if a.owner == Some(id) && !a.complete {
                a.owner = None;
                a.requested = None;
            }
        }
    }

    /// Makes sure an asset will be available locally.
    fn need(&mut self, id: u64) {
        let role = self.role;
        let a = self.assets.entry(id).or_insert_with(Asset::new);
        if a.complete || a.failed.is_some() {
            return;
        }
        let retry = a.requested.map(|t| t.elapsed() > Duration::from_secs(20)).unwrap_or(true);
        if !retry {
            return;
        }
        match role {
            Role::Client => {
                a.requested = Some(Instant::now());
                if let Some(c) = self.conns.get(&HOST_ID) {
                    c.conn.json(&json!({ "t": "get", "id": asset_key(id) }));
                }
            }
            Role::Host => {
                if let Some(owner) = a.owner {
                    a.requested = Some(Instant::now());
                    if let Some(c) = self.conns.get(&owner) {
                        c.conn.json(&json!({ "t": "upload", "id": asset_key(id) }));
                    }
                }
            }
        }
    }

    fn send_asset(&self, to: u16, id: u64) {
        let (Some(c), Some(a)) = (self.conns.get(&to), self.assets.get(&id)) else { return };
        let Some(meta) = &a.meta else { return };
        c.conn.json(&json!({ "t": "asset", "meta": meta }));
        let total = a.data.len() as u32;
        for (k, chunk) in a.data.chunks(CHUNK).enumerate() {
            let mut p = Vec::with_capacity(chunk.len() + 16);
            p.extend_from_slice(&id.to_le_bytes());
            p.extend_from_slice(&total.to_le_bytes());
            p.extend_from_slice(&((k * CHUNK) as u32).to_le_bytes());
            p.extend_from_slice(chunk);
            c.conn.push(frame(K_CHUNK, &p), true);
        }
    }

    fn on_asset_complete(&mut self, id: u64) {
        let Some(a) = self.assets.get_mut(&id) else { return };
        // Verify the zip is what the id says: recompute the content hash.
        let ok = verify(&a.data, id);
        if !ok {
            a.failed = Some("checksum mismatch".into());
            a.data = Vec::new();
            return;
        }
        a.complete = true;
        let dir = self.cache.join(asset_key(id));
        let data = a.data.clone();
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            *s2.lock().unwrap() = Some(extract(&data, &dir));
        });
        a.extract = Some(slot);
        let waiting: Vec<u16> = a.waiting.drain().collect();
        for w in waiting {
            self.send_asset(w, id);
        }
    }

    fn on_chunk(&mut self, from: u16, p: &[u8]) {
        let mut r = Rd(p, 0);
        let (Some(id), Some(total), Some(off)) = (r.u64(), r.u32(), r.u32()) else { return };
        let data = &p[16..];
        let (total, off) = (total as usize, off as usize);
        if total > MAX_ASSET_BYTES || off + data.len() > total {
            return;
        }
        let role = self.role;
        let a = self.assets.entry(id).or_insert_with(Asset::new);
        if a.complete {
            return;
        }
        // Host only accepts uploads from the owner it asked; clients only from the host.
        let allowed = match role {
            Role::Host => a.owner == Some(from),
            Role::Client => from == HOST_ID,
        };
        if !allowed {
            return;
        }
        if a.data.len() != total {
            a.data = vec![0u8; total];
            a.filled = 0;
        }
        a.data[off..off + data.len()].copy_from_slice(data);
        a.filled += data.len();
        if a.filled >= total {
            self.on_asset_complete(id);
        }
    }

    fn on_state(&mut self, from: u16, p: &[u8]) {
        let mut r = Rd(p, 0);
        let Some(pid) = r.u16() else { return };
        let pid = if self.role == Role::Host { from } else { pid };
        if pid == self.my_id {
            return;
        }
        let Some(st) = parse_state(&mut r) else { return };
        let mut needed = Vec::new();
        if let Some(peer) = self.peers.get_mut(&pid) {
            if let Some((a, _, _)) = &st.avatar {
                needed.push(*a);
            }
            needed.extend(st.items.iter().map(|i| i.0));
            peer.target = st;
            peer.last_state = Some(Instant::now());
        }
        for a in needed {
            self.need(a);
        }
        if self.role == Role::Host {
            // Relay with the sender's id.
            let mut q = p.to_vec();
            q[..2].copy_from_slice(&pid.to_le_bytes());
            self.broadcast_raw(&frame(K_STATE, &q), Some(pid));
        }
    }

    fn handle_json(&mut self, from: u16, v: Value) {
        let t = v["t"].as_str().unwrap_or("").to_string();
        if self.role == Role::Host {
            let authed = self.conns.get(&from).map(|c| c.authed).unwrap_or(false);
            if !authed {
                if t == "hello" {
                    self.host_hello(from, &v);
                }
                return;
            }
        }
        match t.as_str() {
            // ---- client side
            "challenge" if self.role == Role::Client => {
                let n = v["nonce"].as_str().unwrap_or("");
                self.send_to(HOST_ID, &json!({ "t": "hello", "v": PROTOCOL, "nick": self.nick, "proof": proof(n, &self.password), "app": env!("CARGO_PKG_VERSION") }));
            }
            "welcome" if self.role == Role::Client => {
                self.my_id = v["id"].as_u64().unwrap_or(0) as u16;
                self.data_fps = v["fps"].as_u64().unwrap_or(30).clamp(10, 60) as u32;
                self.host_nick = v["host"].as_str().unwrap_or("").to_string();
                self.connected = true;
                self.status = "connected".into();
                for p in v["peers"].as_array().into_iter().flatten() {
                    let id = p["id"].as_u64().unwrap_or(0) as u16;
                    if id != self.my_id {
                        self.peers.insert(id, Peer::new(id, p["nick"].as_str().unwrap_or("?").to_string()));
                    }
                }
                // Tell the host about the files we already packed.
                let metas: Vec<AssetMeta> = self.assets.values().filter(|a| a.complete && a.owner == Some(u16::MAX)).filter_map(|a| a.meta.clone()).collect();
                for m in metas {
                    self.send_to(HOST_ID, &json!({ "t": "asset", "meta": m }));
                }
            }
            "reject" => {
                self.ended = Some(v["reason"].as_str().unwrap_or("rejected").to_string());
            }
            "end" => self.ended = Some("the host ended the session".into()),
            "join" if self.role == Role::Client => {
                let id = v["id"].as_u64().unwrap_or(0) as u16;
                if id != self.my_id {
                    self.peers.insert(id, Peer::new(id, v["nick"].as_str().unwrap_or("?").to_string()));
                }
            }
            "leave" if self.role == Role::Client => {
                let id = v["id"].as_u64().unwrap_or(0) as u16;
                self.peers.remove(&id);
            }
            "fps" if self.role == Role::Client => self.data_fps = v["fps"].as_u64().unwrap_or(30).clamp(10, 60) as u32,
            "ping" => self.send_to(from, &json!({ "t": "pong", "ts": v["ts"] })),
            "pings" if self.role == Role::Client => {
                for e in v["list"].as_array().into_iter().flatten() {
                    let id = e[0].as_u64().unwrap_or(0) as u16;
                    if id == self.my_id {
                        self.my_ping = e[1].as_f64().unwrap_or(0.0) as f32;
                    }
                    if let Some(p) = self.peers.get_mut(&id) {
                        p.ping_ms = e[1].as_f64().unwrap_or(0.0) as f32;
                    }
                }
            }
            "upload" if self.role == Role::Client => {
                if let Some(id) = v["id"].as_str().and_then(parse_key) {
                    if self.assets.get(&id).map(|a| a.complete && a.owner == Some(u16::MAX)).unwrap_or(false) {
                        self.send_asset(HOST_ID, id);
                    }
                }
            }
            "asset" => {
                let Ok(meta) = serde_json::from_value::<AssetMeta>(v["meta"].clone()) else { return };
                let Some(id) = parse_key(&meta.id) else { return };
                if meta.size as usize > MAX_ASSET_BYTES {
                    return;
                }
                let role = self.role;
                let a = self.assets.entry(id).or_insert_with(Asset::new);
                if a.owner == Some(u16::MAX) {
                    return; // our own
                }
                a.meta = Some(meta);
                if role == Role::Host && a.owner.is_none() {
                    a.owner = Some(from);
                    // Somebody may already be waiting for it.
                    if !a.waiting.is_empty() || a.requested.is_some() {
                        a.requested = None;
                        self.need(id);
                    }
                }
            }
            // ---- host side
            "get" if self.role == Role::Host => {
                let Some(id) = v["id"].as_str().and_then(parse_key) else { return };
                let complete = self.assets.get(&id).map(|a| a.complete).unwrap_or(false);
                if complete {
                    self.send_asset(from, id);
                } else {
                    self.assets.entry(id).or_insert_with(Asset::new).waiting.insert(from);
                    self.need(id);
                }
            }
            "pong" if self.role == Role::Host => {
                if let Some(c) = self.conns.get_mut(&from) {
                    if let Some(t) = c.ping_sent.take() {
                        if let Some(p) = self.peers.get_mut(&from) {
                            p.ping_ms = t.elapsed().as_secs_f32() * 1000.0;
                        }
                    }
                }
            }
            "bye" => {
                if self.role == Role::Host {
                    if let Some(c) = self.conns.remove(&from) {
                        c.conn.close();
                    }
                    self.peer_left(from);
                } else {
                    self.ended = Some("the host ended the session".into());
                }
            }
            _ => {}
        }
    }

    fn host_hello(&mut self, from: u16, v: &Value) {
        let Some(c) = self.conns.get(&from) else { return };
        let reject = |c: &HostConn, why: &str| {
            c.conn.json(&json!({ "t": "reject", "reason": why }));
            c.conn.close();
        };
        if v["v"].as_u64() != Some(PROTOCOL as u64) {
            reject(c, "different app/protocol version");
            self.conns.remove(&from);
            return;
        }
        if v["proof"].as_str() != Some(proof(&c.nonce, &self.password).as_str()) {
            reject(c, "wrong password");
            self.conns.remove(&from);
            return;
        }
        if self.participant_count() >= MAX_PARTICIPANTS {
            reject(c, "the session is full (6 participants)");
            self.conns.remove(&from);
            return;
        }
        let nick: String = v["nick"].as_str().unwrap_or("Player").chars().take(24).collect();
        let mut list = vec![json!({ "id": HOST_ID, "nick": self.nick })];
        list.extend(self.peers.values().map(|p| json!({ "id": p.id, "nick": p.nick })));
        c.conn.json(&json!({ "t": "welcome", "id": from, "fps": self.data_fps, "host": self.nick, "peers": list }));
        self.broadcast(&json!({ "t": "join", "id": from, "nick": nick }), Some(from));
        if let Some(c) = self.conns.get_mut(&from) {
            c.authed = true;
        }
        self.peers.insert(from, Peer::new(from, nick));
    }

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::Accepted(id, conn) => {
                let n = nonce();
                conn.json(&json!({ "t": "challenge", "nonce": n, "v": PROTOCOL }));
                self.conns.insert(id, HostConn { conn, authed: false, nonce: n, since: Instant::now(), ping_sent: None });
            }
            Event::Connected(conn) => {
                self.status = "authenticating…".into();
                self.conns.insert(HOST_ID, HostConn { conn, authed: true, nonce: String::new(), since: Instant::now(), ping_sent: None });
            }
            Event::ConnectFailed(e) => self.ended = Some(format!("cannot connect: {e}")),
            Event::Frame(from, kind, data) => match kind {
                K_JSON => {
                    if let Ok(v) = serde_json::from_slice::<Value>(&data) {
                        self.handle_json(from, v);
                    }
                }
                K_STATE => {
                    let authed = self.conns.get(&from).map(|c| c.authed).unwrap_or(false);
                    if authed && (self.role == Role::Client || self.peers.contains_key(&from)) {
                        self.on_state(from, &data);
                    }
                }
                K_CHUNK => {
                    if self.conns.get(&from).map(|c| c.authed).unwrap_or(false) {
                        self.on_chunk(from, &data);
                    }
                }
                _ => {}
            },
            Event::Closed(id) => match self.role {
                Role::Host => {
                    self.conns.remove(&id);
                    self.peer_left(id);
                }
                Role::Client => {
                    if self.ended.is_none() {
                        self.ended = Some("connection to the host lost".into());
                    }
                }
            },
        }
    }

    /// Starts packing a local file if needed; returns the asset id once ready.
    fn local_asset(&mut self, path: &Path, kind: &str) -> Option<u64> {
        match self.packed.get(path) {
            Some(Ok(id)) => return Some(*id),
            Some(Err(_)) => return None,
            None => {}
        }
        if self.pack_jobs.iter().any(|j| j.0 == path) {
            return None;
        }
        let slot = Arc::new(Mutex::new(None));
        let (s2, p2, k2) = (slot.clone(), path.to_path_buf(), kind.to_string());
        std::thread::spawn(move || {
            *s2.lock().unwrap() = Some(pack(&p2, &k2));
        });
        self.pack_jobs.push((path.to_path_buf(), kind.to_string(), slot));
        None
    }

    fn poll_jobs(&mut self) {
        let mut done = Vec::new();
        self.pack_jobs.retain(|(p, _, slot)| match slot.lock().unwrap().take() {
            Some(r) => {
                done.push((p.clone(), r));
                false
            }
            None => true,
        });
        for (p, r) in done {
            match r {
                Ok((id, data, meta)) => {
                    let a = self.assets.entry(id).or_insert_with(Asset::new);
                    a.data = data;
                    a.filled = a.data.len();
                    a.complete = true;
                    // u16::MAX marks "our own files".
                    a.owner = Some(u16::MAX);
                    a.meta = Some(meta.clone());
                    let waiting: Vec<u16> = a.waiting.drain().collect();
                    if self.role == Role::Client && self.connected {
                        self.send_to(HOST_ID, &json!({ "t": "asset", "meta": meta }));
                    }
                    for w in waiting {
                        self.send_asset(w, id);
                    }
                    self.packed.insert(p, Ok(id));
                }
                Err(e) => {
                    self.packed.insert(p, Err(e));
                }
            }
        }
        for a in self.assets.values_mut() {
            let res = a.extract.as_ref().and_then(|slot| slot.lock().unwrap().take());
            {
                if let Some(r) = res {
                    match r {
                        Ok(d) => a.dir = Some(d),
                        Err(e) => a.failed = Some(e),
                    }
                    a.extract = None;
                    // The zip is no longer needed on clients (the host keeps it to serve others).
                }
            }
        }
    }

    /// Every remote item to draw (avatars and their items).
    pub fn scene_items(&self) -> Vec<&Item> {
        let mut v = Vec::new();
        for p in self.peers.values() {
            if let Some(a) = p.avatar.as_ref().filter(|_| p.show_model) {
                v.push(a);
            }
            if p.show_items {
                v.extend(p.items.iter().filter_map(|i| i.item.as_ref()));
            }
        }
        v
    }

    pub fn scene_items_mut(&mut self) -> Vec<&mut Item> {
        let mut v = Vec::new();
        for p in self.peers.values_mut() {
            if let Some(a) = p.avatar.as_mut().filter(|_| p.show_model) {
                v.push(a);
            }
            if p.show_items {
                v.extend(p.items.iter_mut().filter_map(|i| i.item.as_mut()));
            }
        }
        v
    }

    pub fn asset_state(&self, id: Option<u64>) -> &'static str {
        let Some(id) = id else { return "" };
        match self.assets.get(&id) {
            Some(a) if a.failed.is_some() => "error",
            Some(a) if a.dir.is_some() => "ready",
            Some(a) if a.complete => "unpacking",
            Some(a) if a.filled > 0 => "downloading",
            _ => "waiting",
        }
    }
}

fn verify(data: &[u8], id: u64) -> bool {
    let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(data)) else { return false };
    let mut parts: Vec<Vec<u8>> = Vec::new();
    for i in 0..z.len() {
        let Ok(mut f) = z.by_index(i) else { return false };
        let name = f.name().to_string();
        let mut buf = Vec::new();
        if (&mut f).take(MAX_FILE_BYTES + 1).read_to_end(&mut buf).is_err() {
            return false;
        }
        parts.push(name.into_bytes());
        parts.push(sha1_bytes(&[&buf]).to_vec());
    }
    let refs: Vec<&[u8]> = parts.iter().map(|v| v.as_slice()).collect();
    let h = sha1_bytes(&refs);
    u64::from_le_bytes(h[..8].try_into().unwrap()) == id
}

fn parse_state(r: &mut Rd) -> Option<RemoteState> {
    let mut st = RemoteState::default();
    if r.u8()? == 1 {
        let asset = r.u64()?;
        let xf = Xform { pos: [r.f32()?, r.f32()?], scale: r.f32()?, rot: r.f32()?, flip: r.u8()? != 0 };
        let n = r.u16()? as usize;
        let mut params = Vec::with_capacity(n);
        for _ in 0..n {
            params.push(r.f32()?.clamp(-PARAM_LIMIT, PARAM_LIMIT));
        }
        st.avatar = Some((asset, xf, params));
    }
    let m = (r.u16()? as usize).min(64);
    for _ in 0..m {
        let asset = r.u64()?;
        let xf = Xform { pos: [r.f32()?, r.f32()?], scale: r.f32()?, rot: r.f32()?, flip: r.u8()? != 0 };
        st.items.push((asset, xf, r.i16()?));
    }
    Some(st)
}

fn put_xf(out: &mut Vec<u8>, xf: &Xform, vh: f32) {
    for v in [xf.pos[0] / vh, xf.pos[1] / vh, xf.scale / vh, xf.rot] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.push(xf.flip as u8);
}

fn denorm(xf: &Xform, vh: f32) -> Xform {
    Xform { pos: [xf.pos[0] * vh, xf.pos[1] * vh], scale: xf.scale * vh, rot: xf.rot, flip: xf.flip }
}

// ------------------------------------------------------------------ app integration

/// Starts hosting or joining with the current settings.
pub fn start(app: &mut App) {
    let s = app.settings.net.clone();
    let cache = app.paths.data.join("net_cache");
    let r = if s.host { Session::host(&s, cache) } else { Session::join(&s, cache) };
    match r {
        Ok(sess) => app.net = Some(sess),
        Err(e) => app.toast(format!("Коллаб: {e:#}")),
    }
}

pub fn stop(app: &mut App) {
    if let Some(s) = &app.net {
        if s.role == Role::Host {
            s.broadcast(&json!({ "t": "end" }), None);
        }
    }
    app.net = None;
}

/// Called once per frame before the scene update.
pub fn pump(app: &mut App, dt: f32) {
    let Some(mut s) = app.net.take() else { return };
    for _ in 0..512 {
        match s.rx.try_recv() {
            Ok(ev) => s.handle(ev),
            Err(_) => break,
        }
    }
    s.poll_jobs();
    s.update_rate();
    if let Some(e) = s.ended.clone() {
        app.toast(format!("Коллаб завершён: {e}"));
        drop(s);
        return;
    }
    // Host housekeeping: unauthenticated connections, pings, the 24 h limit.
    if s.role == Role::Host {
        let stale: Vec<u16> = s.conns.iter().filter(|(_, c)| !c.authed && c.since.elapsed() > Duration::from_secs(10)).map(|(id, _)| *id).collect();
        for id in stale {
            if let Some(c) = s.conns.remove(&id) {
                c.conn.close();
            }
        }
        if s.last_ping.elapsed() > Duration::from_secs(2) {
            s.last_ping = Instant::now();
            let list: Vec<Value> = std::iter::once(json!([HOST_ID, 0.0])).chain(s.peers.values().map(|p| json!([p.id, p.ping_ms]))).collect();
            s.broadcast(&json!({ "t": "pings", "list": list }), None);
            for c in s.conns.values_mut().filter(|c| c.authed) {
                c.ping_sent = Some(Instant::now());
                c.conn.json(&json!({ "t": "ping", "ts": 0 }));
            }
        }
        let up: Vec<(u16, u64)> = s.conns.iter().map(|(id, c)| (*id, c.conn.uploading())).collect();
        for (id, b) in up {
            if let Some(p) = s.peers.get_mut(&id) {
                p.uploading = b;
            }
        }
        if s.started.elapsed() > Duration::from_secs(24 * 3600) {
            s.broadcast(&json!({ "t": "end" }), None);
            app.toast("Коллаб завершён: прошло 24 часа (как в VTS)");
            return;
        }
    }

    // Own state.
    let (_, vh) = app.viewport();
    let vh = vh.max(1.0);
    if s.connected && s.last_state.elapsed().as_secs_f32() >= 1.0 / s.data_fps as f32 {
        s.last_state = Instant::now();
        let mut out = Vec::with_capacity(2048);
        out.extend_from_slice(&s.my_id.to_le_bytes());
        let avatar = app.avatar.as_ref().map(|a| (a.model3.clone(), a.eff_xf(), a.model.params().to_vec()));
        match avatar.and_then(|(p, xf, params)| s.local_asset(&p, "model").map(|id| (id, xf, params))) {
            Some((id, xf, params)) => {
                out.push(1);
                out.extend_from_slice(&id.to_le_bytes());
                put_xf(&mut out, &xf, vh);
                let n = params.len().min(4096);
                out.extend_from_slice(&(n as u16).to_le_bytes());
                for v in &params[..n] {
                    out.extend_from_slice(&v.clamp(-PARAM_LIMIT, PARAM_LIMIT).to_le_bytes());
                }
            }
            None => out.push(0),
        }
        let items: Vec<(PathBuf, Xform, i32)> = if app.settings.net.share_items {
            app.items.iter().filter(|i| i.alpha_target > 0.0).map(|i| (i.source.clone(), i.xf, i.cfg.order)).collect()
        } else {
            Vec::new()
        };
        let mut ready = Vec::new();
        for (p, xf, order) in items.into_iter().take(64) {
            if let Some(id) = s.local_asset(&p, "item") {
                ready.push((id, xf, order));
            }
        }
        out.extend_from_slice(&(ready.len() as u16).to_le_bytes());
        for (id, xf, order) in ready {
            out.extend_from_slice(&id.to_le_bytes());
            put_xf(&mut out, &xf, vh);
            out.extend_from_slice(&(order.clamp(-500, 500) as i16).to_le_bytes());
        }
        let f = frame(K_STATE, &out);
        s.broadcast_raw(&f, None);
    }

    // Remote participants: load assets, apply movement.
    let smoothing = app.settings.net.smoothing;
    let mut ids: Vec<u16> = s.peers.keys().copied().collect();
    if s.fixed_order {
        ids.sort_by_key(|id| s.peers[id].rank);
    }
    for (slot, pid) in ids.into_iter().enumerate() {
        let tau = s.peers[&pid].buffer.max(0.0) * 0.5;
        let k = if !smoothing || tau <= 0.0 { 1.0 } else { 1.0 - (-dt / tau).exp() };
        let target = s.peers[&pid].target.clone();
        // Avatar.
        let want = target.avatar.as_ref().map(|a| a.0);
        if s.peers[&pid].avatar_asset != want {
            let p = s.peers.get_mut(&pid).unwrap();
            p.avatar = None;
            p.avatar_asset = want;
            p.params.clear();
            p.xf = None;
        }
        if let Some(id) = want {
            if s.peers[&pid].avatar.is_none() {
                if let Some(path) = s.assets.get(&id).and_then(|a| a.entry_path()) {
                    match app.make_item(&path) {
                        Ok(mut it) => {
                            it.pivot = [0.0, 0.0];
                            it.cfg.default_unpinned = true;
                            if let Body::Live2D(l) = &mut it.body {
                                l.net_params = Some(Vec::new());
                            }
                            s.peers.get_mut(&pid).unwrap().avatar = Some(it);
                        }
                        Err(e) => {
                            if let Some(a) = s.assets.get_mut(&id) {
                                a.failed = Some(format!("{e:#}"));
                            }
                        }
                    }
                }
            }
        }
        // Items: keep loaded ones that are still used (matched by asset).
        let p = s.peers.get_mut(&pid).unwrap();
        let same = p.items.len() == target.items.len() && p.items.iter().zip(&target.items).all(|(a, b)| a.asset == b.0);
        if !same {
            let mut old: Vec<RemoteItem> = std::mem::take(&mut p.items);
            for (asset, _, _) in &target.items {
                let reuse = old.iter().position(|o| o.asset == *asset).map(|i| old.remove(i));
                p.items.push(reuse.unwrap_or(RemoteItem { asset: *asset, item: None }));
            }
        }
        let need_load: Vec<(usize, u64)> = p.items.iter().enumerate().filter(|(_, i)| i.item.is_none()).map(|(k, i)| (k, i.asset)).collect();
        for (k_i, asset) in need_load {
            let Some(path) = s.assets.get(&asset).and_then(|a| a.entry_path()) else { continue };
            match app.make_item(&path) {
                Ok(it) => s.peers.get_mut(&pid).unwrap().items[k_i].item = Some(it),
                Err(e) => {
                    if let Some(a) = s.assets.get_mut(&asset) {
                        a.failed = Some(format!("{e:#}"));
                    }
                }
            }
        }

        // Movement.
        let p = s.peers.get_mut(&pid).unwrap();
        let base: i32 = if p.front { 1000 + slot as i32 * 100 } else { -1000 - (6 - slot as i32) * 100 };
        if let (Some(av), Some((_, xf, params))) = (&mut p.avatar, &target.avatar) {
            let mut tx = denorm(xf, vh);
            if p.local_control {
                tx.pos = [p.offset[0] * vh, p.offset[1] * vh];
                tx.scale *= p.offset[2];
            }
            let cur = p.xf.get_or_insert(tx);
            cur.pos[0] += (tx.pos[0] - cur.pos[0]) * k;
            cur.pos[1] += (tx.pos[1] - cur.pos[1]) * k;
            cur.scale += (tx.scale - cur.scale) * k;
            cur.rot += (tx.rot - cur.rot) * k;
            cur.flip = tx.flip;
            if p.params.len() != params.len() {
                p.params = params.clone();
            }
            for (c, t) in p.params.iter_mut().zip(params) {
                *c += (t - *c) * k;
            }
            av.xf = *cur;
            av.cfg.order = base;
            if let Body::Live2D(l) = &mut av.body {
                l.net_params = Some(p.params.clone());
            }
            av.update(dt, &NEUTRAL, &NEUTRAL, false, app.settings.physics);
        }
        for (ri, (_, xf, order)) in p.items.iter_mut().zip(&target.items) {
            if let Some(it) = &mut ri.item {
                let t = denorm(xf, vh);
                let (dx, dy, sk) = if p.local_control {
                    let a = target.avatar.as_ref().map(|a| denorm(&a.1, vh)).unwrap_or_default();
                    (p.offset[0] * vh - a.pos[0], p.offset[1] * vh - a.pos[1], p.offset[2])
                } else {
                    (0.0, 0.0, 1.0)
                };
                it.xf.pos[0] += (t.pos[0] + dx - it.xf.pos[0]) * k;
                it.xf.pos[1] += (t.pos[1] + dy - it.xf.pos[1]) * k;
                it.xf.scale += (t.scale * sk - it.xf.scale) * k;
                it.xf.rot += (t.rot - it.xf.rot) * k;
                it.xf.flip = t.flip;
                it.cfg.order = base + *order as i32;
                it.update(dt, &NEUTRAL, &NEUTRAL, false, app.settings.physics);
            }
        }
    }
    app.net = Some(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_verify_extract() {
        let dir = std::env::temp_dir().join(format!("rigrs-net-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("m/tex")).unwrap();
        std::fs::write(dir.join("m/a.model3.json"), b"{}").unwrap();
        std::fs::write(dir.join("m/tex/t.png"), vec![7u8; 100_000]).unwrap();
        let (id, data, meta) = pack(&dir.join("m/a.model3.json"), "model").unwrap();
        assert_eq!(meta.entry, "a.model3.json");
        assert!(verify(&data, id));
        assert!(!verify(&data, id ^ 1));
        let out = extract(&data, &dir.join("out")).unwrap();
        assert_eq!(std::fs::read(out.join("tex/t.png")).unwrap().len(), 100_000);
        // Same content → same id.
        let (id2, _, _) = pack(&dir.join("m/a.model3.json"), "model").unwrap();
        assert_eq!(id, id2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_roundtrip() {
        let mut out = Vec::new();
        out.extend_from_slice(&7u16.to_le_bytes());
        out.push(1);
        out.extend_from_slice(&42u64.to_le_bytes());
        put_xf(&mut out, &Xform { pos: [100.0, -50.0], scale: 2.0, rot: 5.0, flip: true }, 1000.0);
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&0.5f32.to_le_bytes());
        out.extend_from_slice(&99999f32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        let mut r = Rd(&out, 0);
        assert_eq!(r.u16(), Some(7));
        let st = parse_state(&mut r).unwrap();
        let (a, xf, p) = st.avatar.unwrap();
        assert_eq!(a, 42);
        assert!(xf.flip);
        assert!((denorm(&xf, 1000.0).pos[0] - 100.0).abs() < 1e-3);
        assert_eq!(p, vec![0.5, PARAM_LIMIT]);
    }

    /// Full host ↔ client handshake over localhost, including a model transfer.
    #[test]
    fn handshake_and_transfer() {
        let base = std::env::temp_dir().join(format!("rigrs-net-hs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("m")).unwrap();
        std::fs::write(base.join("m/a.model3.json"), b"{\"x\":1}").unwrap();
        let mut hs = NetSettings { port: 25591, password: "pw".into(), nickname: "Host".into(), ..Default::default() };
        let mut host = Session::host(&hs, base.join("hc")).unwrap();
        hs.host = false;
        hs.address = "127.0.0.1".into();
        hs.nickname = "Guest".into();
        let mut client = Session::join(&hs, base.join("cc")).unwrap();
        let step = |s: &mut Session| {
            while let Ok(ev) = s.rx.try_recv() {
                s.handle(ev);
            }
            s.poll_jobs();
        };
        let t0 = Instant::now();
        while !(client.connected && host.peers.len() == 1) && t0.elapsed() < Duration::from_secs(5) {
            step(&mut host);
            step(&mut client);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(client.connected, "client did not connect: {:?}", client.ended);
        assert_eq!(client.peers.get(&HOST_ID).map(|p| p.nick.as_str()), Some("Host"));
        // Client packs a model; the host needs it.
        let path = base.join("m/a.model3.json");
        while client.local_asset(&path, "model").is_none() && t0.elapsed() < Duration::from_secs(5) {
            step(&mut client);
            std::thread::sleep(Duration::from_millis(10));
        }
        let id = client.local_asset(&path, "model").unwrap();
        let t1 = Instant::now();
        while host.asset_state(Some(id)) != "ready" && t1.elapsed() < Duration::from_secs(5) {
            host.need(id);
            step(&mut host);
            step(&mut client);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(host.asset_state(Some(id)), "ready");
        let p = host.assets[&id].entry_path().unwrap();
        assert_eq!(std::fs::read(p).unwrap(), b"{\"x\":1}");
        // Wrong password is rejected.
        hs.password = "nope".into();
        let mut bad = Session::join(&hs, base.join("bc")).unwrap();
        let t2 = Instant::now();
        while bad.ended.is_none() && t2.elapsed() < Duration::from_secs(5) {
            step(&mut host);
            step(&mut bad);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(bad.ended.as_deref(), Some("wrong password"));
        drop(client);
        drop(host);
        let _ = std::fs::remove_dir_all(&base);
    }
}
