//! Builds kai's init (crates/guest, the program `kai-init`) and the tests'
//! probe (its example `probe`) for x86_64-unknown-linux-musl, statically
//! linked, so that kai carries its init inside and needs no second build
//! step. `rust-toolchain.toml` names the target; rustup installs it.
//!
//! The build is a cargo of its own in a target directory of its own under
//! `OUT_DIR`, offline and locked, with the outer build's flags and wrappers
//! taken away. When it fails, kai is still built: `vm::INIT` is then empty,
//! `vm::INIT_ERROR` says why, and `kai vm doctor` reports it.

#![allow(
    clippy::print_stdout,
    reason = "a build script talks to cargo on standard output"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "x86_64-unknown-linux-musl";

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let root = crate_dir.join("../..");
    for path in [
        "crates/guest/src",
        "crates/guest/examples",
        "crates/guest/Cargo.toml",
        "Cargo.lock",
    ] {
        println!("cargo::rerun-if-changed={}", root.join(path).display());
    }

    let error = match build(&root, &out) {
        Ok(()) => String::new(),
        Err(err) => {
            println!("cargo::warning=kai's guest init was not built: {err}");
            for name in ["kai-init", "probe"] {
                std::fs::write(out.join(name), b"").expect("OUT_DIR is writable");
            }
            err
        }
    };
    println!(
        "cargo::rustc-env=KAI_INIT_ERROR={}",
        error.replace('\n', " ")
    );
}

fn build(root: &Path, out: &Path) -> Result<(), String> {
    let target_dir = out.join("guest");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args([
            "build",
            "--release",
            "--locked",
            "--offline",
            "--package",
            "guest",
        ])
        .args([
            "--bin",
            "kai-init",
            "--example",
            "probe",
            "--target",
            TARGET,
        ])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir);
    for name in [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET",
    ] {
        command.env_remove(name);
    }
    let output = command
        .output()
        .map_err(|err| format!("cannot run cargo: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr.lines().rev().find(|line| !line.trim().is_empty());
        return Err(format!(
            "cargo build for {TARGET}: {}",
            last.unwrap_or("failed")
        ));
    }
    let release = target_dir.join(TARGET).join("release");
    for (from, to) in [("kai-init", "kai-init"), ("examples/probe", "probe")] {
        std::fs::copy(release.join(from), out.join(to))
            .map_err(|err| format!("copy {from}: {err}"))?;
    }
    Ok(())
}
