// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Script de compilación para incrustar el icono de aplicación en `mule-daemon.exe`
//! cuando el destino de compilación es Windows (entorno MSVC con `rc.exe`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app_icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }

    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        return;
    };

    let ico_path = PathBuf::from(&manifest_dir).join("assets/app_icon.ico");
    if !ico_path.is_file() {
        return;
    }

    let Some(rc_exe) = find_rc_exe() else {
        return;
    };

    let out_path = PathBuf::from(out_dir);
    let rc_file = out_path.join("app_icon.rc");
    let res_file = out_path.join("app_icon.res");

    let ico_escaped = ico_path.to_string_lossy().replace('\\', "/");
    let rc_content = format!("1 ICON \"{ico_escaped}\"\n");
    if std::fs::write(&rc_file, rc_content).is_err() {
        return;
    }

    let status = Command::new(rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_file)
        .arg(&rc_file)
        .status();

    if matches!(status, Ok(s) if s.success()) && res_file.is_file() {
        println!("cargo:rustc-link-arg-bin=mule-daemon={}", res_file.display());
    }
}

fn find_rc_exe() -> Option<PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let arch_dir = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };

    let kits_bin = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let entries = std::fs::read_dir(kits_bin).ok()?;
    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();
    versions.reverse();

    for ver_dir in versions {
        let candidate = ver_dir.join(arch_dir).join("rc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
