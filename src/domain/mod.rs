//! Data model layer: wire DTOs, the admin-authored pack config, and pure
//! version rules. No I/O. Submodules are flattened here so callers reach
//! every model type via `crate::domain::*`.

pub mod manifest;
pub mod pack;
pub mod server;
pub mod version;

pub use manifest::*;
pub use pack::*;
pub use server::*;
pub use version::*;
