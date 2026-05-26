use crate::types::{Display, LoaderSpec};
use serde::{Deserialize, Serialize};

/// Admin-authored declaration of a pack. The build subcommand turns this into
/// a wire `PackManifest` by resolving each source against Modrinth or the
/// local storage tree. Distinct from `PackManifest` because authoring does
/// not require admin to hand-write `sha1` and `size_bytes` for Modrinth
/// sources -- those are looked up at build time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackConfig {
    pub pack_id: String,
    pub display_name: String,
    pub tagline: String,
    pub minecraft_version: String,
    pub loader: LoaderSpec,
    pub java_major: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub featured: bool,
    pub mods: Vec<DeclaredMod>,
    #[serde(default)]
    pub assets: Vec<DeclaredAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclaredMod {
    pub filename: String,
    #[serde(default = "default_true")]
    pub required: bool,
    pub source: SourceDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclaredAsset {
    pub dest: String,
    #[serde(default = "default_true")]
    pub required: bool,
    pub source: SourceDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceDecl {
    Modrinth {
        project_id: String,
        version_id: String,
    },
    SmrtCache {
        sha1: String,
    },
    SmrtStatic {
        rel_path: String,
    },
}

fn default_true() -> bool {
    true
}
