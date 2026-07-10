//! Authored (precious) writers: the manual-moderation layer. Every row written
//! here carries `source = 'authored'`, which the harvest's never-clobber guard
//! (`WHERE source NOT IN ('curator','authored')`) preserves across re-harvests.
//! Driven through the `Registry` methods (used by the CLI + admin HTTP).

use super::model::{RelKind, Source};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

/// The valid `mod_release.channel` values (mirrors the schema CHECK).
pub const CHANNELS: &[&str] = &["release", "beta", "dev", "unknown"];

fn valid_channel(ch: &str) -> bool {
    CHANNELS.contains(&ch)
}

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

// ── mod / release / file identity (the authoring door) ───────────────────────

/// Which mod a file belongs to: an existing surrogate id the operator picked, or
/// a new authored identity to create with this display name.
pub enum ModRef<'a> {
    Existing(i64),
    New { name: &'a str },
}

/// The full identity an operator sets for one cached jar (by sha1): which mod it
/// is, which release (version_number + channel) it belongs to, and the file's own
/// loader + Minecraft-version facets. Everything written is `source='authored'`,
/// so a re-harvest never overwrites it (the never-clobber guard).
pub struct FileIdentity<'a> {
    pub sha1: &'a str,
    pub size_bytes: i64,
    pub filename: Option<&'a str>,
    pub mod_ref: ModRef<'a>,
    pub version_number: &'a str,
    pub channel: &'a str,
    /// Loader ids the jar suits; empty falls back to `any`.
    pub loaders: &'a [String],
    pub mc_versions: &'a [String],
}

/// Assign a cached jar its mod + release + facets as an operator decision. Creates
/// the mod (when `New`) and the release (by `mod_id, version_number, channel`) if
/// absent, then upserts the file as `authored` and (re)sets its loader targets.
/// Returns the `mod_version` id. This IS the authored write, so it overwrites even
/// a prior authored row for the same sha1.
pub fn author_file_identity(conn: &Connection, id: &FileIdentity, now: &str) -> Result<i64> {
    if id.version_number.trim().is_empty() {
        bail!("version_number must not be empty");
    }
    if !valid_channel(id.channel) {
        bail!(
            "channel must be one of release/beta/dev/unknown, got {:?}",
            id.channel
        );
    }

    let mod_id = match id.mod_ref {
        ModRef::Existing(mid) => {
            let exists = conn
                .query_row("SELECT 1 FROM mods WHERE id = ?1", params![mid], |_| Ok(()))
                .optional()?
                .is_some();
            if !exists {
                bail!("mod #{mid} does not exist");
            }
            mid
        }
        ModRef::New { name } => {
            let name = name.trim();
            if name.is_empty() {
                bail!("new mod name must not be empty");
            }
            conn.execute(
                "INSERT INTO mods (slug, canonical_name, source, confidence, created_at, updated_at)
                 VALUES (NULL, ?1, 'authored', ?2, ?3, ?3)",
                params![name, Source::Authored.rank(), now],
            )?;
            conn.last_insert_rowid()
        }
    };

    let release_id = match conn
        .query_row(
            "SELECT id FROM mod_release WHERE mod_id = ?1 AND version_number = ?2 AND channel = ?3",
            params![mod_id, id.version_number, id.channel],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        Some(rid) => rid,
        None => {
            conn.execute(
                "INSERT INTO mod_release
                   (mod_id, version_number, channel, source, confidence, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'authored', ?4, ?5, ?5)",
                params![
                    mod_id,
                    id.version_number,
                    id.channel,
                    Source::Authored.rank(),
                    now
                ],
            )?;
            conn.last_insert_rowid()
        }
    };

    let mc = if id.mc_versions.is_empty() {
        None
    } else {
        Some(serde_json::to_string(id.mc_versions)?)
    };
    conn.execute(
        "INSERT INTO mod_version
           (mod_id, version, sha1, size_bytes, filename, mc_versions, release_id,
            source, confidence, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'authored', ?8, ?9, ?9)
         ON CONFLICT(sha1) DO UPDATE SET
           mod_id = excluded.mod_id,
           version = excluded.version,
           size_bytes = excluded.size_bytes,
           filename = COALESCE(excluded.filename, mod_version.filename),
           mc_versions = excluded.mc_versions,
           release_id = excluded.release_id,
           source = 'authored',
           confidence = excluded.confidence,
           updated_at = excluded.updated_at",
        params![
            mod_id,
            id.version_number,
            id.sha1,
            id.size_bytes,
            id.filename,
            mc,
            release_id,
            Source::Authored.rank(),
            now
        ],
    )?;
    let mv_id: i64 = conn.query_row(
        "SELECT id FROM mod_version WHERE sha1 = ?1",
        params![id.sha1],
        |r| r.get(0),
    )?;

    conn.execute(
        "DELETE FROM mod_version_target WHERE mod_version_id = ?1",
        params![mv_id],
    )?;
    let any = [String::from("any")];
    let effective: &[String] = if id.loaders.is_empty() {
        &any
    } else {
        id.loaders
    };
    for t in effective {
        conn.execute(
            "INSERT OR IGNORE INTO mod_version_target (mod_version_id, target) VALUES (?1, ?2)",
            params![mv_id, t],
        )?;
    }
    Ok(mv_id)
}

/// Rename a mod (canonical name and/or Modrinth slug) as an operator decision,
/// stamping `source='authored'` so a re-harvest's `set_mod_meta` won't revert it.
pub fn rename_mod(
    conn: &Connection,
    mod_id: i64,
    name: Option<&str>,
    slug: Option<&str>,
    now: &str,
) -> Result<()> {
    if name.is_none() && slug.is_none() {
        return Ok(());
    }
    let n = conn.execute(
        "UPDATE mods SET
           canonical_name = COALESCE(?2, canonical_name),
           slug           = COALESCE(?3, slug),
           source = 'authored', updated_at = ?4
         WHERE id = ?1",
        params![mod_id, name, slug, now],
    )?;
    if n == 0 {
        bail!("mod #{mod_id} does not exist");
    }
    Ok(())
}

/// Edit a release's version number and/or channel as an operator decision
/// (`source='authored'`). Errors if the change collides with a sibling release
/// (the `mod_id, version_number, channel` uniqueness).
pub fn edit_release(
    conn: &Connection,
    release_id: i64,
    version_number: Option<&str>,
    channel: Option<&str>,
    now: &str,
) -> Result<()> {
    if let Some(ch) = channel
        && !valid_channel(ch)
    {
        bail!("channel must be one of release/beta/dev/unknown, got {ch:?}");
    }
    if version_number.is_none() && channel.is_none() {
        return Ok(());
    }
    let n = conn.execute(
        "UPDATE mod_release SET
           version_number = COALESCE(?2, version_number),
           channel        = COALESCE(?3, channel),
           source = 'authored', updated_at = ?4
         WHERE id = ?1",
        params![release_id, version_number, channel, now],
    )?;
    if n == 0 {
        bail!("release #{release_id} does not exist");
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
    fn author_file_identity_creates_grouped_authored_and_survives_reharvest() {
        use super::{FileIdentity, ModRef, author_file_identity};
        let r = Registry::open_in_memory().unwrap();

        // label a loose cached jar: brand-new mod, a dev release, forge/1.7.10
        let mv_id = r
            .with_txn(|c| {
                author_file_identity(
                    c,
                    &FileIdentity {
                        sha1: "sha_new",
                        size_bytes: 123,
                        filename: Some("cool.jar"),
                        mod_ref: ModRef::New { name: "Cool Mod" },
                        version_number: "2.3",
                        channel: "dev",
                        loaders: &["forge".to_string()],
                        mc_versions: &["1.7.10".to_string()],
                    },
                    "T0",
                )
            })
            .unwrap();

        r.with_conn(|c| {
            let (src, ver, rel): (String, String, i64) = c.query_row(
                "SELECT source, version, release_id FROM mod_version WHERE sha1 = 'sha_new'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(src, "authored");
            assert_eq!(ver, "2.3");
            let (vn, ch, msrc): (String, String, String) = c.query_row(
                "SELECT mr.version_number, mr.channel, m.source
                 FROM mod_release mr JOIN mods m ON m.id = mr.mod_id WHERE mr.id = ?1",
                [rel],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(
                (vn.as_str(), ch.as_str(), msrc.as_str()),
                ("2.3", "dev", "authored")
            );
            let t: String = c.query_row(
                "SELECT target FROM mod_version_target WHERE mod_version_id = ?1",
                [mv_id],
                |r| r.get(0),
            )?;
            assert_eq!(t, "forge");
            Ok(())
        })
        .unwrap();

        // a re-harvest reporting a different mod/loader must not touch the file
        r.with_txn(|c| {
            let mid = queries::mod_id_for_sha1(c, "sha_new")?.unwrap();
            upsert::upsert_mod_version(
                c,
                mid,
                "HARVEST",
                &["fabric"],
                "sha_new",
                999,
                Some("x.jar"),
                None,
                "T1",
            )
        })
        .unwrap();
        r.with_conn(|c| {
            let ver: String = c.query_row(
                "SELECT version FROM mod_version WHERE sha1 = 'sha_new'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(ver, "2.3", "authored file survives a re-harvest");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn author_file_identity_reuses_release_and_can_relabel() {
        use super::{FileIdentity, ModRef, author_file_identity};
        let r = Registry::open_in_memory().unwrap();
        // two files under the same mod + version + channel share one release
        let mk = |sha: &'static str, mod_ref| FileIdentity {
            sha1: sha,
            size_bytes: 1,
            filename: None,
            mod_ref,
            version_number: "1.0",
            channel: "release",
            loaders: &[],
            mc_versions: &[],
        };
        let mid = r
            .with_txn(|c| {
                let a = author_file_identity(c, &mk("sha_a", ModRef::New { name: "M" }), "T0")?;
                let owner: i64 =
                    c.query_row("SELECT mod_id FROM mod_version WHERE id = ?1", [a], |r| {
                        r.get(0)
                    })?;
                author_file_identity(c, &mk("sha_b", ModRef::Existing(owner)), "T0")?;
                Ok(owner)
            })
            .unwrap();
        r.with_conn(|c| {
            let releases: i64 = c.query_row(
                "SELECT count(*) FROM mod_release WHERE mod_id = ?1",
                [mid],
                |r| r.get(0),
            )?;
            assert_eq!(releases, 1, "same (mod, version, channel) -> one release");
            // an empty loader set records 'any'
            let anyc: i64 = c.query_row(
                "SELECT count(*) FROM mod_version_target t JOIN mod_version mv ON mv.id = t.mod_version_id
                 WHERE mv.sha1 = 'sha_a' AND t.target = 'any'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(anyc, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn edit_release_and_rename_mod_are_authored() {
        use super::{FileIdentity, ModRef, author_file_identity, edit_release, rename_mod};
        let r = Registry::open_in_memory().unwrap();
        let (mid, rel) = r
            .with_txn(|c| {
                let mv = author_file_identity(
                    c,
                    &FileIdentity {
                        sha1: "sha_x",
                        size_bytes: 1,
                        filename: None,
                        mod_ref: ModRef::New { name: "X" },
                        version_number: "0.1",
                        channel: "unknown",
                        loaders: &[],
                        mc_versions: &[],
                    },
                    "T0",
                )?;
                let (mid, rel): (i64, i64) = c.query_row(
                    "SELECT mod_id, release_id FROM mod_version WHERE id = ?1",
                    [mv],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                Ok((mid, rel))
            })
            .unwrap();

        r.with_txn(|c| edit_release(c, rel, Some("0.2"), Some("beta"), "T1"))
            .unwrap();
        r.with_txn(|c| rename_mod(c, mid, Some("Xenon"), None, "T1"))
            .unwrap();
        r.with_conn(|c| {
            let (vn, ch): (String, String) = c.query_row(
                "SELECT version_number, channel FROM mod_release WHERE id = ?1",
                [rel],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!((vn.as_str(), ch.as_str()), ("0.2", "beta"));
            let (name, src): (String, String) = c.query_row(
                "SELECT canonical_name, source FROM mods WHERE id = ?1",
                [mid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!((name.as_str(), src.as_str()), ("Xenon", "authored"));
            Ok(())
        })
        .unwrap();

        // an invalid channel is rejected
        assert!(
            r.with_txn(|c| edit_release(c, rel, None, Some("nightly"), "T2"))
                .is_err()
        );
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
