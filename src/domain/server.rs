//! Operational DTOs: curated server metadata, the featured set, mod-cache
//! inventory, and the health probe. Small wire types with no cross-deps.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ── Server metadata ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ServerEntry {
    pub schema_version: u32,
    pub server_id: String,
    pub pack_id: String,
    pub display_name: String,
    pub tagline: String,
    pub description_md: String,
    pub banner_url: String,
    #[serde(default)]
    pub gallery_urls: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub discord_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub website_url: Option<String>,
    pub owner_display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub motd_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub founded_at: Option<String>,
    #[serde(default)]
    pub featured: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ServerListing {
    pub schema_version: u32,
    pub generated_at: String,
    pub servers: Vec<ServerEntry>,
}

// ── Featured ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct Featured {
    pub schema_version: u32,
    pub generated_at: String,
    pub featured_servers: Vec<String>,
    pub featured_packs: Vec<String>,
}

// ── Cache inventory ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct CacheInventory {
    pub schema_version: u32,
    pub generated_at: String,
    pub entries: Vec<CacheInventoryEntry>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct CacheInventoryEntry {
    pub sha1: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
}

// ── Health ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct Health {
    pub schema_version: u32,
    pub status: &'static str,
    pub version: &'static str,
}
