//! The validate pass: cross-reference a `PackConfig` against an SC archive
//! by mod filename, so a curator can confirm the declared set matches what
//! the SC server's FML handshake expects.

use super::archive::extract_mods;
use crate::domain::PackConfig;
use anyhow::Result;
use std::collections::HashSet;

/// Result of cross-referencing a `PackConfig` against an SC archive by
/// mod filename. `missing_in_config` (in the archive but not declared)
/// would break the FML handshake; `extra_in_config` (declared but not in
/// the archive) is expected when the curator adds mods on top.
pub struct ValidateReport {
    pub sc_mod_count: usize,
    pub declared_mods: usize,
    pub declared_assets: usize,
    pub matched: usize,
    pub missing_in_config: Vec<String>,
    pub extra_in_config: Vec<String>,
}

/// Cross-reference a `PackConfig` against an SC archive's `mods/*.jar` set
/// by filename. Pure: returns the report, leaves printing / failing to the
/// caller.
pub fn validate(cfg: &PackConfig, sc_archive_bytes: &[u8]) -> Result<ValidateReport> {
    let sc_mods = extract_mods(sc_archive_bytes)?;

    let sc_filenames: HashSet<&str> = sc_mods.iter().map(|m| m.filename.as_str()).collect();
    let config_filenames: HashSet<&str> = cfg.mods.iter().map(|m| m.filename.as_str()).collect();

    let mut missing_in_config: Vec<String> = sc_filenames
        .difference(&config_filenames)
        .map(|s| s.to_string())
        .collect();
    let mut extra_in_config: Vec<String> = config_filenames
        .difference(&sc_filenames)
        .map(|s| s.to_string())
        .collect();
    let matched = sc_filenames.intersection(&config_filenames).count();
    missing_in_config.sort();
    extra_in_config.sort();

    Ok(ValidateReport {
        sc_mod_count: sc_mods.len(),
        declared_mods: cfg.mods.len(),
        declared_assets: cfg.assets.len(),
        matched,
        missing_in_config,
        extra_in_config,
    })
}
