//! Sound effects engine (VTS "Sound Effects - SFX"): wav / mp3 / ogg decoding (symphonia),
//! a mixer with volume, pitch, pan, delay, echo and fade-out, playback on the default output
//! device (WASAPI) and/or sending to OBS over VBAN (UDP, like VTS, default port 6980).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SoundCfg {
    /// "Play in app" (desktop audio).
    pub play_local: bool,
    /// "Send to OBS" (VBAN).
    pub vban: bool,
    pub vban_ip: String,
    pub vban_port: u16,
    pub vban_stream: String,
    /// Global volume 0..100.
    pub volume: f32,
}

impl Default for SoundCfg {
    fn default() -> Self {
        SoundCfg { play_local: true, vban: false, vban_ip: "127.0.0.1".into(), vban_port: 6980, vban_stream: "rigrs".into(), volume: 80.0 }
    }
}

/// Decoded sound: interleaved stereo f32 at `rate`.
pub struct Clip {
    pub data: Vec<f32>,
    pub rate: f32,
}

/// One playback request.
#[derive(Clone, Debug)]
pub struct Play {
    pub file: PathBuf,
    pub volume: f32,
    pub pitch: f32,
    /// -1 (left) … 1 (right)
    pub pan: f32,
    pub delay: f32,
    pub echo_count: u32,
    pub echo_delay: f32,
    pub echo_volume: f32,
    /// Group used to stop sounds (hotkey id, model key …).
    pub group: String,
}

enum Cmd {
    Play(Play),
    Stop(String, f32),
    Cfg(SoundCfg),
}

struct Voice {
    clip: Arc<Clip>,
    pos: f64,
    step: f64,
    gain_l: f32,
    gain_r: f32,
    delay: usize,
    group: String,
    fade: Option<(f32, f32)>,
}

pub struct Audio {
    tx: Sender<Cmd>,
    stop: Arc<AtomicBool>,
    /// Peak level of the last mixed block (0..1) for the VU meter (f32 bits).
    peak: Arc<AtomicU32>,
    error: Arc<Mutex<Option<String>>>,
}

impl Audio {
    pub fn start(cfg: &SoundCfg) -> Audio {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU32::new(0));
        let error = Arc::new(Mutex::new(None));
        {
            let (stop, peak, error, cfg) = (stop.clone(), peak.clone(), error.clone(), cfg.clone());
            std::thread::Builder::new()
                .name("audio".into())
                .spawn(move || {
                    if let Err(e) = mixer(rx, cfg, stop, peak) {
                        *error.lock().unwrap() = Some(format!("{e:#}"));
                    }
                })
                .ok();
        }
        Audio { tx, stop, peak, error }
    }

    pub fn play(&self, p: Play) {
        let _ = self.tx.send(Cmd::Play(p));
    }

    /// Stops a group with a fade-out (seconds).
    pub fn stop(&self, group: &str, fade: f32) {
        let _ = self.tx.send(Cmd::Stop(group.to_string(), fade));
    }

    pub fn set_cfg(&self, cfg: &SoundCfg) {
        let _ = self.tx.send(Cmd::Cfg(cfg.clone()));
    }

    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Decodes wav / mp3 / ogg into interleaved stereo f32.
pub fn decode(path: &Path) -> anyhow::Result<Clip> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow::anyhow!("no audio track"))?;
    let tid = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(48000) as f32;
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut data = Vec::new();
    loop {
        let pkt = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if pkt.track_id() != tid {
            continue;
        }
        let Ok(buf) = dec.decode(&pkt) else { continue };
        let spec = *buf.spec();
        let ch = spec.channels.count().max(1);
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        for f in sb.samples().chunks(ch) {
            let l = f[0];
            let r = if ch > 1 { f[1] } else { f[0] };
            data.push(l);
            data.push(r);
        }
    }
    Ok(Clip { data, rate })
}

fn vban_packet(stream: &str, frame: u32, samples: &[f32]) -> Vec<u8> {
    // VBAN audio: 28-byte header, then 16-bit PCM stereo (<= 256 frames per packet).
    let n = samples.len() / 2;
    let mut p = Vec::with_capacity(28 + samples.len() * 2);
    p.extend_from_slice(b"VBAN");
    p.push(3); // sub-protocol audio (0x00) | sample rate index 3 = 48000 Hz
    p.push((n - 1) as u8);
    p.push(1); // channels - 1
    p.push(0x01); // INT16, PCM codec
    let mut name = [0u8; 16];
    for (i, b) in stream.bytes().take(16).enumerate() {
        name[i] = b;
    }
    p.extend_from_slice(&name);
    p.extend_from_slice(&frame.to_le_bytes());
    for s in samples {
        p.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    p
}

#[cfg(windows)]
fn mixer(rx: Receiver<Cmd>, mut cfg: SoundCfg, stop: Arc<AtomicBool>, peak: Arc<AtomicU32>) -> anyhow::Result<()> {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    let mut cache: HashMap<PathBuf, Option<Arc<Clip>>> = HashMap::new();
    let mut voices: Vec<Voice> = Vec::new();
    let udp = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok();
    let mut vban_frame = 0u32;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let fmt = client.GetMixFormat()?;
        let out_ch = (*fmt).nChannels as usize;
        let out_rate = (*fmt).nSamplesPerSec as f32;
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 300_000, 0, fmt, None)?;
        let size = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;
        client.Start()?;
        let mut vban_buf: Vec<f32> = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            while let Ok(c) = rx.try_recv() {
                match c {
                    Cmd::Cfg(c) => cfg = c,
                    Cmd::Stop(g, fade) => {
                        for v in voices.iter_mut().filter(|v| v.group == g) {
                            v.fade = Some((1.0, 1.0 / (fade.max(0.01) * out_rate)));
                        }
                    }
                    Cmd::Play(p) => {
                        let clip = cache.entry(p.file.clone()).or_insert_with(|| decode(&p.file).ok().map(Arc::new)).clone();
                        let Some(clip) = clip else { continue };
                        let pan = p.pan.clamp(-1.0, 1.0);
                        let (gl, gr) = (((1.0 - pan) * 0.5).sqrt() * std::f32::consts::SQRT_2, ((1.0 + pan) * 0.5).sqrt() * std::f32::consts::SQRT_2);
                        for e in 0..=p.echo_count.min(8) {
                            let vol = p.volume * p.echo_volume.clamp(0.0, 1.0).powi(e as i32);
                            if vol <= 0.001 {
                                continue;
                            }
                            voices.push(Voice {
                                clip: clip.clone(),
                                pos: 0.0,
                                step: (clip.rate / out_rate * p.pitch.max(0.05)) as f64,
                                gain_l: vol * gl.min(1.0),
                                gain_r: vol * gr.min(1.0),
                                delay: ((p.delay + p.echo_delay * e as f32).max(0.0) * out_rate) as usize,
                                group: p.group.clone(),
                                fade: None,
                            });
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            let pad = client.GetCurrentPadding()?;
            let n = (size - pad) as usize;
            if n == 0 {
                continue;
            }
            let master = (cfg.volume / 100.0).clamp(0.0, 1.0);
            let mut mix = vec![0.0f32; n * 2];
            for v in voices.iter_mut() {
                for i in 0..n {
                    if v.delay > 0 {
                        v.delay -= 1;
                        continue;
                    }
                    let idx = v.pos as usize;
                    if idx + 1 >= v.clip.data.len() / 2 {
                        v.pos = f64::MAX;
                        break;
                    }
                    let f = (v.pos - idx as f64) as f32;
                    let d = &v.clip.data;
                    let l = d[idx * 2] * (1.0 - f) + d[idx * 2 + 2] * f;
                    let r = d[idx * 2 + 1] * (1.0 - f) + d[idx * 2 + 3] * f;
                    let mut g = 1.0;
                    if let Some((lvl, sp)) = v.fade.as_mut() {
                        *lvl -= *sp;
                        g = lvl.max(0.0);
                        if *lvl <= 0.0 {
                            v.pos = f64::MAX;
                            break;
                        }
                    }
                    mix[i * 2] += l * v.gain_l * g;
                    mix[i * 2 + 1] += r * v.gain_r * g;
                    v.pos += v.step;
                }
            }
            voices.retain(|v| v.pos != f64::MAX);
            let mut pk = 0.0f32;
            for s in mix.iter_mut() {
                *s = (*s * master).clamp(-1.0, 1.0);
                pk = pk.max(s.abs());
            }
            let prev = f32::from_bits(peak.load(Ordering::Relaxed));
            peak.store((pk.max(prev * 0.9)).to_bits(), Ordering::Relaxed);
            let p = render.GetBuffer(n as u32)?;
            let out = p as *mut f32;
            for i in 0..n {
                for c in 0..out_ch {
                    *out.add(i * out_ch + c) = if cfg.play_local { mix[i * 2 + (c & 1)] } else { 0.0 };
                }
            }
            render.ReleaseBuffer(n as u32, 0)?;
            // VBAN: resample to 48 kHz (if needed) and send 256-frame packets.
            if cfg.vban {
                if let Some(sock) = &udp {
                    let ratio = out_rate / 48000.0;
                    let frames = (n as f32 / ratio) as usize;
                    for k in 0..frames {
                        let src = ((k as f32 * ratio) as usize).min(n - 1);
                        vban_buf.push(mix[src * 2]);
                        vban_buf.push(mix[src * 2 + 1]);
                    }
                    while vban_buf.len() >= 512 {
                        let chunk: Vec<f32> = vban_buf.drain(..512).collect();
                        let pkt = vban_packet(&cfg.vban_stream, vban_frame, &chunk);
                        vban_frame = vban_frame.wrapping_add(1);
                        let _ = sock.send_to(&pkt, (cfg.vban_ip.as_str(), cfg.vban_port));
                    }
                }
            } else {
                vban_buf.clear();
            }
        }
        client.Stop()?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn mixer(_rx: Receiver<Cmd>, _cfg: SoundCfg, _stop: Arc<AtomicBool>, _peak: Arc<AtomicU32>) -> anyhow::Result<()> {
    anyhow::bail!("audio output is Windows-only")
}

// ------------------------------------------------------------------ shared sound settings types

/// A sound file with its own base volume and pitch (VTS sound lists).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct SoundFile {
    pub file: String,
    pub volume: f32,
    pub pitch: f32,
}

impl Default for SoundFile {
    fn default() -> Self {
        SoundFile { file: String::new(), volume: 1.0, pitch: 1.0 }
    }
}

/// Randomized playback settings shared by hotkey sound slots and parameter triggers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct SoundParams {
    pub files: Vec<SoundFile>,
    pub volume: [f32; 2],
    pub pitch: [f32; 2],
    pub pan: [f32; 2],
    pub delay: [f32; 2],
    pub echo_count: u32,
    pub echo_delay: f32,
    pub echo_volume: f32,
}

impl Default for SoundParams {
    fn default() -> Self {
        SoundParams { files: Vec::new(), volume: [1.0, 1.0], pitch: [1.0, 1.0], pan: [0.0, 0.0], delay: [0.0, 0.0], echo_count: 0, echo_delay: 0.25, echo_volume: 0.5 }
    }
}

pub fn rnd() -> f32 {
    use std::cell::Cell;
    thread_local!(static S: Cell<u64> = Cell::new(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1));
    S.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 40) as f32 / (1u64 << 24) as f32
    })
}

fn pick(r: [f32; 2]) -> f32 {
    r[0] + (r[1] - r[0]) * rnd()
}

/// Builds a randomized play request (random file from the list).
pub fn request(p: &SoundParams, resolve: impl Fn(&str) -> Option<PathBuf>, group: &str, extra_volume: f32, extra_pitch: f32, extra_pan: f32) -> Option<Play> {
    if p.files.is_empty() {
        return None;
    }
    let f = &p.files[((rnd() * p.files.len() as f32) as usize).min(p.files.len() - 1)];
    let file = resolve(&f.file)?;
    Some(Play {
        file,
        volume: f.volume * pick(p.volume) * extra_volume,
        pitch: f.pitch * pick(p.pitch) * extra_pitch,
        pan: (pick(p.pan) + extra_pan).clamp(-1.0, 1.0),
        delay: pick(p.delay),
        echo_count: p.echo_count,
        echo_delay: p.echo_delay,
        echo_volume: p.echo_volume,
        group: group.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vban_header() {
        let p = vban_packet("rigrs", 7, &[0.5, -0.5, 0.0, 1.0]);
        assert_eq!(&p[..4], b"VBAN");
        assert_eq!(p[4], 3);
        assert_eq!(p[5], 1); // 2 frames - 1
        assert_eq!(p[6], 1); // stereo
        assert_eq!(&p[8..13], b"rigrs");
        assert_eq!(u32::from_le_bytes([p[24], p[25], p[26], p[27]]), 7);
        assert_eq!(p.len(), 28 + 8);
    }

    #[test]
    fn decodes_wav() {
        // 0.01 s of a 440 Hz mono 16-bit WAV.
        let rate = 8000u32;
        let n = 80usize;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&((36 + n * 2) as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&((n * 2) as u32).to_le_bytes());
        for i in 0..n {
            let v = ((i as f32 * 440.0 * 6.283 / rate as f32).sin() * 16000.0) as i16;
            w.extend_from_slice(&v.to_le_bytes());
        }
        let dir = std::env::temp_dir().join(format!("rigrs_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.wav");
        std::fs::write(&p, &w).unwrap();
        let c = decode(&p).unwrap();
        assert_eq!(c.rate, rate as f32);
        assert_eq!(c.data.len(), n * 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    #[ignore]
    fn vban_receives_mixed_sound() {
        let rx = std::net::UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
        let port = rx.local_addr().unwrap().port();
        // 0.5 s 440 Hz mono WAV at 48 kHz.
        let rate = 48000u32;
        let n = 24000usize;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&((36 + n * 2) as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        for v in [16u32] { w.extend_from_slice(&v.to_le_bytes()); }
        for v in [1u16, 1u16] { w.extend_from_slice(&v.to_le_bytes()); }
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 2).to_le_bytes());
        for v in [2u16, 16u16] { w.extend_from_slice(&v.to_le_bytes()); }
        w.extend_from_slice(b"data");
        w.extend_from_slice(&((n * 2) as u32).to_le_bytes());
        for i in 0..n {
            w.extend_from_slice(&(((i as f32 * 440.0 * 6.283 / rate as f32).sin() * 12000.0) as i16).to_le_bytes());
        }
        let p = std::env::temp_dir().join(format!("rigrs_beep_{}.wav", std::process::id()));
        std::fs::write(&p, &w).unwrap();
        let cfg = SoundCfg { play_local: false, vban: true, vban_ip: "127.0.0.1".into(), vban_port: port, vban_stream: "rigrs".into(), volume: 100.0 };
        let a = Audio::start(&cfg);
        a.play(Play { file: p.clone(), volume: 1.0, pitch: 1.0, pan: 0.0, delay: 0.0, echo_count: 0, echo_delay: 0.2, echo_volume: 0.5, group: "t".into() });
        let mut buf = [0u8; 2048];
        let mut loud = 0;
        for _ in 0..200 {
            let Ok(n) = rx.recv(&mut buf) else { break };
            assert_eq!(&buf[..4], b"VBAN");
            let pcm = &buf[28..n];
            let peak = pcm.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap_or(0);
            if peak > 5000 {
                loud += 1;
            }
        }
        println!("packets with sound: {loud}, error: {:?}", a.error());
        let _ = std::fs::remove_file(&p);
        assert!(loud > 10);
    }
}
