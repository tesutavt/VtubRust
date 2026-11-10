//! Built-in face tracker: a Rust port of OpenSeeFace (BSD-2-Clause, © 2019 Emiliana,
//! https://github.com/emilianavt/OpenSeeFace). Uses the original ONNX models and
//! ONNX Runtime; no Python, no separate tracker process.

pub mod ort;
pub mod face3d;
pub mod math;
pub mod tracker;
