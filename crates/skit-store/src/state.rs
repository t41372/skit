//! Per-entry form-state filesystem adapter.

use std::{collections::BTreeMap, fs, path::PathBuf};

use skit_application::form_state::{
    FormStateRepository, LastRunState, PersistedFormState, StateWriteError,
};
use skit_domain::Slug;
use toml::{Table, Value};

use crate::fs_ops::{acquire_lock, atomic_write_bytes};

/// Filesystem-backed form state rooted at the configured skit state directory.
#[derive(Clone, Debug)]
pub struct FileFormStateStore {
    state_dir: PathBuf,
}

/// Result of rolling back authoritative entry state and derived external artifacts.
#[derive(Debug)]
pub struct ExternalRollbackOutcome<E> {
    authoritative_restored: bool,
    error: Option<E>,
}

impl<E> ExternalRollbackOutcome<E> {
    /// Report that authoritative state and all derived artifacts were restored.
    #[must_use]
    pub const fn complete() -> Self {
        Self {
            authoritative_restored: true,
            error: None,
        }
    }

    /// Report that authoritative state was restored but a derived rollback failed.
    #[must_use]
    pub const fn restored_with_error(error: E) -> Self {
        Self {
            authoritative_restored: true,
            error: Some(error),
        }
    }

    /// Report that authoritative state could not be restored.
    #[must_use]
    pub const fn authoritative_failure(error: E) -> Self {
        Self {
            authoritative_restored: false,
            error: Some(error),
        }
    }

    /// Whether the source and metadata returned to their authoritative prior state.
    #[must_use]
    pub const fn authoritative_restored(&self) -> bool {
        self.authoritative_restored
    }

    /// Consume the optional rollback detail.
    #[must_use]
    pub fn into_error(self) -> Option<E> {
        self.error
    }
}

impl<E> From<Result<(), E>> for ExternalRollbackOutcome<E> {
    fn from(result: Result<(), E>) -> Self {
        match result {
            Ok(()) => Self::complete(),
            Err(error) => Self::authoritative_failure(error),
        }
    }
}

/// Failure from a state update coordinated with one external commit.
#[derive(Debug)]
pub enum CoordinatedStateError<E> {
    /// The state lock or initial read failed before the external commit ran.
    State(StateWriteError),
    /// The external commit failed before state changed.
    Commit(E),
    /// State could not commit after the external commit. The optional error is a failed rollback.
    StateAfterCommit {
        /// State persistence failure.
        state: StateWriteError,
        /// External rollback failure. `None` means rollback succeeded.
        rollback: Option<E>,
    },
    /// Final cleanup failed after state committed.
    FinalizeAfterState {
        /// Failure from finalizing the external transaction.
        finalize: E,
        /// External rollback failure, including a derived-artifact restoration failure.
        rollback: Option<E>,
        /// Failure from restoring the byte-exact original state after external rollback succeeded.
        state_rollback: Option<StateWriteError>,
        /// Whether authoritative source and metadata rollback succeeded.
        authoritative_restored: bool,
    },
}

impl FileFormStateStore {
    /// Use the supplied skit state root (the parent of `values/` and `.locks/`).
    #[must_use]
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    fn values_path(&self, slug: &Slug) -> PathBuf {
        self.state_dir
            .join("values")
            .join(format!("{}.toml", slug.as_str()))
    }

    fn lock_path(&self, slug: &Slug) -> PathBuf {
        self.state_dir
            .join(".locks")
            .join(format!("{}.values.lock", slug.as_str()))
    }

    /// Hold the state lock across an external commit and its dependent state mutation.
    ///
    /// If state persistence fails, `rollback` runs before the state lock is released. A concurrent
    /// run therefore cannot restore a stale public value between a secrecy commit and its purge.
    pub fn update_after_external_commit<C, T, E, R>(
        &self,
        slug: &Slug,
        commit: impl FnOnce() -> Result<C, E>,
        update: impl FnOnce(&mut PersistedFormState) -> T,
        rollback: impl FnOnce(&mut C) -> R,
    ) -> Result<(C, T), CoordinatedStateError<E>>
    where
        R: Into<ExternalRollbackOutcome<E>>,
    {
        self.update_after_external_commit_and_finalize(slug, commit, update, |_| Ok(()), rollback)
    }

    /// Coordinate an external commit, state write, and fallible final cleanup.
    pub fn update_after_external_commit_and_finalize<C, T, E, R>(
        &self,
        slug: &Slug,
        commit: impl FnOnce() -> Result<C, E>,
        update: impl FnOnce(&mut PersistedFormState) -> T,
        finalize: impl FnOnce(&mut C) -> Result<(), E>,
        rollback: impl FnOnce(&mut C) -> R,
    ) -> Result<(C, T), CoordinatedStateError<E>>
    where
        R: Into<ExternalRollbackOutcome<E>>,
    {
        self.update_after_external_commit_and_finalize_with(
            slug,
            commit,
            update,
            finalize,
            rollback,
            |path, bytes| {
                atomic_write_bytes(path, bytes)
                    .map_err(|error| io_error("write", path, error.to_string()))
            },
            restore_state_snapshot,
        )
    }

    /// Injectable state-writer variant of [`Self::update_after_external_commit`].
    #[doc(hidden)]
    pub fn update_after_external_commit_with<C, T, E, R>(
        &self,
        slug: &Slug,
        commit: impl FnOnce() -> Result<C, E>,
        update: impl FnOnce(&mut PersistedFormState) -> T,
        rollback: impl FnOnce(&mut C) -> R,
        write_state: impl FnOnce(&std::path::Path, &[u8]) -> Result<(), StateWriteError>,
    ) -> Result<(C, T), CoordinatedStateError<E>>
    where
        R: Into<ExternalRollbackOutcome<E>>,
    {
        self.update_after_external_commit_and_finalize_with(
            slug,
            commit,
            update,
            |_| Ok(()),
            rollback,
            write_state,
            restore_state_snapshot,
        )
    }

    /// Injectable writer and restore variant of the complete coordinated transaction.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn update_after_external_commit_and_finalize_with<C, T, E, R>(
        &self,
        slug: &Slug,
        commit: impl FnOnce() -> Result<C, E>,
        update: impl FnOnce(&mut PersistedFormState) -> T,
        finalize: impl FnOnce(&mut C) -> Result<(), E>,
        rollback: impl FnOnce(&mut C) -> R,
        write_state: impl FnOnce(&std::path::Path, &[u8]) -> Result<(), StateWriteError>,
        restore_state: impl FnOnce(&std::path::Path, Option<&[u8]>) -> Result<(), StateWriteError>,
    ) -> Result<(C, T), CoordinatedStateError<E>>
    where
        R: Into<ExternalRollbackOutcome<E>>,
    {
        let lock_path = self.lock_path(slug);
        let _lock = acquire_lock(&lock_path).map_err(|error| {
            CoordinatedStateError::State(io_error("lock", &lock_path, error.to_string()))
        })?;
        let path = self.values_path(slug);
        let (mut document, original_bytes) =
            load_document_for_write_snapshot(&path).map_err(CoordinatedStateError::State)?;
        sanitize_document(&mut document);
        let mut state = state_from_document(&document);
        let before = state.clone();
        let result = update(&mut state);
        if state == before {
            let mut committed = commit().map_err(CoordinatedStateError::Commit)?;
            return match finalize(&mut committed) {
                Ok(()) => Ok((committed, result)),
                Err(finalize) => {
                    let rollback = rollback(&mut committed).into();
                    Err(CoordinatedStateError::FinalizeAfterState {
                        finalize,
                        authoritative_restored: rollback.authoritative_restored(),
                        rollback: rollback.into_error(),
                        state_rollback: None,
                    })
                }
            };
        }
        merge_state(&mut document, &before, &state);
        let encoded =
            toml::to_string_pretty(&document).expect("a parsed TOML value tree must serialize");
        let mut committed = commit().map_err(CoordinatedStateError::Commit)?;
        if let Err(error) = write_state(&path, encoded.as_bytes()) {
            return Err(CoordinatedStateError::StateAfterCommit {
                state: error,
                rollback: rollback(&mut committed).into().into_error(),
            });
        }
        match finalize(&mut committed) {
            Ok(()) => Ok((committed, result)),
            Err(finalize) => {
                let rollback = rollback(&mut committed).into();
                let authoritative_restored = rollback.authoritative_restored();
                let state_rollback = if authoritative_restored {
                    restore_state(&path, original_bytes.as_deref()).err()
                } else {
                    None
                };
                Err(CoordinatedStateError::FinalizeAfterState {
                    finalize,
                    rollback: rollback.into_error(),
                    state_rollback,
                    authoritative_restored,
                })
            }
        }
    }
}

impl FormStateRepository for FileFormStateStore {
    fn load(&self, slug: &Slug) -> PersistedFormState {
        let mut document = load_document(&self.values_path(slug));
        sanitize_document(&mut document);
        state_from_document(&document)
    }

    fn last_run(&self, slug: &Slug) -> LastRunState {
        last_run_from_document(&load_document(&self.values_path(slug)), false)
    }

    fn update<T, F>(&self, slug: &Slug, update: F) -> Result<T, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> T,
    {
        let lock_path = self.lock_path(slug);
        let _lock = acquire_lock(&lock_path)
            .map_err(|error| io_error("lock", &lock_path, error.to_string()))?;
        let path = self.values_path(slug);
        let mut document = load_document_for_write(&path)?;
        sanitize_document(&mut document);
        let mut state = state_from_document(&document);
        let before = state.clone();
        let result = update(&mut state);
        merge_state(&mut document, &before, &state);
        let encoded =
            toml::to_string_pretty(&document).expect("a parsed TOML value tree must serialize");
        atomic_write_bytes(&path, encoded.as_bytes())
            .map_err(|error| io_error("write", &path, error.to_string()))?;
        Ok(result)
    }

    fn try_update<T, E, F>(&self, slug: &Slug, update: F) -> Result<Result<T, E>, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> Result<T, E>,
    {
        let lock_path = self.lock_path(slug);
        let _lock = acquire_lock(&lock_path)
            .map_err(|error| io_error("lock", &lock_path, error.to_string()))?;
        let path = self.values_path(slug);
        let mut document = load_document_for_write(&path)?;
        sanitize_document(&mut document);
        let mut state = state_from_document(&document);
        let before = state.clone();
        let result = match update(&mut state) {
            Ok(result) => result,
            Err(error) => return Ok(Err(error)),
        };
        merge_state(&mut document, &before, &state);
        let encoded =
            toml::to_string_pretty(&document).expect("a parsed TOML value tree must serialize");
        atomic_write_bytes(&path, encoded.as_bytes())
            .map_err(|error| io_error("write", &path, error.to_string()))?;
        Ok(Ok(result))
    }

    fn forget(&self, slug: &Slug) -> Result<(), StateWriteError> {
        let lock_path = self.lock_path(slug);
        let _lock = acquire_lock(&lock_path)
            .map_err(|error| io_error("lock", &lock_path, error.to_string()))?;
        let path = self.values_path(slug);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("remove", &path, error.to_string())),
        }
    }
}

fn load_document(path: &std::path::Path) -> Table {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<Table>(&text).ok())
        .unwrap_or_default()
}

fn load_document_for_write(path: &std::path::Path) -> Result<Table, StateWriteError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str::<Table>(&text).unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Table::new()),
        Err(error) => Err(io_error("read", path, error.to_string())),
    }
}

fn load_document_for_write_snapshot(
    path: &std::path::Path,
) -> Result<(Table, Option<Vec<u8>>), StateWriteError> {
    match fs::read(path) {
        Ok(bytes) => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|error| io_error("read", path, error.to_string()))?;
            Ok((
                toml::from_str::<Table>(text).unwrap_or_default(),
                Some(bytes),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((Table::new(), None)),
        Err(error) => Err(io_error("read", path, error.to_string())),
    }
}

fn restore_state_snapshot(
    path: &std::path::Path,
    original: Option<&[u8]>,
) -> Result<(), StateWriteError> {
    if let Some(original) = original {
        return atomic_write_bytes(path, original)
            .map_err(|error| io_error("rollback write", path, error.to_string()));
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("rollback remove", path, error.to_string())),
    }
}

fn sanitize_document(document: &mut Table) {
    remove_unless_table(document, "values");
    remove_unless_table(document, "presets");
    if let Some(Value::Table(presets)) = document.get_mut("presets") {
        presets.retain(|_, value| value.is_table());
    }

    if document
        .get("extra_args")
        .is_some_and(|value| !value.is_array())
    {
        document.remove("extra_args");
    }
    if document
        .get("extra_args_raw")
        .is_some_and(|value| !value.is_bool())
    {
        document.remove("extra_args_raw");
    }

    remove_unless_table(document, "last_run");
    if let Some(Value::Table(last_run)) = document.get_mut("last_run")
        && last_run
            .get("values")
            .is_some_and(|value| !value.is_table())
    {
        last_run.remove("values");
    }
}

fn remove_unless_table(document: &mut Table, key: &str) {
    if document.get(key).is_some_and(|value| !value.is_table()) {
        document.remove(key);
    }
}

fn state_from_document(document: &Table) -> PersistedFormState {
    PersistedFormState {
        values: document
            .get("values")
            .and_then(Value::as_table)
            .map(string_map)
            .unwrap_or_default(),
        extra_args: document
            .get("extra_args")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        extra_args_raw: document
            .get("extra_args_raw")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        presets: document
            .get("presets")
            .and_then(Value::as_table)
            .map(|presets| {
                presets
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .as_table()
                            .map(|values| (name.clone(), string_map(values)))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        last_run: last_run_from_document(document, true),
    }
}

fn last_run_from_document(document: &Table, include_values: bool) -> LastRunState {
    let last_run = document.get("last_run").and_then(Value::as_table);
    LastRunState {
        at: last_run
            .and_then(|last_run| last_run.get("at"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        exit: last_run
            .and_then(|last_run| last_run.get("exit"))
            .and_then(Value::as_integer),
        values: include_values
            .then(|| {
                last_run
                    .and_then(|last_run| last_run.get("values"))
                    .and_then(Value::as_table)
                    .map(string_map)
            })
            .flatten(),
    }
}

fn string_map(table: &Table) -> BTreeMap<String, String> {
    table
        .iter()
        .filter_map(|(name, value)| value.as_str().map(|value| (name.clone(), value.to_owned())))
        .collect()
}

fn merge_state(document: &mut Table, before: &PersistedFormState, state: &PersistedFormState) {
    patch_string_map(document, "values", &before.values, &state.values);
    set_extra_args(document, before, state);
    set_presets(document, &before.presets, &state.presets);
    set_last_run(document, &before.last_run, &state.last_run);
}

fn set_extra_args(document: &mut Table, before: &PersistedFormState, state: &PersistedFormState) {
    if before.extra_args != state.extra_args {
        if state.extra_args.is_empty() {
            document.remove("extra_args");
        } else {
            document.insert(
                "extra_args".to_owned(),
                Value::Array(
                    state
                        .extra_args
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
    }

    if before.extra_args_raw != state.extra_args_raw || state.extra_args.is_empty() {
        if state.extra_args_raw && !state.extra_args.is_empty() {
            document.insert("extra_args_raw".to_owned(), Value::Boolean(true));
        } else {
            document.remove("extra_args_raw");
        }
    }
}

fn set_presets(
    document: &mut Table,
    before: &BTreeMap<String, BTreeMap<String, String>>,
    presets: &BTreeMap<String, BTreeMap<String, String>>,
) {
    if before == presets {
        return;
    }

    let section = document
        .entry("presets".to_owned())
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .expect("presets was shape-checked before merge");

    for name in before.keys() {
        if !presets.contains_key(name) {
            section.remove(name);
        }
    }
    for (name, values) in presets {
        match before.get(name) {
            None => {
                section.insert(name.clone(), Value::Table(table_from_map(values)));
            }
            Some(previous) if previous != values => {
                let table = section
                    .entry(name.clone())
                    .or_insert_with(|| Value::Table(Table::new()))
                    .as_table_mut()
                    .expect("preset rows were shape-checked before merge");
                patch_table(table, previous, values);
            }
            Some(_) => {}
        }
    }
    if section.is_empty() {
        document.remove("presets");
    }
}

fn set_last_run(document: &mut Table, before: &LastRunState, state: &LastRunState) {
    if before == state {
        return;
    }
    let last_run = document
        .entry("last_run".to_owned())
        .or_insert_with(|| Value::Table(Table::new()));
    let table = last_run
        .as_table_mut()
        .expect("last_run was shape-checked before merge");

    if before.at != state.at {
        set_optional_string(table, "at", state.at.as_deref());
    }
    if before.exit != state.exit {
        set_optional_integer(table, "exit", state.exit);
    }
    if before.values != state.values {
        match (&before.values, &state.values) {
            (Some(previous), Some(values)) => {
                let values_table = table
                    .entry("values".to_owned())
                    .or_insert_with(|| Value::Table(Table::new()))
                    .as_table_mut()
                    .expect("last_run.values was shape-checked before merge");
                patch_table(values_table, previous, values);
            }
            (_, Some(values)) => {
                table.insert("values".to_owned(), Value::Table(table_from_map(values)));
            }
            (_, None) => {
                table.remove("values");
            }
        }
    }
    if table.is_empty() {
        document.remove("last_run");
    }
}

fn set_optional_string(table: &mut Table, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        table.insert(key.to_owned(), Value::String(value.to_owned()));
    } else {
        table.remove(key);
    }
}

fn set_optional_integer(table: &mut Table, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        table.insert(key.to_owned(), Value::Integer(value));
    } else {
        table.remove(key);
    }
}

fn patch_string_map(
    document: &mut Table,
    key: &str,
    before: &BTreeMap<String, String>,
    values: &BTreeMap<String, String>,
) {
    if before == values {
        return;
    }
    let table = document
        .entry(key.to_owned())
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .expect("value sections were shape-checked before merge");
    patch_table(table, before, values);
    if table.is_empty() {
        document.remove(key);
    }
}

fn patch_table(
    table: &mut Table,
    before: &BTreeMap<String, String>,
    values: &BTreeMap<String, String>,
) {
    for name in before.keys() {
        if !values.contains_key(name) {
            table.remove(name);
        }
    }
    for (name, value) in values {
        if before.get(name) != Some(value) {
            table.insert(name.clone(), Value::String(value.clone()));
        }
    }
}

fn table_from_map(values: &BTreeMap<String, String>) -> Table {
    values
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect()
}

fn io_error(operation: &'static str, path: &std::path::Path, reason: String) -> StateWriteError {
    StateWriteError::Io {
        operation,
        path: path.display().to_string(),
        reason,
    }
}
