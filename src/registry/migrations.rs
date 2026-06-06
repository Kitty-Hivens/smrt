//! Hand-rolled, dependency-free migrations. Numbered SQL embedded at compile
//! time, applied transactionally, tracked in `registry_meta.schema_version`.
//! Adding a migration = append a `(n, include_str!(...))` line.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("schema/0001_init.sql")),
    (2, include_str!("schema/0002_seed_loaders.sql")),
    (3, include_str!("schema/0003_pack_build_fingerprint.sql")),
];

/// Apply every migration newer than the recorded schema version, each in its
/// own transaction. Idempotent: a no-op when already current.
pub fn apply_pending(conn: &mut Connection) -> Result<()> {
    let current = current_version(conn)?;
    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        let tx = conn.transaction().context("begin migration txn")?;
        tx.execute_batch(sql)
            .with_context(|| format!("applying migration {version}"))?;
        // migration 1 creates registry_meta; the upsert is safe from then on.
        tx.execute(
            "INSERT INTO registry_meta (k, v) VALUES ('schema_version', ?1)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            [version.to_string()],
        )
        .context("recording schema_version")?;
        tx.commit().context("committing migration")?;
    }
    Ok(())
}

/// Recorded schema version, or 0 on a fresh database (no `registry_meta` yet).
fn current_version(conn: &Connection) -> Result<u32> {
    let has_meta: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'registry_meta'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !has_meta {
        return Ok(0);
    }
    let v: Option<String> = conn
        .query_row(
            "SELECT v FROM registry_meta WHERE k = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_pending(&mut conn).unwrap();
        let v1 = current_version(&conn).unwrap();
        // re-running applies nothing
        apply_pending(&mut conn).unwrap();
        let v2 = current_version(&conn).unwrap();
        assert_eq!(v1, v2);
        assert_eq!(v1, MIGRATIONS.last().unwrap().0);

        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table'
                 AND name IN ('mods','mod_alias','mod_version','mod_version_target','pack',
                              'pack_build','pack_build_mod','relation','loader','loader_parent',
                              'registry_meta')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 11);

        let loaders: i64 = conn
            .query_row("SELECT count(*) FROM loader", [], |r| r.get(0))
            .unwrap();
        assert!(loaders >= 6);
    }
}
