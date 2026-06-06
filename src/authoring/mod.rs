//! Authoring (service layer): turn an admin-authored `PackConfig` into the
//! wire manifest, bootstrap a starter config from an SC archive, run the
//! curator chain, and resolve Modrinth sources. The compute core shared by
//! the `smrt-pack` CLI and the panel's build endpoints. `archive` and
//! `sources` are internal helpers; the passes are the public surface.

mod archive;
mod sources;

pub mod bootstrap;
pub mod build;
pub mod curator;
pub mod harvest;
pub mod modrinth;
pub mod reconstruct;
pub mod validate;

pub use bootstrap::{BootstrapArgs, bootstrap};
pub use build::{build_manifest, make_pack_summary};
pub use curator::*;
pub use modrinth::*;
pub use reconstruct::reconstruct_config;
pub use validate::{ValidateReport, validate};
