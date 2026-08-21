//! Provide file-system and TOML adapters for skit.
//!
//! Entry data and user state use separate roots.

#![forbid(unsafe_code)]

mod config;
mod fs_ops;
mod library_snapshot;
mod mutations;
mod path_completion;
mod path_glob;
mod paths;
mod prompt_selection;
mod read;
mod state;
mod toml_document;

pub use config::{
    CONFIG_KEYS, ConfigError, ConfigRecovery, FileConfigStore, MirrorSettings, PromptRunner,
    PromptRunnerRow,
};
#[doc(hidden)]
pub use fs_ops::replace_with_retry_impl;
pub use mutations::{
    FileAgentSkillStore, FileRunnerManagementStore, PreparedLaunch, RegistryRebuildProblem,
    RegistryRebuildReport, RunnerManagementStoreError, RunnerRemovalCas, content_hash,
};
pub use path_completion::SystemDirectoryReader;
pub use path_glob::FileGlobExpander;
pub use paths::{expand_user_path, stored_filename, stored_filenames};
pub use prompt_selection::FilePromptSelectionStore;
pub use read::FileStore;
pub use state::FileFormStateStore;
