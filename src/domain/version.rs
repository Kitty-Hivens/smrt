//! Pack version comparison and channel classification. Pure rules shared by
//! clients and the mirror; plain `String` sort would order `.10` before `.2`
//! and break update detection, so both sides must use this comparator.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Release channel of a pack build -- the Modrinth `version_type` vocabulary,
/// shared with mod releases in the registry so the whole mirror speaks one
/// dialect. Stored on the manifest as its own field (`channel`); the version
/// string carries no channel semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export, export_to = "bindings/")]
#[serde(rename_all = "snake_case")]
pub enum VersionChannel {
    Release,
    Beta,
    Alpha,
}

impl VersionChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            VersionChannel::Release => "release",
            VersionChannel::Beta => "beta",
            VersionChannel::Alpha => "alpha",
        }
    }
    /// Inverse of [`as_str`]; `None` for an unrecognised value.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "release" => VersionChannel::Release,
            "beta" => VersionChannel::Beta,
            "alpha" => VersionChannel::Alpha,
            _ => return None,
        })
    }
}

/// Channel of a manifest built before the stored `channel` field existed,
/// recovered from the legacy string forms: the panel used to stamp
/// work-in-progress builds `SNAPSHOT-<semver>-<date>[.N]` (a beta by today's
/// vocabulary); everything else was an operator-published release.
pub fn legacy_version_channel(version: &str) -> VersionChannel {
    if version.starts_with("SNAPSHOT-") {
        VersionChannel::Beta
    } else {
        VersionChannel::Release
    }
}

/// Numeric-tuple representation of a `YYYY.MM.DD[.N]` style version string.
/// Splits on `.` and parses each segment as `u64`; non-numeric segments
/// degrade to 0 so a malformed version still produces a comparable value
/// rather than panicking.
pub fn pack_version_tuple(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|seg| seg.parse::<u64>().unwrap_or(0))
        .collect()
}

/// Compare two pack versions per the spec rules: numeric tuple comparison
/// with missing trailing segments treated as `0`. So `2026.05.22` equals
/// `2026.05.22.0` and is strictly less than `2026.05.22.1`, and
/// `2026.05.22.10` sorts after `2026.05.22.2`.
pub fn compare_pack_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let mut at = pack_version_tuple(a);
    let mut bt = pack_version_tuple(b);
    let n = at.len().max(bt.len());
    at.resize(n, 0);
    bt.resize(n, 0);
    at.cmp(&bt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn compare_orders_two_digit_subversions_after_single_digit() {
        assert_eq!(
            compare_pack_versions("2026.05.22.2", "2026.05.22.10"),
            Ordering::Less
        );
    }

    #[test]
    fn compare_orders_dates_correctly() {
        assert_eq!(
            compare_pack_versions("2026.05.22", "2026.05.23"),
            Ordering::Less
        );
    }

    #[test]
    fn compare_treats_missing_trailing_segment_as_zero() {
        assert_eq!(
            compare_pack_versions("2026.05.22", "2026.05.22.0"),
            Ordering::Equal
        );
        assert_eq!(
            compare_pack_versions("2026.05.22", "2026.05.22.1"),
            Ordering::Less
        );
        assert_eq!(
            compare_pack_versions("2026.05.22.0.0", "2026.05.22"),
            Ordering::Equal
        );
    }

    #[test]
    fn legacy_channel_maps_snapshot_prefix_to_beta() {
        assert_eq!(
            legacy_version_channel("SNAPSHOT-0.0.0-2026.07.18.7"),
            VersionChannel::Beta
        );
        assert_eq!(
            legacy_version_channel("2026.05.22.2"),
            VersionChannel::Release
        );
        // Only the exact uppercase marker counts; a version that merely
        // contains the word is a release.
        assert_eq!(
            legacy_version_channel("1.0-snapshot"),
            VersionChannel::Release
        );
    }

    #[test]
    fn channel_round_trips_the_wire_vocabulary() {
        for c in [
            VersionChannel::Release,
            VersionChannel::Beta,
            VersionChannel::Alpha,
        ] {
            assert_eq!(VersionChannel::parse(c.as_str()), Some(c));
        }
        assert_eq!(VersionChannel::parse("snapshot"), None);
    }
}
