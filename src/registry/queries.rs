//! Read queries over the registry. Pure: each takes `&Connection`. Callers run
//! them inside `spawn_blocking` via `Registry::with_conn`.

use super::model::*;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Decode a `mod_version.mc_versions` cell (a JSON array of strings, or NULL)
/// into a plain vec. Tolerant: a NULL or unparseable cell yields an empty vec.
fn decode_mc(raw: Option<String>) -> Vec<String> {
    let mut v = raw
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default();
    sort_mc(&mut v);
    v
}

/// Order Minecraft versions numerically -- 1.7.10 sorts below 1.10.2, not above
/// it the way a lexical compare would. Splits on '.', reading the leading digits
/// of each segment; a non-numeric segment sinks to the front and ties break on
/// the raw string so snapshots stay deterministic.
fn mc_version_key(v: &str) -> Vec<i64> {
    v.split('.')
        .map(|seg| {
            seg.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<i64>()
                .unwrap_or(-1)
        })
        .collect()
}

fn sort_mc(v: &mut [String]) {
    v.sort_by(|a, b| {
        mc_version_key(a)
            .cmp(&mc_version_key(b))
            .then_with(|| a.cmp(b))
    });
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

/// A mod's primary `modid` alias, used to fill a `relation.target_modid`
/// selector when the derivation knows the target only by its surrogate id.
pub fn modid_for_mod(conn: &Connection, mod_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT external_key FROM mod_alias WHERE mod_id = ?1 AND source = 'modid' LIMIT 1",
            params![mod_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// A mod's Modrinth project id, when it carries one. The fallback selector for a
/// derived edge whose target has no modid but is Modrinth-identified.
pub fn modrinth_id_for_mod(conn: &Connection, mod_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT external_key FROM mod_alias WHERE mod_id = ?1 AND source = 'modrinth' LIMIT 1",
            params![mod_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The single mod that owns a package prefix, or `None` when no mod or more than
/// one owns it. A multiply-owned prefix is an ambiguous shaded library, so it is
/// deliberately not resolved to an edge.
pub fn owner_mod_for_prefix(conn: &Connection, prefix: &str) -> Result<Option<i64>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT mod_id FROM mod_package WHERE prefix = ?1 LIMIT 2")?;
    let ids = stmt
        .query_map(params![prefix], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(if ids.len() == 1 { Some(ids[0]) } else { None })
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

/// The mod's files grouped under their release (version node) for the management
/// view: Mod -> Release (version_number + channel) -> Files (loader/mc/sha1).
/// Every file has a release_id post-migration, so an inner join is complete; a
/// file whose release was removed (release_id SET NULL) would be omitted, which
/// is acceptable until a delete-release path exists.
pub fn releases_of_mod_by_id(conn: &Connection, mod_id: i64) -> Result<Vec<ReleaseRow>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.version_number, r.channel, r.source,
                mv.id, mv.version, mv.sha1, mv.size_bytes, mv.source, mv.filename,
                mv.mc_versions, mv.modrinth_version_id,
                (SELECT external_key FROM mod_alias WHERE mod_id = mv.mod_id AND source = 'modrinth' LIMIT 1) AS mr_project,
                mvt.target
         FROM mod_version mv
         JOIN mod_release r ON r.id = mv.release_id
         LEFT JOIN mod_version_target mvt ON mvt.mod_version_id = mv.id
         WHERE mv.mod_id = ?1
         ORDER BY r.channel, r.version_number, r.id, mv.id, mvt.target",
    )?;
    // rows are ordered so a release's files, and a file's targets, are contiguous
    let mut out: Vec<ReleaseRow> = Vec::new();
    let mut cur_rel: Option<i64> = None;
    let mut cur_file: Option<i64> = None;
    let mut rows = stmt.query(params![mod_id])?;
    while let Some(row) = rows.next()? {
        let rid: i64 = row.get(0)?;
        let fid: i64 = row.get(4)?;
        let target: Option<String> = row.get(13)?;
        if cur_rel != Some(rid) {
            cur_rel = Some(rid);
            cur_file = None;
            out.push(ReleaseRow {
                release_id: rid,
                version_number: row.get(1)?,
                channel: row.get(2)?,
                source: row.get(3)?,
                files: Vec::new(),
            });
        }
        if cur_file != Some(fid) {
            cur_file = Some(fid);
            out.last_mut().unwrap().files.push(VersionRow {
                version: row.get(5)?,
                targets: Vec::new(),
                mc_versions: decode_mc(row.get(10)?),
                sha1: row.get(6)?,
                size_bytes: row.get(7)?,
                filename: row.get(9)?,
                source: row.get(8)?,
                cached: false, // set by the handler against the live cache
                modrinth_version_id: row.get(11)?,
                modrinth_project_id: row.get(12)?,
            });
        }
        if let Some(t) = target {
            out.last_mut()
                .unwrap()
                .files
                .last_mut()
                .unwrap()
                .targets
                .push(t);
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
                .map(|s| {
                    let mut v: Vec<String> = s.iter().cloned().collect();
                    sort_mc(&mut v);
                    v
                })
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
pub fn build_mods(
    conn: &Connection,
    pack_id: &str,
    pack_version: &str,
) -> Result<Vec<BuildModRow>> {
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
            let name = canonical
                .or(slug)
                .or(modid)
                .unwrap_or_else(|| filename.clone());
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

/// Every sha1 the registry has a `mod_version` row for. The handler diffs this
/// against the live cache inventory to surface jars on disk that carry no
/// identity yet -- the "needs identity" bucket the authoring UI works from
/// (harvest drops an aliasless jar, so it never gets a row).
pub fn all_mod_version_shas(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT sha1 FROM mod_version")?;
    let out = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<String>>>()?;
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::sort_mc;

    #[test]
    fn mc_versions_sort_numerically() {
        let mut v = ["1.10.2", "1.7.10", "1.12.2", "1.16.5", "1.8.9"]
            .map(String::from)
            .to_vec();
        sort_mc(&mut v);
        assert_eq!(v, ["1.7.10", "1.8.9", "1.10.2", "1.12.2", "1.16.5"]);
    }
}
