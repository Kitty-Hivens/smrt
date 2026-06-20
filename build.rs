//! Stamp the build with the current git commit so the running mirror reports a
//! version that actually moves when the code does, instead of a frozen
//! `Cargo.toml` number. `SMRT_BUILD_VERSION` = `<crate version>+<short sha>`
//! (or `+unknown` when git is unavailable), surfaced via /v1/health.

use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rustc-env=SMRT_BUILD_VERSION={version}+{sha}");

    // re-run when HEAD moves so the embedded sha follows the checked-out commit
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
}
