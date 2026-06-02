//! Pack version comparison. Pure rules shared by clients and the mirror;
//! plain `String` sort would order `.10` before `.2` and break update
//! detection, so both sides must use this comparator.

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
}
