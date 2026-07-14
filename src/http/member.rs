//! Member-accessible API: endpoints any signed-in user may call, gated by
//! `require_session` and scoped to what they own. Distinct from `admin`, which
//! requires the admin role -- this is the member tier of the ladder.

use super::ApiError;
use crate::accounts::{Identity, UploadRow};
use crate::authoring::curator::{clean_mc_version, jar_facts, read_mcmod_info};
use crate::authoring::modmeta;
use crate::domain::{PackConfig, PackSummary, Visibility};
use crate::registry::queries;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;

const UPLOAD_BODY_LIMIT: usize = 256 * 1024 * 1024;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/me/packs", get(my_packs))
        .route("/v1/me/authoring", get(my_authoring))
        .route("/v1/me/packs/:pack_id/uploads", post(upload_jar))
        .route("/v1/me/uploads", get(my_uploads))
        .route("/v1/me/forks", post(fork_pack))
        .route("/v1/me/accept-terms", post(accept_terms))
        .layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT))
        .layer(from_fn_with_state(
            state.clone(),
            super::auth::require_session,
        ))
        .with_state(state)
}

/// The caller's own packs -- the "my packs" view. Draft and community packs the
/// public `/v1/packs` listing hides show here for their owner; an admin sees all.
async fn my_packs(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<PackSummary>>, ApiError> {
    let mine = state
        .storage
        .list_pack_summaries()
        .await?
        .into_iter()
        .filter(|p| identity.owns_or_admin(p.owner))
        .collect();
    Ok(Json(mine))
}

/// The caller's own authoring pack ids, including unbuilt drafts that have no
/// summary yet. The "my packs" list unions this with the built summaries so a
/// freshly-created draft is reachable before its first build.
async fn my_authoring(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<String>>, ApiError> {
    let mine = state
        .storage
        .list_authoring_packs()
        .await?
        .into_iter()
        .filter(|id| super::auth::may_author(&identity, id))
        .collect();
    Ok(Json(mine))
}

#[derive(Deserialize)]
struct UploadParams {
    filename: String,
}

/// Upload a self-hosted jar for one of the caller's community packs. Two auto-
/// gates enforce "self-host archival only": a jar whose sha1 Modrinth already
/// serves is the genuine file (rejected), and a Modrinth-known mod whose
/// (mc, loader) target Modrinth already carries is a relabel (rejected). Anything
/// else stages under `uploads/` and enters the moderation queue as `pending`; an
/// operator promotes it to the shared cache on approval. See the upload-moderation
/// policy.
async fn upload_jar(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
    Query(p): Query<UploadParams>,
    body: Bytes,
) -> Result<(StatusCode, Json<UploadRow>), ApiError> {
    if !super::auth::may_author(&identity, &pack_id) {
        return Err(ApiError::Forbidden);
    }
    let sha1 = crate::storage::sha1_hex(&body);

    // Auto-gate: a jar Modrinth already serves is the genuine file, not archival.
    let known = state
        .modrinth
        .version_files_by_sha1(std::slice::from_ref(&sha1))
        .await
        .map_err(ApiError::Internal)?;
    if known.contains_key(&sha1) {
        return Err(ApiError::BadRequest(
            "this jar is on Modrinth -- add it via the Modrinth picker, not a self-hosted upload"
                .into(),
        ));
    }

    // Coverage gate: a Modrinth-known mod whose (mc, loader) target Modrinth
    // already carries is a relabel, not archival. Conservative -- any uncertainty
    // falls through to the human moderation queue.
    if let Some(reason) = modrinth_covers_upload(&state, &body).await? {
        return Err(ApiError::BadRequest(reason));
    }

    state.storage.stage_upload(&sha1, &body).await?;

    let uid = identity.uid;
    let size = body.len() as i64;
    let (acc, pid, fname, sha) = (state.accounts.clone(), pack_id, p.filename, sha1);
    let id = tokio::task::spawn_blocking(move || acc.enqueue_upload(uid, &pid, &fname, &sha, size))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("enqueue task: {e}")))??;

    let acc = state.accounts.clone();
    let row = tokio::task::spawn_blocking(move || acc.get_upload(id))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("read upload task: {e}")))??
        .ok_or(ApiError::NotFound)?;
    Ok((StatusCode::CREATED, Json(row)))
}

/// The identity facts the coverage gate reads from a jar: its modid, loader, and
/// declared Minecraft version. `None` if any is undeterminable -- not enough to
/// judge coverage, so the caller lets it through to the human queue.
fn extract_upload_facts(body: &[u8]) -> Option<(String, String, String)> {
    let loader = jar_facts(body).loader?;
    let mcmod = read_mcmod_info(body).ok().flatten();
    let meta = modmeta::read_mod_meta(body);
    let modid = mcmod
        .as_ref()
        .map(|i| i.modid.clone())
        .filter(|s| !s.is_empty())
        .or(meta.modid)?;
    let mc = mcmod
        .as_ref()
        .and_then(|i| clean_mc_version(&i.mcversion))
        .or(meta.mc)?;
    Some((modid, loader, mc))
}

/// Whether Modrinth already carries this upload's mod for its (mc, loader) target
/// -- in which case it is a relabelled counterfeit, not archival. Registry-based:
/// the modid resolves to a Modrinth project only if we have already harvested that
/// mod with its Modrinth identity. `Ok(None)` (let a human decide) whenever the
/// mod is not Modrinth-known here, its facts are undeterminable, or Modrinth has
/// the mod but not this target. Returns the rejection message when covered.
async fn modrinth_covers_upload(state: &AppState, body: &[u8]) -> Result<Option<String>, ApiError> {
    let Some((modid, loader, mc)) = extract_upload_facts(body) else {
        return Ok(None);
    };
    // modid -> our mod -> its Modrinth project id (a blocking DB read)
    let registry = state.registry.clone();
    let key = modid.clone();
    let project = tokio::task::spawn_blocking(move || {
        registry.with_conn(|c| {
            let Some(mod_id) = queries::mod_id_for_alias(c, "modid", &key)? else {
                return Ok(None);
            };
            queries::modrinth_id_for_mod(c, mod_id)
        })
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("registry lookup task: {e}")))?
    .map_err(ApiError::Internal)?;
    let Some(project) = project else {
        return Ok(None); // not a Modrinth-known mod in our registry
    };
    let carried = state
        .modrinth
        .project_versions(&project, Some(&mc))
        .await
        .map_err(ApiError::Internal)?
        .iter()
        .any(|v| v.loaders.iter().any(|l| l.eq_ignore_ascii_case(&loader)));
    Ok(carried.then(|| {
        format!(
            "Modrinth already carries {modid} for Minecraft {mc} ({loader}) -- \
             add it via the Modrinth picker, not a self-hosted upload"
        )
    }))
}

#[derive(Deserialize)]
struct ForkReq {
    source: String,
    name: String,
}

/// Fork a pack into the caller's namespace: copy its config + static under
/// `u/<uid>/<name>` as a community draft with `fork_of` set to the source. The
/// caller may fork any published pack, or one they already own (their draft).
async fn fork_pack(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(req): Json<ForkReq>,
) -> Result<(StatusCode, Json<PackConfig>), ApiError> {
    super::auth::require_terms(&state, identity.uid).await?;
    let published = state
        .storage
        .load_pack_summary(&req.source)
        .await
        .map(|s| s.visibility == Visibility::Published)
        .unwrap_or(false);
    if !published && !super::auth::may_author(&identity, &req.source) {
        return Err(ApiError::Forbidden);
    }
    let target = format!("u/{}/{}", identity.uid, req.name);
    let cfg = state
        .storage
        .duplicate_pack(
            &req.source,
            &target,
            None,
            identity.uid,
            Some(req.source.clone()),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(cfg)))
}

/// Record that the caller has accepted the rules of use.
async fn accept_terms(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<StatusCode, ApiError> {
    let uid = identity.uid;
    let acc = state.accounts.clone();
    tokio::task::spawn_blocking(move || acc.accept_terms(uid))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("accept task: {e}")))??;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's own uploads and their moderation status.
async fn my_uploads(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<UploadRow>>, ApiError> {
    let uid = identity.uid;
    let acc = state.accounts.clone();
    let rows = tokio::task::spawn_blocking(move || acc.list_user_uploads(uid))
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("uploads task: {e}")))??;
    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::classfile::fixtures::jar;

    #[test]
    fn extract_upload_facts_reads_modid_loader_mc() {
        // 1.12.2 forge: mcmod.info carries modid + mcversion
        let forge = jar(&[(
            "mcmod.info",
            br#"[{"modid":"thaumcraft","version":"6","mcversion":"1.12.2"}]"#,
        )]);
        assert_eq!(
            extract_upload_facts(&forge),
            Some(("thaumcraft".into(), "forge".into(), "1.12.2".into()))
        );

        // fabric: id + depends.minecraft
        let fabric = jar(&[(
            "fabric.mod.json",
            br#"{"id":"sodium","depends":{"minecraft":">=1.20.1"}}"#,
        )]);
        assert_eq!(
            extract_upload_facts(&fabric),
            Some(("sodium".into(), "fabric".into(), "1.20.1".into()))
        );

        // no readable metadata -> None (let a human decide)
        assert_eq!(extract_upload_facts(b"not a jar"), None);
    }
}
