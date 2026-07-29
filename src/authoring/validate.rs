//! The validate pass: cross-reference a `PackConfig` against an instance
//! archive by mod filename, so a curator can confirm the declared set matches
//! what a server's FML handshake expects.
//!
//! An instance archive is any zip with a top-level `mods/` -- an export from a
//! launcher, or a plain instance directory. Nothing here knows or cares which
//! one made it.

use super::archive::extract_mods;
use crate::domain::PackConfig;
use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;
use ts_rs::TS;

/// Result of cross-referencing a `PackConfig` against an instance archive by
/// mod filename. `missing_in_config` (in the archive but not declared)
/// would break the FML handshake; `extra_in_config` (declared but not in
/// the archive) is expected when the curator adds mods on top.
#[derive(Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ValidateReport {
    #[ts(type = "number")]
    pub archive_mod_count: usize,
    #[ts(type = "number")]
    pub declared_mods: usize,
    #[ts(type = "number")]
    pub declared_assets: usize,
    #[ts(type = "number")]
    pub matched: usize,
    pub missing_in_config: Vec<String>,
    pub extra_in_config: Vec<String>,
}

/// Cross-reference a `PackConfig` against an instance archive's `mods/*.jar` set
/// by filename. Pure: returns the report, leaves printing / failing to the
/// caller.
pub fn validate(cfg: &PackConfig, archive_bytes: &[u8]) -> Result<ValidateReport> {
    let archive_mods = extract_mods(archive_bytes)?;

    let archive_filenames: HashSet<&str> =
        archive_mods.iter().map(|m| m.filename.as_str()).collect();
    let config_filenames: HashSet<&str> = cfg.mods.iter().map(|m| m.filename.as_str()).collect();

    let mut missing_in_config: Vec<String> = archive_filenames
        .difference(&config_filenames)
        .map(|s| s.to_string())
        .collect();
    let mut extra_in_config: Vec<String> = config_filenames
        .difference(&archive_filenames)
        .map(|s| s.to_string())
        .collect();
    let matched = archive_filenames.intersection(&config_filenames).count();
    missing_in_config.sort();
    extra_in_config.sort();

    Ok(ValidateReport {
        archive_mod_count: archive_mods.len(),
        declared_mods: cfg.mods.len(),
        declared_assets: cfg.assets.len(),
        matched,
        missing_in_config,
        extra_in_config,
    })
}
