//! Plain row/result structs + the source/kind vocab. No I/O.

use serde::Serialize;

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
}

/// Q1: a (pack build, version, filename) that ships a given mod.
#[derive(Debug, Clone, Serialize)]
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

/// Q3: one version of a mod.
#[derive(Debug, Clone, Serialize)]
pub struct VersionRow {
    pub version: String,
    pub target: String,
    pub sha1: String,
    pub size_bytes: i64,
    pub source: String,
}

/// Q4: an artifact eligible for a build loader, with specificity (0 exact,
/// 1 ancestor/family, 2 any) so the most-specific row per mod wins.
#[derive(Debug, Clone, Serialize)]
pub struct EligibleArtifact {
    pub mod_id: i64,
    pub version: String,
    pub target: String,
    pub sha1: String,
    pub specificity: i64,
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
