//! Authoring (service layer): turn an admin-authored `PackConfig` into the
//! wire manifest, bootstrap a starter config from an SC archive, run the
//! build enrichment passes, and resolve Modrinth sources. The compute core
//! shared by the `smrt-pack` CLI and the panel's build endpoints. `archive`
//! and `sources` are internal helpers; the passes are the public surface.

mod archive;
mod sources;

pub mod bootstrap;
pub mod build;
pub mod bytecode;
pub mod classfile;
pub mod curator;
pub mod harvest;
pub mod harvest_sched;
pub mod jardiff;
pub mod modmeta;
pub mod modrinth;
pub mod reconstruct;
pub mod resolve;
pub mod validate;

pub use bootstrap::{BootstrapArgs, bootstrap};
pub use build::{build_manifest, make_pack_summary};
pub use curator::{
    McModInfo, RoleTable, apply_role_table, enrich_from_mcmod_info, infer_requires_from_mcmod_info,
    jar_icon, load_role_table, read_mcmod_info,
};
pub use harvest_sched::HarvestScheduler;
pub use jardiff::{JarDiff, diff_jars};
pub use modrinth::*;
pub use reconstruct::reconstruct_config;
pub use resolve::{ResolveReport, pack_graph, resolve_pack};
pub use validate::{ValidateReport, validate};
