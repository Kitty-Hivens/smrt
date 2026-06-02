//! Panel auth. The admin token is the single credential: the CLI sends it as
//! `Authorization: Bearer`, and the browser logs in once via `/admin/api/login`
//! after which the token rides in an HttpOnly, SameSite=Strict session cookie.
//! `require_auth` accepts either. SameSite=Strict is the CSRF defence for the
//! cookie-authenticated state-changing endpoints.

use super::ApiError;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

const COOKIE_NAME: &str = "smrt_session";
const MAX_AGE_SECS: u32 = 86_400;

pub fn router(state: AppState) -> Router {
    let guard = from_fn_with_state(state.clone(), require_auth);
    Router::new()
        .route("/admin/api/login", post(login))
        .route(
            "/admin/api/session",
            get(session).route_layer(guard.clone()),
        )
        .route("/admin/api/logout", post(logout).route_layer(guard))
        .with_state(state)
}

#[derive(Deserialize)]
struct LoginReq {
    token: String,
}

async fn login(State(state): State<AppState>, Json(req): Json<LoginReq>) -> Response {
    let Some(expected) = state.config.admin_token.as_deref() else {
        return ApiError::Unauthorized.into_response();
    };
    if !constant_time_eq(expected.as_bytes(), req.token.as_bytes()) {
        return ApiError::Unauthorized.into_response();
    }
    let cookie = build_cookie(&req.token, state.config.cookie_secure, false);
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "authenticated": true })),
    )
        .into_response()
}

async fn session() -> Json<serde_json::Value> {
    Json(json!({ "authenticated": true }))
}

async fn logout(State(state): State<AppState>) -> Response {
    let cookie = build_cookie("", state.config.cookie_secure, true);
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

/// Accept either `Authorization: Bearer <token>` (CLI) or the session cookie
/// (panel). The cookie holds the admin token directly; HttpOnly keeps it out
/// of JS and SameSite=Strict blocks cross-site (CSRF) sends.
pub async fn require_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let expected = state
        .config
        .admin_token
        .as_deref()
        .ok_or(ApiError::Unauthorized)?;
    let headers = req.headers();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_string);
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(session_cookie_value);
    match bearer.or(cookie) {
        Some(tok) if constant_time_eq(expected.as_bytes(), tok.as_bytes()) => {
            Ok(next.run(req).await)
        }
        _ => Err(ApiError::Unauthorized),
    }
}

fn session_cookie_value(cookie_header: &str) -> Option<String> {
    let prefix = format!("{COOKIE_NAME}=");
    cookie_header
        .split(';')
        .map(str::trim)
        .find_map(|kv| kv.strip_prefix(&prefix).map(str::to_string))
}

fn build_cookie(value: &str, secure: bool, clear: bool) -> String {
    let mut c = format!("{COOKIE_NAME}={value}; HttpOnly; SameSite=Strict; Path=/");
    if secure {
        c.push_str("; Secure");
    }
    if clear {
        c.push_str("; Max-Age=0");
    } else {
        c.push_str(&format!("; Max-Age={MAX_AGE_SECS}"));
    }
    c
}

/// Constant-time byte comparison: avoid leaking how many leading bytes of the
/// presented token matched via early-exit timing.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_and_rejects() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secre"));
        assert!(!constant_time_eq(b"secret", b"sekret"));
    }

    #[test]
    fn session_cookie_value_extracts_from_multi_cookie_header() {
        assert_eq!(
            session_cookie_value("foo=1; smrt_session=tok123; bar=2").as_deref(),
            Some("tok123")
        );
        assert_eq!(session_cookie_value("other=x").as_deref(), None);
    }
}
