use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

    pub async fn project_version(
        &self,
        project_id: &str,
        version_id: &str,
    ) -> Result<Version> {
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
        Ok(resp.json().await.context("decode")?)
    }
}

#[derive(Serialize)]
struct VersionFilesRequest<'a> {
    hashes: &'a [String],
    algorithm: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionFile {
    pub hashes: VersionFileHashes,
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
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
