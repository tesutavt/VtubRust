//! Face tracking backends. All of them run outside our process and stream UDP to us:
//!
//! * OpenSeeFace / Mediapipe: `facetracker.exe` (BSD-2, a copy ships with VTube Studio).
//! * NVIDIA Broadcast (RTX): `ExpressionApp.exe` from VTube Studio's free
//!   "NVIDIA Broadcast Tracker" DLC (`StreamingAssets\MXTracker`). It sends JSON with
//!   53 ARKit-style expression values and a head quaternion to UDP 9140..9145.
//! * Phone: VTube Studio for iPhone / Android (or any app speaking the same protocol,
//!   e.g. MeowFace). We ask the phone for data with an `iOSTrackingDataRequest` on UDP
//!   21412 and it streams JSON with the 52 ARKit blendshapes back to us. The same phone can
//!   feed VBridger at the same time (the request lists our own port only).
//!
//! Everything is converted into VTS-style tracking inputs.

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::mapping::{input, Inputs, NEUTRAL};

const FACE_PACKET: usize = 1785;
/// Mediapipe face packet: 24-byte header + 4x4 matrix + 52 blendshapes + 478 xyz landmarks.
const MP_PACKET: usize = 24 + (16 + 52 + 478 * 3) * 4;
/// Hand packet: 40-byte header + 2 hands x 21 points x (flag, x, y, z).
const HAND_PACKET: usize = 712;

/// MediaPipe / ARKit blendshape indices used here.
pub mod bs {
    pub const BROW_DOWN_L: usize = 1;
    pub const BROW_DOWN_R: usize = 2;
    pub const BROW_INNER_UP: usize = 3;
    pub const BROW_OUTER_UP_L: usize = 4;
    pub const BROW_OUTER_UP_R: usize = 5;
    pub const CHEEK_PUFF: usize = 6;
    pub const EYE_BLINK_L: usize = 9;
    pub const EYE_BLINK_R: usize = 10;
    pub const EYE_LOOK_DOWN_L: usize = 11;
    pub const EYE_LOOK_DOWN_R: usize = 12;
    pub const EYE_LOOK_IN_L: usize = 13;
    pub const EYE_LOOK_IN_R: usize = 14;
    pub const EYE_LOOK_OUT_L: usize = 15;
    pub const EYE_LOOK_OUT_R: usize = 16;
    pub const EYE_LOOK_UP_L: usize = 17;
    pub const EYE_LOOK_UP_R: usize = 18;
    pub const EYE_WIDE_L: usize = 21;
    pub const EYE_WIDE_R: usize = 22;
    pub const JAW_OPEN: usize = 25;
    pub const MOUTH_FROWN_L: usize = 30;
    pub const MOUTH_FROWN_R: usize = 31;
    pub const MOUTH_LEFT: usize = 33;
    pub const MOUTH_RIGHT: usize = 39;
    pub const MOUTH_SMILE_L: usize = 44;
    pub const MOUTH_SMILE_R: usize = 45;
    pub const NOSE_SNEER_L: usize = 50;
    pub const NOSE_SNEER_R: usize = 51;
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FaceKind {
    OpenSeeFace,
    Mediapipe,
    /// NVIDIA Broadcast expression tracker (ExpressionApp.exe).
    Nvidia,
    /// VTube Studio phone app (iPhone ARKit / Android).
    Phone,
}

impl FaceKind {
    /// True when `bs` holds ARKit blendshapes.
    pub fn has_blendshapes(self) -> bool {
        self != FaceKind::OpenSeeFace
    }
}

/// Tracking backends (`Settings::track_backend`).
pub mod backend {
    pub const OPENSEEFACE: u8 = 0;
    pub const MEDIAPIPE: u8 = 1;
    pub const NVIDIA: u8 = 2;
    pub const PHONE: u8 = 3;
}

#[derive(Clone, Copy)]
pub struct FaceFrame {
    pub kind: FaceKind,
    /// ARKit blendshapes in Mediapipe order (zero for OpenSeeFace), see `perfect_sync::ARKIT_NAMES`.
    pub bs: [f32; 52],
    /// ARKit `tongueOut` (phone tracking only).
    pub tongue: f32,
    /// Landmarks in normalised image coordinates (preview only): Mediapipe / NVIDIA.
    pub mp: [[f32; 2]; 478],
    /// Number of valid points in `mp`.
    pub n_mp: u16,
    pub success: bool,
    /// [right, left] eye openness as reported by the tracker (subject's perspective).
    pub eye_open: [f32; 2],
    pub euler: [f32; 3],
    pub translation: [f32; 3],
    pub lms: [[f32; 2]; 68],
    pub features: [f32; 14],
    /// Mean landmark confidence 0..1.
    pub confidence: f32,
    /// Camera frame size in pixels.
    pub size: [f32; 2],
    pub received: Instant,
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn parse(b: &[u8]) -> Option<FaceFrame> {
    if b.len() < FACE_PACKET {
        return None;
    }
    let mut f = FaceFrame {
        kind: FaceKind::OpenSeeFace,
        bs: [0.0; 52],
        tongue: 0.0,
        mp: [[0.0; 2]; 478],
        n_mp: 0,
        success: b[28] != 0,
        eye_open: [f32_at(b, 20), f32_at(b, 24)],
        euler: [f32_at(b, 49), f32_at(b, 53), f32_at(b, 57)],
        translation: [f32_at(b, 61), f32_at(b, 65), f32_at(b, 69)],
        lms: [[0.0; 2]; 68],
        features: [0.0; 14],
        confidence: (0..68).map(|i| f32_at(b, 73 + i * 4)).sum::<f32>() / 68.0,
        size: [f32_at(b, 12), f32_at(b, 16)],
        received: Instant::now(),
    };
    let mut o = 345;
    for p in f.lms.iter_mut() {
        *p = [f32_at(b, o), f32_at(b, o + 4)];
        o += 8;
    }
    let mut o = 1729;
    for v in f.features.iter_mut() {
        *v = f32_at(b, o);
        o += 4;
    }
    Some(f)
}

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Mediapipe face packet. Pose comes as a row-major 4x4 matrix (translation in cm).
fn parse_mp(b: &[u8]) -> Option<FaceFrame> {
    if b.len() < MP_PACKET || i32_at(b, 8) < 1 {
        return None;
    }
    let fl = |i: usize| f32_at(b, 24 + i * 4);
    let r = |row: usize, col: usize| fl(row * 4 + col);
    let pitch = (-r(1, 2)).clamp(-1.0, 1.0).asin().to_degrees();
    let yaw = r(0, 2).atan2(r(2, 2)).to_degrees();
    let roll = r(1, 0).atan2(r(1, 1)).to_degrees();
    let mut f = FaceFrame {
        kind: FaceKind::Mediapipe,
        bs: [0.0; 52],
        tongue: 0.0,
        mp: [[0.0; 2]; 478],
        n_mp: 478,
        success: true,
        eye_open: [0.0; 2],
        euler: [pitch, yaw, roll],
        translation: [r(0, 3), r(1, 3), r(2, 3)],
        lms: [[0.0; 2]; 68],
        features: [0.0; 14],
        confidence: 1.0,
        size: [i32_at(b, 16) as f32, i32_at(b, 20) as f32],
        received: Instant::now(),
    };
    for (i, v) in f.bs.iter_mut().enumerate() {
        *v = fl(16 + i);
    }
    for (i, p) in f.mp.iter_mut().enumerate() {
        *p = [fl(68 + i * 3), fl(68 + i * 3 + 1)];
    }
    f.eye_open = [1.0 - f.bs[bs::EYE_BLINK_R], 1.0 - f.bs[bs::EYE_BLINK_L]];
    Some(f)
}

fn empty_frame(kind: FaceKind) -> FaceFrame {
    FaceFrame {
        kind,
        bs: [0.0; 52],
        tongue: 0.0,
        mp: [[0.0; 2]; 478],
        n_mp: 0,
        success: false,
        eye_open: [1.0; 2],
        euler: [0.0; 3],
        translation: [0.0; 3],
        lms: [[0.0; 2]; 68],
        features: [0.0; 14],
        confidence: 0.0,
        size: [1280.0, 720.0],
        received: Instant::now(),
    }
}

/// `ExpressionApp` `exp` array order (NVIDIA AR SDK expression sample) → index into
/// `FaceFrame::bs`. Inner brow and cheek puff come per side and are averaged.
const NV_TO_BS: [usize; 53] = [
    1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
];

fn quat_to_euler(q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let rx = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let ry = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    let rz = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()]
}

fn json_f32s(v: &serde_json::Value) -> Vec<f32> {
    v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default()
}

/// What an `ExpressionApp` datagram contained.
pub enum NvPacket {
    Face(Box<FaceFrame>),
    /// Result of a `calibrate` command: coefficients for `--expr_calibration`.
    Calibration(Vec<f32>),
}

/// NVIDIA `ExpressionApp.exe` packet: a JSON object followed by a NUL byte.
pub fn parse_nvidia(b: &[u8]) -> Option<NvPacket> {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let v: serde_json::Value = serde_json::from_slice(&b[..end]).ok()?;
    let cal = json_f32s(&v["cal"]);
    if !cal.is_empty() {
        return Some(NvPacket::Calibration(cal));
    }
    let exp = json_f32s(&v["exp"]);
    let rot = json_f32s(&v["rot"]);
    let pts = json_f32s(&v["pts"]);
    let mut f = empty_frame(FaceKind::Nvidia);
    // `pts` = [width, height, x0, y0, x1, y1, ...]; empty when the face is lost.
    f.success = exp.len() >= 53 && rot.len() >= 4 && pts.len() >= 70;
    f.confidence = (v["cnf"].as_f64().unwrap_or(0.0) as f32 / 50.0).clamp(0.0, 1.0);
    if !f.success {
        return Some(NvPacket::Face(Box::new(f)));
    }
    let mut sum = [0.0f32; 52];
    let mut cnt = [0u8; 52];
    for (i, &k) in NV_TO_BS.iter().enumerate() {
        sum[k] += exp[i].clamp(0.0, 1.0);
        cnt[k] += 1;
    }
    for k in 1..52 {
        f.bs[k] = if cnt[k] > 0 { sum[k] / cnt[k] as f32 } else { 0.0 };
    }
    // NVIDIA's left/right is the subject's, like ARKit.
    f.eye_open = [1.0 - f.bs[bs::EYE_BLINK_R], 1.0 - f.bs[bs::EYE_BLINK_L]];
    let e = quat_to_euler([rot[0], rot[1], rot[2], rot[3]]);
    // Camera space: x right, y up, z towards the camera. Pitch up / yaw / roll like OpenSeeFace.
    f.euler = [-e[0], e[1], -e[2]];
    let (w, h) = (pts[0].max(1.0), pts[1].max(1.0));
    f.size = [w, h];
    let n = ((pts.len() - 2) / 2).min(478);
    for i in 0..n {
        f.mp[i] = [pts[2 + i * 2] / w, pts[3 + i * 2] / h];
    }
    f.n_mp = n as u16;
    // `pos` is always zero: estimate head position from the jawline end points (0 and 32).
    let (a, c) = (f.mp[0], f.mp[32.min(n - 1)]);
    let mid = [(a[0] + c[0]) * 0.5, (a[1] + c[1]) * 0.5];
    let width = ((c[0] - a[0]).powi(2) + (c[1] - a[1]).powi(2)).sqrt();
    f.translation = [(mid[0] - 0.5) * 1.6, (mid[1] - 0.5) * 1.2, -width * 3.0];
    Some(NvPacket::Face(Box::new(f)))
}

fn vec3(v: &serde_json::Value) -> [f32; 3] {
    let g = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    [g("x"), g("y"), g("z")]
}

/// VTube Studio phone app packet (JSON, see `VTubeStudioRawTrackingData` in DenchiSoft's
/// `VTubeStudioBlendshapeUDPReceiverTest`). Returns the frame and the on-screen hotkey
/// (1..8, -1 = none).
pub fn parse_phone(b: &[u8]) -> Option<(FaceFrame, i32)> {
    let v: serde_json::Value = serde_json::from_slice(b).ok()?;
    let shapes = v["BlendShapes"].as_array()?;
    let mut f = empty_frame(FaceKind::Phone);
    f.success = v["FaceFound"].as_bool().unwrap_or(false);
    f.confidence = if f.success { 1.0 } else { 0.0 };
    for e in shapes {
        let (Some(k), Some(val)) = (e["k"].as_str(), e["v"].as_f64()) else { continue };
        let val = val as f32;
        if k.eq_ignore_ascii_case("TongueOut") {
            f.tongue = val;
        } else if let Some(i) = crate::perfect_sync::arkit_index(k) {
            f.bs[i] = val;
        }
    }
    f.eye_open = [1.0 - f.bs[bs::EYE_BLINK_R], 1.0 - f.bs[bs::EYE_BLINK_L]];
    // Unity euler angles in degrees (0..360): x = pitch (down positive), y = yaw, z = roll.
    let r = vec3(&v["Rotation"]);
    f.euler = [-wrap_deg(r[0]), wrap_deg(r[1]), wrap_deg(r[2])];
    let p = vec3(&v["Position"]);
    f.translation = [p[0] * 0.1, p[1] * 0.1, p[2] * 0.1];
    Some((f, v["Hotkey"].as_i64().unwrap_or(-1) as i32))
}

/// Camera list from `ExpressionApp.exe --print_caps`: (camera id, name, caps), each cap
/// being (cap id, width, height, fps).
pub type NvCamera = (i32, String, Vec<(i32, u32, u32, u32)>);

pub fn nvidia_caps(exe: &Path) -> Vec<NvCamera> {
    let mut cmd = Command::new(exe);
    cmd.current_dir(exe.parent().unwrap_or(Path::new("."))).arg("--print_caps");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(out) = cmd.output() else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout);
    // Some log lines come first; the JSON array is the last block.
    let Some(start) = text.find("[{").or_else(|| text.find('[')) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text[start..].trim_end_matches(|c: char| c != ']')) else { return Vec::new() };
    v.as_array()
        .map(|cams| {
            cams.iter()
                .enumerate()
                .map(|(i, c)| {
                    let caps = c["caps"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .enumerate()
                                .map(|(k, cap)| {
                                    let interval = cap["minInterval"].as_f64().unwrap_or(333_333.0).max(1.0);
                                    (
                                        cap["id"].as_i64().unwrap_or(k as i64) as i32,
                                        cap["maxCX"].as_u64().unwrap_or(0) as u32,
                                        cap["maxCY"].as_u64().unwrap_or(0) as u32,
                                        (10_000_000.0 / interval).round() as u32,
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    (c["id"].as_i64().unwrap_or(i as i64) as i32, c["name"].as_str().unwrap_or("").to_string(), caps)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Picks the `--camera_cap` closest to the requested mode.
fn nvidia_pick_cap(cams: &[NvCamera], camera: i32, w: u32, h: u32, fps: u32) -> Option<(i32, u32, u32, u32)> {
    let cam = cams.iter().find(|c| c.0 == camera)?;
    cam.2.iter().copied().min_by_key(|c| (c.1 as i64 - w as i64).abs() + (c.2 as i64 - h as i64).abs() + (c.3 as i64 - fps as i64).abs() * 8)
}

#[derive(Clone, Copy, Debug)]
pub struct Hand {
    /// 21 MediaPipe hand landmarks, normalised image coordinates (x, y) + relative depth z.
    pub pts: [[f32; 3]; 21],
}

#[derive(Clone, Copy, Debug)]
pub struct HandsFrame {
    pub hands: [Option<Hand>; 2],
    pub received: Instant,
}

fn parse_hands(b: &[u8]) -> Option<HandsFrame> {
    if b.len() < HAND_PACKET {
        return None;
    }
    let count = i32_at(b, 8).clamp(0, 2) as usize;
    let mut hands = [None, None];
    for (h, slot) in hands.iter_mut().enumerate().take(count) {
        let mut pts = [[0.0f32; 3]; 21];
        for (k, p) in pts.iter_mut().enumerate() {
            let o = 40 + h * 21 * 16 + k * 16 + 4;
            *p = [f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8)];
        }
        if pts.iter().any(|p| p[0] != 0.0 || p[1] != 0.0) {
            *slot = Some(Hand { pts });
        }
    }
    Some(HandsFrame { hands, received: Instant::now() })
}

#[derive(Clone, Debug)]
pub struct TrackerConfig {
    pub exe: PathBuf,
    pub camera: i32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub model: i32,
    pub threads: u32,
    /// Camera capability line from `facetracker -a` (-1 = driver default).
    pub dcap: i32,
    pub port: u16,
    pub mediapipe: bool,
    pub hands: bool,
    /// If false we only listen on the port (tracker started by someone else).
    pub spawn: bool,
    /// `backend::*`.
    pub backend: u8,
    /// NVIDIA `ExpressionApp.exe`.
    pub nv_exe: PathBuf,
    /// Saved NVIDIA calibration (`--expr_calibration`).
    pub nv_cal: Vec<f32>,
    /// Phone IP (VTube Studio phone app, "3rd Party PC Clients" enabled).
    pub phone_ip: String,
    /// Port the phone app listens on (shown in the app, 21412 by default).
    pub phone_port: u16,
}

pub struct Tracker {
    child: Option<Child>,
    latest: Arc<Mutex<Option<FaceFrame>>>,
    latest_hands: Arc<Mutex<Option<HandsFrame>>>,
    stop: Arc<AtomicBool>,
    packets: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
    pub error: Arc<Mutex<Option<String>>>,
    rate_mark: (Instant, u64),
    pub rate: f32,
    /// NVIDIA calibration received from ExpressionApp (taken by the app and saved).
    pub nv_cal: Arc<Mutex<Option<Vec<f32>>>>,
    /// Last on-screen hotkey pressed in the phone app (1..8), taken by the app.
    pub phone_hotkey: Arc<Mutex<Option<i32>>>,
    backend: u8,
    camera: i32,
}

impl Tracker {
    pub fn start(cfg: &TrackerConfig) -> Result<Tracker> {
        match cfg.backend {
            backend::NVIDIA => Self::start_nvidia(cfg),
            backend::PHONE => Self::start_phone(cfg),
            _ => Self::start_osf(cfg),
        }
    }

    fn new_shell(cfg: &TrackerConfig, child: Option<Child>) -> Tracker {
        Tracker {
            child,
            latest: Arc::new(Mutex::new(None)),
            latest_hands: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            packets: Arc::new(AtomicU64::new(0)),
            thread: None,
            error: Arc::new(Mutex::new(None)),
            rate_mark: (Instant::now(), 0),
            rate: 0.0,
            nv_cal: Arc::new(Mutex::new(None)),
            phone_hotkey: Arc::new(Mutex::new(None)),
            backend: cfg.backend,
            camera: cfg.camera,
        }
    }

    /// NVIDIA Broadcast tracker: spawns VTS's `ExpressionApp.exe` the way VTS does.
    fn start_nvidia(cfg: &TrackerConfig) -> Result<Tracker> {
        // ExpressionApp sends to every port 9140..9145; take the first free one.
        let sock = (9140..=9145)
            .find_map(|p| UdpSocket::bind(("127.0.0.1", p)).ok())
            .context("UDP ports 9140-9145 are busy (is VTube Studio running with NVIDIA tracking?)")?;
        sock.set_read_timeout(Some(Duration::from_millis(200)))?;
        if !cfg.nv_exe.is_file() {
            anyhow::bail!(
                "ExpressionApp.exe not found ({}). Install the free \"NVIDIA Broadcast Tracker\" DLC of VTube Studio on Steam (RTX GPU required)",
                cfg.nv_exe.display()
            );
        }
        let child = if cfg.spawn {
            let dir = cfg.nv_exe.parent().unwrap_or(Path::new(".")).to_path_buf();
            let cap = nvidia_pick_cap(&nvidia_caps(&cfg.nv_exe), cfg.camera, cfg.width, cfg.height, cfg.fps);
            let (cap_id, w, h, fps) = cap.unwrap_or((0, cfg.width, cfg.height, cfg.fps));
            let fps = fps.min(cfg.fps.max(10));
            let mut cmd = Command::new(&cfg.nv_exe);
            cmd.current_dir(&dir)
                .arg("--show=False")
                .arg("--landmarks=True")
                .arg(format!("--model_path={}", dir.join("models").display()))
                .arg(format!("--cam_res={w}x{h}"))
                .arg("--expr_mode=2")
                .arg(format!("--camera={}", cfg.camera))
                .arg(format!("--camera_cap={cap_id}"))
                .arg(format!("--cam_fps={fps}"))
                .arg(format!("--fps_limit={fps}"))
                .arg("--use_opencl=False")
                .arg("--cam_api=0");
            if !cfg.nv_cal.is_empty() {
                let c: Vec<String> = cfg.nv_cal.iter().map(|v| format!("{v:.6}")).collect();
                cmd.arg(format!("--expr_calibration={}", c.join(";")));
            }
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000);
            }
            Some(cmd.spawn().with_context(|| format!("cannot start {}", cfg.nv_exe.display()))?)
        } else {
            None
        };
        let mut t = Self::new_shell(cfg, child);
        let (latest, stop, packets, cal) = (t.latest.clone(), t.stop.clone(), t.packets.clone(), t.nv_cal.clone());
        t.thread = Some(std::thread::Builder::new().name("tracking-nvidia".into()).spawn(move || {
            let mut buf = vec![0u8; 65536];
            while !stop.load(Ordering::Relaxed) {
                let Ok(n) = sock.recv(&mut buf) else { continue };
                match parse_nvidia(&buf[..n]) {
                    Some(NvPacket::Face(f)) => {
                        *latest.lock().unwrap() = Some(*f);
                        packets.fetch_add(1, Ordering::Relaxed);
                    }
                    Some(NvPacket::Calibration(c)) => *cal.lock().unwrap() = Some(c),
                    None => {}
                }
            }
        })?);
        Ok(t)
    }

    /// Phone tracking: requests data from the VTube Studio phone app every 2 s.
    fn start_phone(cfg: &TrackerConfig) -> Result<Tracker> {
        let ip: std::net::IpAddr = cfg.phone_ip.trim().parse().with_context(|| format!("invalid phone IP \"{}\"", cfg.phone_ip))?;
        let sock = UdpSocket::bind(("0.0.0.0", cfg.port)).with_context(|| format!("UDP port {} is busy", cfg.port))?;
        sock.set_read_timeout(Some(Duration::from_millis(100)))?;
        let target = std::net::SocketAddr::new(ip, cfg.phone_port);
        let request = serde_json::json!({ "messageType": "iOSTrackingDataRequest", "time": 5.0, "sentBy": "rigrs", "ports": [cfg.port] }).to_string();
        let mut t = Self::new_shell(cfg, None);
        let (latest, stop, packets, hotkey, error) = (t.latest.clone(), t.stop.clone(), t.packets.clone(), t.phone_hotkey.clone(), t.error.clone());
        t.thread = Some(std::thread::Builder::new().name("tracking-phone".into()).spawn(move || {
            let mut buf = vec![0u8; 65536];
            let mut last_req = Instant::now() - Duration::from_secs(10);
            let mut last_hotkey = -1;
            while !stop.load(Ordering::Relaxed) {
                if last_req.elapsed() >= Duration::from_secs(2) {
                    if let Err(e) = sock.send_to(request.as_bytes(), target) {
                        *error.lock().unwrap() = Some(format!("{target}: {e}"));
                    }
                    last_req = Instant::now();
                }
                let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
                if from.ip() != ip {
                    continue;
                }
                if let Some((f, hk)) = parse_phone(&buf[..n]) {
                    *latest.lock().unwrap() = Some(f);
                    packets.fetch_add(1, Ordering::Relaxed);
                    *error.lock().unwrap() = None;
                    if hk != last_hotkey && hk >= 1 {
                        *hotkey.lock().unwrap() = Some(hk);
                    }
                    last_hotkey = hk;
                }
            }
        })?);
        Ok(t)
    }

    /// Asks the NVIDIA tracker to calibrate the neutral face (look straight, neutral face).
    pub fn nvidia_calibrate(&self) -> bool {
        if self.backend != backend::NVIDIA {
            return false;
        }
        let Ok(s) = UdpSocket::bind(("127.0.0.1", 0)) else { return false };
        s.send_to(br#"{"cmd":" calibrate"}"#, ("127.0.0.1", 9160 + self.camera.max(0) as u16)).is_ok()
    }

    pub fn backend(&self) -> u8 {
        self.backend
    }

    fn start_osf(cfg: &TrackerConfig) -> Result<Tracker> {
        let sock = UdpSocket::bind(("127.0.0.1", cfg.port)).with_context(|| format!("UDP port {} is busy", cfg.port))?;
        sock.set_read_timeout(Some(Duration::from_millis(200)))?;

        let child = if cfg.spawn {
            let mut cmd = Command::new(&cfg.exe);
            cmd.current_dir(cfg.exe.parent().unwrap_or(Path::new(".")))
                .args(["-c", &cfg.camera.to_string()])
                .args(["-W", &cfg.width.to_string(), "-H", &cfg.height.to_string()])
                .args(["-F", &cfg.fps.to_string()])
                .args(["-D", &cfg.dcap.to_string()])
                .args(["--model", &cfg.model.to_string()])
                .args(["-m", &cfg.threads.to_string()])
                .args(["-i", "127.0.0.1", "-p", &cfg.port.to_string()])
                .args(["--discard-after", "0", "--scan-every", "0", "--no-3d-adapt", "1"])
                .args(["--max-feature-updates", "900", "-s", "1"])
                .args(if cfg.mediapipe { &["--mediapipe", "1"][..] } else { &[][..] })
                .args(if cfg.hands { &["--hands", "1"][..] } else { &[][..] })
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            Some(cmd.spawn().with_context(|| format!("cannot start {}", cfg.exe.display()))?)
        } else {
            None
        };

        let latest = Arc::new(Mutex::new(None));
        let latest_hands = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let packets = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        let thread = {
            let (latest, latest_hands, stop, packets) = (latest.clone(), latest_hands.clone(), stop.clone(), packets.clone());
            std::thread::Builder::new()
                .name("tracking-udp".into())
                .spawn(move || {
                    let mut buf = vec![0u8; 65536];
                    while !stop.load(Ordering::Relaxed) {
                        let Ok(n) = sock.recv(&mut buf) else { continue };
                        let b = &buf[..n];
                        if n == HAND_PACKET {
                            if let Some(h) = parse_hands(b) {
                                *latest_hands.lock().unwrap() = Some(h);
                            }
                            continue;
                        }
                        // A datagram may hold several faces; we use the first one.
                        let f = if n >= MP_PACKET { parse_mp(b) } else { parse(b) };
                        if let Some(f) = f {
                            *latest.lock().unwrap() = Some(f);
                            packets.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })?
        };
        let mut t = Self::new_shell(cfg, child);
        t.latest = latest;
        t.latest_hands = latest_hands;
        t.stop = stop;
        t.packets = packets;
        t.thread = Some(thread);
        t.error = error;
        Ok(t)
    }

    pub fn latest(&self) -> Option<FaceFrame> {
        *self.latest.lock().unwrap()
    }

    pub fn latest_hands(&self) -> Option<HandsFrame> {
        *self.latest_hands.lock().unwrap()
    }

    /// Checks that the tracker process is still alive; updates packet rate.
    pub fn poll(&mut self) {
        let now = Instant::now();
        let el = now.duration_since(self.rate_mark.0).as_secs_f32();
        if el >= 1.0 {
            let p = self.packets.load(Ordering::Relaxed);
            self.rate = (p - self.rate_mark.1) as f32 / el;
            self.rate_mark = (now, p);
        }
        if let Some(c) = &mut self.child {
            if let Ok(Some(status)) = c.try_wait() {
                *self.error.lock().unwrap() = Some(format!(
                    "трекер завершился ({status}). Камера занята или неверный номер камеры?"
                ));
                self.child = None;
            }
        }
    }

    #[cfg(windows)]
    pub fn process_handle(&self) -> Option<isize> {
        use std::os::windows::io::AsRawHandle;
        self.child.as_ref().map(|c| c.as_raw_handle() as isize)
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One capability line of a camera, e.g. `0: Resolution: 640x480 FPS: 1.000-29.970 Format: XRGB`.
#[derive(Clone, Debug)]
pub struct CamCap {
    pub index: i32,
    pub width: u32,
    pub height: u32,
    pub fps_min: f32,
    pub fps_max: f32,
    pub format: String,
}

/// Lists camera capabilities via `facetracker -a <camera>`.
pub fn list_caps(exe: &Path, camera: i32) -> Vec<CamCap> {
    let mut cmd = Command::new(exe);
    cmd.current_dir(exe.parent().unwrap_or(Path::new("."))).args(["-a", &camera.to_string()]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(out) = cmd.output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (idx, rest) = l.trim().split_once(':')?;
            let index = idx.trim().parse().ok()?;
            let res = rest.split("Resolution:").nth(1)?.split_whitespace().next()?;
            let (w, h) = res.split_once('x')?;
            let fps = rest.split("FPS:").nth(1)?.split_whitespace().next()?;
            let (lo, hi) = fps.split_once('-').unwrap_or((fps, fps));
            let format = rest.split("Format:").nth(1).map(|f| f.trim().to_string()).unwrap_or_default();
            Some(CamCap { index, width: w.parse().ok()?, height: h.parse().ok()?, fps_min: lo.parse().ok()?, fps_max: hi.parse().ok()?, format })
        })
        .collect()
}

/// Lists cameras via `facetracker -l 1`. Returns (index, name).
pub fn list_cameras(exe: &Path) -> Vec<(i32, String)> {
    let mut cmd = Command::new(exe);
    cmd.current_dir(exe.parent().unwrap_or(Path::new("."))).args(["-l", "1"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(out) = cmd.output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (n, name) = l.split_once(':')?;
            Some((n.trim().parse().ok()?, name.trim().to_string()))
        })
        .collect()
}

// ------------------------------------------------------------------ conversion

#[derive(Clone, Copy, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Calibration {
    pub euler: [f32; 3],
    pub translation: [f32; 3],
    pub valid: bool,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TrackOptions {
    /// Avatar behaves like a mirror (VTS default).
    pub mirror: bool,
    pub invert: [bool; 3],
    pub invert_gaze: [bool; 2],
    /// 0 = never, 1 = always, 2 = when the head is turned
    pub blink_sync: u8,
    /// 0..100, 50 = neutral: blink, eye open, mouth open, smile, brows
    pub sens: [f32; 5],
}

impl Default for TrackOptions {
    fn default() -> Self {
        TrackOptions { mirror: true, invert: [false; 3], invert_gaze: [false; 2], blink_sync: 0, sens: [50.0; 5] }
    }
}

fn wrap_deg(mut a: f32) -> f32 {
    while a > 180.0 {
        a -= 360.0;
    }
    while a < -180.0 {
        a += 360.0;
    }
    a
}

impl Calibration {
    pub fn from_frame(f: &FaceFrame) -> Calibration {
        Calibration { euler: f.euler, translation: f.translation, valid: true }
    }
}

fn gaze(lms: &[[f32; 2]; 68], a: usize, b: usize, pupil: usize) -> (f32, f32) {
    let (p0, p1, p) = (lms[a], lms[b], lms[pupil]);
    let ax = [p1[0] - p0[0], p1[1] - p0[1]];
    let len = (ax[0] * ax[0] + ax[1] * ax[1]).sqrt().max(1e-3);
    let dir = [ax[0] / len, ax[1] / len];
    let mid = [(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5];
    let d = [p[0] - mid[0], p[1] - mid[1]];
    let along = (d[0] * dir[0] + d[1] * dir[1]) / len;
    let across = (d[0] * -dir[1] + d[1] * dir[0]) / len;
    (along * 4.0, across * 6.0)
}

pub fn to_inputs(f: &FaceFrame, c: &Calibration, o: &TrackOptions) -> Inputs {
    let mut v = NEUTRAL;
    let s = |inv: bool| if inv { -1.0 } else { 1.0 };
    let m = if o.mirror { -1.0 } else { 1.0 };

    let pitch = wrap_deg(f.euler[0] - c.euler[0]);
    let yaw = wrap_deg(f.euler[1] - c.euler[1]);
    let roll = wrap_deg(f.euler[2] - c.euler[2]);
    v[input::FACE_ANGLE_X] = yaw * m * s(o.invert[0]);
    v[input::FACE_ANGLE_Y] = pitch * s(o.invert[1]);
    v[input::FACE_ANGLE_Z] = roll * m * s(o.invert[2]);

    v[input::FACE_POSITION_X] = (f.translation[0] - c.translation[0]) * m * 10.0;
    v[input::FACE_POSITION_Y] = (f.translation[1] - c.translation[1]) * -10.0;
    v[input::FACE_POSITION_Z] = (f.translation[2] - c.translation[2]) * 10.0;

    let k = |i: usize| o.sens[i] / 50.0;
    // Blink sensitivity raises the "closed" threshold, eye-open sensitivity scales openness.
    let closed = 0.1 + 0.15 * (k(0) - 1.0);
    let eye = |x: f32| (((x - closed) / (0.9 - closed).max(0.1)) * k(1)).clamp(0.0, 1.2);
    let mp = f.kind.has_blendshapes();
    let b = &f.bs;
    let wide = |w: f32| if mp { w * 0.4 } else { 0.0 };
    let (mut er, mut el) = (eye(f.eye_open[0]) + wide(b[bs::EYE_WIDE_R]), eye(f.eye_open[1]) + wide(b[bs::EYE_WIDE_L]));
    if o.blink_sync == 1 || (o.blink_sync == 2 && yaw.abs() > 20.0) {
        let m = (er + el) * 0.5;
        er = m;
        el = m;
    }
    // "Left"/"Right" inputs refer to the avatar's eyes.
    let (left, right) = if o.mirror { (er, el) } else { (el, er) };
    v[input::EYE_OPEN_LEFT] = left;
    v[input::EYE_OPEN_RIGHT] = right;

    let (gx0, gy0) = if mp {
        // Positive x = the subject looks to their own left.
        let x = ((b[bs::EYE_LOOK_OUT_L] - b[bs::EYE_LOOK_IN_L]) + (b[bs::EYE_LOOK_IN_R] - b[bs::EYE_LOOK_OUT_R])) * 0.5 * 1.6;
        let y = ((b[bs::EYE_LOOK_UP_L] + b[bs::EYE_LOOK_UP_R]) - (b[bs::EYE_LOOK_DOWN_L] + b[bs::EYE_LOOK_DOWN_R])) * 0.5 * 1.6;
        (-x, y)
    } else {
        let (gx1, gy1) = gaze(&f.lms, 36, 39, 66);
        let (gx2, gy2) = gaze(&f.lms, 42, 45, 67);
        ((gx1 + gx2) * 0.5, (gy1 + gy2) * 0.5)
    };
    let gx = (gx0 * m * s(o.invert_gaze[0])).clamp(-1.0, 1.0);
    let gy = (gy0 * s(o.invert_gaze[1])).clamp(-1.0, 1.0);
    for i in [input::EYE_LEFT_X, input::EYE_RIGHT_X] {
        v[i] = gx;
    }
    for i in [input::EYE_LEFT_Y, input::EYE_RIGHT_Y] {
        v[i] = gy;
    }

    let ft = &f.features;
    let (brow_l, brow_r) = if mp {
        let up = |outer: usize, down: usize| (b[outer] * 0.5 + b[bs::BROW_INNER_UP] * 0.5 - b[down]) * 0.8 * k(4);
        (0.5 + up(bs::BROW_OUTER_UP_L, bs::BROW_DOWN_L), 0.5 + up(bs::BROW_OUTER_UP_R, bs::BROW_DOWN_R))
    } else {
        (0.5 + ft[3] * 0.5 * k(4), 0.5 + ft[6] * 0.5 * k(4))
    };
    let (brow_l, brow_r) = if o.mirror { (brow_r, brow_l) } else { (brow_l, brow_r) };
    v[input::BROW_LEFT_Y] = brow_l.clamp(0.0, 1.0);
    v[input::BROW_RIGHT_Y] = brow_r.clamp(0.0, 1.0);
    v[input::BROWS] = ((brow_l + brow_r) * 0.5).clamp(0.0, 1.0);

    if mp {
        let smile = (b[bs::MOUTH_SMILE_L] + b[bs::MOUTH_SMILE_R]) * 0.5 - (b[bs::MOUTH_FROWN_L] + b[bs::MOUTH_FROWN_R]) * 0.35;
        v[input::MOUTH_SMILE] = (0.5 + smile * 0.8 * k(3)).clamp(0.0, 1.0);
        v[input::MOUTH_OPEN] = (b[bs::JAW_OPEN] * 1.4 * k(2)).clamp(0.0, 1.0);
        v[input::MOUTH_X] = ((b[bs::MOUTH_RIGHT] - b[bs::MOUTH_LEFT]) * m).clamp(-1.0, 1.0);
        v[input::CHEEK_PUFF] = b[bs::CHEEK_PUFF].clamp(0.0, 1.0);
        v[input::TONGUE_OUT] = f.tongue.clamp(0.0, 1.0);
        v[input::FACE_ANGRY] = ((b[bs::NOSE_SNEER_L] + b[bs::NOSE_SNEER_R]) * 0.5 + (b[bs::BROW_DOWN_L] + b[bs::BROW_DOWN_R]) * 0.25).clamp(0.0, 1.0);
    } else {
        v[input::MOUTH_SMILE] = (0.5 + (ft[8] + ft[10]) * 0.25 * k(3)).clamp(0.0, 1.0);
        v[input::MOUTH_OPEN] = (ft[12] * k(2)).clamp(0.0, 1.0);
        v[input::MOUTH_X] = ((ft[9] - ft[11]) * 0.5 * m).clamp(-1.0, 1.0);
    }
    v
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Fills the hand inputs. "Left" is the avatar's left hand (mirror-aware, like the eyes).
pub fn hand_inputs(v: &mut Inputs, h: &HandsFrame, o: &TrackOptions) {
    use input::*;
    // Identify the subject's left/right hand. With two hands: the subject's left hand is
    // further right in the (unmirrored) camera image. With one: thumb side of the palm.
    let mut subject: [Option<&Hand>; 2] = [None, None]; // [left, right]
    let present: Vec<&Hand> = h.hands.iter().flatten().collect();
    match present.len() {
        2 => {
            let (a, b) = (present[0], present[1]);
            if a.pts[0][0] > b.pts[0][0] {
                subject = [Some(a), Some(b)];
            } else {
                subject = [Some(b), Some(a)];
            }
        }
        1 => {
            let a = present[0];
            let right = a.pts[4][0] > a.pts[20][0];
            subject[if right { 1 } else { 0 }] = Some(a);
        }
        _ => {}
    }
    let (left, right) = if o.mirror { (subject[1], subject[0]) } else { (subject[0], subject[1]) };
    let m = if o.mirror { -1.0 } else { 1.0 };
    let fill = |v: &mut Inputs, hand: Option<&Hand>, base: [usize; 11]| {
        let [found, px, py, pz, ax, az, open, f1, f2, f3, f4] = base;
        let Some(hd) = hand else {
            v[found] = 0.0;
            return;
        };
        let p = &hd.pts;
        v[found] = 1.0;
        let c = [(p[0][0] + p[5][0] + p[17][0]) / 3.0, (p[0][1] + p[5][1] + p[17][1]) / 3.0];
        v[px] = ((c[0] - 0.5) * 2.0 * m).clamp(-1.0, 1.0);
        v[py] = ((0.5 - c[1]) * 2.0).clamp(-1.0, 1.0);
        let size = dist3(p[0], p[9]).max(1e-4);
        v[pz] = ((size - 0.15) / 0.1).clamp(-1.0, 1.0);
        v[az] = (p[9][0] - p[0][0]).atan2(p[0][1] - p[9][1]).to_degrees() * m;
        v[ax] = (p[9][2] - p[0][2]).atan2(size).to_degrees();
        // Finger extension: tip-to-wrist distance relative to knuckle-to-wrist distance.
        let fingers = [(2usize, 4usize, 1.45f32), (5, 8, 1.9), (9, 12, 1.95), (13, 16, 1.9), (17, 20, 1.8)];
        let mut sum = 0.0;
        let outs = [f1, f2, f3, f4, base[10] + 1];
        for (i, &(mcp, tip, ext)) in fingers.iter().enumerate() {
            let r = dist3(p[tip], p[0]) / dist3(p[mcp], p[0]).max(1e-4);
            let e = ((r - 1.05) / (ext - 1.05)).clamp(0.0, 1.0);
            v[outs[i]] = e;
            sum += e;
        }
        v[open] = sum / 5.0;
    };
    fill(v, left, [HAND_LEFT_FOUND, HAND_LEFT_POSITION_X, HAND_LEFT_POSITION_Y, HAND_LEFT_POSITION_Z, HAND_LEFT_ANGLE_X, HAND_LEFT_ANGLE_Z, HAND_LEFT_OPEN, HAND_LEFT_FINGER_1, HAND_LEFT_FINGER_2, HAND_LEFT_FINGER_3, HAND_LEFT_FINGER_4]);
    fill(v, right, [HAND_RIGHT_FOUND, HAND_RIGHT_POSITION_X, HAND_RIGHT_POSITION_Y, HAND_RIGHT_POSITION_Z, HAND_RIGHT_ANGLE_X, HAND_RIGHT_ANGLE_Z, HAND_RIGHT_OPEN, HAND_RIGHT_FINGER_1, HAND_RIGHT_FINGER_2, HAND_RIGHT_FINGER_3, HAND_RIGHT_FINGER_4]);
    v[BOTH_HANDS_FOUND] = if left.is_some() && right.is_some() { 1.0 } else { 0.0 };
    v[HAND_DISTANCE] = match (left, right) {
        (Some(a), Some(b)) => (dist3(a.pts[9], b.pts[9]) * 1.5).clamp(0.0, 1.0),
        _ => 0.0,
    };
}
