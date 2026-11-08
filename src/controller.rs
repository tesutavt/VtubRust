//! Game controller input (VTS "Controller Input"): XInput controllers (Xbox and compatible)
//! and DualShock 4 / DualSense / DualSense Edge over HID (USB and Bluetooth reports), with
//! touchpad, swipe gestures and gyro orientation. Produces the 39 VTS controller parameters
//! and button states usable as hotkey triggers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Buttons as bits (also used as hotkey "keys" via `PAD_KEYS`).
pub mod btn {
    pub const CROSS: u64 = 1 << 0;
    pub const CIRCLE: u64 = 1 << 1;
    pub const SQUARE: u64 = 1 << 2;
    pub const TRIANGLE: u64 = 1 << 3;
    pub const L1: u64 = 1 << 4;
    pub const R1: u64 = 1 << 5;
    pub const L2: u64 = 1 << 6;
    pub const R2: u64 = 1 << 7;
    pub const L3: u64 = 1 << 8;
    pub const R3: u64 = 1 << 9;
    pub const OPT_L: u64 = 1 << 10;
    pub const OPT_R: u64 = 1 << 11;
    pub const HOME: u64 = 1 << 12;
    pub const TOUCH: u64 = 1 << 13;
    pub const UP: u64 = 1 << 14;
    pub const DOWN: u64 = 1 << 15;
    pub const LEFT: u64 = 1 << 16;
    pub const RIGHT: u64 = 1 << 17;
    pub const LS_UP: u64 = 1 << 18;
    pub const LS_DOWN: u64 = 1 << 19;
    pub const LS_LEFT: u64 = 1 << 20;
    pub const LS_RIGHT: u64 = 1 << 21;
    pub const RS_UP: u64 = 1 << 22;
    pub const RS_DOWN: u64 = 1 << 23;
    pub const RS_LEFT: u64 = 1 << 24;
    pub const RS_RIGHT: u64 = 1 << 25;
    pub const SWIPE_LEFT: u64 = 1 << 26;
    pub const SWIPE_RIGHT: u64 = 1 << 27;
    pub const SWIPE_UP: u64 = 1 << 28;
    pub const SWIPE_DOWN: u64 = 1 << 29;
}

/// Hotkey key names for controller inputs (virtual codes 0x1000 + bit).
pub const PAD_KEYS: [(&str, u64); 30] = [
    ("ControllerCross", btn::CROSS), ("ControllerCircle", btn::CIRCLE), ("ControllerSquare", btn::SQUARE), ("ControllerTriangle", btn::TRIANGLE),
    ("ControllerShoulderLeft", btn::L1), ("ControllerShoulderRight", btn::R1), ("ControllerTriggerLeft", btn::L2), ("ControllerTriggerRight", btn::R2),
    ("ControllerStickPressLeft", btn::L3), ("ControllerStickPressRight", btn::R3), ("ControllerOptionLeft", btn::OPT_L), ("ControllerOptionRight", btn::OPT_R),
    ("ControllerHome", btn::HOME), ("ControllerTouchPadPress", btn::TOUCH), ("ControllerDPadUp", btn::UP), ("ControllerDPadDown", btn::DOWN),
    ("ControllerDPadLeft", btn::LEFT), ("ControllerDPadRight", btn::RIGHT), ("ControllerStickLeftUp", btn::LS_UP), ("ControllerStickLeftDown", btn::LS_DOWN),
    ("ControllerStickLeftLeft", btn::LS_LEFT), ("ControllerStickLeftRight", btn::LS_RIGHT), ("ControllerStickRightUp", btn::RS_UP), ("ControllerStickRightDown", btn::RS_DOWN),
    ("ControllerStickRightLeft", btn::RS_LEFT), ("ControllerStickRightRight", btn::RS_RIGHT), ("ControllerSwipeLeft", btn::SWIPE_LEFT), ("ControllerSwipeRight", btn::SWIPE_RIGHT),
    ("ControllerSwipeUp", btn::SWIPE_UP), ("ControllerSwipeDown", btn::SWIPE_DOWN),
];

/// Currently pressed controller buttons (read by the hotkey system).
pub static PAD_DOWN: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Default, Debug)]
pub struct Pad {
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
    pub lt: f32,
    pub rt: f32,
    pub buttons: u64,
    /// Touch points (active, x 0..1, y 0..1).
    pub touch: [(bool, f32, f32); 2],
    /// Gyro rates in deg/s (pitch, yaw, roll) when available.
    pub gyro: Option<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub extended: bool,
}

#[derive(Default)]
struct Swipe {
    start: Option<(f32, f32)>,
    locked: u64,
}

pub struct Controllers {
    #[cfg(windows)]
    hid: Option<hidapi::HidApi>,
    #[cfg(windows)]
    open: Vec<(String, hidapi::HidDevice, Kind)>,
    pub devices: Vec<Device>,
    last_scan: Option<Instant>,
    pub pad: Pad,
    pub connected: bool,
    pub orientation: [f32; 3],
    swipes: [Swipe; 2],
    reset_combo: Option<Instant>,
    last: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Ds4,
    DualSense,
}

const SONY: u16 = 0x054C;
const DS4_IDS: [u16; 3] = [0x05C4, 0x09CC, 0x0BA0];
const DS_IDS: [u16; 2] = [0x0CE6, 0x0DF2];

impl Default for Controllers {
    fn default() -> Self {
        Controllers {
            #[cfg(windows)]
            hid: None,
            #[cfg(windows)]
            open: Vec::new(),
            devices: Vec::new(),
            last_scan: None,
            pad: Pad::default(),
            connected: false,
            orientation: [0.0; 3],
            swipes: Default::default(),
            reset_combo: None,
            last: None,
        }
    }
}

fn i16le(b: &[u8], o: usize) -> f32 {
    i16::from_le_bytes([b[o], b[o + 1]]) as f32
}

fn axis(v: u8) -> f32 {
    ((v as f32 - 127.5) / 127.5).clamp(-1.0, 1.0)
}

fn hat(h: u8) -> u64 {
    match h & 0x0F {
        0 => btn::UP,
        1 => btn::UP | btn::RIGHT,
        2 => btn::RIGHT,
        3 => btn::RIGHT | btn::DOWN,
        4 => btn::DOWN,
        5 => btn::DOWN | btn::LEFT,
        6 => btn::LEFT,
        7 => btn::LEFT | btn::UP,
        _ => 0,
    }
}

/// Parses a DualShock 4 / DualSense input report. Returns None for unknown reports.
fn parse_sony(kind: Kind, r: &[u8]) -> Option<Pad> {
    let mut p = Pad::default();
    match kind {
        Kind::Ds4 => {
            // USB: report 0x01 (64 bytes). Bluetooth: 0x11 with 2 extra bytes.
            let o = match r.first()? {
                0x01 if r.len() >= 43 => 0,
                0x11 if r.len() >= 45 => 2,
                _ => return None,
            };
            let b = &r[o..];
            p.lx = axis(b[1]);
            p.ly = -axis(b[2]);
            p.rx = axis(b[3]);
            p.ry = -axis(b[4]);
            let (b5, b6, b7) = (b[5], b[6], b[7]);
            p.buttons = hat(b5);
            for (bit, v) in [(0x10, btn::SQUARE), (0x20, btn::CROSS), (0x40, btn::CIRCLE), (0x80, btn::TRIANGLE)] {
                if b5 & bit != 0 {
                    p.buttons |= v;
                }
            }
            for (bit, v) in [(0x01, btn::L1), (0x02, btn::R1), (0x10, btn::OPT_L), (0x20, btn::OPT_R), (0x40, btn::L3), (0x80, btn::R3)] {
                if b6 & bit != 0 {
                    p.buttons |= v;
                }
            }
            if b7 & 0x01 != 0 {
                p.buttons |= btn::HOME;
            }
            if b7 & 0x02 != 0 {
                p.buttons |= btn::TOUCH;
            }
            p.lt = b[8] as f32 / 255.0;
            p.rt = b[9] as f32 / 255.0;
            p.gyro = Some([i16le(b, 13) / 16.384, i16le(b, 15) / 16.384, i16le(b, 17) / 16.384]);
            for k in 0..2 {
                let t = 35 + k * 4;
                let active = b[t] & 0x80 == 0;
                let x = (b[t + 1] as u32 | ((b[t + 2] as u32 & 0x0F) << 8)) as f32 / 1920.0;
                let y = ((b[t + 2] as u32 >> 4) | ((b[t + 3] as u32) << 4)) as f32 / 942.0;
                p.touch[k] = (active, x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
            }
        }
        Kind::DualSense => {
            // USB: report 0x01 (64 bytes). Bluetooth: 0x31 with 1 extra byte.
            let o = match r.first()? {
                0x01 if r.len() >= 41 => 0,
                0x31 if r.len() >= 42 => 1,
                _ => return None,
            };
            let b = &r[o..];
            p.lx = axis(b[1]);
            p.ly = -axis(b[2]);
            p.rx = axis(b[3]);
            p.ry = -axis(b[4]);
            p.lt = b[5] as f32 / 255.0;
            p.rt = b[6] as f32 / 255.0;
            let (b8, b9, b10) = (b[8], b[9], b[10]);
            p.buttons = hat(b8);
            for (bit, v) in [(0x10, btn::SQUARE), (0x20, btn::CROSS), (0x40, btn::CIRCLE), (0x80, btn::TRIANGLE)] {
                if b8 & bit != 0 {
                    p.buttons |= v;
                }
            }
            for (bit, v) in [(0x01, btn::L1), (0x02, btn::R1), (0x10, btn::OPT_L), (0x20, btn::OPT_R), (0x40, btn::L3), (0x80, btn::R3)] {
                if b9 & bit != 0 {
                    p.buttons |= v;
                }
            }
            if b10 & 0x01 != 0 {
                p.buttons |= btn::HOME;
            }
            if b10 & 0x02 != 0 {
                p.buttons |= btn::TOUCH;
            }
            p.gyro = Some([i16le(b, 16) / 16.384, i16le(b, 18) / 16.384, i16le(b, 20) / 16.384]);
            for k in 0..2 {
                let t = 33 + k * 4;
                let active = b[t] & 0x80 == 0;
                let x = (b[t + 1] as u32 | ((b[t + 2] as u32 & 0x0F) << 8)) as f32 / 1920.0;
                let y = ((b[t + 2] as u32 >> 4) | ((b[t + 3] as u32) << 4)) as f32 / 1080.0;
                p.touch[k] = (active, x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
            }
        }
    }
    if p.lt > 0.3 {
        p.buttons |= btn::L2;
    }
    if p.rt > 0.3 {
        p.buttons |= btn::R2;
    }
    Some(p)
}

#[cfg(windows)]
fn xinput(index: u32) -> Option<Pad> {
    use windows::Win32::UI::Input::XboxController::*;
    let mut st = XINPUT_STATE::default();
    if unsafe { XInputGetState(index, &mut st) } != 0 {
        return None;
    }
    let g = st.Gamepad;
    let ax = |v: i16, dz: i16| if v.unsigned_abs() < dz as u16 { 0.0 } else { (v as f32 / 32767.0).clamp(-1.0, 1.0) };
    let w = g.wButtons.0;
    let mut b = 0u64;
    for (bit, v) in [
        (XINPUT_GAMEPAD_A.0, btn::CROSS), (XINPUT_GAMEPAD_B.0, btn::CIRCLE), (XINPUT_GAMEPAD_X.0, btn::SQUARE), (XINPUT_GAMEPAD_Y.0, btn::TRIANGLE),
        (XINPUT_GAMEPAD_LEFT_SHOULDER.0, btn::L1), (XINPUT_GAMEPAD_RIGHT_SHOULDER.0, btn::R1), (XINPUT_GAMEPAD_LEFT_THUMB.0, btn::L3), (XINPUT_GAMEPAD_RIGHT_THUMB.0, btn::R3),
        (XINPUT_GAMEPAD_BACK.0, btn::OPT_L), (XINPUT_GAMEPAD_START.0, btn::OPT_R), (XINPUT_GAMEPAD_DPAD_UP.0, btn::UP), (XINPUT_GAMEPAD_DPAD_DOWN.0, btn::DOWN),
        (XINPUT_GAMEPAD_DPAD_LEFT.0, btn::LEFT), (XINPUT_GAMEPAD_DPAD_RIGHT.0, btn::RIGHT),
    ] {
        if w & bit != 0 {
            b |= v;
        }
    }
    let lt = g.bLeftTrigger as f32 / 255.0;
    let rt = g.bRightTrigger as f32 / 255.0;
    if lt > 0.3 {
        b |= btn::L2;
    }
    if rt > 0.3 {
        b |= btn::R2;
    }
    Some(Pad { lx: ax(g.sThumbLX, 7849), ly: ax(g.sThumbLY, 7849), rx: ax(g.sThumbRX, 8689), ry: ax(g.sThumbRY, 8689), lt, rt, buttons: b, touch: [(false, 0.0, 0.0); 2], gyro: None })
}

fn merge(a: &mut Pad, b: &Pad) {
    let pick = |x: f32, y: f32| if y.abs() > x.abs() { y } else { x };
    a.lx = pick(a.lx, b.lx);
    a.ly = pick(a.ly, b.ly);
    a.rx = pick(a.rx, b.rx);
    a.ry = pick(a.ry, b.ry);
    a.lt = a.lt.max(b.lt);
    a.rt = a.rt.max(b.rt);
    a.buttons |= b.buttons;
    if a.gyro.is_none() {
        a.gyro = b.gyro;
        a.touch = b.touch;
    }
}

impl Controllers {
    /// Polls controllers. `mode`: 0 = none, 1 = all, 2 = only `selected`.
    pub fn poll(&mut self, mode: u8, selected: &str, ui_open: bool) {
        let now = Instant::now();
        let dt = self.last.map(|l| now.duration_since(l).as_secs_f32()).unwrap_or(0.0).min(0.1);
        self.last = Some(now);
        if (mode != 0 || ui_open) && self.last_scan.map(|t| now.duration_since(t).as_secs() >= 3).unwrap_or(true) {
            self.last_scan = Some(now);
            self.scan();
        }
        if mode == 0 {
            self.pad = Pad::default();
            self.connected = false;
            PAD_DOWN.store(0, Ordering::Relaxed);
            return;
        }
        let mut pad = Pad::default();
        let mut any = false;
        #[cfg(windows)]
        {
            for i in 0..4u32 {
                let id = format!("xinput:{i}");
                if mode == 2 && selected != id {
                    continue;
                }
                if let Some(p) = xinput(i) {
                    merge(&mut pad, &p);
                    any = true;
                }
            }
            let mut dead = Vec::new();
            for (k, (id, dev, kind)) in self.open.iter().enumerate() {
                if mode == 2 && selected != id {
                    continue;
                }
                let mut buf = [0u8; 128];
                let mut latest = None;
                loop {
                    match dev.read_timeout(&mut buf, 0) {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Some(p) = parse_sony(*kind, &buf[..n]) {
                                latest = Some(p);
                            }
                        }
                        Err(_) => {
                            dead.push(k);
                            break;
                        }
                    }
                }
                if let Some(p) = latest {
                    merge(&mut pad, &p);
                    any = true;
                } else if let Some(prev) = self.pad.gyro.map(|_| self.pad) {
                    // No new report this frame: keep the last state of this controller.
                    merge(&mut pad, &prev);
                    any = true;
                }
            }
            for k in dead.into_iter().rev() {
                self.open.remove(k);
            }
        }
        // Orientation from the gyro (integrated, drifts; reset to zero on demand).
        if let Some(g) = pad.gyro {
            self.orientation[0] = (self.orientation[0] + g[1] * dt).clamp(-60.0, 60.0);
            self.orientation[1] = (self.orientation[1] + g[0] * dt).clamp(-60.0, 60.0);
            self.orientation[2] = (self.orientation[2] - g[2] * dt).clamp(-60.0, 60.0);
        }
        // L1 + R1 + L2 + R2 + Triangle held for 2 s resets the orientation (VTS).
        let combo = btn::L1 | btn::R1 | btn::L2 | btn::R2 | btn::TRIANGLE;
        if pad.buttons & combo == combo {
            let t = *self.reset_combo.get_or_insert(now);
            if now.duration_since(t).as_secs_f32() >= 2.0 {
                self.orientation = [0.0; 3];
                self.reset_combo = Some(now + std::time::Duration::from_secs(3600));
            }
        } else {
            self.reset_combo = None;
        }
        // Stick directions as buttons.
        for (v, neg, pos) in [(pad.lx, btn::LS_LEFT, btn::LS_RIGHT), (pad.ly, btn::LS_DOWN, btn::LS_UP), (pad.rx, btn::RS_LEFT, btn::RS_RIGHT), (pad.ry, btn::RS_DOWN, btn::RS_UP)] {
            if v < -0.6 {
                pad.buttons |= neg;
            }
            if v > 0.6 {
                pad.buttons |= pos;
            }
        }
        // Touchpad swipes: direction locked per touch until the finger is lifted.
        for k in 0..2 {
            let (active, x, y) = pad.touch[k];
            let s = &mut self.swipes[k];
            if !active {
                *s = Swipe::default();
                continue;
            }
            let (sx, sy) = *s.start.get_or_insert((x, y));
            if s.locked == 0 {
                let (dx, dy) = (x - sx, y - sy);
                if dx.abs() > 0.2 && dx.abs() > dy.abs() * 2.0 {
                    s.locked = if dx < 0.0 { btn::SWIPE_LEFT } else { btn::SWIPE_RIGHT };
                } else if dy.abs() > 0.2 && dy.abs() > dx.abs() * 2.0 {
                    s.locked = if dy < 0.0 { btn::SWIPE_UP } else { btn::SWIPE_DOWN };
                }
            }
            pad.buttons |= s.locked;
        }
        self.connected = any;
        self.pad = pad;
        PAD_DOWN.store(if any { pad.buttons } else { 0 }, Ordering::Relaxed);
    }

    pub fn reset_orientation(&mut self) {
        self.orientation = [0.0; 3];
    }

    fn scan(&mut self) {
        let mut devs = Vec::new();
        #[cfg(windows)]
        {
            for i in 0..4u32 {
                if xinput(i).is_some() {
                    devs.push(Device { id: format!("xinput:{i}"), name: format!("XInput / Xbox #{}", i + 1), extended: false });
                }
            }
            if self.hid.is_none() {
                self.hid = hidapi::HidApi::new().ok();
            }
            if let Some(api) = self.hid.as_mut() {
                let _ = api.refresh_devices();
                for d in api.device_list() {
                    if d.vendor_id() != SONY {
                        continue;
                    }
                    let kind = if DS4_IDS.contains(&d.product_id()) {
                        Kind::Ds4
                    } else if DS_IDS.contains(&d.product_id()) {
                        Kind::DualSense
                    } else {
                        continue;
                    };
                    let id = format!("hid:{}", d.path().to_string_lossy());
                    if devs.iter().any(|x: &Device| x.id == id) {
                        continue;
                    }
                    let name = match (kind, d.product_id()) {
                        (Kind::Ds4, _) => "DualShock 4".to_string(),
                        (_, 0x0DF2) => "DualSense Edge".to_string(),
                        _ => "DualSense".to_string(),
                    };
                    devs.push(Device { id: id.clone(), name, extended: true });
                    if !self.open.iter().any(|o| o.0 == id) {
                        if let Ok(h) = d.open_device(api) {
                            let _ = h.set_blocking_mode(false);
                            if kind == Kind::Ds4 {
                                // Feature report 0x02 switches a Bluetooth DS4 to full reports (gyro, touchpad).
                                let mut f = [0u8; 37];
                                f[0] = 0x02;
                                let _ = h.get_feature_report(&mut f);
                            }
                            self.open.push((id, h, kind));
                        }
                    }
                }
            }
        }
        self.devices = devs;
    }

    /// Writes the VTS controller parameters into `inputs` (from `input::CONTROLLER_FIRST`).
    pub fn write_inputs(&self, inputs: &mut crate::mapping::Inputs) {
        let base = crate::mapping::input::CONTROLLER_FIRST;
        let p = &self.pad;
        let b = |m: u64| if p.buttons & m != 0 { 1.0 } else { 0.0 };
        let left = [btn::UP, btn::DOWN, btn::LEFT, btn::RIGHT, btn::L1, btn::L2, btn::L3, btn::OPT_L];
        let right = [btn::CROSS, btn::CIRCLE, btn::SQUARE, btn::TRIANGLE, btn::R1, btn::R2, btn::R3, btn::OPT_R];
        let cl = left.iter().filter(|m| p.buttons & **m != 0).count() as f32;
        let cr = right.iter().filter(|m| p.buttons & **m != 0).count() as f32;
        let thumb = |dpad: bool, stick: bool, opt: bool, touch: bool| if touch { 3.0 } else if opt { 2.0 } else if dpad { 0.0 } else if stick { 1.0 } else { 1.0 };
        let finger = |sh: bool, tr: bool| match (sh, tr) {
            (true, true) => 2.0,
            (false, true) => 1.0,
            _ => 0.0,
        };
        let touches = p.touch.iter().filter(|t| t.0).count() as f32;
        let vals: [f32; 39] = [
            p.lx, p.ly, p.rx, p.ry, b(btn::L3), b(btn::R3),
            b(btn::RIGHT) - b(btn::LEFT), b(btn::UP) - b(btn::DOWN),
            b(btn::TRIANGLE), b(btn::CROSS), b(btn::SQUARE), b(btn::CIRCLE),
            b(btn::L1), b(btn::R1), p.lt, p.rt,
            b(btn::OPT_L), b(btn::OPT_R), b(btn::HOME), b(btn::TOUCH),
            if cl > 0.0 { 1.0 } else { 0.0 }, if cr > 0.0 { 1.0 } else { 0.0 }, cl, cr,
            ((cr - cl) * 0.3 + (p.lx + p.rx) * 0.5).clamp(-1.0, 1.0),
            thumb(p.buttons & (btn::UP | btn::DOWN | btn::LEFT | btn::RIGHT) != 0, p.lx.abs() + p.ly.abs() > 0.1 || p.buttons & btn::L3 != 0, p.buttons & btn::OPT_L != 0, p.touch[0].0 && p.touch[0].1 < 0.5),
            thumb(p.buttons & (btn::CROSS | btn::CIRCLE | btn::SQUARE | btn::TRIANGLE) != 0, p.rx.abs() + p.ry.abs() > 0.1 || p.buttons & btn::R3 != 0, p.buttons & btn::OPT_R != 0, p.touch.iter().any(|t| t.0 && t.1 >= 0.5)),
            finger(p.buttons & btn::L1 != 0, p.lt > 0.05), finger(p.buttons & btn::R1 != 0, p.rt > 0.05),
            touches, if p.touch[0].0 { 1.0 } else { 0.0 }, if p.touch[1].0 { 1.0 } else { 0.0 },
            p.touch[0].1, p.touch[0].2, p.touch[1].1, p.touch[1].2,
            self.orientation[0], self.orientation[1], self.orientation[2],
        ];
        inputs[base..base + 39].copy_from_slice(&vals);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ds4_usb_report() {
        let mut r = vec![0u8; 64];
        r[0] = 0x01;
        r[1] = 255; // LX right
        r[2] = 0; // LY up (inverted)
        r[3] = 128;
        r[4] = 128;
        r[5] = 0x08 | 0x20; // hat none + cross
        r[6] = 0x01; // L1
        r[8] = 255; // L2
        r[35] = 0x00; // touch 0 active
        r[36] = 0x80; // x low
        r[37] = 0x07; // x high nibble
        let p = parse_sony(Kind::Ds4, &r).unwrap();
        assert!(p.lx > 0.9 && p.ly > 0.9);
        assert!(p.buttons & btn::CROSS != 0 && p.buttons & btn::L1 != 0 && p.buttons & btn::L2 != 0);
        assert!(p.buttons & (btn::UP | btn::DOWN | btn::LEFT | btn::RIGHT) == 0);
        assert!(p.touch[0].0 && (p.touch[0].1 - 1920.0f32.recip() * 0x780 as f32).abs() < 1e-3);
    }

    #[test]
    fn parses_dualsense_bt_report() {
        let mut r = vec![0u8; 78];
        r[0] = 0x31;
        r[1 + 8] = 0x02 | 0x80; // hat right + triangle
        r[1 + 10] = 0x01; // PS
        r[1 + 33] = 0x80; // touch inactive
        r[1 + 37] = 0x80;
        let p = parse_sony(Kind::DualSense, &r).unwrap();
        assert!(p.buttons & btn::RIGHT != 0 && p.buttons & btn::TRIANGLE != 0 && p.buttons & btn::HOME != 0);
        assert!(!p.touch[0].0);
    }
}
