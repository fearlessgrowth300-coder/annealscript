//! Stages runtime DLLs next to the built executable.
//!
//! `ort`'s `copy-dylibs` feature already does this for the ONNX Runtime
//! execution providers (e.g. DirectML.dll) -- ONNX Runtime itself links
//! statically, so there's no onnxruntime.dll to copy at all. What it does
//! NOT do is z3: the `gh-release` feature downloads a real libz3.dll but
//! leaves it sitting in z3-sys's own build-script OUT_DIR, so the compiled
//! exe fails to load with STATUS_DLL_NOT_FOUND the moment it's run from
//! outside `cargo run` (confirmed via `dumpbin /DEPENDENTS`). This copies
//! every .dll produced under any `z3-sys-*` build directory next to the
//! final binary, so `target/<profile>/annealscript-compiler.exe` is
//! actually standalone.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // OUT_DIR = target/<profile>/build/annealscript-compiler-<hash>/out
    let Some(build_root) = out_dir.ancestors().nth(2) else { return };
    let Some(target_dir) = out_dir.ancestors().nth(3) else { return };

    let Ok(entries) = fs::read_dir(build_root) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("z3-sys-") {
            continue;
        }
        copy_dlls_recursive(&entry.path().join("out"), target_dir, 0);
    }
}

fn copy_dlls_recursive(dir: &Path, target_dir: &Path, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            copy_dlls_recursive(&path, target_dir, depth + 1);
        } else if path.extension().is_some_and(|e| e == "dll") {
            let dest = target_dir.join(path.file_name().unwrap());
            if !dest.exists() {
                let _ = fs::copy(&path, &dest);
            }
        }
    }
}
