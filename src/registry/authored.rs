//! Authored (precious) writers: the manual-moderation layer. Every row written
//! here carries `source = 'authored'`, which the harvest's never-clobber guard
//! (`WHERE source NOT IN ('curator','authored')`) preserves across re-harvests.
//! Driven through the `Registry` methods (used by the CLI + admin HTTP).

use super::model::{RelKind, Source};
use anyhow::{Result, bail};
use rusqlite::{Connection, params};

/// Mark a pack's provenance as an operator decision. Creates the pack row if it
/// doesn't exist yet (an unbuilt pack), and stamps `source='authored'` so a
/// re-harvest's `upsert_pack` won't revert it.
pub fn set_pack_provenance(
    conn: &Connection,
    pack_id: &str,
    provenance: &str,
    now: &str,
) -> Result<()> {
    if provenance != "sc" && provenance != "hivens" {
        bail!("provenance must be 'sc' or 'hivens', got {provenance:?}");
    }
    conn.execute(
        "INSERT INTO pack (id, provenance, source, created_at, updated_at)
         VALUES (?1, ?2, 'authored', ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET
           provenance = excluded.provenance,
           source = 'authored',
           updated_at = excluded.updated_at",
        params![pack_id, provenance, now],
    )?;
    Ok(())
}

/// Record an operator-asserted relation. De-duped against an identical authored
/// row by the unique index; coexists with rows of other sources (precedence
/// resolves later).
pub fn add_authored_relation(
    conn: &Connection,
    from_mod_id: i64,
    target_modid: &str,
    kind: RelKind,
    now: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO relation
           (from_mod_id, target_modid, target_version_range, kind, source, confidence, created_at)
         VALUES (?1, ?2, NULL, ?3, 'authored', ?4, ?5)",
        params![
            from_mod_id,
            target_modid,
            kind.as_str(),
            Source::Authored.rank(),
            now
        ],
    )?;
    Ok(())
}

/// Remove an authored relation. Only `authored` rows -- a harvested fact would
/// just reappear on the next harvest (open-world; there is no "not" assertion).
pub fn remove_authored_relation(
    conn: &Connection,
    from_mod_id: i64,
    target_modid: &str,
    kind: RelKind,
) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM relation
         WHERE from_mod_id = ?1 AND target_modid = ?2 AND kind = ?3 AND source = 'authored'",
        params![from_mod_id, target_modid, kind.as_str()],
    )?)
}

/// Add or remove a mutual authored conflict between two mods (the launcher
/// treats conflicts as bidirectional, so both directions are written).
#[allow(clippy::too_many_arguments)]
pub fn set_authored_conflict(
    conn: &Connection,
    a_mod_id: i64,
    a_modid: &str,
    b_mod_id: i64,
    b_modid: &str,
    now: &str,
    remove: bool,
) -> Result<()> {
    if remove {
        remove_authored_relation(conn, a_mod_id, b_modid, RelKind::Conflicts)?;
        remove_authored_relation(conn, b_mod_id, a_modid, RelKind::Conflicts)?;
    } else {
        add_authored_relation(conn, a_mod_id, b_modid, RelKind::Conflicts, now)?;
        add_authored_relation(conn, b_mod_id, a_modid, RelKind::Conflicts, now)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::registry::{Registry, queries, upsert};

    #[test]
    fn authored_facts_survive_reharvest() {
        let r = Registry::open_in_memory().unwrap();
        // harvest two mods + a pack (source='harvested')
        r.with_txn(|c| {
            let a = upsert::upsert_mod_by_alias(c, &[("modid", "amod")], "T0")?;
            upsert::upsert_mod_version(c, a, "1", &["forge"], "sha_a", 1, None, None, "T0")?;
            let b = upsert::upsert_mod_by_alias(c, &[("modid", "bmod")], "T0")?;
            upsert::upsert_mod_version(c, b, "1", &["forge"], "sha_b", 1, None, None, "T0")?;
            upsert::upsert_pack(c, "P", "hivens", "T0")?;
            Ok(())
        })
        .unwrap();

        // operator moderation: provenance + a conflict metadata never declared
        r.set_provenance("P", "sc").unwrap();
        r.set_conflict("amod", "bmod", false).unwrap();

        // a re-harvest (source='harvested') must not revert either
        r.with_txn(|c| upsert::upsert_pack(c, "P", "hivens", "T2"))
            .unwrap();

        r.with_conn(|c| {
            let (prov, src): (String, String) = c.query_row(
                "SELECT provenance, source FROM pack WHERE id = 'P'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!(prov, "sc");
            assert_eq!(src, "authored");
            let conflicts: i64 = c.query_row(
                "SELECT count(*) FROM relation WHERE kind='conflicts' AND source='authored'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(conflicts, 2, "mutual authored conflict, both directions");
            Ok(())
        })
        .unwrap();

        // an unknown mod is rejected, not silently ignored
        assert!(r.set_conflict("amod", "ghost", false).is_err());

        // removal clears both directions
        r.set_conflict("amod", "bmod", true).unwrap();
        r.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT count(*) FROM relation WHERE kind='conflicts' AND source='authored'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(n, 0);
            // sanity: the alias resolution the methods rely on
            assert!(queries::mod_id_for_alias(c, "modid", "amod")?.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn backup_writes_a_readable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let r = Registry::open(dir.path().join("registry.db")).unwrap();
        r.with_txn(|c| upsert::upsert_mod_by_alias(c, &[("modid", "x")], "T0").map(|_| ()))
            .unwrap();
        let backup = dir.path().join("backup.db");
        r.backup_into(&backup).unwrap();

        let restored = Registry::open(&backup).unwrap();
        let n: i64 = restored
            .with_conn(|c| Ok(c.query_row("SELECT count(*) FROM mods", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(n, 1);
    }
}
