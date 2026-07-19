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
        // Tuple comparison, not string sort: `.10` must land after `.2`
        // (the spec's ordering rule; string sort inverts it).
        out.sort_by(|a, b| compare_pack_versions(a, b));
        Ok(out)
    }

    /// Per-build metadata for every retained manifest of a pack, newest first
    /// (by `generated_at`, tuple version comparison as the tiebreak). Reads
    /// each manifest's header only -- the mod/asset arrays are counted, not
    /// materialized.
    pub async fn list_manifest_builds(
        &self,
        pack_id: &str,
    ) -> Result<Vec<ManifestBuildInfo>, ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let dir = self.root.join("packs").join(pack_id).join("manifests");
        let mut out = Vec::new();
        let mut entries = fs::read_dir(&dir).await.map_err(|_| ApiError::NotFound)?;
        while let Some(entry) = entries.next_entry().await.map_err(io_err)? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "latest" || !name.ends_with(".json") {
                continue;
            }
            let bytes = fs::read(entry.path()).await.map_err(io_err)?;
            // A manifest that no longer parses is skipped, not fatal: one
            // corrupt historical file must not take the whole listing down.
            let Ok(head) = serde_json::from_slice::<ManifestHead>(&bytes) else {
                continue;
            };
            out.push(build_info_from_head(head));
        }
        out.sort_by(|a, b| {
            b.date_published
                .cmp(&a.date_published)
                .then_with(|| compare_pack_versions(&b.version_number, &a.version_number))
        });
        Ok(out)
    }

    /// The version `manifests/latest` points at, resolved from the symlink
    /// target. `None` when the pack has no published build (or the link is
    /// unreadable) -- callers treat both the same way.
    pub async fn latest_manifest_version(&self, pack_id: &str) -> Result<Option<String>, ApiError> {
        if !is_safe_id(pack_id) {
            return Err(ApiError::BadRequest("invalid pack id".into()));
        }
        let latest = self
            .root
            .join("packs")
            .join(pack_id)
            .join("manifests")
            .join("latest");
        let Ok(target) = fs::read_link(&latest).await else {
            return Ok(None);
        };
        Ok(target
            .file_name()
            .map(|n| n.to_string_lossy().trim_end_matches(".json").to_string()))
    }

    /// Header metadata of the latest build only (one link resolve + one file
    /// read) -- the cheap form behind read-time summary enrichment.
    pub async fn latest_build_info(
        &self,
        pack_id: &str,
    ) -> Result<Option<ManifestBuildInfo>, ApiError> {
        let Some(version) = self.latest_manifest_version(pack_id).await? else {
            return Ok(None);
        };
        if !is_safe_version(&version) {
            return Ok(None);
        }
        let path = self
            .root
            .join("packs")
            .join(pack_id)
            .join("manifests")
            .join(format!("{version}.json"));
        let Ok(bytes) = fs::read(&path).await else {
            return Ok(None);
        };
        let Ok(head) = serde_json::from_slice::<ManifestHead>(&bytes) else {
            return Ok(None);
        };
        Ok(Some(build_info_from_head(head)))
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

    /// Free a cached jar's bytes. Reversible: the sha1 is not blocked, so the
    /// same jar can be re-added later. A deliberate policy block is `takedown`,
    /// a separate act -- deleting to reclaim space must not tombstone a jar (#14).
    pub async fn delete_cache_jar(&self, sha1: &str) -> Result<(), ApiError> {
        if !is_hex(sha1) || sha1.len() != 40 {
            return Err(ApiError::BadRequest("invalid sha1".into()));
        }
        let path = cache_jar_path_in(&self.root, sha1)
            .ok_or_else(|| ApiError::BadRequest("invalid sha1".into()))?;
        fs::remove_file(&path)
            .await
            .map_err(|_| ApiError::NotFound)?;
        Ok(())
    }

    /// Block a jar: drop any cached copy and add its sha1 to the removed list so
    /// it can neither be served nor re-ingested. The deliberate act for copyright
    /// or policy, distinct from `delete_cache_jar` which only frees bytes (#14).
    pub async fn takedown(&self, sha1: &str) -> Result<(), ApiError> {
        if !is_hex(sha1) || sha1.len() != 40 {
            return Err(ApiError::BadRequest("invalid sha1".into()));
        }
        if let Some(path) = cache_jar_path_in(&self.root, sha1) {
            let _ = fs::remove_file(&path).await; // best-effort: may not be cached
        }
        self.record_removed(sha1).await
    }

    /// Lift a takedown: remove the sha1 from the removed list so the jar may be
    /// cached and served again. The bytes are not restored -- re-add to recache.
    pub async fn restore(&self, sha1: &str) -> Result<(), ApiError> {
        if !is_hex(sha1) || sha1.len() != 40 {
            return Err(ApiError::BadRequest("invalid sha1".into()));
        }
        let _guard = self.removed_lock.lock().await;
        let path = self.root.join("removed.txt");
        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };
        let mut out: String = content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && *l != sha1)
            .collect::<Vec<_>>()
            .join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        atomic_write(&path, out.as_bytes()).await
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

impl Storage {
    // -- Job snapshots ------------------------------------------------------

    /// Persist a job's snapshot to `jobs/<id>.json` (atomic). Job ids are
    /// self-generated (hex millis + counter), but validate anyway so a
    /// hand-crafted id can never traverse.
    pub async fn save_job_snapshot(&self, snap: &crate::jobs::JobSnapshot) -> Result<(), ApiError> {
        if !is_safe_job_id(&snap.job_id) {
            return Err(ApiError::BadRequest("invalid job id".into()));
        }
        let dir = self.root.join("jobs");
        fs::create_dir_all(&dir).await.map_err(io_err)?;
        let bytes = serde_json::to_vec_pretty(snap)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("job snapshot encode: {e}")))?;
        atomic_write(&dir.join(format!("{}.json", snap.job_id)), &bytes).await
    }

    pub async fn load_job_snapshot(
        &self,
        job_id: &str,
    ) -> Result<Option<crate::jobs::JobSnapshot>, ApiError> {
        if !is_safe_job_id(job_id) {
            return Err(ApiError::BadRequest("invalid job id".into()));
        }
        let path = self.root.join("jobs").join(format!("{job_id}.json"));
        let bytes = match fs::read(&path).await {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    /// Startup sweep: a snapshot still `running` belonged to a process that no
    /// longer exists -- mark it failed with an explicit line, so a client
    /// polling the id learns the truth instead of waiting forever. Then prune
    /// to the newest `keep` snapshots (ids are zero-padded millis + counter,
    /// so lexical order is chronological).
    pub async fn sweep_job_snapshots(&self, keep: usize) -> Result<usize, ApiError> {
        let dir = self.root.join("jobs");
        let mut entries = match fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => return Ok(0), // no jobs dir yet
        };
        let mut ids: Vec<String> = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(io_err)? {
            if let Some(name) = entry.file_name().to_str()
                && let Some(stem) = name.strip_suffix(".json")
            {
                ids.push(stem.to_string());
            }
        }
        let mut interrupted = 0usize;
        for id in &ids {
            let Some(mut snap) = self.load_job_snapshot(id).await? else {
                continue;
            };
            if snap.status == crate::jobs::Status::Running {
                snap.status = crate::jobs::Status::Failed;
                snap.log.push("interrupted by service restart".to_string());
                self.save_job_snapshot(&snap).await?;
                interrupted += 1;
            }
        }
        ids.sort();
        if ids.len() > keep {
            for id in &ids[..ids.len() - keep] {
                let _ = fs::remove_file(dir.join(format!("{id}.json"))).await;
            }
        }
        Ok(interrupted)
    }
}

/// Job ids as `JobRegistry::create` mints them: lowercase hex + `-`.
fn is_safe_job_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase() || c == '-')
}

/// The slice of a manifest the version listing needs: identity, timestamp,
/// fingerprint, and the array LENGTHS. `IgnoredAny` elements make serde count
/// `mods`/`assets` without materializing entries, so listing a pack's history
/// stays cheap no matter how large its manifests grow.
#[derive(serde::Deserialize)]
struct ManifestHead {
    pack_version: String,
    #[serde(default)]
    channel: Option<VersionChannel>,
    #[serde(default)]
    changelog: Option<String>,
    generated_at: String,
    #[serde(default)]
    fingerprint: Option<String>,
    #[serde(default)]
    mods: Vec<serde::de::IgnoredAny>,
    #[serde(default)]
    assets: Vec<serde::de::IgnoredAny>,
}

/// The stored channel wins; a manifest from before the field falls back to
/// the legacy string rule.
fn build_info_from_head(head: ManifestHead) -> ManifestBuildInfo {
    ManifestBuildInfo {
        version_type: head
            .channel
            .unwrap_or_else(|| legacy_version_channel(&head.pack_version)),
        version_number: head.pack_version,
        date_published: head.generated_at,
        fingerprint: head.fingerprint,
        changelog: head.changelog,
        mods_count: head.mods.len() as u64,
        assets_count: head.assets.len() as u64,
    }
}

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

    fn manifest(version: &str, generated_at: &str, mods: usize) -> PackManifest {
        PackManifest {
            schema_version: 2,
            pack_id: "Industrial".into(),
            pack_version: version.into(),
            channel: None,
            changelog: None,
            generated_at: generated_at.into(),
            fingerprint: Some(format!("fp-{version}")),
            minecraft: MinecraftSpec {
                version: "1.12.2".into(),
            },
            loader: LoaderSpec {
                name: "forge".into(),
                version: "14.23.5.2922".into(),
            },
            java: JavaSpec { major: 8 },
            mods: (0..mods)
                .map(|i| ModEntry {
                    filename: format!("m{i}.jar"),
                    sha1: format!("sha{i}"),
                    size_bytes: 1,
                    required: true,
                    default_enabled: true,
                    source: Source::SmrtCache { url: "u".into() },
                    display: None,
                    slug: None,
                })
                .collect(),
            assets: vec![],
        }
    }

    #[tokio::test]
    async fn manifest_builds_carry_metadata_newest_first_and_resolve_latest() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        // an operator release, then two panel snapshots on a later day
        s.save_manifest(
            "Industrial",
            &manifest("2026.05.22.2", "2026-05-22T10:00:00Z", 3),
        )
        .await
        .unwrap();
        s.save_manifest(
            "Industrial",
            &manifest("SNAPSHOT-0.0.0-2026.07.18", "2026-07-18T09:00:00Z", 4),
        )
        .await
        .unwrap();
        s.save_manifest(
            "Industrial",
            &manifest("SNAPSHOT-0.0.0-2026.07.18.1", "2026-07-18T11:00:00Z", 5),
        )
        .await
        .unwrap();
        s.set_latest_manifest("Industrial", "SNAPSHOT-0.0.0-2026.07.18.1")
            .await
            .unwrap();

        let builds = s.list_manifest_builds("Industrial").await.unwrap();
        let versions: Vec<&str> = builds.iter().map(|b| b.version_number.as_str()).collect();
        assert_eq!(
            versions,
            vec![
                "SNAPSHOT-0.0.0-2026.07.18.1",
                "SNAPSHOT-0.0.0-2026.07.18",
                "2026.05.22.2"
            ],
            "newest first by built_at"
        );
        // no stored channel on these -> the legacy string rule applies
        assert_eq!(builds[0].version_type, VersionChannel::Beta);
        assert_eq!(builds[2].version_type, VersionChannel::Release);
        assert_eq!(builds[0].mods_count, 5);
        assert_eq!(
            builds[0].fingerprint.as_deref(),
            Some("fp-SNAPSHOT-0.0.0-2026.07.18.1")
        );
        assert_eq!(builds[0].date_published, "2026-07-18T11:00:00Z");

        assert_eq!(
            s.latest_manifest_version("Industrial").await.unwrap(),
            Some("SNAPSHOT-0.0.0-2026.07.18.1".to_string())
        );
        let info = s.latest_build_info("Industrial").await.unwrap().unwrap();
        assert_eq!(info.version_number, "SNAPSHOT-0.0.0-2026.07.18.1");
        assert_eq!(info.version_type, VersionChannel::Beta);

        // a pack with no builds yields no latest, not an error
        assert_eq!(s.latest_manifest_version("Ghost").await.unwrap(), None);
        assert!(s.latest_build_info("Ghost").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn stored_channel_wins_over_the_legacy_string_rule() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        // a modern build: semver label, channel stored on the manifest
        let mut m = manifest("0.0.3", "2026-07-19T10:00:00Z", 2);
        m.channel = Some(VersionChannel::Alpha);
        s.save_manifest("Industrial", &m).await.unwrap();
        s.set_latest_manifest("Industrial", "0.0.3").await.unwrap();

        let info = s.latest_build_info("Industrial").await.unwrap().unwrap();
        assert_eq!(info.version_number, "0.0.3");
        assert_eq!(
            info.version_type,
            VersionChannel::Alpha,
            "the stored channel wins; the string rule would have said release"
        );
    }

    #[tokio::test]
    async fn job_snapshot_sweep_marks_orphans_and_prunes() {
        use crate::jobs::{JobSnapshot, Status};
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        let snap = |id: &str, status: Status| JobSnapshot {
            job_id: id.into(),
            kind: "build".into(),
            pack_id: "Industrial".into(),
            status,
            log: vec!["building".into()],
        };
        // oldest done, middle running (orphan), newest done
        s.save_job_snapshot(&snap("0000000000001-00000000", Status::Done))
            .await
            .unwrap();
        s.save_job_snapshot(&snap("0000000000002-00000000", Status::Running))
            .await
            .unwrap();
        s.save_job_snapshot(&snap("0000000000003-00000000", Status::Done))
            .await
            .unwrap();

        let interrupted = s.sweep_job_snapshots(2).await.unwrap();
        assert_eq!(interrupted, 1, "one orphaned running job");

        // the orphan is now failed, with the reason on its log
        let orphan = s
            .load_job_snapshot("0000000000002-00000000")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(orphan.status, Status::Failed);
        assert_eq!(
            orphan.log.last().map(String::as_str),
            Some("interrupted by service restart")
        );
        // pruned to the newest two: the oldest snapshot is gone
        assert!(
            s.load_job_snapshot("0000000000001-00000000")
                .await
                .unwrap()
                .is_none(),
            "oldest pruned past the keep bound"
        );
        assert!(
            s.load_job_snapshot("0000000000003-00000000")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn manifest_version_strings_sort_by_tuple_not_string() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().to_path_buf());
        for v in ["2026.05.22.10", "2026.05.22.2"] {
            s.save_manifest("Industrial", &manifest(v, "2026-05-22T00:00:00Z", 1))
                .await
                .unwrap();
        }
        assert_eq!(
            s.list_manifest_versions("Industrial").await.unwrap(),
            vec!["2026.05.22.2".to_string(), "2026.05.22.10".to_string()],
            ".10 lands after .2 under tuple comparison"
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
            .duplicate_pack(
                "Industrial",
                "Industrial-cleanroom",
                Some(cleanroom),
                7,
                None,
            )
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
            s.duplicate_pack("Industrial", "Industrial", None, 1, None)
                .await,
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
