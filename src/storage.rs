use crate::domain::*;
use crate::http::ApiError;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone)]
pub struct Storage {
    root: PathBuf,
    /// Serializes the read-modify-write of removed.txt so concurrent takedowns
    /// don't lose each other's appends.
    removed_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Storage {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            removed_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // ── Packs ──────────────────────────────────────────────────────────────

    pub async fn list_pack_summaries(&self) -> Result<Vec<PackSummary>, ApiError> {
        let packs_dir = self.root.join("packs");
        let mut out = Vec::new();
        // official packs: packs/<id>/summary.json
        read_summaries_in(&packs_dir, &mut out).await?;
        // community packs live a level deeper: packs/u/<uid>/<pack>/summary.json
        let u_dir = packs_dir.join("u");
        if let Ok(mut uids) = fs::read_dir(&u_dir).await {
            while let Some(uid) = uids.next_entry().await.map_err(io_err)? {
                if uid.file_type().await.map_err(io_err)?.is_dir() {
                    read_summaries_in(&uid.path(), &mut out).await?;
                }
            }
        }
        out.sort_by(|a, b| a.pack_id.cmp(&b.pack_id));
        Ok(out)
    }

    pub async fn load_pack_summary(&self, pack_id: &str) -> Result<PackSummary, ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let path = self.root.join("packs").join(pack_id).join("summary.json");
        let bytes = fs::read(&path).await.map_err(|_| ApiError::NotFound)?;
        serde_json::from_slice(&bytes).map_err(json_err)
    }

    pub async fn load_latest_manifest(&self, pack_id: &str) -> Result<PackManifest, ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
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
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
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
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
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

    /// Write a built manifest to `packs/<id>/manifests/<version>.json`. The
    /// authoring layer computes the manifest; persistence lives here so the
    /// on-disk layout has a single owner shared by the CLI and the panel.
    pub async fn save_manifest(
        &self,
        pack_id: &str,
        manifest: &PackManifest,
    ) -> Result<(), ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        if !is_safe_version(&manifest.pack_version) {
            return Err(ApiError::BadRequest("invalid pack version".into()));
        }
        let path = self
            .root
            .join("packs")
            .join(pack_id)
            .join("manifests")
            .join(format!("{}.json", manifest.pack_version));
        let bytes = serde_json::to_vec_pretty(manifest)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("manifest json encode: {e}")))?;
        atomic_write(&path, &bytes).await
    }

    /// Point `manifests/latest` at `<version>.json` via an atomic symlink
    /// swap, so concurrent readers never observe a missing target. The
    /// public read path resolves `latest` by following the link.
    pub async fn set_latest_manifest(&self, pack_id: &str, version: &str) -> Result<(), ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        if !is_safe_version(version) {
            return Err(ApiError::BadRequest("invalid version slug".into()));
        }
        let manifests_dir = self.root.join("packs").join(pack_id).join("manifests");
        fs::create_dir_all(&manifests_dir).await.map_err(io_err)?;
        let target = format!("{version}.json");
        let latest = manifests_dir.join("latest");
        let latest_tmp = manifests_dir.join("latest.tmp");
        let _ = fs::remove_file(&latest_tmp).await;
        #[cfg(unix)]
        fs::symlink(&target, &latest_tmp).await.map_err(io_err)?;
        fs::rename(&latest_tmp, &latest).await.map_err(io_err)?;
        Ok(())
    }

    /// Write the pack's `summary.json` (the Browse-list / PackDetail card).
    pub async fn save_pack_summary(&self, summary: &PackSummary) -> Result<(), ApiError> {
        if !is_safe_id(&summary.pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let path = self
            .root
            .join("packs")
            .join(&summary.pack_id)
            .join("summary.json");
        let bytes = serde_json::to_vec_pretty(summary)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("summary json encode: {e}")))?;
        atomic_write(&path, &bytes).await
    }

    // ── Pack authoring inputs ────────────────────────────────────────────────
    //
    // The editable PackConfig the panel works from, kept on the mirror under
    // packs/<id>/authoring/ so the box that holds the mod cache is the single
    // home of both the authoring inputs and the built outputs.

    pub async fn save_pack_config(&self, pack_id: &str, cfg: &PackConfig) -> Result<(), ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let path = self.authoring_path(pack_id, "config.json");
        let bytes = serde_json::to_vec_pretty(cfg)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("pack config json encode: {e}")))?;
        atomic_write(&path, &bytes).await
    }

    pub async fn load_pack_config(&self, pack_id: &str) -> Result<PackConfig, ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let bytes = fs::read(self.authoring_path(pack_id, "config.json"))
            .await
            .map_err(|_| ApiError::NotFound)?;
        serde_json::from_slice(&bytes).map_err(json_err)
    }

    /// Set a pack's publication state (the publish/unpublish toggle). Patches
    /// both the built `summary.json` -- what the public listing filters on, so
    /// the change takes effect without a rebuild -- and the authoring config, so
    /// a later rebuild keeps it. `NotFound` if the pack has neither file.
    pub async fn set_pack_visibility(
        &self,
        pack_id: &str,
        visibility: Visibility,
    ) -> Result<(), ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let mut found = false;
        if let Ok(mut summary) = self.load_pack_summary(pack_id).await {
            summary.visibility = visibility;
            self.save_pack_summary(&summary).await?;
            found = true;
        }
        if let Ok(mut cfg) = self.load_pack_config(pack_id).await {
            cfg.visibility = visibility;
            self.save_pack_config(pack_id, &cfg).await?;
            found = true;
        }
        if !found {
            return Err(ApiError::NotFound);
        }
        Ok(())
    }

    /// Delete a pack and everything under it -- config, summary, manifests, and
    /// static assets. `NotFound` if the pack does not exist. The shared mod cache
    /// is content-addressed and left untouched; other packs keep their jars.
    pub async fn delete_pack(&self, pack_id: &str) -> Result<(), ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let dir = self.root.join("packs").join(pack_id);
        fs::remove_dir_all(&dir)
            .await
            .map_err(|_| ApiError::NotFound)
    }

    /// Clone an existing pack's authoring inputs under a new id: copy its
    /// `authoring/config.json` with `pack_id` rewritten (and the loader
    /// optionally overridden, for a loader-variant fork), then copy the whole
    /// per-pack `static/` tree. The mod cache is content-addressed and shared,
    /// so every `smrt_cache` / Modrinth source resolves under the new pack with
    /// no jar re-upload. Returns the new config.
    ///
    /// Refuses to overwrite a target that already has a config, so a variant
    /// can never clobber a working pack. Static is copied before the config is
    /// written -- a pack "exists" only once its config lands, so a mid-copy
    /// failure leaves no config.json and a retry runs clean.
    pub async fn duplicate_pack(
        &self,
        from: &str,
        to: &str,
        loader: Option<LoaderSpec>,
        owner: i64,
        fork_of: Option<String>,
    ) -> Result<PackConfig, ApiError> {
        if !is_safe_id(to) {
            return Err(ApiError::BadRequest("invalid target pack id".into()));
        }
        if from == to {
            return Err(ApiError::BadRequest(
                "source and target pack id are the same".into(),
            ));
        }
        if fs::metadata(self.authoring_path(to, "config.json"))
            .await
            .is_ok()
        {
            return Err(ApiError::Conflict(format!(
                "pack {to:?} already has a config"
            )));
        }

        let mut cfg = self.load_pack_config(from).await?;
        cfg.pack_id = to.to_string();
        // the clone is a fresh draft owned by whoever made it; its tier follows
        // the target namespace, and fork_of is set only when this is a fork.
        cfg.owner = owner;
        cfg.tier = if to.starts_with("u/") {
            PackTier::Community
        } else {
            PackTier::Official
        };
        cfg.visibility = Visibility::Draft;
        cfg.fork_of = fork_of;
        if let Some(loader) = loader {
            cfg.loader = loader;
        }

        for rel in self.list_pack_static(from).await? {
            let bytes = fs::read(self.pack_static_path(from, &rel)?)
                .await
                .map_err(io_err)?;
            self.save_pack_static(to, &rel, &bytes).await?;
        }
        self.save_pack_config(to, &cfg).await?;
        Ok(cfg)
    }

    /// Pack ids that have authoring inputs (an `authoring/config.json`),
    /// including packs not yet built (so no summary.json). Sorted.
    pub async fn list_authoring_packs(&self) -> Result<Vec<String>, ApiError> {
        let packs_dir = self.root.join("packs");
        let mut out = Vec::new();
        // official ids sit at the top level; community ids are the full
        // `u/<uid>/<pack>` key, reconstructed from the nesting.
        read_authoring_ids_in(&packs_dir, "", &mut out).await?;
        let u_dir = packs_dir.join("u");
        if let Ok(mut uids) = fs::read_dir(&u_dir).await {
            while let Some(uid) = uids.next_entry().await.map_err(io_err)? {
                if uid.file_type().await.map_err(io_err)?.is_dir() {
                    let prefix = format!("u/{}/", uid.file_name().to_string_lossy());
                    read_authoring_ids_in(&uid.path(), &prefix, &mut out).await?;
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn authoring_path(&self, pack_id: &str, file: &str) -> PathBuf {
        self.root
            .join("packs")
            .join(pack_id)
            .join("authoring")
            .join(file)
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

    /// Every file under the pack's static area as relative paths (sorted).
    /// Backs the panel's Branding section (uploaded icons / banners / assets).
    pub async fn list_pack_static(&self, pack_id: &str) -> Result<Vec<String>, ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let base = self.root.join("packs").join(pack_id).join("static");
        let mut out = Vec::new();
        let mut stack = vec![base.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries = match fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };
            while let Some(entry) = entries.next_entry().await.map_err(io_err)? {
                let path = entry.path();
                let ft = entry.file_type().await.map_err(io_err)?;
                if ft.is_dir() {
                    stack.push(path);
                } else if ft.is_file()
                    && let Ok(rel) = path.strip_prefix(&base)
                {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        out.sort();
        Ok(out)
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
        if !sha1.starts_with(prefix) {
            return Err(ApiError::BadRequest("prefix does not match sha1".into()));
        }
        cache_jar_path_in(&self.root, sha1)
            .ok_or_else(|| ApiError::BadRequest("invalid sha1".into()))
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
        let path = cache_jar_path_in(&self.root, sha1)
            .ok_or_else(|| ApiError::BadRequest("invalid sha1".into()))?;
        if fs::metadata(&path).await.is_ok() {
            return Ok(());
        }
        atomic_write(&path, bytes).await
    }

    pub async fn delete_cache_jar(&self, sha1: &str) -> Result<(), ApiError> {
        if !is_hex(sha1) || sha1.len() != 40 {
            return Err(ApiError::BadRequest("invalid sha1".into()));
        }
        let path = cache_jar_path_in(&self.root, sha1)
            .ok_or_else(|| ApiError::BadRequest("invalid sha1".into()))?;
        fs::remove_file(&path)
            .await
            .map_err(|_| ApiError::NotFound)?;
        self.record_removed(sha1).await?;
        Ok(())
    }

    /// Stage a member upload pending moderation, content-addressed by sha1 under
    /// `uploads/`. Not the shared cache -- an operator promotes it on approval.
    pub async fn stage_upload(&self, sha1: &str, bytes: &[u8]) -> Result<(), ApiError> {
        if !is_hex(sha1) || sha1.len() != 40 {
            return Err(ApiError::BadRequest("invalid sha1".into()));
        }
        let actual = sha1_hex(bytes);
        if actual != sha1 {
            return Err(ApiError::BadRequest(format!(
                "sha1 mismatch: {sha1} vs {actual}"
            )));
        }
        atomic_write(&self.staged_upload_path(sha1), bytes).await
    }

    /// Promote a staged upload into the shared cache (moderation approved), then
    /// remove the staging copy. `NotFound` if nothing is staged.
    pub async fn promote_upload(&self, sha1: &str) -> Result<(), ApiError> {
        let staged = self.staged_upload_path(sha1);
        let bytes = fs::read(&staged).await.map_err(|_| ApiError::NotFound)?;
        self.save_cache_jar(sha1, &bytes).await?;
        let _ = fs::remove_file(&staged).await;
        Ok(())
    }

    /// Drop a staged upload (moderation rejected). Idempotent.
    pub async fn discard_upload(&self, sha1: &str) -> Result<(), ApiError> {
        let _ = fs::remove_file(self.staged_upload_path(sha1)).await;
        Ok(())
    }

    fn staged_upload_path(&self, sha1: &str) -> PathBuf {
        self.root.join("uploads").join(format!("{sha1}.jar"))
    }

    pub async fn save_featured(&self, featured: &Featured) -> Result<(), ApiError> {
        let path = self.root.join("featured.json");
        let bytes = serde_json::to_vec_pretty(featured)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("featured json encode: {e}")))?;
        atomic_write(&path, &bytes).await
    }

    pub async fn is_sha1_removed(&self, sha1: &str) -> Result<bool, ApiError> {
        let path = self.root.join("removed.txt");
        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(_) => return Ok(false),
        };
        Ok(content.lines().any(|line| line.trim() == sha1))
    }

    async fn record_removed(&self, sha1: &str) -> Result<(), ApiError> {
        let _guard = self.removed_lock.lock().await;
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

    /// The takedown list: sha1s blocked from (re-)ingestion. Surfaced in the
    /// panel's Cache tab so an operator can see what has been removed.
    pub async fn list_removed(&self) -> Result<Vec<String>, ApiError> {
        let path = self.root.join("removed.txt");
        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect())
    }
}

/// Per-process temp-file sequence so concurrent writers to the same target use
/// distinct temp files instead of colliding on one shared `<file>.tmp`.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    let parent = path.parent().ok_or_else(|| {
        ApiError::Internal(anyhow::anyhow!("path {} has no parent", path.display()))
    })?;
    fs::create_dir_all(parent).await.map_err(io_err)?;
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut tmp = path.to_path_buf();
    let mut tmp_name = tmp
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    tmp_name.push(format!(".tmp.{}.{seq}", std::process::id()));
    tmp.set_file_name(tmp_name);

    // Write + fsync the temp, then rename. fsync before rename so a crash can't
    // leave a zero-length file at the target; clean up the temp on any error.
    let result = async {
        let mut f = fs::File::create(&tmp).await.map_err(io_err)?;
        f.write_all(bytes).await.map_err(io_err)?;
        f.sync_all().await.map_err(io_err)?;
        fs::rename(&tmp, path).await.map_err(io_err)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&tmp).await;
    }
    result
}

pub(crate) fn sha1_hex(bytes: &[u8]) -> String {
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

/// Read `summary.json` from each immediate child directory of `dir`, pushing the
/// parsed summaries. Missing dir -> no-op; an unreadable / invalid summary is
/// skipped with a warning. Shared by the official and community listing passes.
async fn read_summaries_in(dir: &Path, out: &mut Vec<PackSummary>) -> Result<(), ApiError> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return Ok(()),
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
    Ok(())
}

/// Push the id of each immediate child directory of `dir` that carries an
/// `authoring/config.json`, prefixed -- so community ids come back as the full
/// `u/<uid>/<pack>` key, official ids bare.
async fn read_authoring_ids_in(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<String>,
) -> Result<(), ApiError> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    while let Some(entry) = entries.next_entry().await.map_err(io_err)? {
        if !entry.file_type().await.map_err(io_err)?.is_dir() {
            continue;
        }
        let cfg = entry.path().join("authoring").join("config.json");
        if fs::metadata(&cfg).await.is_ok() {
            out.push(format!("{prefix}{}", entry.file_name().to_string_lossy()));
        }
    }
    Ok(())
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The cache shard for a content hash: the first two hex chars of the sha1.
/// Both the on-disk layout and the public cache URL bucket by this, so the
/// sharding width has one definition. Precondition: a validated (>= 2 char) sha1.
pub(crate) fn sha1_shard(sha1: &str) -> &str {
    sha1.get(..2).unwrap_or(sha1)
}

/// The one definition of the content-addressed cache layout: a jar with the
/// given sha1 lives at `<root>/cache/<sha1[..2]>/<sha1>.jar`. Both the HTTP and
/// authoring layers build their cache paths on this, so the sharding scheme has
/// a single source of truth. `None` for a non-40-hex sha1.
pub(crate) fn cache_jar_path_in(root: &Path, sha1: &str) -> Option<PathBuf> {
    if sha1.len() != 40 || !is_hex(sha1) {
        return None;
    }
    Some(
        root.join("cache")
            .join(sha1_shard(sha1))
            .join(format!("{sha1}.jar")),
    )
}

// Pack ID / server ID safety: no path traversal, no leading dots, basic alnum +
// dashes + dots only. Stops `..`, absolute paths, and silly characters before
// they reach the filesystem layer.
/// A pack id is either a flat official id (`<seg>`) or a namespaced community id
/// (`u/<uid>/<pack>`, uid all-digits). Both leaf segments are conservative and
/// cannot traverse (no leading dot, no slash); the literal `u` is reserved as the
/// community namespace so an official pack can never collide with `packs/u/`.
pub(crate) fn is_safe_id(s: &str) -> bool {
    match s.strip_prefix("u/") {
        Some(rest) => match rest.split_once('/') {
            Some((uid, pack)) => {
                !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()) && is_flat_id(pack)
            }
            None => false,
        },
        None => s != "u" && is_flat_id(s),
    }
}

/// One conservative path segment: non-empty, <=64, no leading dot (so no `..`),
/// alphanumeric plus `-_.`.
fn is_flat_id(s: &str) -> bool {
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
    if is_safe_rel_path(rel) {
        Ok(rel)
    } else {
        Err(ApiError::BadRequest("invalid rel_path".into()))
    }
}

/// Boolean core of [validate_rel_path], shared with the authoring layer so its
/// curator- and archive-driven writes reject the same traversal the HTTP layer
/// does. Allows nested dirs and real-world resourcepack/shaderpack filenames
/// (spaces, parens, plus, comma, square brackets) but forbids `..`, leading dots per segment,
/// absolute paths, and backslashes. ASCII-only -- non-ASCII filenames break
/// some Forge launchers on Windows.
pub(crate) fn is_safe_rel_path(rel: &str) -> bool {
    if rel.is_empty() || rel.len() > 512 {
        return false;
    }
    if rel.starts_with('/') || rel.contains('\\') {
        return false;
    }
    rel.split('/').all(|segment| {
        !segment.is_empty()
            && !segment.starts_with('.')
            && segment.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '-' | '_' | '.' | ' ' | '(' | ')' | '+' | ',' | '[' | ']')
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{LoaderSpec, PackConfig};

    #[test]
    fn safe_id_rejects_traversal() {
        // the guard added to the public pack-read paths leans on this
        assert!(!is_safe_id("../x"));
        assert!(!is_safe_id(".."));
        assert!(!is_safe_id("../../etc/passwd"));
        assert!(!is_safe_id("a/b"));
        assert!(is_safe_id("Industrial"));
        assert!(is_safe_id("pack_1.0"));
    }

    #[test]
    fn rel_path_rejects_traversal_accepts_real_filenames() {
        assert!(is_safe_rel_path("_nexira/icon.png"));
        assert!(is_safe_rel_path("resourcepacks/Chocapic13 V7.1 High.zip"));
        assert!(is_safe_rel_path("config/foamfix.cfg"));
        // real resourcepack names carry square-bracketed version ranges; the URL
        // layer percent-encodes them, so the dest validator must allow them too
        assert!(is_safe_rel_path(
            "resourcepacks/NewDefault+v1.82[MC1.9-1.12.2].zip"
        ));
        assert!(!is_safe_rel_path("../etc/passwd"));
        assert!(!is_safe_rel_path("a/../../b"));
        assert!(!is_safe_rel_path("/abs/path"));
        assert!(!is_safe_rel_path("a\\b"));
        assert!(!is_safe_rel_path(".hidden/x"));
        assert!(!is_safe_rel_path("ok/.././bad"));
        assert!(!is_safe_rel_path(""));
    }

    fn sample_config() -> PackConfig {
        PackConfig {
            pack_id: "Industrial".into(),
            display_name: "Industrial".into(),
            tagline: String::new(),
            minecraft_version: "1.12.2".into(),
            loader: LoaderSpec {
                name: "forge".into(),
                version: "14.23.5.2922".into(),
            },
            java_major: 8,
            version: None,
            tags: vec![],
            featured: false,
            mods: vec![],
            assets: vec![],
            pack_meta: Default::default(),
            owner: 211033194,
            tier: PackTier::Official,
            visibility: Visibility::Published,
            fork_of: None,
        }
    }

    #[tokio::test]
    async fn pack_config_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        s.save_pack_config("Industrial", &sample_config())
            .await
            .unwrap();
        let loaded = s.load_pack_config("Industrial").await.unwrap();
        assert_eq!(loaded.pack_id, "Industrial");
        assert_eq!(loaded.minecraft_version, "1.12.2");
    }

    #[tokio::test]
    async fn list_authoring_packs_finds_configured_pack() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        assert!(s.list_authoring_packs().await.unwrap().is_empty());
        s.save_pack_config("Industrial", &sample_config())
            .await
            .unwrap();
        assert_eq!(
            s.list_authoring_packs().await.unwrap(),
            vec!["Industrial".to_string()]
        );
    }

    #[tokio::test]
    async fn duplicate_pack_clones_config_and_static_with_loader_override() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        s.save_pack_config("Industrial", &sample_config())
            .await
            .unwrap();
        s.save_pack_static("Industrial", "config/foamfix.cfg", b"a=1")
            .await
            .unwrap();

        let cleanroom = LoaderSpec {
            name: "cleanroom".into(),
            version: "0.2.3".into(),
        };
        let new_cfg = s
            .duplicate_pack("Industrial", "Industrial-cleanroom", Some(cleanroom), 7, None)
            .await
            .unwrap();

        // returned + persisted config carries the new id and the overridden loader
        assert_eq!(new_cfg.pack_id, "Industrial-cleanroom");
        assert_eq!(new_cfg.loader.name, "cleanroom");
        // a duplicate is a fresh draft owned by the cloner, not the source
        assert_eq!(new_cfg.owner, 7);
        assert_eq!(new_cfg.visibility, Visibility::Draft);
        assert!(new_cfg.fork_of.is_none());
        let loaded = s.load_pack_config("Industrial-cleanroom").await.unwrap();
        assert_eq!(loaded.loader.version, "0.2.3");

        // static tree copied verbatim
        assert_eq!(
            s.list_pack_static("Industrial-cleanroom").await.unwrap(),
            vec!["config/foamfix.cfg".to_string()]
        );

        // source is untouched -- still Forge
        let src = s.load_pack_config("Industrial").await.unwrap();
        assert_eq!(src.loader.name, "forge");
    }

    #[tokio::test]
    async fn duplicate_pack_refuses_existing_target_and_self() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        s.save_pack_config("Industrial", &sample_config())
            .await
            .unwrap();
        s.save_pack_config("Taken", &sample_config()).await.unwrap();

        assert!(matches!(
            s.duplicate_pack("Industrial", "Taken", None, 1, None).await,
            Err(ApiError::Conflict(_))
        ));
        assert!(matches!(
            s.duplicate_pack("Industrial", "Industrial", None, 1, None).await,
            Err(ApiError::BadRequest(_))
        ));
        // unknown source -> NotFound (from the underlying config load)
        assert!(matches!(
            s.duplicate_pack("Nope", "Fresh", None, 1, None).await,
            Err(ApiError::NotFound)
        ));
    }

    #[test]
    fn is_safe_id_accepts_official_and_community_and_blocks_traversal() {
        assert!(is_safe_id("Industrial"));
        assert!(is_safe_id("pack_1-2.3"));
        assert!(is_safe_id("u/211033194/CoolPack"));
        // reserved namespace + malformed community keys
        assert!(!is_safe_id("u"));
        assert!(!is_safe_id("u/211033194")); // no pack segment
        assert!(!is_safe_id("u/abc/CoolPack")); // non-digit uid
        assert!(!is_safe_id("u//CoolPack")); // empty uid
        // no traversal in either form
        assert!(!is_safe_id(".."));
        assert!(!is_safe_id("u/1/.."));
        assert!(!is_safe_id("a/b"));
        assert!(!is_safe_id(""));
    }

    fn sample_summary(pack_id: &str) -> PackSummary {
        let json = format!(
            r#"{{"pack_id":"{pack_id}","display_name":"X","tagline":"",
                 "minecraft_version":"1.12.2","latest_pack_version":"1","tags":[]}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[tokio::test]
    async fn list_pack_summaries_includes_community_packs() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        s.save_pack_summary(&sample_summary("Industrial"))
            .await
            .unwrap();
        s.save_pack_summary(&sample_summary("u/42/CoolPack"))
            .await
            .unwrap();

        let ids: Vec<String> = s
            .list_pack_summaries()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.pack_id)
            .collect();
        assert!(ids.contains(&"Industrial".to_string()), "official listed");
        assert!(
            ids.contains(&"u/42/CoolPack".to_string()),
            "community listed"
        );
    }
}
