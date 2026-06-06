//! Read queries over the registry. Pure: each takes `&Connection`. Callers run
//! them inside `spawn_blocking` via `Registry::with_conn`.

use super::model::*;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// The `mod` row id owning the artifact with this sha1, if harvested.
pub fn mod_id_for_sha1(conn: &Connection, sha1: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT mod_id FROM mod_version WHERE sha1 = ?1",
            params![sha1],
            |r| r.get(0),
        )
        .optional()?)
}

/// The `mod` row id for an external alias, if known.
pub fn mod_id_for_alias(
    conn: &Connection,
    alias_source: &str,
    external_key: &str,
) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT mod_id FROM mod_alias WHERE source = ?1 AND external_key = ?2",
            params![alias_source, external_key],
            |r| r.get(0),
        )
        .optional()?)
}

/// Q1 -- which pack builds ship the mod identified by `(alias_source, key)`.
pub fn packs_using_mod(
    conn: &Connection,
    alias_source: &str,
    external_key: &str,
) -> Result<Vec<ModUse>> {
    let mut stmt = conn.prepare(
        "SELECT pb.pack_id, pb.pack_version, mv.version, pbm.filename
         FROM mod_alias a
         JOIN mod_version mv ON mv.mod_id = a.mod_id
         JOIN pack_build_mod pbm ON pbm.mod_version_id = mv.id
         JOIN pack_build pb ON pb.id = pbm.build_id
         WHERE a.source = ?1 AND a.external_key = ?2
         ORDER BY pb.pack_id, pb.pack_version",
    )?;
    let rows = stmt
        .query_map(params![alias_source, external_key], |r| {
            Ok(ModUse {
                pack_id: r.get(0)?,
                pack_version: r.get(1)?,
                version: r.get(2)?,
                filename: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Q2 -- artifacts on disk (in `mod_version`) no build references.
pub fn orphan_jars(conn: &Connection) -> Result<Vec<OrphanJar>> {
    let mut stmt = conn.prepare(
        "SELECT mv.sha1, mv.size_bytes, mv.filename
         FROM mod_version mv
         LEFT JOIN pack_build_mod pbm ON pbm.mod_version_id = mv.id
         WHERE pbm.build_id IS NULL
         ORDER BY mv.sha1",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(OrphanJar {
                sha1: r.get(0)?,
                size_bytes: r.get(1)?,
                filename: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Q3 -- all versions of the mod identified by `(alias_source, key)`, each with
/// its full target set folded in from `mod_version_target`.
pub fn versions_of_mod(
    conn: &Connection,
    alias_source: &str,
    external_key: &str,
) -> Result<Vec<VersionRow>> {
    let mut stmt = conn.prepare(
        "SELECT mv.id, mv.version, mv.sha1, mv.size_bytes, mv.source, mvt.target
         FROM mod_alias a
         JOIN mod_version mv ON mv.mod_id = a.mod_id
         LEFT JOIN mod_version_target mvt ON mvt.mod_version_id = mv.id
         WHERE a.source = ?1 AND a.external_key = ?2
         ORDER BY mv.version, mv.id, mvt.target",
    )?;
    // rows for one artifact are contiguous (ORDER BY mv.id); fold targets in
    let mut out: Vec<VersionRow> = Vec::new();
    let mut cur_id: Option<i64> = None;
    let mut rows = stmt.query(params![alias_source, external_key])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let target: Option<String> = r.get(5)?;
        if cur_id != Some(id) {
            cur_id = Some(id);
            out.push(VersionRow {
                version: r.get(1)?,
                targets: Vec::new(),
                sha1: r.get(2)?,
                size_bytes: r.get(3)?,
                source: r.get(4)?,
            });
        }
        if let Some(t) = target {
            out.last_mut().unwrap().targets.push(t);
        }
    }
    Ok(out)
}

/// Q4 -- artifacts eligible for a build whose loader is `loader`. An artifact is
/// eligible iff one of its targets is `any`, equals `loader`, or is an ancestor
/// `loader` inherits through the `loader_parent` family DAG. Each eligible
/// artifact reports its best-match `specificity` (the most specific of its
/// targets) and the result is ordered most-specific first per mod, so the caller
/// picks the first row per `mod_id`.
pub fn eligible_for_loader(conn: &Connection, loader: &str) -> Result<Vec<EligibleArtifact>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE ancestors(id) AS (
            SELECT ?1
            UNION
            SELECT lp.parent_id FROM loader_parent lp
            JOIN ancestors anc ON lp.child_id = anc.id
         )
         SELECT mv.mod_id, mv.version, mv.sha1,
                MIN(CASE WHEN mvt.target = ?1 THEN 0
                         WHEN mvt.target = 'any' THEN 2
                         ELSE 1 END) AS specificity
         FROM mod_version mv
         JOIN mod_version_target mvt ON mvt.mod_version_id = mv.id
         WHERE mvt.target = 'any' OR mvt.target IN (SELECT id FROM ancestors)
         GROUP BY mv.id
         ORDER BY mv.mod_id, specificity",
    )?;
    let rows = stmt
        .query_map(params![loader], |r| {
            Ok(EligibleArtifact {
                mod_id: r.get(0)?,
                version: r.get(1)?,
                sha1: r.get(2)?,
                specificity: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn stats(conn: &Connection) -> Result<RegistryStats> {
    let count = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    Ok(RegistryStats {
        mods: count("SELECT count(*) FROM mods")?,
        mod_versions: count("SELECT count(*) FROM mod_version")?,
        relations: count("SELECT count(*) FROM relation")?,
        packs: count("SELECT count(*) FROM pack")?,
        builds: count("SELECT count(*) FROM pack_build")?,
        orphans: count(
            "SELECT count(*) FROM mod_version mv
             LEFT JOIN pack_build_mod pbm ON pbm.mod_version_id = mv.id
             WHERE pbm.build_id IS NULL",
        )?,
    })
}
