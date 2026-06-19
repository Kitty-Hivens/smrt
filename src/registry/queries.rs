//! Read queries over the registry. Pure: each takes `&Connection`. Callers run
//! them inside `spawn_blocking` via `Registry::with_conn`.

use super::model::*;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeSet, HashMap};

/// Decode a `mod_version.mc_versions` cell (a JSON array of strings, or NULL)
/// into a plain vec. Tolerant: a NULL or unparseable cell yields an empty vec.
fn decode_mc(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

/// Escape the `LIKE` metacharacters in an operator's search value so a literal
/// `%` or `_` matches itself rather than acting as a wildcard. Pair with
/// `ESCAPE '\'` on the clause.
fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

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

/// Q3 -- all versions of the mod identified by `(alias_source, key)`. Resolves
/// the alias to its surrogate id, then defers to [`versions_of_mod_by_id`].
pub fn versions_of_mod(
    conn: &Connection,
    alias_source: &str,
    external_key: &str,
) -> Result<Vec<VersionRow>> {
    match mod_id_for_alias(conn, alias_source, external_key)? {
        Some(id) => versions_of_mod_by_id(conn, id),
        None => Ok(Vec::new()),
    }
}

/// All artifacts of one mod (by surrogate id), each with its full target set and
/// Minecraft-version set folded in. The picker browses by id (a mod may carry
/// several aliases).
pub fn versions_of_mod_by_id(conn: &Connection, mod_id: i64) -> Result<Vec<VersionRow>> {
    let mut stmt = conn.prepare(
        "SELECT mv.id, mv.version, mv.sha1, mv.size_bytes, mv.source, mv.filename,
                mv.mc_versions, mv.modrinth_version_id,
                (SELECT external_key FROM mod_alias WHERE mod_id = mv.mod_id AND source = 'modrinth' LIMIT 1) AS mr_project,
                mvt.target
         FROM mod_version mv
         LEFT JOIN mod_version_target mvt ON mvt.mod_version_id = mv.id
         WHERE mv.mod_id = ?1
         ORDER BY mv.version, mv.id, mvt.target",
    )?;
    // rows for one artifact are contiguous (ORDER BY mv.id); fold targets in
    let mut out: Vec<VersionRow> = Vec::new();
    let mut cur_id: Option<i64> = None;
    let mut rows = stmt.query(params![mod_id])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let target: Option<String> = r.get(9)?;
        if cur_id != Some(id) {
            cur_id = Some(id);
            out.push(VersionRow {
                version: r.get(1)?,
                targets: Vec::new(),
                mc_versions: decode_mc(r.get(6)?),
                sha1: r.get(2)?,
                size_bytes: r.get(3)?,
                filename: r.get(5)?,
                source: r.get(4)?,
                cached: false, // set by the handler against the live cache
                modrinth_version_id: r.get(7)?,
                modrinth_project_id: r.get(8)?,
            });
        }
        if let Some(t) = target {
            out.last_mut().unwrap().targets.push(t);
        }
    }
    Ok(out)
}

/// Registry browser: mods matching an optional name query, narrowed to an
/// optional loader (the loader itself or a loader-agnostic `any` artifact) and/or
/// an optional Minecraft version. Each row carries the facets aggregated across
/// the mod's artifacts so the panel can show loader/mc chips without a per-mod
/// round-trip.
pub fn list_mods(
    conn: &Connection,
    q: Option<&str>,
    loader: Option<&str>,
    mc: Option<&str>,
) -> Result<Vec<ModSummary>> {
    // facet maps over the whole registry: mod_id -> its loader / mc sets. Folded
    // in Rust because mc_versions is JSON; the registry is single-operator-sized.
    let mut loaders_by_mod: HashMap<i64, BTreeSet<String>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT mv.mod_id, t.target
             FROM mod_version mv JOIN mod_version_target t ON t.mod_version_id = mv.id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let t: String = r.get(1)?;
            loaders_by_mod.entry(id).or_default().insert(t);
        }
    }
    let mut mc_by_mod: HashMap<i64, BTreeSet<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT mod_id, mc_versions FROM mod_version WHERE mc_versions IS NOT NULL")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let set = mc_by_mod.entry(id).or_default();
            for v in decode_mc(r.get(1)?) {
                set.insert(v);
            }
        }
    }

    let q_like = q.map(|s| format!("%{}%", like_escape(s)));
    let mc_like = mc.map(|s| format!("%\"{}\"%", like_escape(s)));
    // loader matches the family DAG, not just the exact id: a cleanroom/quilt
    // pack can use forge/fabric artifacts, so the filter accepts the loader, its
    // `loader_parent` ancestors, or `any` -- the same reachability eligible_for_
    // loader uses. Seeded case-insensitively so a pack's free-text "Forge" hits
    // the registry's "forge" target.
    let mut stmt = conn.prepare(
        "WITH RECURSIVE ancestors(id) AS (
            SELECT lower(?2) WHERE ?2 IS NOT NULL
            UNION
            SELECT lp.parent_id FROM loader_parent lp JOIN ancestors a ON lp.child_id = a.id
         )
         SELECT m.id, m.canonical_name, m.slug, m.author,
                (SELECT external_key FROM mod_alias WHERE mod_id = m.id AND source = 'modid' LIMIT 1) AS modid,
                (SELECT count(*) FROM mod_version mv WHERE mv.mod_id = m.id) AS vcount
         FROM mods m
         WHERE (?1 IS NULL
                OR m.canonical_name LIKE ?1 ESCAPE '\\' OR m.slug LIKE ?1 ESCAPE '\\'
                OR EXISTS (SELECT 1 FROM mod_alias a WHERE a.mod_id = m.id AND a.external_key LIKE ?1 ESCAPE '\\'))
           AND (?2 IS NULL OR EXISTS (
                 SELECT 1 FROM mod_version mv JOIN mod_version_target t ON t.mod_version_id = mv.id
                 WHERE mv.mod_id = m.id
                   AND (t.target = 'any' OR lower(t.target) IN (SELECT id FROM ancestors))))
           AND (?3 IS NULL OR EXISTS (
                 SELECT 1 FROM mod_version mv
                 WHERE mv.mod_id = m.id AND mv.mc_versions LIKE ?4 ESCAPE '\\'))
         ORDER BY lower(COALESCE(m.canonical_name, m.slug, '')), m.id",
    )?;
    let rows = stmt
        .query_map(params![q_like, loader, mc, mc_like], |r| {
            let id: i64 = r.get(0)?;
            let canonical: Option<String> = r.get(1)?;
            let slug: Option<String> = r.get(2)?;
            let author: Option<String> = r.get(3)?;
            let modid: Option<String> = r.get(4)?;
            let version_count: i64 = r.get(5)?;
            let name = canonical
                .clone()
                .or_else(|| slug.clone())
                .or(modid)
                .unwrap_or_else(|| format!("#{id}"));
            Ok(ModSummary {
                mod_id: id,
                name,
                slug,
                author,
                loaders: Vec::new(),
                mc_versions: Vec::new(),
                version_count,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows
        .into_iter()
        .map(|mut m| {
            m.loaders = loaders_by_mod
                .get(&m.mod_id)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            m.mc_versions = mc_by_mod
                .get(&m.mod_id)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            m
        })
        .collect())
}

/// Registry browser: every published build, newest/latest first per pack, with
/// its mod count.
pub fn list_builds(conn: &Connection) -> Result<Vec<BuildSummary>> {
    let mut stmt = conn.prepare(
        "SELECT pb.pack_id, pb.pack_version, pb.mc_version, pb.loader_id, pb.loader_version,
                pb.java_major, pb.is_latest,
                (SELECT count(*) FROM pack_build_mod pbm WHERE pbm.build_id = pb.id) AS mod_count
         FROM pack_build pb
         ORDER BY pb.pack_id, pb.is_latest DESC, pb.pack_version DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(BuildSummary {
                pack_id: r.get(0)?,
                pack_version: r.get(1)?,
                mc_version: r.get(2)?,
                loader_id: r.get(3)?,
                loader_version: r.get(4)?,
                java_major: r.get(5)?,
                is_latest: r.get::<_, i64>(6)? != 0,
                mod_count: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Registry browser: the mods a given build ships, each resolved to its artifact
/// (sha1) so the operator can re-add one -- or all -- into another pack.
pub fn build_mods(conn: &Connection, pack_id: &str, pack_version: &str) -> Result<Vec<BuildModRow>> {
    let mut stmt = conn.prepare(
        "SELECT m.canonical_name, m.slug,
                (SELECT external_key FROM mod_alias WHERE mod_id = m.id AND source = 'modid' LIMIT 1) AS modid,
                mv.version, mv.sha1, pbm.filename, mv.size_bytes,
                pbm.required, pbm.default_enabled, mv.mc_versions,
                mv.modrinth_version_id,
                (SELECT external_key FROM mod_alias WHERE mod_id = m.id AND source = 'modrinth' LIMIT 1) AS mr_project,
                t.target
         FROM pack_build pb
         JOIN pack_build_mod pbm ON pbm.build_id = pb.id
         JOIN mod_version mv ON mv.id = pbm.mod_version_id
         JOIN mods m ON m.id = mv.mod_id
         LEFT JOIN mod_version_target t ON t.mod_version_id = mv.id
         WHERE pb.pack_id = ?1 AND pb.pack_version = ?2
         ORDER BY pbm.filename, t.target",
    )?;
    // rows for one mod (one filename within a build) are contiguous; fold targets
    let mut out: Vec<BuildModRow> = Vec::new();
    let mut cur: Option<String> = None;
    let mut rows = stmt.query(params![pack_id, pack_version])?;
    while let Some(r) = rows.next()? {
        let filename: String = r.get(5)?;
        let target: Option<String> = r.get(12)?;
        if cur.as_deref() != Some(filename.as_str()) {
            cur = Some(filename.clone());
            let canonical: Option<String> = r.get(0)?;
            let slug: Option<String> = r.get(1)?;
            let modid: Option<String> = r.get(2)?;
            let name = canonical.or(slug).or(modid).unwrap_or_else(|| filename.clone());
            out.push(BuildModRow {
                name,
                version: r.get(3)?,
                sha1: r.get(4)?,
                filename,
                size_bytes: r.get(6)?,
                required: r.get::<_, i64>(7)? != 0,
                default_enabled: r.get::<_, i64>(8)? != 0,
                targets: Vec::new(),
                mc_versions: decode_mc(r.get(9)?),
                cached: false, // set by the handler against the live cache
                modrinth_version_id: r.get(10)?,
                modrinth_project_id: r.get(11)?,
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
