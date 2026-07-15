//! Version comparison: ordering artifacts and matching a declared version window.
//!
//! Deliberately conservative. It compares only plain dotted-numeric versions;
//! anything with a classifier or an embedded prefix is left unanswered rather than
//! risk a false "out of window", because the whole value of the validator is that
//! a flag it raises is real. A caller never acts on a guess about a version string
//! it could not actually read.
//!
//! Lives in the registry (the lower layer) because both the resolver's window
//! check and the registry's own artifact ordering ride on it.

use std::cmp::Ordering;

/// Compare two versions iff both are plain dotted-numeric (`1`, `1.2`,
/// `1.12.2`, an optional leading `v`). Any non-numeric segment (a `-beta`
/// classifier, an embedded MC prefix such as `1.12.2-4.1.0`, a git hash)
/// yields `None`. Missing trailing components read as 0, so `1.2` == `1.2.0`.
pub fn cmp(a: &str, b: &str) -> Option<Ordering> {
    let (pa, pb) = (parse(a)?, parse(b)?);
    let n = pa.len().max(pb.len());
    for i in 0..n {
        match pa
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&pb.get(i).copied().unwrap_or(0))
        {
            Ordering::Equal => continue,
            ord => return Some(ord),
        }
    }
    Some(Ordering::Equal)
}

fn parse(s: &str) -> Option<Vec<u64>> {
    let s = s.trim().strip_prefix(['v', 'V']).unwrap_or(s.trim());
    if s.is_empty() {
        return None;
    }
    s.split('.').map(|seg| seg.parse::<u64>().ok()).collect()
}

enum Op {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
    /// `~x` / `^x` -- treated as a lower bound only. Their upper bound is
    /// skipped so a newer-but-fine version is never falsely flagged.
    Lower,
}

/// `Some(true/false)` when both the version and the window are comparable;
/// `None` when the constraint is absent (`*`, empty) or either side is not
/// plainly comparable. Supports a single Maven interval, the comparator forms
/// (`>=x`, `>x`, `<=x`, `<x`, `=x`), a bare version (Maven's soft `>=`), and
/// `~x`/`^x` as a lower bound.
pub fn in_range(version: &str, range: &str) -> Option<bool> {
    let r = range.trim();
    if r.is_empty() || r == "*" || r.eq_ignore_ascii_case("any") {
        return None;
    }
    if r.starts_with('[') || r.starts_with('(') {
        return interval(version, r);
    }
    let (op, bound) = split_op(r);
    let ord = cmp(version, bound.trim())?;
    Some(match op {
        Op::Ge | Op::Lower => ord != Ordering::Less,
        Op::Gt => ord == Ordering::Greater,
        Op::Le => ord != Ordering::Greater,
        Op::Lt => ord == Ordering::Less,
        Op::Eq => ord == Ordering::Equal,
    })
}

fn split_op(r: &str) -> (Op, &str) {
    for (p, op) in [(">=", Op::Ge), ("<=", Op::Le), ("==", Op::Eq)] {
        if let Some(rest) = r.strip_prefix(p) {
            return (op, rest);
        }
    }
    for (c, op) in [
        ('>', Op::Gt),
        ('<', Op::Lt),
        ('=', Op::Eq),
        ('~', Op::Lower),
        ('^', Op::Lower),
    ] {
        if let Some(rest) = r.strip_prefix(c) {
            return (op, rest);
        }
    }
    (Op::Ge, r) // bare -> Maven's soft lower bound
}

fn interval(version: &str, r: &str) -> Option<bool> {
    let lower_inc = r.starts_with('[');
    let upper_inc = r.ends_with(']');
    if !(r.ends_with(']') || r.ends_with(')')) {
        return None;
    }
    let inner = &r[1..r.len() - 1];
    let mut parts = inner.splitn(2, ',');
    let lo = parts.next()?.trim();
    let Some(hi) = parts.next() else {
        // a bracketed single value "[x]" pins exactly x
        return Some(cmp(version, lo)? == Ordering::Equal);
    };
    let hi = hi.trim();
    if !lo.is_empty() {
        let ok = match cmp(version, lo)? {
            Ordering::Less => false,
            Ordering::Equal => lower_inc,
            Ordering::Greater => true,
        };
        if !ok {
            return Some(false);
        }
    }
    if !hi.is_empty() {
        let ok = match cmp(version, hi)? {
            Ordering::Greater => false,
            Ordering::Equal => upper_inc,
            Ordering::Less => true,
        };
        if !ok {
            return Some(false);
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::in_range;

    #[test]
    fn maven_intervals() {
        assert_eq!(in_range("1.2.0", "[1.0,)"), Some(true));
        assert_eq!(in_range("0.9", "[1.0,)"), Some(false));
        assert_eq!(in_range("2.0", "[1.0,2.0)"), Some(false)); // upper exclusive
        assert_eq!(in_range("2.0", "[1.0,2.0]"), Some(true)); // upper inclusive
        assert_eq!(in_range("1.5", "(,2.0]"), Some(true)); // open lower
        assert_eq!(in_range("1.0", "[1.0]"), Some(true)); // pinned
        assert_eq!(in_range("1.1", "[1.0]"), Some(false));
    }

    #[test]
    fn comparators_and_bare() {
        assert_eq!(in_range("1.2.0", ">=1.0.0"), Some(true));
        assert_eq!(in_range("1.0.0", ">1.0.0"), Some(false));
        assert_eq!(in_range("1.0.0", "<=1.0.0"), Some(true));
        assert_eq!(in_range("1.0.1", "<1.0.0"), Some(false));
        assert_eq!(in_range("1.0.0", "=1.0.0"), Some(true));
        assert_eq!(in_range("1.4", "1.2"), Some(true)); // bare == soft >=
        assert_eq!(in_range("1.1", "1.2"), Some(false));
    }

    #[test]
    fn tilde_caret_are_lower_bound_only() {
        // a newer version under ~/^ is never flagged (upper bound skipped)
        assert_eq!(in_range("9.9.9", "~1.2.0"), Some(true));
        assert_eq!(in_range("9.9.9", "^1.2.0"), Some(true));
        assert_eq!(in_range("1.1.0", "^1.2.0"), Some(false));
    }

    #[test]
    fn incomparable_is_unchecked() {
        // classifier / embedded-prefix / wildcard -> None, never a false flag
        assert_eq!(in_range("1.12.2-4.1.0", "[4.0,)"), None);
        assert_eq!(in_range("4.1.0-beta", "[4.0,)"), None);
        assert_eq!(in_range("1.0", "[abc,)"), None);
        assert_eq!(in_range("1.0", "*"), None);
        assert_eq!(in_range("1.0", ""), None);
        assert_eq!(in_range("rv6", ">=1.0"), None);
    }

    #[test]
    fn shorter_version_pads_with_zero() {
        assert_eq!(in_range("1.2", "[1.2.0,)"), Some(true));
        assert_eq!(in_range("1.2.0", "[1.2,)"), Some(true));
    }
}
