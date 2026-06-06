use anyhow::{Context, Result, anyhow};
use reqwest::{Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

const MODRINTH_BASE: &str = "https://api.modrinth.com";
const USER_AGENT: &str = "Kitty-Hivens/smrt-pack (+https://github.com/Kitty-Hivens/smrt)";
const BATCH_SIZE: usize = 100;

pub struct Modrinth {
    http: Client,
}

impl Modrinth {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(30))
            .redirect(Policy::limited(5))
            .build()
            .context("modrinth http client")?;
        Ok(Self { http })
    }

    /// Batch lookup by sha1. Modrinth tolerates up to 100 hashes per call;
    /// chunk transparently. Hashes with no match are simply absent from the
    /// returned map -- the caller distinguishes "not on Modrinth" from a hard
    /// API failure by the absence of an Err.
    pub async fn version_files_by_sha1(
        &self,
        hashes: &[String],
    ) -> Result<HashMap<String, Version>> {
        let mut out = HashMap::new();
        for chunk in hashes.chunks(BATCH_SIZE) {
            let body = VersionFilesRequest {
                hashes: chunk,
                algorithm: "sha1",
            };
            let resp = self
                .http
                .post(format!("{MODRINTH_BASE}/v2/version_files"))
                .json(&body)
                .send()
                .await
                .context("version_files post")?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(anyhow!("modrinth version_files HTTP {status}: {body}"));
            }
            let map: HashMap<String, Version> = resp.json().await.context("decode")?;
            out.extend(map);
        }
        Ok(out)
    }

    /// A project's icon URL -- the launcher's `ModIconResolver` runtime
    /// fallback when a manifest entry carries no `display.icon_url`. `None`
    /// when the project has no icon. Slug or numeric id both work.
    pub async fn project_icon(&self, slug_or_id: &str) -> Result<Option<String>> {
        let resp = self
            .http
            .get(format!("{MODRINTH_BASE}/v2/project/{slug_or_id}"))
            .send()
            .await
            .context("project get")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("modrinth project HTTP {status}: {body}"));
        }
        let p: ProjectIcon = resp.json().await.context("decode project")?;
        Ok(p.icon_url.filter(|s| !s.is_empty()))
    }

    pub async fn project_version(&self, project_id: &str, version_id: &str) -> Result<Version> {
        let resp = self
            .http
            .get(format!(
                "{MODRINTH_BASE}/v2/project/{project_id}/version/{version_id}"
            ))
            .send()
            .await
            .context("project version get")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("modrinth project version HTTP {status}: {body}"));
        }
        resp.json().await.context("decode")
    }

    /// Lists all published versions of [slug_or_id], newest first.
    /// When [mc_filter] is Some, narrows to versions whose
    /// `game_versions` array contains that exact MC string.
    ///
    /// Search Modrinth for mods matching [query], optionally narrowed to an MC
    /// version. Returns the top hits (project id / slug / title / icon) so the
    /// panel can add a mod without the operator pasting ids.
    pub async fn search(
        &self,
        query: &str,
        mc: Option<&str>,
        project_type: &str,
    ) -> Result<Vec<SearchHit>> {
        // project_type is one of Modrinth's known kinds (mod / resourcepack /
        // shader); the caller picks it so the panel can browse packs, not just mods.
        // build the facets as JSON so an mc/query value carrying quotes or
        // brackets can't reshape the structure sent upstream
        let mut groups: Vec<Vec<String>> = vec![vec![format!("project_type:{project_type}")]];
        if let Some(v) = mc {
            groups.push(vec![format!("versions:{v}")]);
        }
        let facets = serde_json::to_string(&groups).context("encode facets")?;
        let resp = self
            .http
            .get(format!("{MODRINTH_BASE}/v2/search"))
            .query(&[
                ("query", query),
                ("facets", facets.as_str()),
                ("limit", "20"),
            ])
            .send()
            .await
            .context("modrinth search get")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("modrinth search HTTP {status}: {body}"));
        }
        let parsed: SearchResponse = resp.json().await.context("decode search")?;
        Ok(parsed.hits)
    }

    /// Slug or numeric project id both work as the URL segment per
    /// the Modrinth API spec.
    pub async fn project_versions(
        &self,
        slug_or_id: &str,
        mc_filter: Option<&str>,
    ) -> Result<Vec<Version>> {
        let mut url = format!("{MODRINTH_BASE}/v2/project/{slug_or_id}/version");
        if let Some(mc) = mc_filter {
            // The Modrinth API expects a JSON-encoded array in this
            // query param, then percent-encoded. `["1.12.2"]` becomes
            // `%5B%221.12.2%22%5D`.
            let encoded = format!("[\"{mc}\"]");
            let qp =
                percent_encoding::utf8_percent_encode(&encoded, percent_encoding::NON_ALPHANUMERIC);
            url.push_str("?game_versions=");
            url.push_str(&qp.to_string());
        }
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("project versions get")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("modrinth project versions HTTP {status}: {body}"));
        }
        resp.json().await.context("decode")
    }

    /// Generic GET for artifacts hosted outside Modrinth (GitHub release
    /// assets), reusing the pooled client + UA. Follows redirects.
    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        // cap the response so a huge or malicious release asset can't OOM the
        // mirror; GitHub release downloads always send a content-length
        const MAX_BYTES: u64 = 512 * 1024 * 1024;
        let mut resp = self.http.get(url).send().await.context("asset GET")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!("asset HTTP {status} for {url}"));
        }
        // content-length is an early reject; a missing or lying header is still
        // bounded by counting the bytes we actually read
        if let Some(len) = resp.content_length()
            && len > MAX_BYTES
        {
            return Err(anyhow!(
                "asset at {url} is {len} bytes, over the {} MiB cap",
                MAX_BYTES / 1024 / 1024
            ));
        }
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp.chunk().await.context("asset body")? {
            if buf.len() as u64 + chunk.len() as u64 > MAX_BYTES {
                return Err(anyhow!(
                    "asset at {url} exceeds the {} MiB cap",
                    MAX_BYTES / 1024 / 1024
                ));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }
}

#[derive(Serialize)]
struct VersionFilesRequest<'a> {
    hashes: &'a [String],
    algorithm: &'static str,
}

#[derive(Deserialize)]
struct ProjectIcon {
    #[serde(default)]
    icon_url: Option<String>,
}

#[derive(Deserialize)]
struct SearchResponse {
    hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    pub files: Vec<VersionFile>,
    /// Project-level deps (required/optional/incompatible/embedded). Additive,
    /// no wire impact; the registry harvest reads these for relation facts.
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

/// A Modrinth version dependency. `project_id` is the target (Modrinth
/// namespace); `dependency_type` is required|optional|incompatible|embedded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub dependency_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFile {
    pub hashes: VersionFileHashes,
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionFileHashes {
    pub sha1: String,
}

impl Version {
    /// Pick the file flagged `primary: true`; if none is, fall back to the
    /// first entry. Matches the wire-spec rule for source resolution.
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| self.files.first())
    }
}
