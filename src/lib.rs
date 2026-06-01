//! smrt mirror: serves pack manifests, server metadata, and self-hosted mod
//! jars to Nexira, and (admin-side) authors packs in place.
//!
//! Layers:
//!   - `domain`    -- data model (wire DTOs, pack config, version rules), no I/O
//!   - `storage`   -- persistence over the on-disk storage tree
//!   - `authoring` -- the build / bootstrap / curator pipeline (service)
//!   - `http`      -- the public read API + admin write API (controllers)
//!
//! Plus `config` (env) and `state` (shared `AppState`).

pub mod authoring;
pub mod config;
pub mod domain;
pub mod http;
pub mod state;
pub mod storage;
