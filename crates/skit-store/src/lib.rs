//! Filesystem and TOML adapters for skit.
//!
//! The authoritative read path is `scripts/<slug>/meta.toml`. `registry.toml` remains an
//! optimization to port only after freshness and self-heal differential tests exist.

#![forbid(unsafe_code)]

mod mutations;
mod read;

pub use mutations::content_hash;
pub use read::FileStore;
