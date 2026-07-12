//! Panel auth over the persistent accounts store. Human sign-in is GitHub-only:
//!   - GitHub OAuth (the only human path): sign in with GitHub; the callback
//!     upserts a `users` row (role from the operator allowlist) and opens a
//!     server-side session. Any GitHub account can identify; the admin role is
//!     what unlocks the operator panel.
//!   - the admin token is machine auth only: the CLI/pipeline sends it as
//!     `Authorization: Bearer` and `resolve_identity` maps it to a synthetic
//!     admin. It is no longer a human login -- the panel's token form is
//!     deprecated, and a valid token there returns 410 and opens no session.
//!
//! `require_auth` guards the admin API: it resolves the caller's [`Identity`]
//! from the session cookie or a bearer token and requires the admin role.
//! `/v1/me` reports the current identity for any authenticated user. SameSite is
//! the CSRF defence: the session cookie is Strict; the short-lived OAuth `state`
//! cookie is Lax so it survives GitHub's cross-site redirect back.

use super::ApiError;
use crate::accounts::{Identity, Role, random_token};
use crate::state::AppState;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
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
    Router::new()
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/github/login", get(github_login))
        .route("/v1/auth/github/callback", get(github_callback))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/me", get(me))
        .with_state(state)
}

// -- deprecated admin-token form login --------------------------------------

#[derive(Deserialize)]
struct LoginReq {
    token: String,
}

/// The panel's legacy token form. The admin token is machine auth only now (a
/// `Bearer` header, resolved in `resolve_identity`); it is no longer a human
/// login. A valid token here is answered with 410 and no session, so the panel
/// can tell the operator the path is gone; an invalid one is a plain 401.
async fn login(State(state): State<AppState>, Json(req): Json<LoginReq>) -> Response {
    let Some(expected) = state.config.admin_token.as_deref() else {
        return ApiError::Unauthorized.into_response();
    };
    if !constant_time_eq(expected.as_bytes(), req.token.as_bytes()) {
        return ApiError::Unauthorized.into_response();
    }
    (StatusCode::GONE, Json(json!({ "deprecated": true }))).into_response()
}

// -- GitHub OAuth -----------------------------------------------------------

async fn github_login(State(state): State<AppState>) -> Response {
    let Some(client_id) = state.config.github_client_id.as_deref() else {
        // Not configured: bounce to the panel, which still offers the token form.
        return Redirect::to("/?auth=unconfigured").into_response();
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
    let cookie_state = cookie_value(req.headers(), STATE_COOKIE);

    // The state param must be present and match the cookie the redirect set --
    // the CSRF check on the callback.
    let (Some(code), Some(ret_state), Some(exp_state)) = (q.code, q.state, cookie_state) else {
        return redirect_clearing_state("/?auth=failed", secure);
    };
    if !constant_time_eq(ret_state.as_bytes(), exp_state.as_bytes()) {
        return redirect_clearing_state("/?auth=failed", secure);
    }
    let (Some(cid), Some(secret)) = (
        state.config.github_client_id.as_deref(),
        state.config.github_client_secret.as_deref(),
    ) else {
        return redirect_clearing_state("/?auth=unconfigured", secure);
    };

    // Exchange the code and read the GitHub account. Every valid account gets an
    // identity; the allowlist only sets whether that identity is an admin.
    let (uid, login) = match exchange_and_fetch(cid, secret, &code, &callback_uri(&state)).await {
        Ok(user) => user,
        Err(_) => return redirect_clearing_state("/?auth=failed", secure),
    };
    let is_admin = state.config.admin_github_uids.contains(&uid);

    let acc = state.accounts.clone();
    let sid =
        match tokio::task::spawn_blocking(move || acc.sign_in_github(uid as i64, &login, is_admin))
            .await
        {
            Ok(Ok(sid)) => sid,
            _ => return redirect_clearing_state("/?auth=failed", secure),
        };

    let mut resp = Redirect::to("/").into_response();
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

/// Exchange the OAuth code for an access token and read the GitHub user, as
/// `(uid, login)`. The caller decides admin-ness against the allowlist.
async fn exchange_and_fetch(
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
) -> anyhow::Result<(u64, String)> {
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

    Ok((user.id, user.login))
}

// -- identity + logout ------------------------------------------------------

async fn me(State(state): State<AppState>, req: Request) -> Response {
    match resolve_identity(&state, req.headers()).await {
        Some(id) => Json(json!({
            "authenticated": true,
            "uid": id.uid,
            "login": id.login,
            "role": id.role.as_str(),
        }))
        .into_response(),
        None => ApiError::Unauthorized.into_response(),
    }
}

async fn logout(State(state): State<AppState>, req: Request) -> Response {
    if let Some(sid) = cookie_value(req.headers(), COOKIE_NAME) {
        let acc = state.accounts.clone();
        let _ = tokio::task::spawn_blocking(move || acc.delete_session(&sid)).await;
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

/// Guard the operator API: resolve the caller's identity and require the admin
/// role. A member is authenticated but has no operator surface yet, so they get
/// 403, not 401. The resolved identity is attached for downstream handlers.
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(identity) = resolve_identity(&state, req.headers()).await else {
        return Err(ApiError::Unauthorized);
    };
    if identity.role != Role::Admin {
        return Err(ApiError::Forbidden);
    }
    req.extensions_mut().insert(identity);
    Ok(next.run(req).await)
}

/// Resolve who is calling: a valid bearer admin token (break-glass) yields the
/// break-glass admin identity; otherwise the session cookie is looked up in the
/// accounts store. `None` means not authenticated.
async fn resolve_identity(state: &AppState, headers: &HeaderMap) -> Option<Identity> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_string);
    if let (Some(tok), Some(expected)) = (&bearer, state.config.admin_token.as_deref())
        && constant_time_eq(expected.as_bytes(), tok.as_bytes())
    {
        return Some(break_glass());
    }

    let sid = cookie_value(headers, COOKIE_NAME)?;
    let acc = state.accounts.clone();
    tokio::task::spawn_blocking(move || acc.session_identity(&sid))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
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
    format!("{}/v1/auth/github/callback", state.config.mirror_base)
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
    build_cookie(STATE_COOKIE, value, secure, "Lax", "/v1/auth", max_age)
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

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())?
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
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            "foo=1; smrt_session=tok123; bar=2".parse().unwrap(),
        );
        assert_eq!(cookie_value(&h, COOKIE_NAME).as_deref(), Some("tok123"));
        assert_eq!(cookie_value(&h, STATE_COOKIE), None);
    }

    #[test]
    fn state_cookie_is_lax_and_scoped_to_the_callback_path() {
        let c = state_cookie("nonce", true, false);
        assert!(c.contains("SameSite=Lax"));
        assert!(c.contains("Path=/v1/auth"));
        assert!(c.contains("Secure"));
    }

    #[test]
    fn session_cookie_is_strict_and_cleared_with_zero_max_age() {
        assert!(session_cookie("id", false, false).contains("SameSite=Strict"));
        assert!(session_cookie("", false, true).contains("Max-Age=0"));
    }
}
