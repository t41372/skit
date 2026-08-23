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
mod stamp;
mod state;
mod toml_document;

pub use config::{
    CONFIG_KEYS, ConfigError, ConfigRecovery, FileConfigStore, MirrorSettings, PromptRunner,
    PromptRunnerRow,
};
pub use mutations::{
    FileAgentSkillStore, FileRunnerManagementStore, PreparedExternalCopyEdit, PreparedLaunch,
    RegistryRebuildProblem, RegistryRebuildReport, RunnerManagementStoreError, RunnerRemovalCas,
    content_hash,
};
pub use path_completion::SystemDirectoryReader;
pub use path_glob::FileGlobExpander;
pub use paths::{
    expand_user_path, override_directory, platform_config_dir, platform_data_dir,
    platform_state_dir, stored_filename, stored_filenames,
};
pub use prompt_selection::FilePromptSelectionStore;
pub use read::FileStore;
pub use stamp::{iso_stamp, now_iso};
pub use state::{CoordinatedStateError, ExternalRollbackOutcome, FileFormStateStore};
