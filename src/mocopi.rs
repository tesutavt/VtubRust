//! Sony mocopi receiver (VTS wiki "mocopi").
//!
//! The mocopi phone / PC app sends "Sony Motion Format" UDP packets to the IP and port shown
//! here (default 12351, "3rd party connection" in the mocopi app). Packets are nested
//! `[u32 length][4-char name][value]` fields (unofficial spec: seagetch/mcp-receiver,
//! doc/Protocol.md): `skdf` packets define the skeleton (bone id + parent id + T-pose),
//! `fram` packets carry the 27 local bone transforms (quaternion xyzw + position xyz).
//!
//! We turn them into the VTS mocopi parameters (`MocopiAngleX`, `MocopiBodyAngleZ`,
//! `MocopiUpperArmLeftAngleY`, ...). Like in VTS, `MocopiAngleX/Y/Z` fall back to face
//! tracking while no mocopi data arrives, and all values fade back to 0 when data stops.

use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 12351;
const BONES: usize = 27;

/// VTS mocopi parameters with their default mapping ranges (VTS wiki).
pub const PARAMS: [(&str, f32, f32); 33] = [
    ("MocopiConnected", 0.0, 1.0),
    ("MocopiHipAngleZ", -30.0, 30.0),
    ("MocopiAngleX", -30.0, 30.0),
    ("MocopiAngleY", -30.0, 30.0),
    ("MocopiAngleZ", -30.0, 30.0),
    ("MocopiBodyAngleX", -10.0, 10.0),
    ("MocopiBodyAngleY", -10.0, 10.0),
    ("MocopiBodyAngleZ", -10.0, 10.0),
    ("MocopiBodyPositionX", -1.0, 1.0),
    ("MocopiBodyPositionY", -1.0, 1.0),
    ("MocopiBodyPositionZ", -1.0, 1.0),
    ("MocopiUpperArmLeftAngleY", -90.0, 90.0),
    ("MocopiUpperArmLeftAngleZ", -180.0, 180.0),
    ("MocopiUpperArmRightAngleY", -90.0, 90.0),
    ("MocopiUpperArmRightAngleZ", -180.0, 180.0),
    ("MocopiLowerArmLeftAngleX", -180.0, 180.0),
    ("MocopiLowerArmLeftAngleY", -90.0, 90.0),
    ("MocopiLowerArmLeftAngleZ", -180.0, 180.0),
    ("MocopiLowerArmRightAngleX", -180.0, 180.0),
    ("MocopiLowerArmRightAngleY", -90.0, 90.0),
    ("MocopiLowerArmRightAngleZ", -180.0, 180.0),
    ("MocopiUpperLegLeftAngleY", -30.0, 30.0),
    ("MocopiUpperLegLeftAngleZ", -30.0, 30.0),
    ("MocopiUpperLegRightAngleY", -30.0, 30.0),
    ("MocopiUpperLegRightAngleZ", -30.0, 30.0),
    ("MocopiLowerLegLeftAngleY", -30.0, 30.0),
    ("MocopiLowerLegLeftAngleZ", -30.0, 30.0),
    ("MocopiLowerLegRightAngleY", -30.0, 30.0),
    ("MocopiLowerLegRightAngleZ", -30.0, 30.0),
    // Not in the VTS list but handy: overall hip yaw and pitch.
    ("MocopiHipAngleX", -30.0, 30.0),
    ("MocopiHipAngleY", -30.0, 30.0),
    // Wrist bend relative to the lower arm.
    ("MocopiHandLeftAngleZ", -90.0, 90.0),
    ("MocopiHandRightAngleZ", -90.0, 90.0),
];

pub fn range_of(name: &str) -> Option<(f32, f32)> {
    PARAMS.iter().find(|p| p.0 == name).map(|p| (p.1, p.2))
}

// ------------------------------------------------------------------ quaternion math

type Q = [f32; 4]; // x, y, z, w

fn qmul(a: Q, b: Q) -> Q {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn qinv(q: Q) -> Q {
    [-q[0], -q[1], -q[2], q[3]]
}

fn qnorm(q: Q) -> Q {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-6 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    }
}

/// Euler angles in degrees (Unity order: Z, then X, then Y): [x (pitch), y (yaw), z (roll)].
fn euler(q: Q) -> [f32; 3] {
    let [x, y, z, w] = q;
    let sinp = (2.0 * (w * x - y * z)).clamp(-1.0, 1.0);
    let pitch = sinp.asin();
    let yaw = (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let roll = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (x * x + z * z));
    [pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees()]
}

// ------------------------------------------------------------------ packet parsing

fn fields(b: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= b.len() {
        let len = u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let name = &b[i + 4..i + 8];
        if !name.iter().all(|c| c.is_ascii_alphabetic()) || i + 8 + len > b.len() {
            break;
        }
        out.push((name, &b[i + 8..i + 8 + len]));
        i += 8 + len;
    }
    out
}

fn field<'a>(b: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    fields(b).into_iter().find(|f| f.0 == name).map(|f| f.1)
}

fn tran(v: &[u8]) -> Option<(Q, [f32; 3])> {
    if v.len() < 28 {
        return None;
    }
    let f = |k: usize| f32::from_le_bytes([v[k * 4], v[k * 4 + 1], v[k * 4 + 2], v[k * 4 + 3]]);
    Some((qnorm([f(0), f(1), f(2), f(3)]), [f(4), f(5), f(6)]))
}

fn u16_of(v: &[u8]) -> Option<usize> {
    (v.len() >= 2).then(|| u16::from_le_bytes([v[0], v[1]]) as usize)
}

pub enum Packet {
    /// Skeleton definition: parent id per bone.
    Skeleton([Option<usize>; BONES]),
    /// Local rotation / position per bone.
    Frame(Box<[(Q, [f32; 3]); BONES]>),
}

pub fn parse(b: &[u8]) -> Option<Packet> {
    let head = field(b, b"head")?;
    if !field(head, b"ftyp").map(|t| t.starts_with(b"sony motion format")).unwrap_or(false) {
        return None;
    }
    if let Some(skdf) = field(b, b"skdf") {
        let bons = field(skdf, b"bons")?;
        let mut parents = [None; BONES];
        for (name, bndt) in fields(bons) {
            if name != b"bndt" {
                continue;
            }
            let (Some(id), Some(p)) = (field(bndt, b"bnid").and_then(u16_of), field(bndt, b"pbid").and_then(u16_of)) else { continue };
            if id < BONES {
                parents[id] = (p < BONES && p != id).then_some(p);
            }
        }
        return Some(Packet::Skeleton(parents));
    }
    let fram = field(b, b"fram")?;
    let btrs = field(fram, b"btrs")?;
    let mut bones = Box::new([([0.0, 0.0, 0.0, 1.0], [0.0; 3]); BONES]);
    let mut n = 0;
    for (name, btdt) in fields(btrs) {
        if name != b"btdt" {
            continue;
        }
        let (Some(id), Some(t)) = (field(btdt, b"bnid").and_then(u16_of), field(btdt, b"tran").and_then(tran)) else { continue };
        if id < BONES {
            bones[id] = t;
            n += 1;
        }
    }
    (n > 0).then_some(Packet::Frame(bones))
}

/// mocopi default hierarchy (used until a skeleton packet arrives).
fn default_parents() -> [Option<usize>; BONES] {
    let mut p = [None; BONES];
    for (i, slot) in p.iter_mut().enumerate().take(11).skip(1) {
        *slot = Some(i - 1);
    }
    for (base, parent) in [(11usize, 7usize), (15, 7)] {
        p[base] = Some(parent);
        for k in 1..4 {
            p[base + k] = Some(base + k - 1);
        }
    }
    for base in [19usize, 23] {
        p[base] = Some(0);
        for k in 1..4 {
            p[base + k] = Some(base + k - 1);
        }
    }
    p
}

/// Converts one frame into the VTS parameters. Bone ids follow the mapping used by
/// open-source receivers: 11-14 right arm, 15-18 left arm, 19-22 right leg, 23-26 left leg.
fn compute(bones: &[(Q, [f32; 3]); BONES], parents: &[Option<usize>; BONES], origin: [f32; 3], mirror: bool) -> HashMap<&'static str, f32> {
    let mut global = [[0.0f32, 0.0, 0.0, 1.0]; BONES];
    for i in 0..BONES {
        // Parents always have lower ids in the mocopi skeleton.
        global[i] = match parents[i].filter(|&p| p < i) {
            Some(p) => qmul(global[p], bones[i].0),
            None => bones[i].0,
        };
    }
    let rel = |child: usize, parent: usize| euler(qmul(qinv(global[parent]), global[child]));
    let m = if mirror { -1.0 } else { 1.0 };
    let mut v: HashMap<&'static str, f32> = HashMap::new();
    let hip = euler(global[0]);
    v.insert("MocopiHipAngleX", hip[1] * m);
    v.insert("MocopiHipAngleY", -hip[0]);
    v.insert("MocopiHipAngleZ", hip[2] * m);
    let head = rel(10, 7);
    v.insert("MocopiAngleX", head[1] * m);
    v.insert("MocopiAngleY", -head[0]);
    v.insert("MocopiAngleZ", head[2] * m);
    let body = rel(7, 0);
    v.insert("MocopiBodyAngleX", body[1] * m);
    v.insert("MocopiBodyAngleY", -body[0]);
    v.insert("MocopiBodyAngleZ", body[2] * m);
    let p = bones[0].1;
    v.insert("MocopiBodyPositionX", ((p[0] - origin[0]) * 2.0 * m).clamp(-1.0, 1.0));
    v.insert("MocopiBodyPositionY", ((p[1] - origin[1]) * 2.0).clamp(-1.0, 1.0));
    v.insert("MocopiBodyPositionZ", ((p[2] - origin[2]) * 2.0).clamp(-1.0, 1.0));
    // Avatar side: with mirror mode the avatar's left arm follows your right arm.
    let (left, right) = if mirror { (11usize, 15usize) } else { (15, 11) };
    let (lleg, rleg) = if mirror { (19usize, 23usize) } else { (23, 19) };
    for (side, arm) in [("Left", left), ("Right", right)] {
        let up = rel(arm + 1, 7);
        let low = rel(arm + 2, arm + 1);
        let hand = rel(arm + 3, arm + 2);
        let put = |v: &mut HashMap<&'static str, f32>, name: String, x: f32| {
            if let Some(k) = PARAMS.iter().find(|p| p.0 == name) {
                v.insert(k.0, x);
            }
        };
        put(&mut v, format!("MocopiUpperArm{side}AngleY"), up[1] * m);
        put(&mut v, format!("MocopiUpperArm{side}AngleZ"), up[2] * m);
        put(&mut v, format!("MocopiLowerArm{side}AngleX"), low[0]);
        put(&mut v, format!("MocopiLowerArm{side}AngleY"), low[1] * m);
        put(&mut v, format!("MocopiLowerArm{side}AngleZ"), low[2] * m);
        put(&mut v, format!("MocopiHand{side}AngleZ"), hand[2] * m);
    }
    for (side, leg) in [("Left", lleg), ("Right", rleg)] {
        let up = rel(leg, 0);
        let low = rel(leg + 1, leg);
        for (name, x) in [
            (format!("MocopiUpperLeg{side}AngleY"), up[1] * m),
            (format!("MocopiUpperLeg{side}AngleZ"), up[2] * m),
            (format!("MocopiLowerLeg{side}AngleY"), low[1] * m),
            (format!("MocopiLowerLeg{side}AngleZ"), low[2] * m),
        ] {
            if let Some(k) = PARAMS.iter().find(|p| p.0 == name) {
                v.insert(k.0, x);
            }
        }
    }
    v
}

// ------------------------------------------------------------------ receiver

#[derive(Default)]
struct Shared {
    parents: Option<[Option<usize>; BONES]>,
    frame: Option<Box<[(Q, [f32; 3]); BONES]>>,
    received: Option<Instant>,
    packets: u64,
    sender: Option<std::net::SocketAddr>,
}

pub struct Receiver {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    pub port: u16,
    origin: Option<[f32; 3]>,
    rate_mark: (Instant, u64),
    pub rate: f32,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Receiver {
    pub fn start(port: u16) -> anyhow::Result<Receiver> {
        let sock = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| anyhow::anyhow!("UDP {port}: {e}"))?;
        sock.set_read_timeout(Some(Duration::from_millis(200)))?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sh, st) = (shared.clone(), stop.clone());
        std::thread::Builder::new().name("mocopi".into()).spawn(move || {
            let mut buf = vec![0u8; 8192];
            while !st.load(Ordering::Relaxed) {
                let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
                let Some(p) = parse(&buf[..n]) else { continue };
                let mut g = sh.lock().unwrap();
                // VTS: only one mocopi sender at a time; others are ignored.
                let fresh = g.received.map(|t| t.elapsed() < Duration::from_secs(2)).unwrap_or(false);
                if fresh && g.sender.map(|s| s != from).unwrap_or(false) {
                    continue;
                }
                g.sender = Some(from);
                match p {
                    Packet::Skeleton(par) => g.parents = Some(par),
                    Packet::Frame(f) => {
                        g.frame = Some(f);
                        g.received = Some(Instant::now());
                        g.packets += 1;
                    }
                }
            }
        })?;
        Ok(Receiver { shared, stop, port, origin: None, rate_mark: (Instant::now(), 0), rate: 0.0 })
    }

    pub fn connected(&self) -> bool {
        self.shared.lock().unwrap().received.map(|t| t.elapsed() < Duration::from_millis(500)).unwrap_or(false)
    }

    pub fn sender(&self) -> Option<std::net::SocketAddr> {
        self.shared.lock().unwrap().sender
    }

    /// Makes the current hip position the centre of `MocopiBodyPosition*`.
    pub fn recenter(&mut self) {
        self.origin = None;
    }

    fn poll(&mut self, mirror: bool) -> Option<HashMap<&'static str, f32>> {
        let g = self.shared.lock().unwrap();
        let el = self.rate_mark.0.elapsed().as_secs_f32();
        if el >= 1.0 {
            self.rate = (g.packets - self.rate_mark.1) as f32 / el;
            self.rate_mark = (Instant::now(), g.packets);
        }
        if !g.received.map(|t| t.elapsed() < Duration::from_millis(500)).unwrap_or(false) {
            return None;
        }
        let f = g.frame.as_ref()?;
        let parents = g.parents.unwrap_or_else(default_parents);
        let origin = *self.origin.get_or_insert(f[0].1);
        Some(compute(f, &parents, origin, mirror))
    }
}

static VALUES: Mutex<Option<HashMap<&'static str, f32>>> = Mutex::new(None);

pub fn value(name: &str) -> Option<f32> {
    VALUES.lock().unwrap().as_ref().and_then(|m| m.get(name).copied())
}

pub fn snapshot() -> Vec<(&'static str, f32)> {
    let g = VALUES.lock().unwrap();
    PARAMS.iter().map(|p| (p.0, g.as_ref().and_then(|m| m.get(p.0).copied()).unwrap_or(0.0))).collect()
}

/// Per frame: new values, or fade towards 0; head angles fall back to face tracking.
pub fn update(rx: Option<&mut Receiver>, face: [f32; 3], mirror: bool, dt: f32) {
    let fresh = rx.and_then(|r| r.poll(mirror));
    let mut g = VALUES.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    match fresh {
        Some(v) => {
            *m = v;
            m.insert("MocopiConnected", 1.0);
        }
        None => {
            let k = (-dt / 0.25).exp();
            for p in PARAMS.iter() {
                let e = m.entry(p.0).or_insert(0.0);
                *e *= k;
            }
            m.insert("MocopiConnected", 0.0);
            m.insert("MocopiAngleX", face[0]);
            m.insert("MocopiAngleY", face[1]);
            m.insert("MocopiAngleZ", face[2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fld(name: &[u8; 4], v: &[u8]) -> Vec<u8> {
        let mut o = (v.len() as u32).to_le_bytes().to_vec();
        o.extend_from_slice(name);
        o.extend_from_slice(v);
        o
    }

    fn frame_packet(rot10: Q) -> Vec<u8> {
        let head = [fld(b"ftyp", b"sony motion format"), fld(b"vrsn", &[1])].concat();
        let mut btrs = Vec::new();
        for id in 0..BONES as u16 {
            let q = if id == 10 { rot10 } else { [0.0, 0.0, 0.0, 1.0] };
            let mut t = Vec::new();
            for f in [q[0], q[1], q[2], q[3], 0.0, 0.0, 0.0] {
                t.extend_from_slice(&f.to_le_bytes());
            }
            btrs.extend(fld(b"btdt", &[fld(b"bnid", &id.to_le_bytes()), fld(b"tran", &t)].concat()));
        }
        let fram = [fld(b"fnum", &1u32.to_le_bytes()), fld(b"time", &0u32.to_le_bytes()), fld(b"btrs", &btrs)].concat();
        [fld(b"head", &head), fld(b"sndf", &[fld(b"ipad", &[0; 8]), fld(b"rcvp", &12351u16.to_le_bytes())].concat()), fld(b"fram", &fram)].concat()
    }

    #[test]
    fn parse_and_head_yaw() {
        // 30° about Y on the head bone.
        let a = 15f32.to_radians();
        let pkt = frame_packet([0.0, a.sin(), 0.0, a.cos()]);
        let Some(Packet::Frame(f)) = parse(&pkt) else { panic!("not parsed") };
        let v = compute(&f, &default_parents(), [0.0; 3], false);
        assert!((v["MocopiAngleX"] - 30.0).abs() < 0.01, "{}", v["MocopiAngleX"]);
        assert!(v["MocopiAngleY"].abs() < 0.01);
        let v = compute(&f, &default_parents(), [0.0; 3], true);
        assert!((v["MocopiAngleX"] + 30.0).abs() < 0.01);
    }
}
