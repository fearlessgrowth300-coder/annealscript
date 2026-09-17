//! Tensor lane, backed by real ONNX Runtime.
//!
//! `models/build_similarity_model.py` hand-authors (not trains) an ONNX
//! graph computing cosine similarity between two int8[32] quantized
//! vectors -- the exact computation Phase 3 first wrote as plain Rust
//! arithmetic. This module runs that graph through an actual ONNX Runtime
//! session in the same process as the deterministic runtime, so `intent`
//! field matching is now genuinely executed via ONNX Runtime rather than
//! hand-written math standing in for it.

use ort::session::Session;
use ort::value::Tensor;
use std::sync::{Mutex, OnceLock};

const MODEL_BYTES: &[u8] = include_bytes!("../models/similarity.onnx");

fn session() -> &'static Mutex<Session> {
    static SESSION: OnceLock<Mutex<Session>> = OnceLock::new();
    SESSION.get_or_init(|| {
        let session = Session::builder()
            .expect("failed to create ONNX Runtime session builder")
            .commit_from_memory(MODEL_BYTES)
            .expect("failed to load the bundled similarity.onnx model");
        Mutex::new(session)
    })
}

pub fn similarity(a: &[i8; 32], b: &[i8; 32]) -> f32 {
    let a_val = Tensor::from_array(([32i64], a.to_vec())).expect("build tensor a");
    let b_val = Tensor::from_array(([32i64], b.to_vec())).expect("build tensor b");
    let mut session = session().lock().expect("session mutex poisoned");
    let outputs = session
        .run(ort::inputs!["a" => a_val, "b" => b_val])
        .expect("onnxruntime inference failed");
    let (_, data) = outputs["similarity"].try_extract_tensor::<f32>().expect("extract output");
    data[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_with(nonzero: &[(usize, i8)]) -> [i8; 32] {
        let mut v = [0i8; 32];
        for (i, val) in nonzero {
            v[*i] = *val;
        }
        v
    }

    #[test]
    fn identical_vectors_score_close_to_one() {
        let v = vec_with(&[(0, 1), (1, 2), (2, 3)]);
        let s = similarity(&v, &v);
        assert!((s - 1.0).abs() < 1e-3, "expected ~1.0, got {s}");
    }

    #[test]
    fn orthogonal_vectors_score_near_zero() {
        let a = vec_with(&[(0, 5)]);
        let b = vec_with(&[(1, 5)]);
        let s = similarity(&a, &b);
        assert!(s.abs() < 1e-3, "expected ~0.0, got {s}");
    }
}
