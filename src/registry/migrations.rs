//! Hand-rolled, dependency-free migrations. Numbered steps applied in order and
//! tracked in `registry_meta.schema_version`. Most steps are SQL embedded at
//! compile time; a step needing imperative logic (inspect the schema, toggle
//! `foreign_keys` for a table rebuild) is a `Code` step. Adding a migration =
//! append `(n, Migration::Sql(include_str!(...)))`, or `Migration::Code(...)`
//! for the rare imperative case.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension};

enum Migration {
    Sql(&'static str),
    /// An imperative step. Unlike `Sql` it is NOT wrapped in an outer
    /// transaction (so it can toggle `foreign_keys`, a no-op inside one), and so
    /// MUST be idempotent -- it may re-run if a later step fails before the
    /// version is recorded.
    Code(fn(&mut Connection) -> Result<()>),
}

const MIGRATIONS: &[(u32, Migration)] = &[
    (1, Migration::Sql(include_str!("schema/0001_init.sql"))),
    (
        2,
        Migration::Sql(include_str!("schema/0002_seed_loaders.sql")),
    ),
    (
        3,
        Migration::Sql(include_str!("schema/0003_pack_build_fingerprint.sql")),
    ),
    (4, Migration::Code(reconcile_target_schema)),
    (
        5,
        Migration::Sql(include_str!("schema/0005_mod_author.sql")),
    ),
    (
        6,
        Migration::Sql(include_str!("schema/0006_mod_version_modrinth.sql")),
    ),
    (
        7,
        Migration::Sql(include_str!("schema/0007_mod_release.sql")),
    ),
];

/// Apply every migration newer than the recorded schema version, each in its
/// own transaction. Idempotent: a no-op when already current.
pub fn apply_pending(conn: &mut Connection) -> Result<()> {
    let current = current_version(conn)?;
    for (version, migration) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        match migration {
            Migration::Sql(sql) => {
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
            Migration::Code(step) => {
                step(conn).with_context(|| format!("applying migration {version}"))?;
                // recorded after the (idempotent) step succeeds; a crash before
                // this just re-runs the step next start.
                conn.execute(
                    "INSERT INTO registry_meta (k, v) VALUES ('schema_version', ?1)
                     ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                    [version.to_string()],
                )
                .context("recording schema_version")?;
            }
        }
    }
    Ok(())
}

/// Migration 4: reconcile a registry created from the pre-rename `0001` (shipped
/// by #18 before the target-set rework) to the current shape, without data loss.
/// Idempotent; a no-op on a DB already built from the current `0001`.
///
/// The pre-rename `mod_version` had a `target` column + `UNIQUE(mod_id, version,
/// target)` and there was no `mod_version_target` table, so the current queries
/// 500 on such a DB. This adds the child table, moves the old single target into
/// it, then rebuilds `mod_version` into the new shape (dropping the column and
/// the stale uniqueness).
fn reconcile_target_schema(conn: &mut Connection) -> Result<()> {
    // The child table is absent on the pre-rename schema; create it either way.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mod_version_target (
           mod_version_id INTEGER NOT NULL REFERENCES mod_version(id) ON DELETE CASCADE,
           target         TEXT NOT NULL,
           PRIMARY KEY (mod_version_id, target)
         );
         CREATE INDEX IF NOT EXISTS idx_mvt_target ON mod_version_target(target);",
    )
    .context("0004: ensure mod_version_target")?;

    if !mod_version_has_target_column(conn)? {
        return Ok(()); // already the post-rename shape
    }

    // `foreign_keys` is a no-op inside a transaction, and dropping mod_version
    // would otherwise cascade-delete the rows just backfilled, so toggle it off
    // around the rebuild (SQLite's documented table-redefinition procedure).
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .context("0004: foreign_keys off")?;
    let tx = conn.transaction().context("0004: begin rebuild")?;
    tx.execute_batch(
        "INSERT OR IGNORE INTO mod_version_target (mod_version_id, target)
           SELECT id, target FROM mod_version;

         CREATE TABLE mod_version_new (
           id           INTEGER PRIMARY KEY,
           mod_id       INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
           version      TEXT NOT NULL,
           sha1         TEXT NOT NULL,
           size_bytes   INTEGER NOT NULL,
           filename     TEXT,
           mc_versions  TEXT,
           source       TEXT NOT NULL DEFAULT 'harvested',
           confidence   INTEGER NOT NULL DEFAULT 10,
           created_at   TEXT NOT NULL,
           updated_at   TEXT NOT NULL,
           CHECK (source IN ('harvested','jar-meta','modrinth','inferred','curator','authored'))
         );
         INSERT INTO mod_version_new
           (id, mod_id, version, sha1, size_bytes, filename, mc_versions,
            source, confidence, created_at, updated_at)
           SELECT id, mod_id, version, sha1, size_bytes, filename, mc_versions,
                  source, confidence, created_at, updated_at
           FROM mod_version;
         DROP TABLE mod_version;
         ALTER TABLE mod_version_new RENAME TO mod_version;
         CREATE UNIQUE INDEX idx_mv_sha1 ON mod_version(sha1);
         CREATE INDEX idx_mv_mod ON mod_version(mod_id);",
    )
    .context("0004: rebuild mod_version")?;
    tx.commit().context("0004: commit rebuild")?;
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .context("0004: foreign_keys on")?;

    // the rebuild must not have orphaned any foreign key.
    let violations = {
        let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
        stmt.query_map([], |_| Ok(()))?.count()
    };
    if violations > 0 {
        bail!("0004 reconcile left {violations} foreign-key violations");
    }
    Ok(())
}

/// True if `mod_version` still carries the pre-rename `target` column.
fn mod_version_has_target_column(conn: &Connection) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(mod_version)")?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(names.iter().any(|name| name == "target"))
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
                 AND name IN ('mods','mod_alias','mod_version','mod_version_target','mod_release',
                              'pack','pack_build','pack_build_mod','relation','loader',
                              'loader_parent','registry_meta')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 12);

        let loaders: i64 = conn
            .query_row("SELECT count(*) FROM loader", [], |r| r.get(0))
            .unwrap();
        assert!(loaders >= 6);
    }

    // A registry created from the pre-rename 0001 (mod_version with a `target`
    // column + UNIQUE(mod_id,version,target), no mod_version_target) must
    // reconcile losslessly and idempotently.
    #[test]
    fn reconcile_0004_migrates_stale_target_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE mods (id INTEGER PRIMARY KEY);
             CREATE TABLE mod_version (
               id INTEGER PRIMARY KEY,
               mod_id INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
               version TEXT NOT NULL,
               target TEXT NOT NULL DEFAULT 'any',
               sha1 TEXT NOT NULL,
               size_bytes INTEGER NOT NULL,
               filename TEXT,
               mc_versions TEXT,
               source TEXT NOT NULL DEFAULT 'harvested',
               confidence INTEGER NOT NULL DEFAULT 10,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL,
               UNIQUE (mod_id, version, target)
             );
             CREATE UNIQUE INDEX idx_mv_sha1 ON mod_version(sha1);
             INSERT INTO mods (id) VALUES (1);
             INSERT INTO mod_version
               (id, mod_id, version, target, sha1, size_bytes, created_at, updated_at)
               VALUES (1, 1, '2.5', 'forge', 'sha_a', 10, 'T', 'T');",
        )
        .unwrap();
        assert!(mod_version_has_target_column(&conn).unwrap());

        reconcile_target_schema(&mut conn).unwrap();

        // the old single target moved into the child table
        let tgt: String = conn
            .query_row(
                "SELECT target FROM mod_version_target WHERE mod_version_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tgt, "forge");
        // the column is gone and the row data survived
        assert!(!mod_version_has_target_column(&conn).unwrap());
        let (v, s): (String, String) = conn
            .query_row(
                "SELECT version, sha1 FROM mod_version WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((v.as_str(), s.as_str()), ("2.5", "sha_a"));
        // the stale UNIQUE(mod_id,version,target) is gone: a second artifact of
        // the same mod+version inserts cleanly now
        conn.execute(
            "INSERT INTO mod_version
               (mod_id, version, sha1, size_bytes, created_at, updated_at)
             VALUES (1, '2.5', 'sha_b', 20, 'T', 'T')",
            [],
        )
        .unwrap();

        // idempotent: a second run is a no-op
        reconcile_target_schema(&mut conn).unwrap();
        assert!(!mod_version_has_target_column(&conn).unwrap());
    }

    // Migration 7 groups existing files into one 'unknown' release per
    // (mod_id, version) and links each file up to it. Two files of one mod at the
    // same number collapse to one release; a different number, or the same number
    // under a different mod, is its own release.
    #[test]
    fn migration_0007_backfills_one_release_per_mod_version_group() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE mods (id INTEGER PRIMARY KEY);
             CREATE TABLE mod_version (
               id INTEGER PRIMARY KEY,
               mod_id INTEGER NOT NULL REFERENCES mods(id),
               version TEXT NOT NULL,
               sha1 TEXT NOT NULL,
               size_bytes INTEGER NOT NULL,
               filename TEXT,
               mc_versions TEXT,
               source TEXT NOT NULL DEFAULT 'harvested',
               confidence INTEGER NOT NULL DEFAULT 10,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             INSERT INTO mods (id) VALUES (1), (2);
             INSERT INTO mod_version (mod_id, version, sha1, size_bytes, created_at, updated_at)
               VALUES (1, '1.0', 'sha_a', 1, 'T', 'T'),
                      (1, '1.0', 'sha_b', 1, 'T', 'T'),
                      (1, '2.0', 'sha_c', 1, 'T', 'T'),
                      (2, '1.0', 'sha_d', 1, 'T', 'T');",
        )
        .unwrap();

        conn.execute_batch(include_str!("schema/0007_mod_release.sql"))
            .unwrap();

        let release_id = |sha: &str| -> i64 {
            conn.query_row(
                "SELECT release_id FROM mod_version WHERE sha1 = ?1",
                [sha],
                |r| r.get(0),
            )
            .unwrap()
        };

        let releases: i64 = conn
            .query_row("SELECT count(*) FROM mod_release", [], |r| r.get(0))
            .unwrap();
        assert_eq!(releases, 3, "(1,1.0) (1,2.0) (2,1.0)");
        assert_eq!(release_id("sha_a"), release_id("sha_b"), "same mod+number");
        assert_ne!(release_id("sha_a"), release_id("sha_c"), "different number");
        assert_ne!(release_id("sha_a"), release_id("sha_d"), "different mod");

        let ungrouped: i64 = conn
            .query_row(
                "SELECT count(*) FROM mod_version WHERE release_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ungrouped, 0, "every file grouped");

        let (vn, ch): (String, String) = conn
            .query_row(
                "SELECT version_number, channel FROM mod_release WHERE id = ?1",
                [release_id("sha_a")],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((vn.as_str(), ch.as_str()), ("1.0", "unknown"));
    }
}
