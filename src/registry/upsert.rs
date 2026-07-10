//! Idempotent writers. Every write is keyed by a natural key and guards the
//! precious layer: harvested rows refresh, but `curator`/`authored` rows are
//! never clobbered (the `WHERE source NOT IN (...)` on each upsert). Call inside
//! `Registry::with_conn_mut` (a write transaction).

use super::model::{RelKind, Source};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

pub fn now_rfc3339() -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Resolve a logical mod from any of its external keys, creating it if none
/// match, and attach any missing aliases. This is how a jar carrying both a
/// `modid` and a Modrinth `project_id` collapses to one identity, and how two
/// jars of the same mod share it. (First-found wins; if two supplied aliases
/// already point at *different* mods they stay separate -- a true merge is a
/// Phase 2 concern.)
pub fn upsert_mod_by_alias(conn: &Connection, aliases: &[(&str, &str)], now: &str) -> Result<i64> {
    let mut found: Option<i64> = None;
    for (src, key) in aliases {
        if let Some(id) = conn
            .query_row(
                "SELECT mod_id FROM mod_alias WHERE source = ?1 AND external_key = ?2",
                params![src, key],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            found = Some(id);
            break;
        }
    }
    let mod_id = match found {
        Some(id) => id,
        None => {
            conn.execute(
                "INSERT INTO mods (source, confidence, created_at, updated_at)
                 VALUES ('harvested', 10, ?1, ?1)",
                params![now],
            )?;
            conn.last_insert_rowid()
        }
    };
    for (src, key) in aliases {
        conn.execute(
            "INSERT INTO mod_alias (mod_id, source, external_key) VALUES (?1, ?2, ?3)
             ON CONFLICT(source, external_key) DO NOTHING",
            params![mod_id, src, key],
        )?;
    }
    Ok(mod_id)
}

/// Fill a mod's human metadata (display name, Modrinth slug, author) from a
/// harvest. `COALESCE(new, existing)` per column: a jar that carries a value
/// fills a gap, a jar that lacks one never erases a value an earlier jar set.
/// Skipped for precious (`curator`/`authored`) rows. Idempotent.
pub fn set_mod_meta(
    conn: &Connection,
    mod_id: i64,
    name: Option<&str>,
    slug: Option<&str>,
    author: Option<&str>,
    now: &str,
) -> Result<()> {
    if name.is_none() && slug.is_none() && author.is_none() {
        return Ok(());
    }
    conn.execute(
        "UPDATE mods SET
           canonical_name = COALESCE(?2, canonical_name),
           slug           = COALESCE(?3, slug),
           author         = COALESCE(?4, author),
           updated_at     = ?5
         WHERE id = ?1 AND source NOT IN ('curator', 'authored')",
        params![mod_id, name, slug, author, now],
    )?;
    Ok(())
}

/// Find-or-create the release (version node) grouping a mod's files that share
/// `(version_number, channel)`. Harvested releases default to channel 'unknown';
/// the authored layer sets a real channel. Idempotent -- a re-harvest reuses the
/// existing release rather than duplicating it.
pub fn upsert_release(
    conn: &Connection,
    mod_id: i64,
    version_number: &str,
    channel: &str,
    now: &str,
) -> Result<i64> {
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM mod_release WHERE mod_id = ?1 AND version_number = ?2 AND channel = ?3",
            params![mod_id, version_number, channel],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO mod_release
           (mod_id, version_number, channel, source, confidence, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'harvested', 10, ?4, ?4)",
        params![mod_id, version_number, channel, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Upsert an artifact keyed by its content hash, and (re)record its
/// compatibility `targets` (loader ids, or `any` when the set is empty).
/// Returns the `mod_version` id. A precious (`curator`/`authored`) row with this
/// sha1 is left untouched -- its columns AND its hand-set targets both survive a
/// re-harvest.
#[allow(clippy::too_many_arguments)]
pub fn upsert_mod_version(
    conn: &Connection,
    mod_id: i64,
    version: &str,
    targets: &[&str],
    sha1: &str,
    size_bytes: i64,
    filename: Option<&str>,
    mc_versions: Option<&str>,
    now: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO mod_version
           (mod_id, version, sha1, size_bytes, filename, mc_versions,
            source, confidence, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'harvested', 10, ?7, ?7)
         ON CONFLICT(sha1) DO UPDATE SET
           mod_id = excluded.mod_id,
           version = excluded.version,
           size_bytes = excluded.size_bytes,
           filename = excluded.filename,
           mc_versions = excluded.mc_versions,
           updated_at = excluded.updated_at
         WHERE mod_version.source NOT IN ('curator', 'authored')",
        params![
            mod_id,
            version,
            sha1,
            size_bytes,
            filename,
            mc_versions,
            now
        ],
    )?;
    let (id, precious): (i64, bool) = conn.query_row(
        "SELECT id, source IN ('curator', 'authored') FROM mod_version WHERE sha1 = ?1",
        params![sha1],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    set_mod_version_targets(conn, id, targets, precious)?;
    // group the file under its release. Harvested files only: an authored file's
    // release is set through the authored layer, so a re-harvest must not move it.
    if !precious {
        let release_id = upsert_release(conn, mod_id, version, "unknown", now)?;
        conn.execute(
            "UPDATE mod_version SET release_id = ?2
             WHERE id = ?1 AND source NOT IN ('curator', 'authored')",
            params![id, release_id],
        )?;
    }
    Ok(id)
}

/// Re-point a harvested file to the release for its `(version, channel)`,
/// creating it if absent. No-op for a precious (authored) file, whose grouping is
/// operator-set. Call after [`upsert_mod_version`] (which sets a provisional
/// `unknown` release); harvest prunes any release this leaves empty.
pub fn set_harvested_release(
    conn: &Connection,
    sha1: &str,
    mod_id: i64,
    version: &str,
    channel: &str,
    now: &str,
) -> Result<()> {
    // absent row or a precious one -> leave it alone
    let precious: Option<bool> = conn
        .query_row(
            "SELECT source IN ('curator', 'authored') FROM mod_version WHERE sha1 = ?1",
            params![sha1],
            |r| r.get(0),
        )
        .optional()?;
    if precious != Some(false) {
        return Ok(());
    }
    let release_id = upsert_release(conn, mod_id, version, channel, now)?;
    conn.execute(
        "UPDATE mod_version SET release_id = ?2, updated_at = ?3 WHERE sha1 = ?1",
        params![sha1, release_id, now],
    )?;
    Ok(())
}

/// Delete harvested releases no file points at any more -- e.g. the provisional
/// `unknown` release [`upsert_mod_version`] creates before [`set_harvested_release`]
/// moves the file to its real (channel) release. Authored releases are preserved
/// even when empty (an operator may create one before attaching files).
pub fn prune_empty_releases(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM mod_release
         WHERE source NOT IN ('curator', 'authored')
           AND id NOT IN (SELECT release_id FROM mod_version WHERE release_id IS NOT NULL)",
        [],
    )?)
}

/// Replace a mod_version's target set with `targets` (empty -> `any`). Skipped
/// for precious rows: an authored artifact's hand-set targets are not a
/// harvested fact and must not be reset. Idempotent for harvested rows.
fn set_mod_version_targets(
    conn: &Connection,
    mod_version_id: i64,
    targets: &[&str],
    precious: bool,
) -> Result<()> {
    if precious {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM mod_version_target WHERE mod_version_id = ?1",
        params![mod_version_id],
    )?;
    let any = ["any"];
    let effective: &[&str] = if targets.is_empty() { &any } else { targets };
    for t in effective {
        conn.execute(
            "INSERT OR IGNORE INTO mod_version_target (mod_version_id, target) VALUES (?1, ?2)",
            params![mod_version_id, t],
        )?;
    }
    Ok(())
}

/// Record the Modrinth version id for an artifact (by sha1), so the panel can
/// re-add a build's Modrinth-sourced mod as a real Modrinth source rather than a
/// local-cache one. `COALESCE` so a later harvest that lost the id never erases
/// it; skipped for precious rows. No-op when there is no id.
pub fn set_mod_version_modrinth(
    conn: &Connection,
    sha1: &str,
    version_id: Option<&str>,
    now: &str,
) -> Result<()> {
    let Some(vid) = version_id else {
        return Ok(());
    };
    conn.execute(
        "UPDATE mod_version
           SET modrinth_version_id = COALESCE(?2, modrinth_version_id), updated_at = ?3
         WHERE sha1 = ?1 AND source NOT IN ('curator', 'authored')",
        params![sha1, vid, now],
    )?;
    Ok(())
}

pub fn mod_version_id_for_sha1(conn: &Connection, sha1: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM mod_version WHERE sha1 = ?1",
            params![sha1],
            |r| r.get(0),
        )
        .optional()?)
}

/// Insert a sourced assertion; de-duped by the (from, target, kind, source,
/// range) unique index, so a re-harvest adds nothing.
pub fn upsert_relation(
    conn: &Connection,
    from_mod_id: i64,
    target_modid: &str,
    version_range: Option<&str>,
    kind: RelKind,
    source: Source,
    now: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO relation
           (from_mod_id, target_modid, target_version_range, kind, source, confidence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            from_mod_id,
            target_modid,
            version_range,
            kind.as_str(),
            source.as_str(),
            source.rank(),
            now
        ],
    )?;
    Ok(())
}

pub fn upsert_pack(conn: &Connection, pack_id: &str, provenance: &str, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO pack (id, provenance, source, created_at, updated_at)
         VALUES (?1, ?2, 'harvested', ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET
           provenance = excluded.provenance, updated_at = excluded.updated_at
         WHERE pack.source NOT IN ('curator', 'authored')",
        params![pack_id, provenance, now],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn upsert_pack_build(
    conn: &Connection,
    pack_id: &str,
    pack_version: &str,
    mc_version: &str,
    loader_id: Option<&str>,
    loader_version: Option<&str>,
    java_major: Option<i64>,
    fingerprint: Option<&str>,
    is_latest: bool,
    now: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO pack_build
           (pack_id, pack_version, mc_version, loader_id, loader_version, java_major,
            fingerprint, is_latest, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'harvested', ?9)
         ON CONFLICT(pack_id, pack_version) DO UPDATE SET
           mc_version = excluded.mc_version,
           loader_id = excluded.loader_id,
           loader_version = excluded.loader_version,
           java_major = excluded.java_major,
           fingerprint = excluded.fingerprint,
           is_latest = excluded.is_latest
         WHERE pack_build.source NOT IN ('curator', 'authored')",
        params![
            pack_id,
            pack_version,
            mc_version,
            loader_id,
            loader_version,
            java_major,
            fingerprint,
            is_latest,
            now
        ],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM pack_build WHERE pack_id = ?1 AND pack_version = ?2",
        params![pack_id, pack_version],
        |r| r.get(0),
    )?)
}

pub fn link_build_mod(
    conn: &Connection,
    build_id: i64,
    mod_version_id: i64,
    filename: &str,
    required: bool,
    default_enabled: bool,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pack_build_mod
           (build_id, mod_version_id, filename, required, default_enabled, source)
         VALUES (?1, ?2, ?3, ?4, ?5, 'harvested')
         ON CONFLICT(build_id, mod_version_id) DO UPDATE SET
           filename = excluded.filename,
           required = excluded.required,
           default_enabled = excluded.default_enabled",
        params![
            build_id,
            mod_version_id,
            filename,
            required,
            default_enabled
        ],
    )?;
    Ok(())
}
