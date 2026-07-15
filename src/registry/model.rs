//! Plain row/result structs + the source/kind vocab. No I/O.

use serde::Serialize;
use ts_rs::TS;

/// Provenance of a fact. Harvested rows are rebuildable; authored/curator rows
/// are precious and never clobbered by a re-harvest. `rank` breaks per-fact
/// precedence ties (used by the Phase 4 resolver; stored now for ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Curator,
    Authored,
    JarMeta,
    Modrinth,
    Inferred,
    Harvested,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Curator => "curator",
            Source::Authored => "authored",
            Source::JarMeta => "jar-meta",
            Source::Modrinth => "modrinth",
            Source::Inferred => "inferred",
            Source::Harvested => "harvested",
        }
    }
    pub fn rank(self) -> i64 {
        match self {
            Source::Curator => 100,
            Source::Authored => 90,
            Source::JarMeta => 50,
            Source::Modrinth => 40,
            Source::Inferred | Source::Harvested => 10,
        }
    }
    /// True for rows a re-harvest must never overwrite.
    pub fn is_precious(self) -> bool {
        matches!(self, Source::Curator | Source::Authored)
    }
    /// Inverse of [`as_str`], for reading a stored `relation.source` back into the
    /// vocab. `None` for an unrecognised cell (a forward-compat guard, not a hard
    /// error -- an unknown source just drops out of the resolver's precedence).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "curator" => Source::Curator,
            "authored" => Source::Authored,
            "jar-meta" => Source::JarMeta,
            "modrinth" => Source::Modrinth,
            "inferred" => Source::Inferred,
            "harvested" => Source::Harvested,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelKind {
    Requires,
    Conflicts,
    OptionalDep,
    Provides,
    Recommends,
    Breaks,
}

impl RelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RelKind::Requires => "requires",
            RelKind::Conflicts => "conflicts",
            RelKind::OptionalDep => "optional_dep",
            RelKind::Provides => "provides",
            RelKind::Recommends => "recommends",
            RelKind::Breaks => "breaks",
        }
    }
    /// Inverse of [`as_str`], for reading a stored `relation.kind` back into the
    /// vocab. `None` for an unrecognised cell (see [`Source::parse`]).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "requires" => RelKind::Requires,
            "conflicts" => RelKind::Conflicts,
            "optional_dep" => RelKind::OptionalDep,
            "provides" => RelKind::Provides,
            "recommends" => RelKind::Recommends,
            "breaks" => RelKind::Breaks,
            _ => return None,
        })
    }
}

/// One edge out of a mod in the dependency graph, as the resolver reads it:
/// the target selector (a bare modid, or `modrinth:<project_id>`), an optional
/// Maven/semver version window, the kind, and the provenance -- `confidence`
/// carries the source rank so a caller can pick the authoritative edge per
/// target without re-deriving it. No I/O; filled by `queries::relations_from`.
#[derive(Debug, Clone)]
pub struct RelationRow {
    pub target: String,
    pub version_range: Option<String>,
    pub kind: RelKind,
    pub source: Source,
    pub confidence: i64,
}

/// Q1: a (pack build, version, filename) that ships a given mod.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ModUse {
    pub pack_id: String,
    pub pack_version: String,
    pub version: String,
    pub filename: String,
}

/// Q2: a cached artifact no build references.
#[derive(Debug, Clone, Serialize)]
pub struct OrphanJar {
    pub sha1: String,
    pub size_bytes: i64,
    pub filename: Option<String>,
}

/// Q3: one version of a mod, with every loader it targets (`any` for a
/// loader-agnostic jar) and the Minecraft versions it was published for.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct VersionRow {
    pub version: String,
    pub targets: Vec<String>,
    pub mc_versions: Vec<String>,
    pub sha1: String,
    #[ts(type = "number")]
    pub size_bytes: i64,
    pub filename: Option<String>,
    pub source: String,
    /// True when the artifact's bytes are in the mirror's local cache, so it can
    /// be re-added as a self-hosted `smrt_cache` source. Set by the handler
    /// against the live cache inventory (not stored in the registry).
    pub cached: bool,
    /// Modrinth identity, when the artifact is one. Lets the panel re-add a
    /// not-locally-cached Modrinth mod as a real Modrinth source.
    pub modrinth_project_id: Option<String>,
    pub modrinth_version_id: Option<String>,
}

/// One release (version node) of a mod for the management view: its version
/// number + channel, the provenance, and the files (artifacts) grouped under it.
/// Files carry the loader/mc facets; the release carries version + channel.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ReleaseRow {
    #[ts(type = "number")]
    pub release_id: i64,
    pub version_number: String,
    pub channel: String,
    pub source: String,
    pub files: Vec<VersionRow>,
}

/// One mod in the registry browser: identity, the human metadata an enriching
/// harvest fills in, and the facets aggregated across all its artifacts.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ModSummary {
    #[ts(type = "number")]
    pub mod_id: i64,
    /// canonical_name -> slug -> modid -> `#<id>`, resolved server-side.
    pub name: String,
    pub slug: Option<String>,
    pub author: Option<String>,
    pub loaders: Vec<String>,
    pub mc_versions: Vec<String>,
    #[ts(type = "number")]
    pub version_count: i64,
}

/// One published build in the registry browser (the mirror hosts builds too, not
/// just loose mods).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct BuildSummary {
    pub pack_id: String,
    pub pack_version: String,
    pub mc_version: String,
    pub loader_id: Option<String>,
    pub loader_version: Option<String>,
    #[ts(type = "number | null")]
    pub java_major: Option<i64>,
    pub is_latest: bool,
    #[ts(type = "number")]
    pub mod_count: i64,
}

/// One mod shipped by a build, resolved to the artifact the operator would
/// re-add (sha1) plus the human metadata to show it.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct BuildModRow {
    pub name: String,
    pub version: String,
    pub sha1: String,
    pub filename: String,
    #[ts(type = "number")]
    pub size_bytes: i64,
    pub required: bool,
    pub default_enabled: bool,
    pub targets: Vec<String>,
    pub mc_versions: Vec<String>,
    /// See [`VersionRow::cached`] -- whether this artifact is locally cached.
    pub cached: bool,
    /// Modrinth identity for a not-locally-cached build mod, so re-adding it
    /// recreates the Modrinth source instead of a missing cache jar.
    pub modrinth_project_id: Option<String>,
    pub modrinth_version_id: Option<String>,
}

/// Q4: an artifact eligible for a build loader, with its best-match specificity
/// (0 exact, 1 ancestor/family, 2 any) across the artifact's targets so the
/// most-specific artifact per mod wins.
#[derive(Debug, Clone, Serialize)]
pub struct EligibleArtifact {
    pub mod_id: i64,
    pub version: String,
    pub sha1: String,
    pub specificity: i64,
}

/// A jar on disk with no registry identity yet (harvest could not derive a
/// modid/Modrinth id). The authoring UI lists these so the operator can assign
/// each a mod + release + facets instead of it vanishing into the cache.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct UnassignedJar {
    pub sha1: String,
    #[ts(type = "number")]
    pub size_bytes: i64,
}

/// One node in the dependency/conflict graph view: a mod that is an endpoint of
/// at least one relation. `name` is resolved server-side (canonical -> slug ->
/// modid -> `#id`); `modrinth` flags a Modrinth-identified mod so the view can
/// mark genuine identities apart from bare-modid ones.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct GraphNode {
    #[ts(type = "number")]
    pub mod_id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub modid: Option<String>,
    pub modrinth: bool,
}

/// One edge in the graph. `to_mod_id` is the resolved target when the selector
/// names a mod the registry knows; `None` marks an external/unresolved target
/// (a modid not harvested, or a `provides` capability), which the view renders
/// as a labelled leaf so the dangling requirement is still visible. `kind` and
/// `source` are the relation vocab strings.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct GraphEdge {
    #[ts(type = "number")]
    pub from_mod_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub to_mod_id: Option<i64>,
    pub target: String,
    pub kind: String,
    pub source: String,
}

/// The whole relation graph for the read-only view + node editor.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// One (Minecraft version, loader) world the registry actually holds, and how many
/// artifacts sit in it. The panel offers these as the graph's slice choices and
/// opens on the busiest, rather than inventing a combination nothing matches (#49).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct GraphSlice {
    pub mc_version: String,
    pub loader: String,
    #[ts(type = "number")]
    pub artifacts: i64,
}

/// One relation touching a mod, from that mod's page perspective. `dir` is "out"
/// (this mod -> other) or "in" (other -> this mod). `other_mod_id` is the
/// catalogued counterpart when the selector resolves; `None` marks an external
/// target (an uncatalogued modid or a `provides` capability), so the page can
/// render it as a plain label rather than a link.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ModEdge {
    pub dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub other_mod_id: Option<i64>,
    pub other_name: String,
    pub kind: String,
    pub source: String,
}

/// The aggregated read model behind one mod's page: identity, the facets across
/// its artifacts, its releases (files), the relations that touch it, and the pack
/// builds that ship it. Backs the public `GET /v1/mods/:id`; the same view serves
/// operators, who additionally reach the gated edit/diff endpoints.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ModDetail {
    #[ts(type = "number")]
    pub mod_id: i64,
    /// canonical_name -> slug -> modid -> `#<id>`, resolved server-side.
    pub name: String,
    pub slug: Option<String>,
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub modid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub modrinth_project_id: Option<String>,
    pub loaders: Vec<String>,
    pub mc_versions: Vec<String>,
    pub releases: Vec<ReleaseRow>,
    pub edges: Vec<ModEdge>,
    /// Pack builds that ship this mod. Filtered to official + published on the
    /// public endpoint so a guest never learns a draft's name from it.
    pub used_by: Vec<ModUse>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RegistryStats {
    pub mods: i64,
    pub mod_versions: i64,
    pub relations: i64,
    pub packs: i64,
    pub builds: i64,
    pub orphans: i64,
}
