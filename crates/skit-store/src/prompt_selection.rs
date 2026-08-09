//! Filesystem adapter for interactive prompt-runner selection state.

use std::{fs, path::PathBuf};

use skit_application::{form_state::StateWriteError, prompt_selection::PromptSelectionStore};
use toml::{Table, Value};

use crate::fs_ops::{acquire_lock, atomic_write_bytes};

/// Filesystem-backed `prompt.toml` state rooted at skit's configured state directory.
#[derive(Clone, Debug)]
pub struct FilePromptSelectionStore {
    state_dir: PathBuf,
}

impl FilePromptSelectionStore {
    /// Use the supplied skit state root.
    #[must_use]
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    fn path(&self) -> PathBuf {
        self.state_dir.join("prompt.toml")
    }

    fn lock_path(&self) -> PathBuf {
        self.state_dir.join(".locks/prompt.lock")
    }
}

impl PromptSelectionStore for FilePromptSelectionStore {
    fn load_last_runner(&self) -> String {
        load_document(&self.path())
            .and_then(|document| {
                document
                    .get("last_runner")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default()
    }

    fn save_last_runner(&self, name: &str) -> Result<(), StateWriteError> {
        let lock_path = self.lock_path();
        let _lock = acquire_lock(&lock_path)
            .map_err(|error| io_error("lock", &lock_path, error.to_string()))?;
        let path = self.path();
        let mut document = load_document(&path).unwrap_or_default();
        document.insert("last_runner".to_owned(), Value::String(name.to_owned()));
        let encoded =
            toml::to_string_pretty(&document).map_err(|error| StateWriteError::Encode {
                reason: error.to_string(),
            })?;
        atomic_write_bytes(&path, encoded.as_bytes())
            .map_err(|error| io_error("write", &path, error.to_string()))
    }
}

fn load_document(path: &std::path::Path) -> Option<Table> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
}

fn io_error(operation: &'static str, path: &std::path::Path, reason: String) -> StateWriteError {
    StateWriteError::Io {
        operation,
        path: path.display().to_string(),
        reason,
    }
}
