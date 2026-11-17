use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");

    let rc_path = PathBuf::from(&manifest_dir).join("app.rc");
    let manifest_path = PathBuf::from(&manifest_dir).join("open-mouse.exe.manifest");

    println!("cargo:rerun-if-changed={}", rc_path.display());
    println!("cargo:rerun-if-changed={}", manifest_path.display());

    let out_o = PathBuf::from(&out_dir).join("app.o");

    // 1. Try windres (MinGW / GNU toolchain)
    let windres_status = Command::new("windres")
        .args([
            "-i",
            rc_path.to_str().unwrap(),
            "-O",
            "coff",
            "-o",
            out_o.to_str().unwrap(),
        ])
        .status();

    if let Ok(status) = windres_status {
        if status.success() {
            println!("cargo:rustc-link-arg={}", out_o.display());
            return;
        }
    }

    // 2. Try rc.exe (MSVC toolchain fallback)
    let out_res = PathBuf::from(&out_dir).join("app.res");
    let rc_status = Command::new("rc.exe")
        .args([
            "/fo",
            out_res.to_str().unwrap(),
            rc_path.to_str().unwrap(),
        ])
        .status();

    if let Ok(status) = rc_status {
        if status.success() {
            println!("cargo:rustc-link-arg={}", out_res.display());
            return;
        }
    }

    println!("cargo:warning=Neither windres nor rc.exe succeeded; application manifest not embedded.");
}
