use crate::error::ApiError;
use crate::types::*;
use sha1::{Digest, Sha1};
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
        let path = self
            .root
            .join("packs")
            .join(pack_id)
            .join("manifests")
            .join("latest");
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

    /// Resolve a curated static asset path under the pack. Used by both the
    /// public GET and the admin PUT/DELETE endpoints; existence is not
    /// checked here so the same routine can produce destination paths for
    /// uploads.
    pub fn pack_static_path(&self, pack_id: &str, rel_path: &str) -> Result<PathBuf, ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let safe = validate_rel_path(rel_path)?;
        Ok(self
            .root
            .join("packs")
            .join(pack_id)
            .join("static")
            .join(safe))
    }

    pub async fn save_pack_static(
        &self,
        pack_id: &str,
        rel_path: &str,
        bytes: &[u8],
    ) -> Result<(), ApiError> {
        let path = self.pack_static_path(pack_id, rel_path)?;
        atomic_write(&path, bytes).await
    }

    pub async fn delete_pack_static(&self, pack_id: &str, rel_path: &str) -> Result<(), ApiError> {
        let path = self.pack_static_path(pack_id, rel_path)?;
        fs::remove_file(&path)
            .await
            .map_err(|_| ApiError::NotFound)?;
        Ok(())
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
                    Err(e) => {
                        tracing::warn!(file = %name, error = %e, "skipping invalid server entry")
                    }
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
        Ok(self
            .root
            .join("cache")
            .join(prefix)
            .join(format!("{sha1}.jar")))
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
                if let Some(sha1) = name.strip_suffix(".jar")
                    && is_hex(sha1)
                    && sha1.len() == 40
                {
                    let meta = jar.metadata().await.map_err(io_err)?;
                    out.push(CacheInventoryEntry {
                        sha1: sha1.to_string(),
                        size_bytes: meta.len(),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.sha1.cmp(&b.sha1));
        Ok(out)
    }

    // ── Admin writes ───────────────────────────────────────────────────────

    pub async fn save_server(&self, entry: &ServerEntry) -> Result<(), ApiError> {
        if !is_safe_id(&entry.server_id) {
            return Err(ApiError::BadRequest("invalid server id".into()));
        }
        let dir = self.root.join("servers");
        fs::create_dir_all(&dir).await.map_err(io_err)?;
        let path = dir.join(format!("{}.json", entry.server_id));
        let bytes = serde_json::to_vec_pretty(entry)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("server json encode: {e}")))?;
        atomic_write(&path, &bytes).await
    }

    pub async fn delete_server(&self, server_id: &str) -> Result<(), ApiError> {
        if !is_safe_id(server_id) {
            return Err(ApiError::BadRequest("invalid server id".into()));
        }
        let path = self
            .root
            .join("servers")
            .join(format!("{}.json", server_id));
        fs::remove_file(&path)
            .await
            .map_err(|_| ApiError::NotFound)?;
        Ok(())
    }

    pub async fn save_cache_jar(&self, sha1: &str, bytes: &[u8]) -> Result<(), ApiError> {
        if !is_hex(sha1) || sha1.len() != 40 {
            return Err(ApiError::BadRequest("invalid sha1".into()));
        }
        // Content-addressed: verify body hashes to the claimed sha1 so a
        // mis-uploaded jar fails loudly rather than corrupting the cache.
        let actual = sha1_hex(bytes);
        if actual != sha1 {
            return Err(ApiError::BadRequest(format!(
                "sha1 mismatch: url claims {sha1} but body hashes to {actual}"
            )));
        }
        // removed.txt blocks re-ingestion of takedown'd jars; honor it on
        // upload too so a takedown survives a retry.
        if self.is_sha1_removed(sha1).await? {
            return Err(ApiError::BadRequest(format!(
                "sha1 {sha1} is on the removed-list and cannot be re-uploaded"
            )));
        }
        let prefix = &sha1[..2];
        let dir = self.root.join("cache").join(prefix);
        fs::create_dir_all(&dir).await.map_err(io_err)?;
        let path = dir.join(format!("{sha1}.jar"));
        if fs::metadata(&path).await.is_ok() {
            return Ok(());
        }
        atomic_write(&path, bytes).await
    }

    pub async fn delete_cache_jar(&self, sha1: &str) -> Result<(), ApiError> {
        if !is_hex(sha1) || sha1.len() != 40 {
            return Err(ApiError::BadRequest("invalid sha1".into()));
        }
        let prefix = &sha1[..2];
        let path = self
            .root
            .join("cache")
            .join(prefix)
            .join(format!("{sha1}.jar"));
        fs::remove_file(&path)
            .await
            .map_err(|_| ApiError::NotFound)?;
        self.record_removed(sha1).await?;
        Ok(())
    }

    pub async fn save_featured(&self, featured: &Featured) -> Result<(), ApiError> {
        let path = self.root.join("featured.json");
        let bytes = serde_json::to_vec_pretty(featured)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("featured json encode: {e}")))?;
        atomic_write(&path, &bytes).await
    }

    async fn is_sha1_removed(&self, sha1: &str) -> Result<bool, ApiError> {
        let path = self.root.join("removed.txt");
        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(_) => return Ok(false),
        };
        Ok(content.lines().any(|line| line.trim() == sha1))
    }

    async fn record_removed(&self, sha1: &str) -> Result<(), ApiError> {
        let path = self.root.join("removed.txt");
        let mut content = fs::read_to_string(&path).await.unwrap_or_default();
        if content.lines().any(|line| line.trim() == sha1) {
            return Ok(());
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(sha1);
        content.push('\n');
        atomic_write(&path, content.as_bytes()).await
    }
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    let parent = path.parent().ok_or_else(|| {
        ApiError::Internal(anyhow::anyhow!("path {} has no parent", path.display()))
    })?;
    fs::create_dir_all(parent).await.map_err(io_err)?;
    let mut tmp = path.to_path_buf();
    let mut tmp_name = tmp
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".tmp");
    tmp.set_file_name(tmp_name);
    fs::write(&tmp, bytes).await.map_err(io_err)?;
    fs::rename(&tmp, path).await.map_err(io_err)?;
    Ok(())
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(40);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
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
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn is_safe_version(s: &str) -> bool {
    is_safe_id(s)
}

/// Reject path traversal and other surprises in user-supplied relative paths.
/// Allows nested directories so curated assets like `_nexira/banner.png` work,
/// but every segment must be a plain `is_safe_id`-style token and there must
/// be no `..`, `.`, leading slashes, or empty segments.
fn validate_rel_path(rel: &str) -> Result<&str, ApiError> {
    if rel.is_empty() || rel.len() > 512 {
        return Err(ApiError::BadRequest("invalid rel_path".into()));
    }
    if rel.starts_with('/') || rel.contains('\\') {
        return Err(ApiError::BadRequest("invalid rel_path".into()));
    }
    for segment in rel.split('/') {
        if segment.is_empty() || segment.starts_with('.') {
            return Err(ApiError::BadRequest("invalid rel_path".into()));
        }
        // Allow spaces and parens too -- real-world resourcepack and
        // shaderpack filenames include them ("Chocapic13 V7.1 High.zip",
        // "BSL (v8.2.04).zip"). The crucial constraint is no path
        // traversal, no leading dot per segment, no NUL or path
        // separators inside a segment. ASCII-only because non-ASCII
        // filenames break some Forge launchers on Windows.
        if !segment.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || c == '-'
                || c == '_'
                || c == '.'
                || c == ' '
                || c == '('
                || c == ')'
                || c == '+'
                || c == ','
        }) {
            return Err(ApiError::BadRequest("invalid rel_path".into()));
        }
    }
    Ok(rel)
}
