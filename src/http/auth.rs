//! Panel auth. Two ways in:
//!   - GitHub OAuth (the daily path): sign in with GitHub; if the account's uid
//!     is on the admin allowlist, a server-side session opens and the browser
//!     holds only an opaque session id.
//!   - the admin token (break-glass): the CLI sends it as `Authorization:
//!     Bearer`, or it is typed into the panel's fallback form. It opens an admin
//!     session too.
//!
//! `require_auth` accepts a valid session cookie or a bearer token and attaches
//! the resolved [`Identity`] to the request. SameSite is the CSRF defence: the
//! session cookie is Strict; the short-lived OAuth `state` cookie is Lax so it
//! survives GitHub's cross-site redirect back to the callback.

use super::ApiError;
use super::session::{Identity, Role, random_token};
use crate::state::AppState;
use axum::extract::{Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::json;

const COOKIE_NAME: &str = "smrt_session";
const STATE_COOKIE: &str = "smrt_oauth_state";
const MAX_AGE_SECS: u32 = 86_400;
const STATE_MAX_AGE_SECS: u32 = 600;
const GH_AUTHORIZE: &str = "https://github.com/login/oauth/authorize";
const GH_TOKEN: &str = "https://github.com/login/oauth/access_token";
const GH_USER: &str = "https://api.github.com/user";

pub fn router(state: AppState) -> Router {
    let guard = from_fn_with_state(state.clone(), require_auth);
    Router::new()
        .route("/admin/api/login", post(login))
        .route("/admin/api/auth/github/login", get(github_login))
        .route("/admin/api/auth/github/callback", get(github_callback))
        .route(
            "/admin/api/session",
            get(session).route_layer(guard.clone()),
        )
        .route("/admin/api/logout", post(logout).route_layer(guard))
        .with_state(state)
}

// -- break-glass admin-token login ------------------------------------------

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
    let id = state.sessions.create(break_glass());
    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            session_cookie(&id, state.config.cookie_secure, false),
        )],
        Json(json!({ "authenticated": true, "login": "break-glass", "role": "admin" })),
    )
        .into_response()
}

// -- GitHub OAuth -----------------------------------------------------------

async fn github_login(State(state): State<AppState>) -> Response {
    let Some(client_id) = state.config.github_client_id.as_deref() else {
        // Not configured: bounce to the panel, which still offers the token form.
        return Redirect::to("/admin/?auth=unconfigured").into_response();
    };
    let csrf = random_token();
    let url = format!(
        "{GH_AUTHORIZE}?client_id={}&redirect_uri={}&scope=read:user&state={}&allow_signup=false",
        enc(client_id),
        enc(&callback_uri(&state)),
        enc(&csrf),
    );
    let mut resp = Redirect::to(&url).into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        header_val(&state_cookie(&csrf, state.config.cookie_secure, false)),
    );
    resp
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

async fn github_callback(
    State(state): State<AppState>,
    Query(q): Query<CallbackQuery>,
    req: Request,
) -> Response {
    let secure = state.config.cookie_secure;
    let cookie_state = req
        .headers()
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| cookie_value(h, STATE_COOKIE));

    // The state param must be present and match the cookie the redirect set --
    // the CSRF check on the callback.
    let (Some(code), Some(ret_state), Some(exp_state)) = (q.code, q.state, cookie_state) else {
        return redirect_clearing_state("/admin/?auth=failed", secure);
    };
    if !constant_time_eq(ret_state.as_bytes(), exp_state.as_bytes()) {
        return redirect_clearing_state("/admin/?auth=failed", secure);
    }
    let (Some(cid), Some(secret)) = (
        state.config.github_client_id.as_deref(),
        state.config.github_client_secret.as_deref(),
    ) else {
        return redirect_clearing_state("/admin/?auth=unconfigured", secure);
    };

    let identity = match exchange_and_fetch(
        cid,
        secret,
        &code,
        &callback_uri(&state),
        &state.config.admin_github_uids,
    )
    .await
    {
        Ok(Some(id)) => id,
        Ok(None) => return redirect_clearing_state("/admin/?auth=denied", secure),
        Err(_) => return redirect_clearing_state("/admin/?auth=failed", secure),
    };

    let sid = state.sessions.create(identity);
    let mut resp = Redirect::to("/admin/").into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        header_val(&session_cookie(&sid, secure, false)),
    );
    resp.headers_mut().append(
        header::SET_COOKIE,
        header_val(&state_cookie("", secure, true)),
    );
    resp
}

/// Exchange the OAuth code for an access token, read the GitHub user, and map
/// them to an [`Identity`] iff their uid is on the admin allowlist. `Ok(None)`
/// means a valid GitHub account that is not an authorized operator.
async fn exchange_and_fetch(
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    admin_uids: &[u64],
) -> anyhow::Result<Option<Identity>> {
    let http = reqwest::Client::builder().user_agent("smrt").build()?;

    #[derive(Deserialize)]
    struct TokenResp {
        access_token: Option<String>,
    }
    let tok: TokenResp = http
        .post(GH_TOKEN)
        .header(header::ACCEPT, "application/json")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await?
        .json()
        .await?;
    let Some(access) = tok.access_token else {
        anyhow::bail!("github returned no access_token");
    };

    #[derive(Deserialize)]
    struct GhUser {
        id: u64,
        login: String,
    }
    let user: GhUser = http
        .get(GH_USER)
        .header(header::AUTHORIZATION, format!("Bearer {access}"))
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(admin_uids.contains(&user.id).then_some(Identity {
        uid: user.id,
        login: user.login,
        role: Role::Admin,
    }))
}

// -- session + logout -------------------------------------------------------

async fn session(Extension(id): Extension<Identity>) -> Json<serde_json::Value> {
    Json(json!({
        "authenticated": true,
        "uid": id.uid,
        "login": id.login,
        "role": id.role.as_str(),
    }))
}

async fn logout(State(state): State<AppState>, req: Request) -> Response {
    if let Some(sid) = req
        .headers()
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| cookie_value(h, COOKIE_NAME))
    {
        state.sessions.remove(&sid);
    }
    (
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            session_cookie("", state.config.cookie_secure, true),
        )],
    )
        .into_response()
}

// -- middleware -------------------------------------------------------------

/// Accept a bearer admin token (break-glass) or a valid session cookie, and
/// attach the resolved [`Identity`] to the request for downstream handlers. A
/// present-but-wrong bearer falls through to the cookie rather than rejecting.
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_string);
    if let (Some(tok), Some(expected)) = (&bearer, state.config.admin_token.as_deref())
        && constant_time_eq(expected.as_bytes(), tok.as_bytes())
    {
        req.extensions_mut().insert(break_glass());
        return Ok(next.run(req).await);
    }

    let sid = req
        .headers()
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| cookie_value(h, COOKIE_NAME));
    if let Some(identity) = sid.and_then(|id| state.sessions.get(&id)) {
        req.extensions_mut().insert(identity);
        return Ok(next.run(req).await);
    }

    Err(ApiError::Unauthorized)
}

// -- helpers ----------------------------------------------------------------

fn break_glass() -> Identity {
    Identity {
        uid: 0,
        login: "break-glass".into(),
        role: Role::Admin,
    }
}

fn callback_uri(state: &AppState) -> String {
    format!(
        "{}/admin/api/auth/github/callback",
        state.config.mirror_base
    )
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn header_val(cookie: &str) -> header::HeaderValue {
    // cookies are ASCII by construction here (hex ids, fixed attributes)
    header::HeaderValue::from_str(cookie).expect("cookie is valid header value")
}

fn redirect_clearing_state(to: &str, secure: bool) -> Response {
    let mut resp = Redirect::to(to).into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        header_val(&state_cookie("", secure, true)),
    );
    resp
}

fn session_cookie(value: &str, secure: bool, clear: bool) -> String {
    let max_age = if clear { 0 } else { MAX_AGE_SECS };
    build_cookie(COOKIE_NAME, value, secure, "Strict", "/", max_age)
}

fn state_cookie(value: &str, secure: bool, clear: bool) -> String {
    // Lax, not Strict: the callback is a top-level navigation from github.com,
    // and a Strict cookie would be withheld on that cross-site redirect.
    let max_age = if clear { 0 } else { STATE_MAX_AGE_SECS };
    build_cookie(
        STATE_COOKIE,
        value,
        secure,
        "Lax",
        "/admin/api/auth",
        max_age,
    )
}

fn build_cookie(
    name: &str,
    value: &str,
    secure: bool,
    same_site: &str,
    path: &str,
    max_age: u32,
) -> String {
    let mut c =
        format!("{name}={value}; HttpOnly; SameSite={same_site}; Path={path}; Max-Age={max_age}");
    if secure {
        c.push_str("; Secure");
    }
    c
}

fn cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    cookie_header
        .split(';')
        .map(str::trim)
        .find_map(|kv| kv.strip_prefix(&prefix).map(str::to_string))
}

/// Constant-time byte comparison: avoid leaking how many leading bytes of a
/// presented secret matched via early-exit timing.
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
    fn cookie_value_extracts_named_cookie() {
        assert_eq!(
            cookie_value("foo=1; smrt_session=tok123; bar=2", COOKIE_NAME).as_deref(),
            Some("tok123")
        );
        assert_eq!(
            cookie_value("smrt_oauth_state=xyz", STATE_COOKIE).as_deref(),
            Some("xyz")
        );
        assert_eq!(cookie_value("other=x", COOKIE_NAME), None);
    }

    #[test]
    fn state_cookie_is_lax_and_scoped_to_the_callback_path() {
        let c = state_cookie("nonce", true, false);
        assert!(c.contains("SameSite=Lax"));
        assert!(c.contains("Path=/admin/api/auth"));
        assert!(c.contains("Secure"));
    }

    #[test]
    fn session_cookie_is_strict_and_cleared_with_zero_max_age() {
        assert!(session_cookie("id", false, false).contains("SameSite=Strict"));
        assert!(session_cookie("", false, true).contains("Max-Age=0"));
    }
}
