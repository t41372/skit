//! Filesystem and TOML adapters for skit.
//!
//! Entry data and user state are separate roots. Library metadata remains authoritative under the
//! data root, while form values/presets use an independently locked state adapter.

#![forbid(unsafe_code)]

mod fs_ops;
mod mutations;
mod path_glob;
mod read;
mod state;

pub use mutations::content_hash;
pub use path_glob::FileGlobExpander;
pub use read::FileStore;
pub use state::FileFormStateStore;
