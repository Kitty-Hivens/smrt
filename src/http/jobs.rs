//! Build-job endpoints: trigger a build, poll its status, and tail the live
//! log over Server-Sent Events. All admin-gated. The orchestration lives in
//! `crate::jobs`; these handlers are the thin HTTP surface.

use super::ApiError;
use crate::jobs::Status;
use crate::state::AppState;
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::middleware::from_fn_with_state;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Serialize;
use std::convert::Infallible;
use std::time::Duration;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/admin/packs/:pack_id/build", post(build_pack))
        .route("/v1/admin/jobs/:job_id", get(job_status))
        .route("/v1/admin/jobs/:job_id/events", get(job_events))
        .layer(from_fn_with_state(state.clone(), super::auth::require_auth))
        .with_state(state)
}

#[derive(Serialize)]
struct JobRef {
    job_id: String,
    kind: &'static str,
    pack_id: String,
}

async fn build_pack(State(state): State<AppState>, Path(pack_id): Path<String>) -> Json<JobRef> {
    let job = state
        .jobs
        .spawn_build(pack_id, state.storage.clone(), state.config.clone());
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
