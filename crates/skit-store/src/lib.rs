//! Provide file-system and TOML adapters for skit.
//!
//! Entry data and user state use separate roots.

#![forbid(unsafe_code)]

mod fs_ops;
mod mutations;
mod path_glob;
mod paths;
mod read;
mod state;

pub use mutations::content_hash;
pub use path_glob::FileGlobExpander;
pub use paths::stored_filename;
pub use read::FileStore;
pub use state::FileFormStateStore;
