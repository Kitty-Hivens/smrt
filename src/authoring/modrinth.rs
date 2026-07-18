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
        // No such project is not an error: it means there is no icon to show (a
        // project deleted or gone from Modrinth, or a stale id). The caller renders
        // a letter avatar for `None`; only a real upstream fault is an error.
        if status.as_u16() == 404 {
            return Ok(None);
        }
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

    /// Batch project lookup by id/slug for harvest enrichment: title, slug, and
    /// the owning team id (the project object carries no author username -- that
    /// comes from [`team_owners_by_ids`]). Chunked like the sha1 lookup. Ids with
    /// no match are simply absent from the returned map.
    pub async fn projects_by_ids(&self, ids: &[String]) -> Result<HashMap<String, Project>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(BATCH_SIZE) {
            let encoded = serde_json::to_string(chunk).context("encode project ids")?;
            let resp = self
                .http
                .get(format!("{MODRINTH_BASE}/v2/projects"))
                .query(&[("ids", encoded.as_str())])
                .send()
                .await
                .context("projects get")?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(anyhow!("modrinth projects HTTP {status}: {body}"));
            }
            let list: Vec<Project> = resp.json().await.context("decode projects")?;
            for p in list {
                out.insert(p.id.clone(), p);
            }
        }
        Ok(out)
    }

    /// Batch team lookup -> the owner's username per team id. Modrinth returns one
    /// member array per requested team; the `Owner`-role member is the author we
    /// attribute the mod to. Teams with no owner row are absent from the map.
    pub async fn team_owners_by_ids(&self, team_ids: &[String]) -> Result<HashMap<String, String>> {
        let mut out = HashMap::new();
        for chunk in team_ids.chunks(BATCH_SIZE) {
            let encoded = serde_json::to_string(chunk).context("encode team ids")?;
            let resp = self
                .http
                .get(format!("{MODRINTH_BASE}/v2/teams"))
                .query(&[("ids", encoded.as_str())])
                .send()
                .await
                .context("teams get")?;
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(anyhow!("modrinth teams HTTP {status}: {body}"));
            }
            // shape: [[member, ...], [member, ...]] -- one inner array per team
            let teams: Vec<Vec<TeamMember>> = resp.json().await.context("decode teams")?;
            for members in teams {
                if let Some(owner) = members
                    .iter()
                    .find(|m| m.role.eq_ignore_ascii_case("owner"))
                {
                    out.insert(owner.team_id.clone(), owner.user.username.clone());
                }
            }
        }
        Ok(out)
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

    /// Fetch a remote image (e.g. a GitHub avatar) through the pooled client,
    /// returning its bytes and content type. Reuses the shared client so an
    /// image proxy does not open a TLS handshake per request, and caps small --
    /// avatars are tiny, and this bounds what the proxy will pull. A non-image
    /// content type is normalised to `image/png`; the caller serves it `nosniff`.
    pub async fn fetch_image(&self, url: &str) -> Result<(Vec<u8>, String)> {
        const MAX_BYTES: u64 = 8 * 1024 * 1024;
        let resp = self.http.get(url).send().await.context("image GET")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!("image HTTP {status} for {url}"));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.starts_with("image/"))
            .unwrap_or("image/png")
            .to_string();
        if let Some(len) = resp.content_length()
            && len > MAX_BYTES
        {
            return Err(anyhow!("image at {url} is {len} bytes, over the cap"));
        }
        let bytes = resp.bytes().await.context("image body")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(anyhow!("image at {url} exceeds the cap"));
        }
        Ok((bytes.to_vec(), content_type))
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

/// A project as the batch `/v2/projects` lookup returns it -- the subset harvest
/// enrichment reads. `team` is the id used to resolve the author username.
#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub team: String,
    /// Declared environment flags: `required` | `optional` | `unsupported`
    /// (upstream also ships `unknown`). Authored by the project owner, so they
    /// can be wrong -- the classifier maps them with priority but the resolve
    /// report surfaces disagreements with the bytecode derivation.
    #[serde(default)]
    pub client_side: String,
    #[serde(default)]
    pub server_side: String,
}

#[derive(Deserialize)]
struct TeamMember {
    team_id: String,
    role: String,
    user: TeamUser,
}

#[derive(Deserialize)]
struct TeamUser {
    username: String,
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
    /// Project owner's username -- a denormalized convenience Modrinth ships only
    /// on search hits (the project object has no author field). The panel shows it
    /// as a pick-time facet.
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    /// Release channel: `release` | `beta` | `alpha`. Lets the panel filter out
    /// pre-releases so the operator isn't wading through every snapshot.
    #[serde(default)]
    pub version_type: String,
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
/// namespace); `version_id` pins an exact version when set; `dependency_type` is
/// required|optional|incompatible|embedded. `file_name` is Modrinth's slot for
/// an external dependency (`project_id` null): the file the mod needs that
/// lives outside Modrinth -- the hybrid-resolution key for matching it against
/// the mirror's own cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub dependency_type: String,
    #[serde(default)]
    pub file_name: Option<String>,
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
