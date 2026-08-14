//! Stamp the build with a calendar version taken from git -- the year of the
//! HEAD commit plus the commit height (`2026.388`) -- so the running mirror
//! reports a version that actually moves when the code does, instead of a
//! frozen `Cargo.toml` number. `SMRT_BUILD_VERSION` is what /v1/health, the
//! panel footer and `smrt-pack --version` show.
//!
//! Without git the answer is `unknown`, and a shallow clone counts as "without
//! git": its commit height is a small number that looks like a real version,
//! so stamping it would be worse than admitting the build doesn't know.

use std::process::Command;

fn main() {
    // re-run when HEAD moves so the embedded version follows the checked-out commit
    println!("cargo::rerun-if-changed=.git/HEAD");
    println!("cargo::rerun-if-changed=.git/refs");

    let version = calendar_version().unwrap_or_else(|| "unknown".to_string());
    println!("cargo::rustc-env=SMRT_BUILD_VERSION={version}");
}

fn calendar_version() -> Option<String> {
    if git(&["rev-parse", "--is-shallow-repository"])? == "true" {
        println!(
            "cargo::warning=shallow clone: commit height is not the real one, version stamped as `unknown`"
        );
        return None;
    }

    // The commit's own recorded offset, not the builder's clock -- two machines
    // building the same commit across a new year still agree on the year.
    let year = git(&["log", "-1", "--format=%cI"])?.get(..4)?.to_string();
    let height = git(&["rev-list", "--count", "HEAD"])?;
    Some(format!("{year}.{height}"))
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}
