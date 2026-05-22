use crate::error::ApiError;
use crate::types::*;
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone)]
pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // ── Packs ──────────────────────────────────────────────────────────────

    pub async fn list_pack_summaries(&self) -> Result<Vec<PackSummary>, ApiError> {
        let packs_dir = self.root.join("packs");
        let mut out = Vec::new();
        let mut entries = match fs::read_dir(&packs_dir).await {
            Ok(e) => e,
            // Empty / missing storage root is not an error -- just no packs yet.
            Err(_) => return Ok(out),
        };
        while let Some(entry) = entries.next_entry().await.map_err(io_err)? {
            if !entry.file_type().await.map_err(io_err)?.is_dir() {
                continue;
            }
            let summary_path = entry.path().join("summary.json");
            if let Ok(bytes) = fs::read(&summary_path).await {
                match serde_json::from_slice::<PackSummary>(&bytes) {
                    Ok(s) => out.push(s),
                    Err(e) => {
                        tracing::warn!(path = %summary_path.display(), error = %e, "skipping invalid summary");
                    }
                }
            }
        }
        out.sort_by(|a, b| a.pack_id.cmp(&b.pack_id));
        Ok(out)
    }

    pub async fn load_pack_summary(&self, pack_id: &str) -> Result<PackSummary, ApiError> {
        let path = self.root.join("packs").join(pack_id).join("summary.json");
        let bytes = fs::read(&path).await.map_err(|_| ApiError::NotFound)?;
        serde_json::from_slice(&bytes).map_err(json_err)
    }

    pub async fn load_latest_manifest(&self, pack_id: &str) -> Result<PackManifest, ApiError> {
        let path = self.root.join("packs").join(pack_id).join("manifests").join("latest");
        let bytes = fs::read(&path).await.map_err(|_| ApiError::NotFound)?;
        serde_json::from_slice(&bytes).map_err(json_err)
    }

    pub async fn load_manifest_version(
        &self,
        pack_id: &str,
        version: &str,
    ) -> Result<PackManifest, ApiError> {
        if !is_safe_version(version) {
            return Err(ApiError::BadRequest("invalid version slug".into()));
        }
        let path = self
            .root
            .join("packs")
            .join(pack_id)
            .join("manifests")
            .join(format!("{version}.json"));
        let bytes = fs::read(&path).await.map_err(|_| ApiError::NotFound)?;
        serde_json::from_slice(&bytes).map_err(json_err)
    }

    pub async fn list_manifest_versions(&self, pack_id: &str) -> Result<Vec<String>, ApiError> {
        let path = self.root.join("packs").join(pack_id).join("manifests");
        let mut out = Vec::new();
        let mut entries = fs::read_dir(&path).await.map_err(|_| ApiError::NotFound)?;
        while let Some(entry) = entries.next_entry().await.map_err(io_err)? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Skip the `latest` symlink and anything not ending in .json.
            if name == "latest" || !name.ends_with(".json") {
                continue;
            }
            out.push(name.trim_end_matches(".json").to_string());
        }
        // Lexicographic sort matches chronological order given YYYY.MM.DD scheme.
        out.sort();
        Ok(out)
    }

    pub async fn pack_extras_path(
        &self,
        pack_id: &str,
        version: &str,
    ) -> Result<PathBuf, ApiError> {
        if !is_safe_version(version) {
            return Err(ApiError::BadRequest("invalid version slug".into()));
        }
        let path = self
            .root
            .join("packs")
            .join(pack_id)
            .join("extras")
            .join(format!("{version}.zip"));
        if fs::metadata(&path).await.is_ok() {
            Ok(path)
        } else {
            Err(ApiError::NotFound)
        }
    }

    // ── Servers ────────────────────────────────────────────────────────────

    pub async fn list_servers(&self) -> Result<Vec<ServerEntry>, ApiError> {
        let dir = self.root.join("servers");
        let mut out = Vec::new();
        let mut entries = match fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => return Ok(out),
        };
        while let Some(entry) = entries.next_entry().await.map_err(io_err)? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".json") {
                continue;
            }
            if let Ok(bytes) = fs::read(entry.path()).await {
                match serde_json::from_slice::<ServerEntry>(&bytes) {
                    Ok(s) => out.push(s),
                    Err(e) => tracing::warn!(file = %name, error = %e, "skipping invalid server entry"),
                }
            }
        }
        out.sort_by(|a, b| a.server_id.cmp(&b.server_id));
        Ok(out)
    }

    pub async fn load_server(&self, server_id: &str) -> Result<ServerEntry, ApiError> {
        if !is_safe_id(server_id) {
            return Err(ApiError::BadRequest("invalid server id".into()));
        }
        let path = self.root.join("servers").join(format!("{server_id}.json"));
        let bytes = fs::read(&path).await.map_err(|_| ApiError::NotFound)?;
        serde_json::from_slice(&bytes).map_err(json_err)
    }

    // ── Featured ───────────────────────────────────────────────────────────

    pub async fn load_featured(&self) -> Result<Featured, ApiError> {
        let path = self.root.join("featured.json");
        let bytes = fs::read(&path).await.map_err(|_| ApiError::NotFound)?;
        serde_json::from_slice(&bytes).map_err(json_err)
    }

    // ── Cache ──────────────────────────────────────────────────────────────

    pub fn cache_jar_path(&self, prefix: &str, sha1: &str) -> Result<PathBuf, ApiError> {
        if !is_hex(prefix) || prefix.len() != 2 {
            return Err(ApiError::BadRequest("invalid sha1 prefix".into()));
        }
        if !is_hex(sha1) || sha1.len() != 40 {
            return Err(ApiError::BadRequest("invalid sha1".into()));
        }
        if !sha1.starts_with(prefix) {
            return Err(ApiError::BadRequest("prefix does not match sha1".into()));
        }
        Ok(self.root.join("cache").join(prefix).join(format!("{sha1}.jar")))
    }

    pub async fn list_cache_inventory(&self) -> Result<Vec<CacheInventoryEntry>, ApiError> {
        let cache_dir = self.root.join("cache");
        let mut out = Vec::new();
        let mut prefix_dirs = match fs::read_dir(&cache_dir).await {
            Ok(e) => e,
            Err(_) => return Ok(out),
        };
        while let Some(prefix_entry) = prefix_dirs.next_entry().await.map_err(io_err)? {
            if !prefix_entry.file_type().await.map_err(io_err)?.is_dir() {
                continue;
            }
            let mut jars = fs::read_dir(prefix_entry.path()).await.map_err(io_err)?;
            while let Some(jar) = jars.next_entry().await.map_err(io_err)? {
                let name = jar.file_name();
                let name = name.to_string_lossy();
                if let Some(sha1) = name.strip_suffix(".jar") {
                    if is_hex(sha1) && sha1.len() == 40 {
                        let meta = jar.metadata().await.map_err(io_err)?;
                        out.push(CacheInventoryEntry {
                            sha1: sha1.to_string(),
                            size_bytes: meta.len(),
                        });
                    }
                }
            }
        }
        out.sort_by(|a, b| a.sha1.cmp(&b.sha1));
        Ok(out)
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

fn io_err(e: std::io::Error) -> ApiError {
    ApiError::Internal(anyhow::Error::from(e))
}

fn json_err(e: serde_json::Error) -> ApiError {
    ApiError::Internal(anyhow::anyhow!("JSON parse error: {e}"))
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

// Pack ID / server ID safety: no path traversal, no leading dots, basic alnum +
// dashes + dots only. Stops `..`, absolute paths, and silly characters before
// they reach the filesystem layer.
fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('.')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn is_safe_version(s: &str) -> bool {
    is_safe_id(s)
}
