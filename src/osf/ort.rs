//! Minimal binding to the ONNX Runtime C API (onnxruntime.dll / libonnxruntime.so, MIT),
//! loaded at run time. Only what the face tracker needs: CPU sessions from memory and
//! float tensors in / out.
//!
//! `OrtApi` is a struct of function pointers whose order is part of the stable ABI
//! (entries are only ever appended), so functions are addressed by index. We ask for an
//! old API version, which every onnxruntime since 1.4 provides.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::ptr::null_mut;

use anyhow::{anyhow, bail, Context, Result};

type Status = *mut c_void;

#[repr(C)]
struct ApiBase {
    get_api: unsafe extern "system" fn(u32) -> *const *const c_void,
    get_version: unsafe extern "system" fn() -> *const c_char,
}

// Indices into OrtApi (onnxruntime_c_api.h).
const GET_ERROR_MESSAGE: usize = 2;
const CREATE_ENV: usize = 3;
const CREATE_SESSION_FROM_ARRAY: usize = 8;
const RUN: usize = 9;
const CREATE_SESSION_OPTIONS: usize = 10;
const SET_GRAPH_OPT_LEVEL: usize = 23;
const SET_INTRA_THREADS: usize = 24;
const SET_INTER_THREADS: usize = 25;
const CREATE_TENSOR_WITH_DATA: usize = 49;
const GET_TENSOR_DATA: usize = 51;
const GET_DIMS_COUNT: usize = 61;
const GET_DIMS: usize = 62;
const GET_TYPE_AND_SHAPE: usize = 65;
const CREATE_CPU_MEMORY_INFO: usize = 69;
const RELEASE_STATUS: usize = 93;
const RELEASE_SESSION: usize = 95;
const RELEASE_VALUE: usize = 96;
const RELEASE_TYPE_AND_SHAPE: usize = 99;
const RELEASE_SESSION_OPTIONS: usize = 100;

pub struct Runtime {
    _lib: libloading::Library,
    api: *const *const c_void,
    env: *mut c_void,
    mem: *mut c_void,
    pub version: String,
}

// The API table is immutable and ONNX Runtime objects are thread-safe for our use
// (one session is only ever used from one thread at a time).
unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

impl Runtime {
    unsafe fn f<T: Copy>(&self, idx: usize) -> T {
        std::mem::transmute_copy::<*const c_void, T>(&*self.api.add(idx))
    }

    fn check(&self, st: Status) -> Result<()> {
        if st.is_null() {
            return Ok(());
        }
        unsafe {
            let msg: unsafe extern "system" fn(Status) -> *const c_char = self.f(GET_ERROR_MESSAGE);
            let text = CStr::from_ptr(msg(st)).to_string_lossy().into_owned();
            let rel: unsafe extern "system" fn(Status) = self.f(RELEASE_STATUS);
            rel(st);
            Err(anyhow!("onnxruntime: {text}"))
        }
    }

    pub fn load(path: &Path) -> Result<Runtime> {
        unsafe {
            let lib = libloading::Library::new(path).with_context(|| format!("cannot load {}", path.display()))?;
            let base: libloading::Symbol<unsafe extern "system" fn() -> *const ApiBase> = lib.get(b"OrtGetApiBase\0").context("OrtGetApiBase not found")?;
            let base = base();
            if base.is_null() {
                bail!("OrtGetApiBase returned null");
            }
            let version = CStr::from_ptr(((*base).get_version)()).to_string_lossy().into_owned();
            let api = [8u32, 7, 6, 5, 4].iter().map(|v| ((*base).get_api)(*v)).find(|p| !p.is_null()).ok_or_else(|| anyhow!("onnxruntime {version} is too old"))?;
            let mut rt = Runtime { _lib: lib, api, env: null_mut(), mem: null_mut(), version };
            let create_env: unsafe extern "system" fn(i32, *const c_char, *mut *mut c_void) -> Status = rt.f(CREATE_ENV);
            let name = CString::new("rigrs").unwrap();
            // Logging level 3 = errors only.
            let st = create_env(3, name.as_ptr(), &mut rt.env);
            rt.check(st)?;
            let mem_info: unsafe extern "system" fn(i32, i32, *mut *mut c_void) -> Status = rt.f(CREATE_CPU_MEMORY_INFO);
            // OrtArenaAllocator = 1, OrtMemTypeDefault = 0.
            let st = mem_info(1, 0, &mut rt.mem);
            rt.check(st)?;
            Ok(rt)
        }
    }

    /// Creates a CPU session from model bytes. `outputs` are the output names to fetch.
    pub fn session(&'static self, model: &[u8], threads: u32, input: &str, outputs: &[&str]) -> Result<Session> {
        unsafe {
            let mut opts = null_mut();
            let create_opts: unsafe extern "system" fn(*mut *mut c_void) -> Status = self.f(CREATE_SESSION_OPTIONS);
            self.check(create_opts(&mut opts))?;
            let set_intra: unsafe extern "system" fn(*mut c_void, i32) -> Status = self.f(SET_INTRA_THREADS);
            let set_inter: unsafe extern "system" fn(*mut c_void, i32) -> Status = self.f(SET_INTER_THREADS);
            let set_opt: unsafe extern "system" fn(*mut c_void, i32) -> Status = self.f(SET_GRAPH_OPT_LEVEL);
            self.check(set_intra(opts, threads.clamp(1, 8) as i32))?;
            self.check(set_inter(opts, 1))?;
            // ORT_ENABLE_ALL = 99.
            self.check(set_opt(opts, 99))?;
            let mut sess = null_mut();
            let create: unsafe extern "system" fn(*mut c_void, *const c_void, usize, *const c_void, *mut *mut c_void) -> Status = self.f(CREATE_SESSION_FROM_ARRAY);
            let st = create(self.env, model.as_ptr() as *const c_void, model.len(), opts, &mut sess);
            let rel_opts: unsafe extern "system" fn(*mut c_void) = self.f(RELEASE_SESSION_OPTIONS);
            rel_opts(opts);
            self.check(st)?;
            Ok(Session { rt: self, sess, input: CString::new(input)?, outputs: outputs.iter().map(|o| CString::new(*o).unwrap()).collect() })
        }
    }
}

pub struct Session {
    rt: &'static Runtime,
    sess: *mut c_void,
    input: CString,
    outputs: Vec<CString>,
}

unsafe impl Send for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let rel: unsafe extern "system" fn(*mut c_void) = self.rt.f(RELEASE_SESSION);
            rel(self.sess);
        }
    }
}

pub struct Output {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

impl Session {
    /// Runs the model on one float tensor and copies every requested output.
    pub fn run(&self, data: &mut [f32], shape: &[i64]) -> Result<Vec<Output>> {
        let rt = self.rt;
        let n: i64 = shape.iter().product();
        if n as usize != data.len() {
            bail!("tensor shape {:?} does not match {} values", shape, data.len());
        }
        unsafe {
            let mut input = null_mut();
            let create: unsafe extern "system" fn(*const c_void, *mut c_void, usize, *const i64, usize, i32, *mut *mut c_void) -> Status = rt.f(CREATE_TENSOR_WITH_DATA);
            // ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT = 1.
            rt.check(create(rt.mem, data.as_mut_ptr() as *mut c_void, std::mem::size_of_val(data), shape.as_ptr(), shape.len(), 1, &mut input))?;
            let rel_value: unsafe extern "system" fn(*mut c_void) = rt.f(RELEASE_VALUE);
            let in_names = [self.input.as_ptr()];
            let out_names: Vec<*const c_char> = self.outputs.iter().map(|o| o.as_ptr()).collect();
            let mut outs: Vec<*mut c_void> = vec![null_mut(); out_names.len()];
            let run: unsafe extern "system" fn(*mut c_void, *const c_void, *const *const c_char, *const *const c_void, usize, *const *const c_char, usize, *mut *mut c_void) -> Status = rt.f(RUN);
            let inputs = [input as *const c_void];
            let st = run(self.sess, std::ptr::null(), in_names.as_ptr(), inputs.as_ptr(), 1, out_names.as_ptr(), out_names.len(), outs.as_mut_ptr());
            rel_value(input);
            rt.check(st)?;
            let mut result = Vec::with_capacity(outs.len());
            let mut err = None;
            for &o in &outs {
                if err.is_none() {
                    match self.read(o) {
                        Ok(v) => result.push(v),
                        Err(e) => err = Some(e),
                    }
                }
                rel_value(o);
            }
            match err {
                Some(e) => Err(e),
                None => Ok(result),
            }
        }
    }

    unsafe fn read(&self, value: *mut c_void) -> Result<Output> {
        let rt = self.rt;
        let mut info = null_mut();
        let get_info: unsafe extern "system" fn(*const c_void, *mut *mut c_void) -> Status = rt.f(GET_TYPE_AND_SHAPE);
        rt.check(get_info(value, &mut info))?;
        let mut rank = 0usize;
        let count: unsafe extern "system" fn(*const c_void, *mut usize) -> Status = rt.f(GET_DIMS_COUNT);
        let dims_fn: unsafe extern "system" fn(*const c_void, *mut i64, usize) -> Status = rt.f(GET_DIMS);
        let rel_info: unsafe extern "system" fn(*mut c_void) = rt.f(RELEASE_TYPE_AND_SHAPE);
        let st = count(info, &mut rank);
        let mut dims = vec![0i64; rank];
        let st2 = if st.is_null() { dims_fn(info, dims.as_mut_ptr(), rank) } else { null_mut() };
        rel_info(info);
        rt.check(st)?;
        rt.check(st2)?;
        let shape: Vec<usize> = dims.iter().map(|d| (*d).max(0) as usize).collect();
        let len: usize = shape.iter().product();
        let mut ptr: *mut c_void = null_mut();
        let get_data: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Status = rt.f(GET_TENSOR_DATA);
        rt.check(get_data(value, &mut ptr))?;
        if ptr.is_null() {
            bail!("empty output tensor");
        }
        Ok(Output { shape, data: std::slice::from_raw_parts(ptr as *const f32, len).to_vec() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(p: &str) -> Vec<f32> {
        std::fs::read(p).unwrap().chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    /// Against reference outputs produced with the Python onnxruntime:
    /// `RIGRS_ORT=libonnxruntime.so RIGRS_OSF_MODELS=dir RIGRS_OSF_REF=dir cargo test ort_matches -- --ignored`
    #[test]
    #[ignore]
    fn ort_matches_reference() {
        let rt: &'static Runtime = Box::leak(Box::new(Runtime::load(Path::new(&std::env::var("RIGRS_ORT").unwrap())).unwrap()));
        println!("onnxruntime {}", rt.version);
        let (m, r) = (std::env::var("RIGRS_OSF_MODELS").unwrap(), std::env::var("RIGRS_OSF_REF").unwrap());
        for (name, shape, outs) in [("lm_model3_opt", [1i64, 3, 224, 224], vec!["output"]), ("mnv3_detection_opt", [1, 3, 224, 224], vec!["output", "maxpool"]), ("mnv3_gaze32_split_opt", [2, 3, 32, 32], vec!["output", "blink"])] {
            let s = rt.session(&std::fs::read(format!("{m}/{name}.onnx")).unwrap(), 1, "input", &outs).unwrap();
            let mut x = rd(&format!("{r}/{name}.in"));
            let out = s.run(&mut x, &shape).unwrap();
            for (k, o) in out.iter().enumerate() {
                let e = rd(&format!("{r}/{name}.out{k}"));
                assert_eq!(o.data.len(), e.len());
                let d = o.data.iter().zip(&e).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                assert!(d < 1e-4, "{name} output {k}: max diff {d}");
            }
            let t = std::time::Instant::now();
            for _ in 0..20 {
                s.run(&mut x, &shape).unwrap();
            }
            println!("{name}: {:?} {:.2} ms/run", out[0].shape, t.elapsed().as_secs_f64() / 20.0 * 1000.0);
        }
    }
}
