//! Microphone lipsync ("Advanced Lipsync" in VTS): WASAPI capture, volume with gain and noise
//! cutoff, A/I/U/E/O vowel detection. Detection compares MFCC features with per-vowel profiles
//! (calibrated with the user's voice, like uLipSync); before calibration a formant (F1/F2)
//! estimate is used. Optional monitoring plays the microphone back on the default output.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub const VOWELS: [&str; 5] = ["A", "I", "U", "E", "O"];
const N_MFCC: usize = 12;
const FFT: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MicCfg {
    pub on: bool,
    /// Device id ("" = default capture device).
    pub device: String,
    pub device_name: String,
    /// VTS "Volume gain" (0..1 slider, boosts up to 10x).
    pub gain: f32,
    /// VTS "Volume cutoff" (noise gate).
    pub cutoff: f32,
    /// VTS "Frequency gain".
    pub freq_gain: f32,
    /// Weights used to combine the vowels into VoiceFrequency (A, I, U, E, O).
    pub freq_weights: [f32; 5],
    pub monitor: bool,
    /// Calibrated MFCC profile per vowel (A..O) and silence.
    pub profiles: HashMap<String, Vec<f32>>,
}

impl Default for MicCfg {
    fn default() -> Self {
        MicCfg { on: false, device: String::new(), device_name: String::new(), gain: 0.3, cutoff: 0.05, freq_gain: 0.5, freq_weights: [1.0, 0.6, 0.2, 0.8, 0.3], monitor: false, profiles: HashMap::new() }
    }
}

/// Analysis result shared with the main thread.
#[derive(Clone, Copy, Default, Debug)]
pub struct Voice {
    pub volume: f32,
    pub vowels: [f32; 5],
    pub silence: f32,
    pub frequency: f32,
}

#[derive(Default)]
struct Shared {
    voice: Voice,
    /// Raw RMS of the last block (for the level meter, before gain).
    raw: f32,
    /// Calibration: collecting MFCC vectors for this key.
    calib: Option<(String, Vec<Vec<f32>>)>,
    error: Option<String>,
    cfg: MicCfg,
}

pub struct Mic {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

impl Mic {
    pub fn start(cfg: &MicCfg) -> Mic {
        let shared = Arc::new(Mutex::new(Shared { cfg: cfg.clone(), ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (shared, stop) = (shared.clone(), stop.clone());
            std::thread::Builder::new().name("mic".into()).spawn(move || {
                if let Err(e) = capture(shared.clone(), stop) {
                    shared.lock().unwrap().error = Some(format!("{e:#}"));
                }
            }).ok();
        }
        Mic { shared, stop }
    }

    pub fn set_cfg(&self, cfg: &MicCfg) {
        self.shared.lock().unwrap().cfg = cfg.clone();
    }

    pub fn voice(&self) -> Voice {
        self.shared.lock().unwrap().voice
    }

    pub fn raw_level(&self) -> f32 {
        self.shared.lock().unwrap().raw
    }

    pub fn error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.clone()
    }

    /// Starts collecting a calibration profile for a vowel ("A".."O" or "Silence").
    pub fn calibrate(&self, key: &str) {
        self.shared.lock().unwrap().calib = Some((key.to_string(), Vec::new()));
    }

    /// Calibration progress for the UI (does not finish the calibration).
    pub fn calib_progress(&self) -> Option<(String, f32)> {
        let g = self.shared.lock().unwrap();
        g.calib.as_ref().map(|(k, f)| (k.clone(), (f.len() as f32 / 90.0).min(1.0)))
    }

    /// Calibration progress 0..1, and the finished profile when done.
    pub fn calib_state(&self) -> Option<(String, f32, Option<Vec<f32>>)> {
        let mut g = self.shared.lock().unwrap();
        let (key, frames) = g.calib.as_ref()?.clone();
        const NEED: usize = 90;
        if frames.len() >= NEED {
            let mut avg = vec![0.0; N_MFCC];
            for f in &frames {
                for k in 0..N_MFCC {
                    avg[k] += f[k] / frames.len() as f32;
                }
            }
            g.calib = None;
            return Some((key, 1.0, Some(avg)));
        }
        Some((key, frames.len() as f32 / NEED as f32, None))
    }
}

impl Drop for Mic {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ------------------------------------------------------------------ DSP

fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Power spectrum of a Hann-windowed block.
fn spectrum(x: &[f32]) -> Vec<f32> {
    let n = FFT;
    let mut re: Vec<f32> = (0..n).map(|i| x.get(i).copied().unwrap_or(0.0) * (0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos())).collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    (0..n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
}

fn mel(f: f32) -> f32 {
    2595.0 * (1.0 + f / 700.0).log10()
}

/// MFCC (uLipSync-style: mel filterbank → log → DCT, without the 0th coefficient).
pub fn mfcc(power: &[f32], rate: f32) -> Vec<f32> {
    const BANDS: usize = 24;
    let (lo, hi) = (mel(100.0), mel(4000.0f32.min(rate / 2.0)));
    let bin = |m: f32| {
        let f = 700.0 * (10f32.powf(m / 2595.0) - 1.0);
        (f / rate * FFT as f32).round() as usize
    };
    let pts: Vec<usize> = (0..BANDS + 2).map(|i| bin(lo + (hi - lo) * i as f32 / (BANDS + 1) as f32)).collect();
    let mut energies = vec![0.0f32; BANDS];
    for b in 0..BANDS {
        let (a, c, d) = (pts[b], pts[b + 1], pts[b + 2]);
        for k in a..d.min(power.len()) {
            let w = if k < c { (k - a) as f32 / (c - a).max(1) as f32 } else { (d - k) as f32 / (d - c).max(1) as f32 };
            energies[b] += power[k] * w;
        }
        energies[b] = (energies[b] + 1e-10).ln();
    }
    (1..=N_MFCC)
        .map(|n| (0..BANDS).map(|b| energies[b] * (std::f32::consts::PI * n as f32 * (b as f32 + 0.5) / BANDS as f32).cos()).sum::<f32>())
        .collect()
}

/// Formant estimate (F1, F2) from the spectrum peaks, used before calibration.
fn formants(power: &[f32], rate: f32) -> (f32, f32) {
    let hz = |k: usize| k as f32 * rate / FFT as f32;
    let k_of = |f: f32| (f / rate * FFT as f32) as usize;
    // Smooth the spectrum to find the envelope peaks.
    let sm: Vec<f32> = (0..power.len()).map(|k| (k.saturating_sub(3)..(k + 4).min(power.len())).map(|j| power[j]).sum::<f32>()).collect();
    let peak = |a: f32, b: f32| {
        let (a, b) = (k_of(a), k_of(b).min(sm.len().saturating_sub(1)));
        (a..=b).max_by(|x, y| sm[*x].partial_cmp(&sm[*y]).unwrap()).map(hz).unwrap_or(0.0)
    };
    let f1 = peak(250.0, 1000.0);
    let f2 = peak((f1 + 300.0).max(700.0), 3000.0);
    (f1, f2)
}

/// Vowel weights from formants (Japanese vowel targets, as uLipSync's defaults are Japanese).
fn vowels_from_formants(f1: f32, f2: f32) -> [f32; 5] {
    let targets = [(800.0, 1250.0), (300.0, 2300.0), (350.0, 1300.0), (500.0, 1900.0), (500.0, 850.0)];
    let mut w = [0.0f32; 5];
    for (i, (t1, t2)) in targets.iter().enumerate() {
        let d = ((f1 - t1) / 200.0).powi(2) + ((f2 - t2) / 450.0).powi(2);
        w[i] = (-d).exp();
    }
    let s: f32 = w.iter().sum::<f32>().max(1e-6);
    w.map(|x| x / s)
}

fn vowels_from_profiles(m: &[f32], profiles: &HashMap<String, Vec<f32>>) -> Option<([f32; 5], f32)> {
    if VOWELS.iter().any(|v| !profiles.contains_key(*v)) {
        return None;
    }
    let dist = |p: &Vec<f32>| m.iter().zip(p).map(|(a, b)| (a - b).powi(2)).sum::<f32>().sqrt();
    let d: Vec<f32> = VOWELS.iter().map(|v| dist(&profiles[*v])).collect();
    let silence_d = profiles.get("Silence").map(dist);
    let min = d.iter().cloned().fold(f32::MAX, f32::min).max(1e-3);
    let mut w = [0.0f32; 5];
    for i in 0..5 {
        w[i] = (-(d[i] / min - 1.0) * 4.0).exp();
    }
    let s: f32 = w.iter().sum::<f32>().max(1e-6);
    let silence = silence_d.map(|sd| if sd < min { 1.0 } else { 0.0 }).unwrap_or(0.0);
    Some((w.map(|x| x / s), silence))
}

fn analyze(block: &[f32], rate: f32, sh: &Mutex<Shared>) {
    let rms = (block.iter().map(|x| x * x).sum::<f32>() / block.len().max(1) as f32).sqrt();
    let mut g = sh.lock().unwrap();
    let cfg = g.cfg.clone();
    g.raw = rms;
    let power = spectrum(block);
    let m = mfcc(&power, rate);
    if let Some((_, frames)) = g.calib.as_mut() {
        frames.push(m.clone());
    }
    // Volume: gain up to 10x, then the noise gate.
    let gain = 1.0 + cfg.gain * 30.0;
    let vol_raw = (rms * gain * 4.0).min(1.0);
    let vol = if vol_raw < cfg.cutoff { 0.0 } else { ((vol_raw - cfg.cutoff) / (1.0 - cfg.cutoff).max(0.01)).min(1.0) };
    let (mut vw, mut silence) = match vowels_from_profiles(&m, &cfg.profiles) {
        Some(x) => x,
        None => {
            let (f1, f2) = formants(&power, rate);
            (vowels_from_formants(f1, f2), 0.0)
        }
    };
    if vol <= 0.01 {
        silence = 1.0;
    }
    let fg = 0.5 + cfg.freq_gain * 1.5;
    for w in vw.iter_mut() {
        *w = (*w * vol * fg).min(1.0) * (1.0 - silence);
    }
    let freq = (vw.iter().zip(cfg.freq_weights.iter()).map(|(a, b)| a * b).sum::<f32>()).min(1.0);
    // Smooth a little (blocks are ~20 ms).
    let prev = g.voice;
    let k = 0.5;
    let mix = |a: f32, b: f32| a + (b - a) * k;
    g.voice = Voice {
        volume: mix(prev.volume, vol),
        vowels: [0, 1, 2, 3, 4].map(|i| mix(prev.vowels[i], vw[i])),
        silence: mix(prev.silence, silence),
        frequency: mix(prev.frequency, freq),
    };
}

// ------------------------------------------------------------------ WASAPI

#[cfg(windows)]
pub fn list_devices() -> Vec<(String, String)> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(en): windows::core::Result<IMMDeviceEnumerator> = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return Vec::new() };
        let Ok(col) = en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) else { return Vec::new() };
        let n = col.GetCount().unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..n {
            let Ok(d) = col.Item(i) else { continue };
            let id = d.GetId().ok().and_then(|p| p.to_string().ok()).unwrap_or_default();
            let name = d
                .OpenPropertyStore(STGM_READ)
                .ok()
                .and_then(|ps| ps.GetValue(&PKEY_Device_FriendlyName).ok())
                .map(|v| v.to_string())
                .unwrap_or_else(|| id.clone());
            out.push((id, name));
        }
        out
    }
}

#[cfg(not(windows))]
pub fn list_devices() -> Vec<(String, String)> {
    Vec::new()
}

#[cfg(windows)]
fn capture(sh: Arc<Mutex<Shared>>, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let dev_id = sh.lock().unwrap().cfg.device.clone();
        let dev = if dev_id.is_empty() { en.GetDefaultAudioEndpoint(eCapture, eConsole)? } else { en.GetDevice(PCWSTR(HSTRING::from(dev_id.as_str()).as_ptr()))? };
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let fmt = client.GetMixFormat()?;
        let f = *fmt;
        let channels = f.nChannels as usize;
        let rate = f.nSamplesPerSec as f32;
        let bits = f.wBitsPerSample;
        // Float32 is the usual shared-mode format; 16-bit PCM is handled too.
        let is_float = bits == 32;
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 200_000, 0, fmt, None)?;
        let cap: IAudioCaptureClient = client.GetService()?;
        // Monitoring output.
        let mut render: Option<(IAudioClient, IAudioRenderClient, usize, u32)> = None;
        let open_render = || -> windows::core::Result<(IAudioClient, IAudioRenderClient, usize, u32)> {
            let out = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let rc: IAudioClient = out.Activate(CLSCTX_ALL, None)?;
            let rf = rc.GetMixFormat()?;
            let ch = (*rf).nChannels as usize;
            rc.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 400_000, 0, rf, None)?;
            let buf = rc.GetBufferSize()?;
            let rcl: IAudioRenderClient = rc.GetService()?;
            rc.Start()?;
            Ok((rc, rcl, ch, buf))
        };
        client.Start()?;
        let block_len = (rate * 0.02) as usize; // 20 ms analysis hop
        let mut buf: Vec<f32> = Vec::with_capacity(FFT * 2);
        let mut since = 0usize;
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(5));
            let monitor = sh.lock().unwrap().cfg.monitor;
            if monitor && render.is_none() {
                render = open_render().ok();
            } else if !monitor {
                if let Some((rc, ..)) = render.take() {
                    let _ = rc.Stop();
                }
            }
            loop {
                let n = cap.GetNextPacketSize()?;
                if n == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let mut mono = Vec::with_capacity(frames as usize);
                let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                for i in 0..frames as usize {
                    let mut s = 0.0;
                    if !silent {
                        for c in 0..channels {
                            s += if is_float { *(data as *const f32).add(i * channels + c) } else { *(data as *const i16).add(i * channels + c) as f32 / 32768.0 };
                        }
                    }
                    mono.push(s / channels as f32);
                }
                cap.ReleaseBuffer(frames)?;
                if let Some((rc, rcl, ch, size)) = &render {
                    if let Ok(pad) = rc.GetCurrentPadding() {
                        let free = (*size - pad) as usize;
                        let n = mono.len().min(free);
                        if n > 0 {
                            if let Ok(p) = rcl.GetBuffer(n as u32) {
                                let out = p as *mut f32;
                                for i in 0..n {
                                    for c in 0..*ch {
                                        *out.add(i * ch + c) = mono[i];
                                    }
                                }
                                let _ = rcl.ReleaseBuffer(n as u32, 0);
                            }
                        }
                    }
                }
                buf.extend_from_slice(&mono);
                since += mono.len();
                if buf.len() > FFT {
                    let cut = buf.len() - FFT;
                    buf.drain(..cut);
                }
                if since >= block_len && buf.len() >= FFT / 2 {
                    since = 0;
                    analyze(&buf, rate, &sh);
                }
            }
        }
        client.Stop()?;
        if let Some((rc, ..)) = render {
            let _ = rc.Stop();
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn capture(_sh: Arc<Mutex<Shared>>, _stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    anyhow::bail!("microphone capture is Windows-only")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freqs: &[(f32, f32)], rate: f32) -> Vec<f32> {
        (0..FFT).map(|i| freqs.iter().map(|(f, a)| a * (2.0 * std::f32::consts::PI * f * i as f32 / rate).sin()).sum()).collect()
    }

    #[test]
    fn formant_vowels() {
        let rate = 48000.0;
        // Strong energy at ~800 Hz and ~1250 Hz → "A"; ~300 / ~2300 → "I".
        let a = vowels_from_formants_test(&tone(&[(800.0, 1.0), (1250.0, 0.7)], rate), rate);
        let i = vowels_from_formants_test(&tone(&[(300.0, 1.0), (2300.0, 0.7)], rate), rate);
        assert_eq!(argmax(&a), 0);
        assert_eq!(argmax(&i), 1);
    }

    fn vowels_from_formants_test(x: &[f32], rate: f32) -> [f32; 5] {
        let p = spectrum(x);
        let (f1, f2) = formants(&p, rate);
        vowels_from_formants(f1, f2)
    }

    fn argmax(v: &[f32; 5]) -> usize {
        (0..5).max_by(|a, b| v[*a].partial_cmp(&v[*b]).unwrap()).unwrap()
    }

    #[test]
    fn mfcc_is_stable() {
        let rate = 48000.0;
        let m = mfcc(&spectrum(&tone(&[(500.0, 1.0)], rate)), rate);
        assert_eq!(m.len(), N_MFCC);
        assert!(m.iter().all(|x| x.is_finite()));
    }
}

#[cfg(test)]
mod device_tests {
    #[test]
    #[ignore]
    fn lists_capture_devices() {
        let d = super::list_devices();
        println!("capture devices: {}", d.len());
        for (_, n) in &d {
            println!("  {n}");
        }
    }
}
