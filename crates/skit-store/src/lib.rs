//! Provide file-system and TOML adapters for skit.
//!
//! Entry data and user state use separate roots.

#![forbid(unsafe_code)]

mod config;
mod fs_ops;
mod library_surface;
mod mutations;
mod path_glob;
mod paths;
mod prompt_selection;
mod read;
mod state;
mod toml_document;

pub use config::{
    ConfigError, ConfigRecovery, FileConfigStore, MirrorSettings, PromptRunner, PromptRunnerRow,
};
pub use library_surface::{library_surface, library_surface_at};
pub use mutations::{
    FileAgentSkillStore, FileRunnerManagementStore, PreparedLaunch, RegistryRebuildProblem,
    RegistryRebuildReport, RunnerManagementStoreError, RunnerRemovalCas, content_hash,
};
pub use path_glob::FileGlobExpander;
pub use paths::{expand_user_path, stored_filename, stored_filenames};
pub use prompt_selection::FilePromptSelectionStore;
pub use read::FileStore;
pub use state::FileFormStateStore;
