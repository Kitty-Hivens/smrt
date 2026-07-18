//! The one decision layer for a mod's side / match-policy classification.
//! Every consumer (resolve, build, panel) reads through [`classify_artifact`];
//! nothing else interprets `mods.client_env` / `jar_class` directly, so the
//! source priority lives in exactly one place.
//!
//! Priority (fixed by the rework plan): a jar that is not a mod at all
//! (coremod/library kind) short-circuits to the coremod branch; otherwise the
//! Modrinth project environment flags decide when the mod carries a Modrinth
//! identity with usable flags; otherwise the bytecode-derived verdict applies.
//! Both raw verdicts stay visible on the result so the resolve report can
//! surface a Modrinth-vs-bytecode disagreement instead of silently trusting
//! the priority.

use super::model::JarClassRow;
use super::queries;
use crate::domain::{MatchPolicy, SideClass, side_from_modrinth_env};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// Where the winning verdict came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// The jar is not a mod (coremod / bare library): the kind alone decides.
    JarKind,
    /// The Modrinth project's declared environment flags.
    ModrinthEnv,
    /// The bytecode classifier.
    Bytecode,
    /// Nothing decided anything: report unclassified, treat as optional.
    Unclassified,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::JarKind => "jar-kind",
            Provenance::ModrinthEnv => "modrinth-env",
            Provenance::Bytecode => "bytecode",
            Provenance::Unclassified => "unclassified",
        }
    }
}

/// The classification of one artifact of one mod.
#[derive(Debug, Clone)]
pub struct Classification {
    /// The winning verdict. `None` axes mean unclassified.
    pub side: Option<SideClass>,
    pub policy: Option<MatchPolicy>,
    /// The scanned jar's kind (`mod` / `coremod` / `library`); `None` when the
    /// jar was never scanned (a Modrinth-only artifact), which reads as a mod.
    pub kind: Option<String>,
    pub provenance: Provenance,
    /// The bytecode verdict, kept beside the winner so a Modrinth-vs-bytecode
    /// side disagreement can be reported rather than silently overridden.
    pub bytecode_side: Option<SideClass>,
    pub bytecode_policy: Option<MatchPolicy>,
    /// Confidence of the winning side verdict: Modrinth flags and explicit
    /// bytecode markers are `high`; the blanket client-surface heuristic is
    /// `low`. `None` when there is no side.
    pub side_confidence: Option<String>,
}

impl Classification {
    /// True when the jar is not a mod at all (the coremod presence branch).
    pub fn is_non_mod(&self) -> bool {
        matches!(self.kind.as_deref(), Some("coremod") | Some("library"))
    }

    /// A Modrinth-vs-bytecode side disagreement worth surfacing: both sources
    /// decided a side and they differ. `(winner, loser)` = (modrinth, bytecode)
    /// since the flags outrank the derivation by design.
    pub fn side_disagreement(&self) -> Option<(SideClass, SideClass)> {
        match (self.provenance, self.side, self.bytecode_side) {
            (Provenance::ModrinthEnv, Some(win), Some(bc)) if win != bc => Some((win, bc)),
            _ => None,
        }
    }

    /// A client verdict soft enough for a declared hard edge to outweigh: the
    /// blanket-surface heuristic only. Explicit markers and Modrinth flags are
    /// never overridden.
    pub fn client_verdict_is_soft(&self) -> bool {
        self.side == Some(SideClass::Client)
            && self.provenance == Provenance::Bytecode
            && self.side_confidence.as_deref() == Some("low")
    }
}

/// Classify one artifact: `mod_id` for the identity-level Modrinth flags,
/// `sha1` (when the pack declares a concrete jar) for the per-jar bytecode
/// verdict. Logs the decision with its provenance.
pub fn classify_artifact(
    conn: &Connection,
    mod_id: Option<i64>,
    sha1: Option<&str>,
) -> Result<Classification> {
    let jar: Option<JarClassRow> = match sha1 {
        Some(s) => queries::jar_class_for_sha1(conn, s)?,
        None => None,
    };
    let bytecode_side = jar
        .as_ref()
        .and_then(|j| j.side.as_deref())
        .and_then(SideClass::parse);
    let bytecode_confidence = jar.as_ref().and_then(|j| j.side_confidence.clone());
    let bytecode_policy = jar
        .as_ref()
        .and_then(|j| j.match_policy.as_deref())
        .and_then(MatchPolicy::parse);
    let kind = jar.as_ref().map(|j| j.kind.clone());

    let env: Option<(Option<String>, Option<String>)> = match mod_id {
        Some(id) => conn
            .query_row(
                "SELECT client_env, server_env FROM mods WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?,
        None => None,
    };
    let modrinth = env.and_then(|(c, s)| side_from_modrinth_env(&c?, &s?));

    let out = if matches!(kind.as_deref(), Some("coremod") | Some("library")) {
        Classification {
            side: None,
            policy: None,
            kind,
            provenance: Provenance::JarKind,
            bytecode_side,
            bytecode_policy,
            side_confidence: None,
        }
    } else if let Some((side, policy)) = modrinth {
        Classification {
            side: Some(side),
            policy: Some(policy),
            kind,
            provenance: Provenance::ModrinthEnv,
            bytecode_side,
            bytecode_policy,
            side_confidence: Some("high".to_string()),
        }
    } else if bytecode_side.is_some() || bytecode_policy.is_some() {
        Classification {
            side: bytecode_side,
            policy: bytecode_policy,
            kind,
            provenance: Provenance::Bytecode,
            bytecode_side,
            bytecode_policy,
            side_confidence: bytecode_confidence,
        }
    } else {
        Classification {
            side: None,
            policy: None,
            kind,
            provenance: Provenance::Unclassified,
            bytecode_side,
            bytecode_policy,
            side_confidence: None,
        }
    };
    tracing::debug!(
        mod_id,
        sha1,
        provenance = out.provenance.as_str(),
        side = out.side.map(|s| s.as_str()),
        policy = out.policy.map(|p| p.as_str()),
        kind = out.kind.as_deref(),
        "classified artifact"
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::{Registry, upsert};
    use super::*;

    const NOW: &str = "2026-07-18T00:00:00Z";

    fn setup(
        env: Option<(&str, &str)>,
        jar: Option<(&str, Option<&str>, Option<&str>)>,
    ) -> (Registry, i64, String) {
        let r = Registry::open_in_memory().unwrap();
        let sha = "s".repeat(40);
        let mod_id = r
            .with_conn_mut(|c| {
                let id = upsert::upsert_mod_by_alias(c, &[("modid", "m")], NOW)?;
                upsert::upsert_mod_version(c, id, "1", &["forge"], &sha, 1, None, None, NOW)?;
                if let Some((client, server)) = env {
                    upsert::set_mod_env_flags(c, id, Some(client), Some(server), NOW)?;
                }
                if let Some((kind, side, policy)) = jar {
                    upsert::set_jar_class(c, &sha, kind, side, policy, None)?;
                }
                Ok(id)
            })
            .unwrap();
        (r, mod_id, sha)
    }

    #[test]
    fn modrinth_env_outranks_bytecode() {
        // Modrinth says client-only; the bytecode saw a both/must_match content
        // mod. The flags win; the bytecode verdict stays visible as the
        // disagreement.
        let (r, id, sha) = setup(
            Some(("required", "unsupported")),
            Some(("mod", Some("both"), Some("must_match"))),
        );
        let c = r
            .with_conn(|conn| classify_artifact(conn, Some(id), Some(&sha)))
            .unwrap();
        assert_eq!(c.provenance, Provenance::ModrinthEnv);
        assert_eq!(c.side, Some(SideClass::Client));
        assert_eq!(c.policy, Some(MatchPolicy::Tolerant));
        assert_eq!(
            c.side_disagreement(),
            Some((SideClass::Client, SideClass::Both)),
            "the losing bytecode side is reportable"
        );
    }

    #[test]
    fn bytecode_decides_without_modrinth_identity() {
        let (r, id, sha) = setup(None, Some(("mod", Some("client"), Some("tolerant"))));
        let c = r
            .with_conn(|conn| classify_artifact(conn, Some(id), Some(&sha)))
            .unwrap();
        assert_eq!(c.provenance, Provenance::Bytecode);
        assert_eq!(c.side, Some(SideClass::Client));
        assert_eq!(c.side_disagreement(), None);
    }

    #[test]
    fn undecidable_env_falls_through_to_bytecode() {
        // Modrinth ships literal "unknown" flags for some projects: they decide
        // nothing and the bytecode verdict applies.
        let (r, id, sha) = setup(
            Some(("unknown", "unknown")),
            Some(("mod", Some("both"), Some("must_match"))),
        );
        let c = r
            .with_conn(|conn| classify_artifact(conn, Some(id), Some(&sha)))
            .unwrap();
        assert_eq!(c.provenance, Provenance::Bytecode);
        assert_eq!(c.policy, Some(MatchPolicy::MustMatch));
    }

    #[test]
    fn non_mod_kind_short_circuits_even_with_env_flags() {
        let (r, id, sha) = setup(
            Some(("required", "required")),
            Some(("coremod", None, None)),
        );
        let c = r
            .with_conn(|conn| classify_artifact(conn, Some(id), Some(&sha)))
            .unwrap();
        assert_eq!(c.provenance, Provenance::JarKind);
        assert!(c.is_non_mod());
        assert_eq!(c.side, None);
        assert_eq!(c.policy, None);
    }

    #[test]
    fn nothing_known_is_unclassified() {
        let (r, id, sha) = setup(None, None);
        let c = r
            .with_conn(|conn| classify_artifact(conn, Some(id), Some(&sha)))
            .unwrap();
        assert_eq!(c.provenance, Provenance::Unclassified);
        assert_eq!(c.side, None);
        assert!(!c.is_non_mod(), "an unscanned jar reads as a mod");
    }

    #[test]
    fn authored_classification_survives_reharvest_and_reads_high() {
        use super::super::authored;
        let (r, id, sha) = setup(None, Some(("mod", Some("client"), Some("tolerant"))));
        // the operator asserts the jar is a both-side tolerant library
        r.with_conn_mut(|c| {
            authored::set_authored_jar_class(c, &sha, "mod", Some("both"), Some("tolerant"), false)
        })
        .unwrap();
        // a re-harvest refresh must not clobber the authored row
        r.with_conn_mut(|c| {
            crate::registry::upsert::set_jar_class(
                c,
                &sha,
                "mod",
                Some("client"),
                Some("tolerant"),
                Some("low"),
            )?;
            Ok(())
        })
        .unwrap();
        let c = r
            .with_conn(|conn| classify_artifact(conn, Some(id), Some(&sha)))
            .unwrap();
        assert_eq!(c.side, Some(SideClass::Both), "authored verdict holds");
        assert_eq!(c.side_confidence.as_deref(), Some("high"));
        assert!(!c.client_verdict_is_soft());

        // clearing the override lets the next harvest re-derive
        r.with_conn_mut(|c| {
            authored::set_authored_jar_class(c, &sha, "", None, None, true)?;
            crate::registry::upsert::set_jar_class(
                c,
                &sha,
                "mod",
                Some("client"),
                Some("tolerant"),
                Some("low"),
            )?;
            Ok(())
        })
        .unwrap();
        let c = r
            .with_conn(|conn| classify_artifact(conn, Some(id), Some(&sha)))
            .unwrap();
        assert_eq!(c.side, Some(SideClass::Client), "derived verdict is back");
    }

    #[test]
    fn authored_classification_is_refused_for_a_modrinth_mod() {
        use super::super::authored;
        let (r, id, sha) = setup(Some(("required", "required")), None);
        r.with_conn_mut(|c| {
            c.execute(
                "INSERT INTO mod_alias (mod_id, source, external_key) VALUES (?1, 'modrinth', 'PROJ')",
                [id],
            )?;
            Ok(())
        })
        .unwrap();
        let err = r
            .with_conn_mut(|c| {
                authored::set_authored_jar_class(c, &sha, "mod", Some("both"), None, false)
            })
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Modrinth-identified"),
            "the env flags stay authoritative: {err}"
        );
    }

    #[test]
    fn partial_bytecode_side_without_policy_still_wins_over_nothing() {
        // fabric env "*" pinned the side while the policy stayed open
        let (r, id, sha) = setup(None, Some(("mod", Some("both"), None)));
        let c = r
            .with_conn(|conn| classify_artifact(conn, Some(id), Some(&sha)))
            .unwrap();
        assert_eq!(c.provenance, Provenance::Bytecode);
        assert_eq!(c.side, Some(SideClass::Both));
        assert_eq!(c.policy, None, "policy stays open -> unclassified policy");
    }
}
