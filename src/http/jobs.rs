//! Build-job endpoints: trigger a build, poll its status, and tail the live
//! log over Server-Sent Events. All admin-gated. The orchestration lives in
//! `crate::jobs`; these handlers are the thin HTTP surface.

use super::ApiError;
use crate::accounts::Identity;
use crate::authoring::BootstrapArgs;
use crate::domain::LoaderSpec;
use crate::jobs::{DryRun, Status};
use crate::state::AppState;
use axum::Extension;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::middleware::from_fn_with_state;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;

pub fn router(state: AppState) -> Router {
    build_router(state.clone()).merge(bootstrap_router(state))
}

/// Build + job polling: any signed-in member may reach it, but `build_pack`
/// gates on pack ownership so a member builds only their own community pack.
/// Job status/events poll by an unguessable job id, so they need only a session.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/authoring/packs/:pack_id/build", post(build_pack))
        .route("/v1/jobs/:job_id", get(job_status))
        .route("/v1/jobs/:job_id/events", get(job_events))
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
        .layer(from_fn_with_state(
            state.clone(),
            super::auth::require_session,
        ))
        .with_state(state)
}

/// Bootstrap-from-archive seeds an official pack from an SC export -- operator
/// content authoring, admin only.
fn bootstrap_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/authoring/packs/:pack_id/bootstrap",
            post(bootstrap_pack),
        )
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
        .layer(from_fn_with_state(state.clone(), super::auth::require_auth))
        .with_state(state)
}

#[derive(Serialize)]
struct JobRef {
    job_id: String,
    kind: &'static str,
    pack_id: String,
}

#[derive(Deserialize)]
struct BuildParams {
    /// `?dry_run=true` computes + stashes the manifest without publishing, so the
    /// panel can preview and diff before committing a real build.
    #[serde(default)]
    dry_run: bool,
    /// Optional canonical `pack_version` override; default is today's UTC slug.
    pack_version: Option<String>,
}

async fn build_pack(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
    Query(p): Query<BuildParams>,
) -> Result<Json<JobRef>, ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    let pack_version = p.pack_version.filter(|v| !v.trim().is_empty());
    let job = state.jobs.spawn_build(
        pack_id,
        state.storage.clone(),
        state.config.clone(),
        p.dry_run,
        pack_version,
        Some(state.harvest.clone()),
    );
    Ok(Json(JobRef {
        job_id: job.id.clone(),
        kind: job.kind,
        pack_id: job.pack_id.clone(),
    }))
}

#[derive(Deserialize)]
struct BootstrapParams {
    display_name: Option<String>,
    tagline: Option<String>,
    minecraft_version: String,
    loader_name: Option<String>,
    loader_version: String,
    java_major: Option<u32>,
}

async fn bootstrap_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Query(p): Query<BootstrapParams>,
    body: Bytes,
) -> Json<JobRef> {
    let nonempty = |s: Option<String>| s.filter(|v| !v.is_empty());
    let args = BootstrapArgs {
        pack_id: pack_id.clone(),
        display_name: nonempty(p.display_name).unwrap_or_else(|| pack_id.clone()),
        tagline: p.tagline.unwrap_or_default(),
        minecraft_version: p.minecraft_version,
        loader: LoaderSpec {
            name: nonempty(p.loader_name).unwrap_or_else(|| "forge".into()),
            version: p.loader_version,
        },
        java_major: p.java_major.unwrap_or(8),
        storage: state.storage.root().to_path_buf(),
    };
    let job = state
        .jobs
        .spawn_bootstrap(pack_id, args, body.to_vec(), state.storage.clone());
    Json(JobRef {
        job_id: job.id.clone(),
        kind: job.kind,
        pack_id: job.pack_id.clone(),
    })
}

#[derive(Serialize)]
struct JobStatusResp {
    job_id: String,
    kind: &'static str,
    pack_id: String,
    status: Status,
    log: Vec<String>,
    /// Present only for a finished dry-run (preview) build.
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<DryRun>,
}

async fn job_status(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<JobStatusResp>, ApiError> {
    let job = state.jobs.get(&job_id).ok_or(ApiError::NotFound)?;
    let (log, status) = job.since(0);
    Ok(Json(JobStatusResp {
        job_id: job.id.clone(),
        kind: job.kind,
        pack_id: job.pack_id.clone(),
        status,
        log,
        result: job.result(),
    }))
}

async fn job_events(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Response, ApiError> {
    let job = state.jobs.get(&job_id).ok_or(ApiError::NotFound)?;
    let stream = async_stream::stream! {
        let mut sent = 0usize;
        loop {
            let (lines, status) = job.since(sent);
            for line in lines {
                sent += 1;
                yield Ok::<Event, Infallible>(Event::default().event("line").data(line));
            }
            match status {
                Status::Running => {
                    // Notify wakes us immediately on a new line; the timeout
                    // bounds latency if a wake is ever missed (we re-read from
                    // `sent`, so nothing is lost either way).
                    let _ = tokio::time::timeout(Duration::from_millis(500), job.wait()).await;
                }
                Status::Done => {
                    yield Ok(Event::default().event("done").data("ok"));
                    break;
                }
                Status::Failed => {
                    yield Ok(Event::default().event("failed").data("failed"));
                    break;
                }
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response())
}
