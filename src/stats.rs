//! Process CPU / memory sampling (Windows), for the performance overlay.

use std::time::Instant;

#[derive(Default, Clone, Copy)]
pub struct ProcSample {
    pub cpu_pct: f32,
    pub working_set_mb: f32,
    pub private_mb: f32,
}

pub struct ProcMeter {
    last: Option<(Instant, u64)>,
    pub sample: ProcSample,
    cores: f32,
}

impl Default for ProcMeter {
    fn default() -> Self {
        ProcMeter {
            last: None,
            sample: ProcSample::default(),
            cores: std::thread::available_parallelism().map(|n| n.get() as f32).unwrap_or(1.0),
        }
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{FILETIME, HANDLE};
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    pub fn current() -> isize {
        unsafe { GetCurrentProcess().0 as isize }
    }

    /// (cpu time in 100ns units, working set bytes, private bytes)
    pub fn query(h: isize) -> Option<(u64, u64, u64)> {
        let h = HANDLE(h as *mut core::ffi::c_void);
        let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe {
            GetProcessTimes(h, &mut a, &mut b, &mut k, &mut u).ok()?;
            let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
            let mut pmc = PROCESS_MEMORY_COUNTERS_EX::default();
            let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            let ok = K32GetProcessMemoryInfo(h, &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, cb).as_bool();
            let (ws, pv) = if ok { (pmc.WorkingSetSize as u64, pmc.PrivateUsage as u64) } else { (0, 0) };
            Some((ft(k) + ft(u), ws, pv))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn current() -> isize {
        0
    }
    pub fn query(_: isize) -> Option<(u64, u64, u64)> {
        None
    }
}

pub fn current_process() -> isize {
    imp::current()
}

impl ProcMeter {
    pub fn update(&mut self, handle: isize) {
        let Some((cpu, ws, pv)) = imp::query(handle) else { return };
        let now = Instant::now();
        if let Some((t, c)) = self.last {
            let wall = now.duration_since(t).as_secs_f64();
            if wall > 0.0 {
                let used = (cpu.saturating_sub(c)) as f64 / 1e7;
                self.sample.cpu_pct = (used / wall * 100.0 / self.cores as f64) as f32;
            }
        }
        self.last = Some((now, cpu));
        self.sample.working_set_mb = ws as f32 / 1048576.0;
        self.sample.private_mb = pv as f32 / 1048576.0;
    }
}

/// Rolling frame statistics.
pub struct FrameStats {
    pub fps: f32,
    pub cpu_ms: f32,
    pub draw_calls: u32,
    pub masks: usize,
    frames: u32,
    cpu_acc: f32,
    mark: Instant,
    pub app: ProcMeter,
    pub tracker: ProcMeter,
}

impl Default for FrameStats {
    fn default() -> Self {
        FrameStats {
            fps: 0.0,
            cpu_ms: 0.0,
            draw_calls: 0,
            masks: 0,
            frames: 0,
            cpu_acc: 0.0,
            mark: Instant::now(),
            app: ProcMeter::default(),
            tracker: ProcMeter::default(),
        }
    }
}

impl FrameStats {
    /// Returns true once per second when the counters were refreshed.
    pub fn frame(&mut self, cpu_ms: f32, tracker: Option<isize>) -> bool {
        self.frames += 1;
        self.cpu_acc += cpu_ms;
        let el = self.mark.elapsed().as_secs_f32();
        if el < 1.0 {
            return false;
        }
        self.fps = self.frames as f32 / el;
        self.cpu_ms = self.cpu_acc / self.frames as f32;
        self.frames = 0;
        self.cpu_acc = 0.0;
        self.mark = Instant::now();
        self.app.update(current_process());
        match tracker {
            Some(h) => self.tracker.update(h),
            None => self.tracker = ProcMeter::default(),
        }
        true
    }
}
