//! Read queries over the registry. Pure: each takes `&Connection`. Callers run
//! them inside `spawn_blocking` via `Registry::with_conn`.

use super::model::*;
use super::semver;
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

/// Every Modrinth project id some mod already owns (has a `modrinth` alias for).
/// Harvest uses this to skip the dependency-target slug lookup for a project that
/// is already resolvable -- a linked or Modrinth-native one needs no self-host
/// bridging -- so the lookup shrinks to nothing once a mirror is warm.
pub fn modrinth_project_aliases(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT external_key FROM mod_alias WHERE source = 'modrinth'")?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(rows)
}

/// Modrinth project ids whose owning mod has no environment flags recorded yet.
/// The scan refetches these projects so a mod that acquired its alias after the
/// fact (the self-host slug bridge) or predates the env columns still gets its
/// flags on the next harvest; the set shrinks to nothing once the registry is
/// warm.
pub fn modrinth_aliases_without_env(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT a.external_key
         FROM mod_alias a JOIN mods m ON m.id = a.mod_id
         WHERE a.source = 'modrinth' AND m.client_env IS NULL",
    )?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(rows)
}

/// Modrinth project ids whose mod already carries a forge `modid` alias. Harvest
/// uses this to skip re-fetching a Modrinth re-upload's jar: once the modid is
/// learned and stored as an alias, the mod is fully identified and the (bandwidth-
/// heavy) jar fetch never runs again for it.
pub fn modrinth_projects_with_modid(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT m.external_key
         FROM mod_alias m
         WHERE m.source = 'modrinth'
           AND EXISTS (
               SELECT 1 FROM mod_alias d
               WHERE d.mod_id = m.mod_id AND d.source = 'modid'
           )",
    )?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(rows)
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

/// The loaders a pack on `loader` natively runs: the loader itself plus every one
/// it inherits from, lowercased. cleanroom reaches forge and quilt reaches fabric
/// through the same `loader_parent` DAG eligibility uses -- a fork runs its
/// parent's artifacts by construction (#37).
pub fn loader_chain(conn: &Connection, loader: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE ancestors(id) AS (
            SELECT lower(?1)
            UNION
            SELECT lp.parent_id FROM loader_parent lp JOIN ancestors a ON lp.child_id = a.id
         )
         SELECT id FROM ancestors",
    )?;
    Ok(stmt
        .query_map(params![loader], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?)
}

/// The loaders one artifact suits. `any` marks a loader-agnostic jar; the harvest
/// guarantees at least one row, so an empty result means the artifact was never
/// read rather than that it suits nothing.
pub fn targets_for_artifact(conn: &Connection, mod_version_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT target FROM mod_version_target WHERE mod_version_id = ?1 ORDER BY target",
    )?;
    Ok(stmt
        .query_map(params![mod_version_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
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

/// Resolve a `relation.target_modid` selector to a mod row id. A bare value is a
/// `modid` alias; a `modrinth:<project_id>` value (the fallback the derivation
/// emits when a target carries no modid) is a Modrinth-project alias.
///
/// A Forge dependency string carries its version window inline as
/// `modid@[range]` (e.g. `forgemultipartcbe@[2.6,1,)`), and the same window can
/// ride a `modrinth:<id>` selector too. The alias tables key on the bare
/// identity, so the range is dropped before the lookup -- otherwise the same
/// dependency resolves through its clean-modid row and stays an unresolved
/// placeholder through its range-suffixed one, drawing the target twice (#1),
/// and a Modrinth-only target never resolves at all (#2). The version window is
/// held separately in `target_version_range` and is unaffected.
pub fn mod_id_for_selector(conn: &Connection, selector: &str) -> Result<Option<i64>> {
    let selector = selector.split('@').next().unwrap_or(selector);
    match selector.strip_prefix("modrinth:") {
        // Modrinth project ids are case-sensitive -- match exactly.
        Some(pid) => mod_id_for_alias(conn, "modrinth", pid),
        // A Forge modid is lowercase by spec, but a dependency string routinely
        // names it in display case (`JEI` for modid `jei`), so a dep would miss its
        // present provider on case alone. Match the modid alias case-insensitively.
        None => Ok(conn
            .query_row(
                "SELECT mod_id FROM mod_alias
                 WHERE source = 'modid' AND external_key = ?1 COLLATE NOCASE",
                params![selector],
                |r| r.get::<_, i64>(0),
            )
            .optional()?),
    }
}

/// The harvested artifact with this sha1: `(mod_version id, mod id, version)`.
/// The resolver uses it to place a self-hosted mod on the graph, read the version
/// it would ship for the version-window check, and scope the mod's relations to
/// the exact jar the pack ships rather than to every version of its mod (#48).
pub fn artifact_by_sha1(conn: &Connection, sha1: &str) -> Result<Option<(i64, i64, String)>> {
    Ok(conn
        .query_row(
            "SELECT id, mod_id, version FROM mod_version WHERE sha1 = ?1",
            params![sha1],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?)
}

/// The version string of a harvested Modrinth artifact, keyed by its Modrinth
/// version id (a pack pins a Modrinth mod by version id, not sha1).
pub fn version_by_modrinth_version_id(
    conn: &Connection,
    version_id: &str,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT version FROM mod_version WHERE modrinth_version_id = ?1 LIMIT 1",
            params![version_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The `mod_version` row id of a harvested Modrinth artifact, keyed by its
/// Modrinth version id. The resolver needs the artifact itself, not just its
/// version string, to scope the artifact's relations (#48).
pub fn mod_version_id_for_modrinth_version_id(
    conn: &Connection,
    version_id: &str,
) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM mod_version WHERE modrinth_version_id = ?1 LIMIT 1",
            params![version_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Every relation that applies to one artifact: the edges derived from that jar,
/// plus the mod-level facts asserted about its mod (#48). Pass a `mod_version_id`
/// that matches nothing (the artifact was never harvested) to get the mod-level
/// facts alone -- which is the honest answer for a jar we have never read, rather
/// than lending it a sibling version's dependencies.
///
/// Ordered by `confidence` descending like [`relations_from`], so the caller still
/// reads the authoritative edge per target first.
pub fn relations_for_artifact(
    conn: &Connection,
    mod_version_id: i64,
    from_mod_id: i64,
) -> Result<Vec<RelationRow>> {
    let mut stmt = conn.prepare(
        "SELECT target_modid, target_version_range, kind, source, confidence
         FROM relation
         WHERE from_mod_version_id = ?1
            OR (from_mod_version_id IS NULL AND from_mod_id = ?2)
         ORDER BY confidence DESC, id",
    )?;
    let mut rows = stmt.query(params![mod_version_id, from_mod_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let kind: String = r.get(2)?;
        let source: String = r.get(3)?;
        let (Some(kind), Some(source)) = (RelKind::parse(&kind), Source::parse(&source)) else {
            continue;
        };
        out.push(RelationRow {
            target: r.get(0)?,
            version_range: r.get(1)?,
            kind,
            source,
            confidence: r.get(4)?,
        });
    }
    Ok(out)
}

/// Every edge out of `from_mod_id` in the dependency graph, for the resolver.
/// Ordered by `confidence` (the stored source rank) descending so the caller
/// reads the authoritative edge per target first -- an authored/curator fact
/// outranks a jar-meta declaration, which outranks a bytecode inference. A row
/// whose `kind`/`source` cell is unrecognised is skipped, not fatal.
pub fn relations_from(conn: &Connection, from_mod_id: i64) -> Result<Vec<RelationRow>> {
    let mut stmt = conn.prepare(
        "SELECT target_modid, target_version_range, kind, source, confidence
         FROM relation
         WHERE from_mod_id = ?1
         ORDER BY confidence DESC, id",
    )?;
    let mut rows = stmt.query(params![from_mod_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let kind: String = r.get(2)?;
        let source: String = r.get(3)?;
        let (Some(kind), Some(source)) = (RelKind::parse(&kind), Source::parse(&source)) else {
            continue;
        };
        out.push(RelationRow {
            target: r.get(0)?,
            version_range: r.get(1)?,
            kind,
            source,
            confidence: r.get(4)?,
        });
    }
    Ok(out)
}

/// For the self-hosted jar `sha1`, its mod's Modrinth `(project_id, version_id)`
/// counterpart to diff against: the mod's Modrinth project alias plus any sibling
/// file that carries a `modrinth_version_id` (the genuine build). `None` when the
/// jar's mod is not Modrinth-known here or has no genuine sibling to compare with.
pub fn repack_counterpart(conn: &Connection, sha1: &str) -> Result<Option<(String, String)>> {
    Ok(conn
        .query_row(
            "SELECT a.external_key, sib.modrinth_version_id
             FROM mod_version mv
             JOIN mod_alias a ON a.mod_id = mv.mod_id AND a.source = 'modrinth'
             JOIN mod_version sib ON sib.mod_id = mv.mod_id
                                 AND sib.modrinth_version_id IS NOT NULL
             WHERE mv.sha1 = ?1
             LIMIT 1",
            params![sha1],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// The whole relation graph for the graph view: every relation as an edge (its
/// target resolved to a mod id when known), and every mod that is an endpoint of
/// one as a node. Isolated mods (no relation) are deliberately omitted -- the
/// view is of the dependency/conflict graph, not the full mod list. The registry
/// is single-operator-sized, so building it in Rust is fine.
pub fn graph(conn: &Connection) -> Result<GraphData> {
    let raw: Vec<(i64, String, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT from_mod_id, target_modid, kind, source FROM relation
             ORDER BY from_mod_id, target_modid, kind",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut node_ids: BTreeSet<i64> = BTreeSet::new();
    let mut edges = Vec::with_capacity(raw.len());
    for (from, target, kind, source) in raw {
        let to = mod_id_for_selector(conn, &target)?;
        node_ids.insert(from);
        if let Some(t) = to {
            node_ids.insert(t);
        }
        edges.push(GraphEdge {
            from_mod_id: from,
            to_mod_id: to,
            target,
            kind,
            source,
        });
    }

    let mut nodes = Vec::with_capacity(node_ids.len());
    for id in node_ids {
        nodes.push(graph_node_for(conn, id)?);
    }
    Ok(GraphData { nodes, edges })
}

/// One mod as a graph node: the name resolved server-side (canonical -> slug ->
/// modid -> `#id`) and whether it carries a Modrinth identity, so the view can
/// mark a genuine identity apart from a bare-modid one.
pub fn graph_node_for(conn: &Connection, id: i64) -> Result<GraphNode> {
    let (canonical, slug): (Option<String>, Option<String>) = conn.query_row(
        "SELECT canonical_name, slug FROM mods WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let modid = modid_for_mod(conn, id)?;
    let modrinth = modrinth_id_for_mod(conn, id)?.is_some();
    let name = canonical
        .or(slug)
        .or_else(|| modid.clone())
        .unwrap_or_else(|| format!("#{id}"));
    Ok(GraphNode {
        mod_id: id,
        name,
        modid,
        modrinth,
    })
}

/// Channel preference when picking which artifact represents a mod in a slice: a
/// stable release outranks a prerelease, which outranks a dev build.
fn channel_rank(channel: &str) -> i32 {
    match channel {
        "release" => 3,
        "beta" => 2,
        "dev" => 1,
        _ => 0,
    }
}

/// The (Minecraft version, loader) worlds the registry actually holds, busiest
/// first, so the graph can offer real choices and open on one that has something
/// in it (#49). Only concrete loader targets make a world -- a loader-agnostic
/// `any` artifact belongs to whichever loader is picked, so it never invents one.
pub fn graph_slices(conn: &Connection) -> Result<Vec<GraphSlice>> {
    let mut stmt = conn.prepare(
        "SELECT mv.mc_versions, t.target
         FROM mod_version mv
         JOIN mod_version_target t ON t.mod_version_id = mv.id
         WHERE mv.mc_versions IS NOT NULL AND t.target <> 'any'",
    )?;
    let mut counts: HashMap<(String, String), i64> = HashMap::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let target: String = r.get(1)?;
        for mc in decode_mc(r.get(0)?) {
            *counts.entry((mc, target.to_lowercase())).or_default() += 1;
        }
    }
    let mut out: Vec<GraphSlice> = counts
        .into_iter()
        .map(|((mc_version, loader), artifacts)| GraphSlice {
            mc_version,
            loader,
            artifacts,
        })
        .collect();
    // busiest first; ties settle on the newer Minecraft version, then the name, so
    // the default the panel picks is stable across calls
    out.sort_by(|a, b| {
        b.artifacts
            .cmp(&a.artifacts)
            .then_with(|| mc_version_key(&b.mc_version).cmp(&mc_version_key(&a.mc_version)))
            .then_with(|| a.loader.cmp(&b.loader))
    });
    Ok(out)
}

/// The one artifact that represents each mod in a (Minecraft version, loader)
/// slice: the mod's latest build that suits this world.
///
/// "Latest" is decided conservatively: a better channel wins first, then the
/// version where both are plainly comparable. Mod version strings routinely are
/// not (`rv6-stable-8`, `1.12.2-4.1.0`), so where they cannot be read the newest
/// harvested row wins -- a proxy, but a deterministic one, and it never pretends
/// to have understood a version it could not parse.
///
/// The loader match is fork-aware: cleanroom reaches forge through `loader_parent`
/// exactly as `eligible_for_loader` does, so selecting a fork sees the artifacts it
/// can actually run (#37). A `None` axis means "do not narrow on it".
fn slice_artifacts(
    conn: &Connection,
    mc: Option<&str>,
    loader: Option<&str>,
) -> Result<HashMap<i64, i64>> {
    let mc_like = mc.map(|s| format!("%\"{}\"%", like_escape(s)));
    let mut stmt = conn.prepare(
        "WITH RECURSIVE ancestors(id) AS (
            SELECT lower(?1) WHERE ?1 IS NOT NULL
            UNION
            SELECT lp.parent_id FROM loader_parent lp JOIN ancestors a ON lp.child_id = a.id
         )
         SELECT mv.mod_id, mv.id, mv.version, COALESCE(r.channel, 'unknown')
         FROM mod_version mv
         LEFT JOIN mod_release r ON r.id = mv.release_id
         WHERE (?1 IS NULL OR EXISTS (
                 SELECT 1 FROM mod_version_target t
                 WHERE t.mod_version_id = mv.id
                   AND (t.target = 'any' OR lower(t.target) IN (SELECT id FROM ancestors))))
           AND (?2 IS NULL OR mv.mc_versions LIKE ?3 ESCAPE '\\')
         ORDER BY mv.mod_id, mv.id",
    )?;
    // mod_id -> the winning (artifact id, version, channel rank)
    let mut best: HashMap<i64, (i64, String, i32)> = HashMap::new();
    let mut rows = stmt.query(params![loader, mc, mc_like])?;
    while let Some(r) = rows.next()? {
        let mod_id: i64 = r.get(0)?;
        let mv_id: i64 = r.get(1)?;
        let version: String = r.get(2)?;
        let rank = channel_rank(&r.get::<_, String>(3)?);
        match best.get(&mod_id) {
            None => {
                best.insert(mod_id, (mv_id, version, rank));
            }
            Some((cur_id, cur_ver, cur_rank)) => {
                let wins = match rank.cmp(cur_rank) {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Less => false,
                    std::cmp::Ordering::Equal => match semver::cmp(&version, cur_ver) {
                        Some(std::cmp::Ordering::Greater) => true,
                        Some(_) => false,
                        // unreadable on either side: fall back to harvest order
                        None => mv_id > *cur_id,
                    },
                };
                if wins {
                    best.insert(mod_id, (mv_id, version, rank));
                }
            }
        }
    }
    Ok(best.into_iter().map(|(m, (mv, _, _))| (m, mv)).collect())
}

/// The relation graph for one (Minecraft version, loader) world (#49).
///
/// The unsliced graph is a union over every version of every mod, which stops
/// meaning anything once the registry holds more than one world: a pack's mods
/// never meet mods from another Minecraft version, and their edges have nothing to
/// do with each other. Here each mod contributes the one artifact that suits the
/// slice, and the edges drawn are that artifact's -- a statement that can be
/// defended.
///
/// A target that resolves to a mod with nothing in this slice stays unresolved, so
/// it renders as an external leaf: within this world, that requirement is not met
/// by anything the registry holds.
pub fn graph_for_slice(
    conn: &Connection,
    mc: Option<&str>,
    loader: Option<&str>,
) -> Result<GraphData> {
    let in_slice = slice_artifacts(conn, mc, loader)?;

    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut endpoints: BTreeSet<i64> = BTreeSet::new();
    // The same dependency can arrive as several relation rows -- a clean modid and
    // the Forge `modid@[range]` form, or one row per source. Once resolved they
    // point at the same node, so dedupe (relations come highest-confidence first,
    // so the authoritative row wins): a resolved edge is keyed by its endpoints
    // and kind; an unresolved one keeps its raw target so two genuinely different
    // missing deps stay distinct.
    let mut seen: HashSet<(i64, Option<i64>, String, String)> = HashSet::new();
    // deterministic output: walk the mods in id order, not hash order
    let mut mods: Vec<(&i64, &i64)> = in_slice.iter().collect();
    mods.sort();
    for (&mod_id, &mv_id) in mods {
        for e in relations_for_artifact(conn, mv_id, mod_id)? {
            let to = mod_id_for_selector(conn, &e.target)?.filter(|t| in_slice.contains_key(t));
            let kind = e.kind.as_str().to_string();
            let key = (
                mod_id,
                to,
                kind.clone(),
                if to.is_some() {
                    String::new()
                } else {
                    e.target.clone()
                },
            );
            if !seen.insert(key) {
                continue;
            }
            endpoints.insert(mod_id);
            if let Some(t) = to {
                endpoints.insert(t);
            }
            edges.push(GraphEdge {
                from_mod_id: mod_id,
                to_mod_id: to,
                target: e.target,
                kind,
                source: e.source.as_str().to_string(),
            });
        }
    }

    // like `graph`, this is the relation graph: a mod with no edge in this slice is
    // not drawn as a lone node
    let mut nodes = Vec::with_capacity(endpoints.len());
    for id in endpoints {
        nodes.push(graph_node_for(conn, id)?);
    }
    Ok(GraphData { nodes, edges })
}

/// A mod's display name, resolved the same way everywhere: canonical_name ->
/// slug -> modid -> `#<id>`.
fn mod_name(conn: &Connection, mod_id: i64) -> Result<String> {
    let (canonical, slug): (Option<String>, Option<String>) = conn.query_row(
        "SELECT canonical_name, slug FROM mods WHERE id = ?1",
        params![mod_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let modid = modid_for_mod(conn, mod_id)?;
    Ok(canonical
        .or(slug)
        .or(modid)
        .unwrap_or_else(|| format!("#{mod_id}")))
}

/// Every relation touching `mod_id`, in both directions, resolved for the mod
/// page. Outgoing edges name the target (resolved to a mod when catalogued);
/// incoming edges are relations whose selector resolves to this mod (matched on
/// its `modid`/`modrinth` alias keys), naming the referencing mod.
pub fn edges_for_mod(conn: &Connection, mod_id: i64) -> Result<Vec<ModEdge>> {
    let mut out = Vec::new();
    // The clean modid and the Forge `modid@[range]` form of one dependency both
    // resolve to the same target now, and a mod that references this one through
    // several aliases matches more than once, so an edge could list the same
    // neighbour twice. Collapse by direction, resolved target, name and kind.
    let mut seen: HashSet<(&str, Option<i64>, String, String)> = HashSet::new();

    // outgoing: this mod -> target
    {
        let mut stmt = conn.prepare(
            "SELECT target_modid, kind, source FROM relation
             WHERE from_mod_id = ?1 ORDER BY kind, target_modid",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![mod_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (target, kind, source) in rows {
            let other_mod_id = mod_id_for_selector(conn, &target)?;
            let other_name = match other_mod_id {
                Some(id) => mod_name(conn, id)?,
                None => target.clone(),
            };
            if !seen.insert(("out", other_mod_id, other_name.clone(), kind.clone())) {
                continue;
            }
            out.push(ModEdge {
                dir: "out".into(),
                other_mod_id,
                other_name,
                kind,
                source,
            });
        }
    }

    // incoming: other -> this mod. Match on the selectors that resolve here: each
    // `modid` alias verbatim, and each `modrinth` alias as `modrinth:<id>`.
    let mut selectors: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT external_key FROM mod_alias WHERE mod_id = ?1 AND source = 'modid'")?;
        stmt.query_map(params![mod_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?
    };
    if let Some(pid) = modrinth_id_for_mod(conn, mod_id)? {
        selectors.push(format!("modrinth:{pid}"));
    }
    for sel in selectors {
        // Also match the Forge `<sel>@[range]` form of the same reference. GLOB is
        // used rather than LIKE because a modid can contain `_`, which LIKE would
        // treat as a wildcard; GLOB's metacharacters (`*?[`) do not occur in a
        // modid or `modrinth:<id>` selector. The in-edge dedupe collapses a mod
        // that references this one both ways into one incoming edge (#1 tail).
        let mut stmt = conn.prepare(
            "SELECT from_mod_id, kind, source FROM relation
             WHERE (target_modid = ?1 OR target_modid GLOB ?1 || '@*')
               AND from_mod_id <> ?2
             ORDER BY kind, from_mod_id",
        )?;
        let rows: Vec<(i64, String, String)> = stmt
            .query_map(params![sel, mod_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (from, kind, source) in rows {
            let other_name = mod_name(conn, from)?;
            if !seen.insert(("in", Some(from), other_name.clone(), kind.clone())) {
                continue;
            }
            out.push(ModEdge {
                dir: "in".into(),
                other_mod_id: Some(from),
                other_name,
                kind,
                source,
            });
        }
    }

    Ok(out)
}

/// The aggregated read model for one mod's page (`None` if the id is unknown).
/// `used_by` is returned unfiltered here; the public endpoint narrows it to
/// official + published packs. File `cached` flags are set by the handler against
/// the live cache, as elsewhere.
pub fn mod_detail(conn: &Connection, mod_id: i64) -> Result<Option<ModDetail>> {
    let identity: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT canonical_name, slug, author FROM mods WHERE id = ?1",
            params![mod_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((canonical, slug, author)) = identity else {
        return Ok(None);
    };

    let modid = modid_for_mod(conn, mod_id)?;
    let modrinth_project_id = modrinth_id_for_mod(conn, mod_id)?;
    let name = canonical
        .or_else(|| slug.clone())
        .or_else(|| modid.clone())
        .unwrap_or_else(|| format!("#{mod_id}"));

    let releases = releases_of_mod_by_id(conn, mod_id)?;

    // facets folded across every file rather than re-queried
    let mut loaders: BTreeSet<String> = BTreeSet::new();
    let mut mc: BTreeSet<String> = BTreeSet::new();
    for rel in &releases {
        for f in &rel.files {
            for t in &f.targets {
                loaders.insert(t.clone());
            }
            for v in &f.mc_versions {
                mc.insert(v.clone());
            }
        }
    }
    let mut mc_versions: Vec<String> = mc.into_iter().collect();
    sort_mc(&mut mc_versions);

    let edges = edges_for_mod(conn, mod_id)?;
    let used_by = packs_using_mod_by_id(conn, mod_id)?;

    Ok(Some(ModDetail {
        mod_id,
        name,
        slug,
        author,
        modid,
        modrinth_project_id,
        loaders: loaders.into_iter().collect(),
        mc_versions,
        releases,
        edges,
        used_by,
    }))
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

/// Which packs ship any version of this mod, one row per pack, resolved straight
/// from the mod id rather than through an alias. `mod_detail` keyed `used_by` on
/// the `modid` alias, which a Modrinth-sourced mod (no modid alias) never has, so
/// a mod that packs do ship reported "used in no pack" (#18). A build references
/// the artifact by `mod_version_id`, whose `mod_id` is the honest join.
///
/// One row per pack, carrying its latest build: a mod that rides every snapshot
/// of a pack is used by that one pack, not by it forty times over -- the page
/// answers "which packs", not "which builds". `MAX(pack_version)` picks the
/// latest, and SQLite reads the bare columns from that same winning row.
pub fn packs_using_mod_by_id(conn: &Connection, mod_id: i64) -> Result<Vec<ModUse>> {
    let mut stmt = conn.prepare(
        "SELECT pb.pack_id, MAX(pb.pack_version), mv.version, pbm.filename
         FROM mod_version mv
         JOIN pack_build_mod pbm ON pbm.mod_version_id = mv.id
         JOIN pack_build pb ON pb.id = pbm.build_id
         WHERE mv.mod_id = ?1
         GROUP BY pb.pack_id
         ORDER BY pb.pack_id",
    )?;
    let rows = stmt
        .query_map(params![mod_id], |r| {
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
                (SELECT count(*) FROM mod_version mv WHERE mv.mod_id = m.id) AS vcount,
                (SELECT external_key FROM mod_alias WHERE mod_id = m.id AND source = 'modrinth' LIMIT 1) AS modrinth_pid,
                (SELECT sha1 FROM mod_version mv WHERE mv.mod_id = m.id ORDER BY mv.id DESC LIMIT 1) AS icon_sha1
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
            let modrinth_project_id: Option<String> = r.get(6)?;
            let icon_sha1: Option<String> = r.get(7)?;
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
                modrinth_project_id,
                icon_sha1,
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
