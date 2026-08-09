use std::path::PathBuf;

use skit_application::RepositoryError;
use skit_domain::EntrySettings;

use crate::{ConfigError, FileConfigStore, FileStore, PromptRunnerRow};

/// Result of one identity- and pin-checked named runner removal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerRemovalCas {
    /// The stable runner key was removed.
    Removed,
    /// One or more raw config rows changed after inspection.
    RowsChanged,
    /// The number of prompt entries pinned to the runner changed after confirmation.
    PinsChanged {
        /// Current count collected while the library mutation lock is held.
        actual: usize,
    },
}

/// Adapter failure before a runner removal can reach a committed result.
#[derive(Debug)]
pub enum RunnerManagementStoreError {
    /// The authoritative entry library or its lock could not be read.
    Library(RepositoryError),
    /// The config document or its lock could not be read or written.
    Config(ConfigError),
}

impl From<RepositoryError> for RunnerManagementStoreError {
    fn from(value: RepositoryError) -> Self {
        Self::Library(value)
    }
}

impl From<ConfigError> for RunnerManagementStoreError {
    fn from(value: ConfigError) -> Self {
        Self::Config(value)
    }
}

/// Filesystem coordinator for runner config and prompt-entry pin transactions.
///
/// The adapter always acquires the library namespace lock before the config lock.
/// Entry metadata mutations use the same namespace lock, so a prompt pin cannot
/// change between the count comparison and the config compare-and-swap.
#[derive(Clone, Debug)]
pub struct FileRunnerManagementStore {
    library: FileStore,
    config: FileConfigStore,
}

impl FileRunnerManagementStore {
    /// Use the supplied data and configuration roots.
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        Self {
            library: FileStore::new(data_dir),
            config: FileConfigStore::new(config_dir),
        }
    }

    /// Remove one named key only if its raw rows and prompt pin count still match.
    pub fn remove_named_if_unchanged(
        &self,
        name: &str,
        expected_rows: &[PromptRunnerRow],
        expected_pinned_count: usize,
    ) -> Result<RunnerRemovalCas, RunnerManagementStoreError> {
        self.remove_named_with_hook(name, expected_rows, expected_pinned_count, || {})
    }

    fn remove_named_with_hook(
        &self,
        name: &str,
        expected_rows: &[PromptRunnerRow],
        expected_pinned_count: usize,
        after_library_lock: impl FnOnce(),
    ) -> Result<RunnerRemovalCas, RunnerManagementStoreError> {
        let _library_lock = self.library.namespace_lock()?;
        after_library_lock();
        let actual = self.prompt_pin_count(name)?;
        if actual != expected_pinned_count {
            return Ok(RunnerRemovalCas::PinsChanged { actual });
        }
        if self
            .config
            .remove_runner_if_unchanged(name, expected_rows)?
        {
            Ok(RunnerRemovalCas::Removed)
        } else {
            Ok(RunnerRemovalCas::RowsChanged)
        }
    }

    fn prompt_pin_count(&self, name: &str) -> Result<usize, RepositoryError> {
        Ok(self
            .library
            .scan_entries()?
            .into_iter()
            .filter(|entry| entry.meta.kind.as_str() == "prompt")
            .filter(|entry| EntrySettings::from_meta(&entry.meta).runner == name)
            .count())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::mpsc,
        thread,
        time::Duration,
    };

    use skit_application::{
        CreateEntry, EntryMutationRepository, EntryPayload, SourcePermissions,
    };
    use skit_domain::{EntryKind, StorageMode};
    use tempfile::TempDir;

    use super::*;
    use crate::PromptRunner;

    fn prompt_request(runner: &str) -> CreateEntry {
        let mut settings = EntrySettings::default();
        settings.runner = runner.to_owned();
        CreateEntry {
            name: "Pinned prompt".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/prompt.md".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"hello".to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings,
        }
    }

    #[test]
    fn a_prompt_pin_mutation_cannot_cross_the_removal_transaction() {
        let data_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();
        let library = FileStore::new(data_dir.path());
        let entry = library.create(prompt_request("victim")).unwrap();
        let config = FileConfigStore::new(config_dir.path());
        config
            .set_runner(
                PromptRunner {
                    name: "victim".to_owned(),
                    argv: vec!["victim".to_owned(), "{{prompt}}".to_owned()],
                },
                false,
            )
            .unwrap();
        let expected = config
            .runner_rows()
            .unwrap()
            .into_iter()
            .filter(|row| row.name.as_deref() == Some("victim"))
            .collect::<Vec<_>>();
        let management = FileRunnerManagementStore::new(data_dir.path(), config_dir.path());
        let (locked_sender, locked_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let removal = thread::spawn(move || {
            management.remove_named_with_hook("victim", &expected, 1, || {
                locked_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
            })
        });

        locked_receiver.recv().unwrap();
        let (started_sender, started_receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        let mutation = thread::spawn(move || {
            started_sender.send(()).unwrap();
            let mut settings = EntrySettings::from_meta(&entry.meta);
            settings.runner = "other".to_owned();
            let result = library.update_settings(&entry, &settings, "invoke");
            done_sender.send(()).unwrap();
            result
        });
        started_receiver.recv().unwrap();
        assert!(
            done_receiver.recv_timeout(Duration::from_millis(50)).is_err(),
            "the metadata mutation must wait for the namespace transaction"
        );

        release_sender.send(()).unwrap();
        assert_eq!(removal.join().unwrap().unwrap(), RunnerRemovalCas::Removed);
        mutation.join().unwrap().unwrap();
        assert!(
            config
                .runners()
                .unwrap()
                .iter()
                .all(|runner| runner.name != "victim")
        );
    }
}
