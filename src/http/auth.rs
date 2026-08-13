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
use crate::accounts::{Identity, PackLevel, Role, random_token};
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
    // debug outranks admin: a uid on both allowlists is granted the higher rung.
    let forced_role = if state.config.debug_github_uids.contains(&uid) {
        Some(Role::Debug)
    } else if state.config.admin_github_uids.contains(&uid) {
        Some(Role::Admin)
    } else {
        None
    };

    let acc = state.accounts.clone();
    let sid = match tokio::task::spawn_blocking(move || {
        acc.sign_in_github(uid as i64, &login, forced_role)
    })
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
    let Some(id) = resolve_identity(&state, req.headers()).await else {
        return ApiError::Unauthorized.into_response();
    };
    let acc = state.accounts.clone();
    let uid = id.uid;
    let accepted_terms = tokio::task::spawn_blocking(move || acc.terms_accepted(uid))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    Json(json!({
        "authenticated": true,
        "uid": id.uid,
        "login": id.login,
        "role": id.role.as_str(),
        "accepted_terms": accepted_terms,
    }))
    .into_response()
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
    if identity.role < Role::Admin {
        return Err(ApiError::Forbidden);
    }
    req.extensions_mut().insert(identity);
    Ok(next.run(req).await)
}

/// Guard the debug surface: compat-affecting registry authoring (#39). Requires
/// the debug rung -- above admin -- so a plain admin (or a break-glass admin
/// token) cannot assert loaders/mc/version or dependency/conflict facts and
/// silently corrupt the derivation graph the resolver rides on.
pub async fn require_debug(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(identity) = resolve_identity(&state, req.headers()).await else {
        return Err(ApiError::Unauthorized);
    };
    if identity.role < Role::Debug {
        return Err(ApiError::Forbidden);
    }
    req.extensions_mut().insert(identity);
    Ok(next.run(req).await)
}

/// Guard a member-accessible endpoint: require any authenticated identity
/// (member and up) and attach it, without requiring the admin role. Own-resource
/// authorization (e.g. pack ownership via [`Identity::owns_or_admin`]) is the
/// handler's job.
pub async fn require_session(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(identity) = resolve_identity(&state, req.headers()).await else {
        return Err(ApiError::Unauthorized);
    };
    req.extensions_mut().insert(identity);
    Ok(next.run(req).await)
}

/// The uid a pack id's namespace belongs to: `Some(uid)` for a community id
/// (`u/<uid>/<pack>`), `None` for an official (flat) id. The owner is encoded in
/// the id, so ownership needs no store lookup.
pub(crate) fn pack_namespace_uid(pack_id: &str) -> Option<i64> {
    pack_id
        .strip_prefix("u/")
        .and_then(|r| r.split_once('/'))
        .and_then(|(uid, _)| uid.parse::<i64>().ok())
}

/// What this caller may do to `pack_id`, without asking the store.
///
/// Two answers need no lookup and never will: the owner of a community
/// namespace owns their pack because the id says so, and an admin owns every
/// pack because that is what the rung means. `None` means the answer is a grant
/// -- which is a read, so only [`authorize`] can finish the sentence.
fn inherent_level(identity: &Identity, pack_id: &str) -> Option<PackLevel> {
    if identity.role >= Role::Admin {
        return Some(PackLevel::Own);
    }
    match pack_namespace_uid(pack_id) {
        Some(uid) if identity.uid == uid => Some(PackLevel::Own),
        _ => None,
    }
}

/// May this caller do `need` to `pack_id`? `Forbidden` when not (ADR 0006).
///
/// One gate for every authored read and write, so the three levels are compared
/// in one place rather than re-derived per handler. Ownership and the admin rung
/// answer without touching the database; everything else is the pack's own
/// access list, which is what lets one person be let into one pack without being
/// handed the mirror.
pub(crate) async fn authorize(
    state: &AppState,
    identity: &Identity,
    pack_id: &str,
    need: PackLevel,
) -> Result<(), ApiError> {
    if let Some(level) = inherent_level(identity, pack_id) {
        return (level >= need).then_some(()).ok_or(ApiError::Forbidden);
    }
    let granted = granted_level(state, identity.uid, pack_id).await;
    match granted {
        Some(level) if level >= need => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

/// The level `pack_id` grants `uid`, or `None`. A store that cannot be read
/// grants nothing: an access check that fails open is not a check.
async fn granted_level(state: &AppState, uid: i64, pack_id: &str) -> Option<PackLevel> {
    let acc = state.accounts.clone();
    let pack = pack_id.to_string();
    tokio::task::spawn_blocking(move || acc.pack_access_level(&pack, uid))
        .await
        .ok()?
        .ok()?
}

/// Whether this caller may do `need`, as a question rather than a refusal --
/// for the handlers that answer differently instead of erroring (a public read
/// that hides a draft, a listing that filters).
pub(crate) async fn may(
    state: &AppState,
    identity: &Identity,
    pack_id: &str,
    need: PackLevel,
) -> bool {
    authorize(state, identity, pack_id, need).await.is_ok()
}

/// Whether somebody who is not the caller may do `need` here -- for a decision
/// about a third person, such as whether they are one of the pack's keepers and
/// so not somebody to block. A uid nobody has ever signed in as is treated as a
/// plain member, which is what they would be on their first login.
pub(crate) async fn may_uid(state: &AppState, pack_id: &str, uid: i64, need: PackLevel) -> bool {
    let acc = state.accounts.clone();
    let known = tokio::task::spawn_blocking(move || acc.identity_of(uid))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
    let identity = known.unwrap_or(Identity {
        uid,
        login: String::new(),
        role: Role::Member,
    });
    may(state, &identity, pack_id, need).await
}

/// Which of `packs` this caller may reach at `need`, with one read of the access
/// list rather than one per pack. The rule is the same one [`authorize`]
/// applies; only the number of round trips differs, which is what makes it worth
/// having for a listing that asks about every pack on the mirror.
pub(crate) async fn filter_may(
    state: &AppState,
    identity: &Identity,
    packs: Vec<String>,
    need: PackLevel,
) -> Vec<String> {
    let acc = state.accounts.clone();
    let uid = identity.uid;
    let granted: std::collections::HashMap<String, PackLevel> =
        tokio::task::spawn_blocking(move || acc.packs_granted_to(uid))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .collect();
    packs
        .into_iter()
        .filter(|id| match inherent_level(identity, id) {
            Some(level) => level >= need,
            None => granted.get(id).is_some_and(|level| *level >= need),
        })
        .collect()
}

/// Gate member content creation on rules-of-use acceptance: `Forbidden` until the
/// user has accepted. The panel accepts on their behalf via `/v1/me/accept-terms`.
pub(crate) async fn require_terms(state: &AppState, uid: i64) -> Result<(), ApiError> {
    let acc = state.accounts.clone();
    let ok = tokio::task::spawn_blocking(move || acc.terms_accepted(uid))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("terms task: {e}")))??;
    if !ok {
        return Err(ApiError::Forbidden);
    }
    Ok(())
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
    // debug token is checked first: it is the higher rung, so a value set as both
    // resolves to debug. Either bearer maps to the synthetic break-glass identity.
    if let Some(tok) = &bearer {
        if let Some(expected) = state.config.debug_token.as_deref()
            && constant_time_eq(expected.as_bytes(), tok.as_bytes())
        {
            return Some(break_glass(Role::Debug));
        }
        if let Some(expected) = state.config.admin_token.as_deref()
            && constant_time_eq(expected.as_bytes(), tok.as_bytes())
        {
            return Some(break_glass(Role::Admin));
        }
    }

    let sid = cookie_value(headers, COOKIE_NAME)?;
    let acc = state.accounts.clone();
    tokio::task::spawn_blocking(move || acc.session_identity(&sid))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
}

/// Who is calling, if anyone -- for a public handler that stays open to guests
/// but tightens for a signed-in owner (e.g. a private draft pack, #17). `None`
/// is a guest, not an error.
pub(crate) async fn optional_identity(state: &AppState, headers: &HeaderMap) -> Option<Identity> {
    resolve_identity(state, headers).await
}

// -- helpers ----------------------------------------------------------------

fn break_glass(role: Role) -> Identity {
    Identity {
        uid: 0,
        login: "break-glass".into(),
        role,
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
    fn ownership_and_the_admin_rung_need_no_lookup() {
        let member = Identity {
            uid: 7,
            login: "m".into(),
            role: Role::Member,
        };
        let other = Identity {
            uid: 9,
            login: "o".into(),
            role: Role::Member,
        };
        let admin = Identity {
            uid: 1,
            login: "a".into(),
            role: Role::Admin,
        };

        // a member owns their own community namespace and nothing else; an
        // answer of `None` is not a refusal, it is "ask the access list"
        assert_eq!(inherent_level(&member, "u/7/MyPack"), Some(PackLevel::Own));
        assert_eq!(inherent_level(&member, "u/9/TheirPack"), None);
        assert_eq!(inherent_level(&other, "u/7/MyPack"), None);
        // an official pack is nobody's by namespace
        assert_eq!(inherent_level(&member, "Industrial"), None);
        // admin owns everything, official or not
        assert_eq!(inherent_level(&admin, "u/7/MyPack"), Some(PackLevel::Own));
        assert_eq!(inherent_level(&admin, "Industrial"), Some(PackLevel::Own));

        // namespace parsing: only a numeric uid segment is a community namespace
        assert_eq!(pack_namespace_uid("u/7/MyPack"), Some(7));
        assert_eq!(pack_namespace_uid("Industrial"), None);
        assert_eq!(pack_namespace_uid("u/abc/x"), None);
    }

    #[test]
    fn levels_rank_low_to_high() {
        // the whole gate is one comparison, so the order is the contract
        assert!(PackLevel::Own > PackLevel::Edit);
        assert!(PackLevel::Edit > PackLevel::View);
        assert_eq!(PackLevel::parse("edit"), Some(PackLevel::Edit));
        assert_eq!(PackLevel::parse("root"), None);
    }

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
