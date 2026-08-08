//! Per-entry form-state filesystem adapter.

use std::{collections::BTreeMap, fs, path::PathBuf};

use skit_application::form_state::{FormStateRepository, PersistedFormState, StateWriteError};
use skit_domain::Slug;
use toml::{Table, Value};

use crate::fs_ops::{acquire_lock, atomic_write_bytes};

/// Filesystem-backed form state rooted at the configured skit state directory.
#[derive(Clone, Debug)]
pub struct FileFormStateStore {
    state_dir: PathBuf,
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
}

impl FormStateRepository for FileFormStateStore {
    fn load(&self, slug: &Slug) -> PersistedFormState {
        let mut document = load_document(&self.values_path(slug));
        sanitize_document(&mut document);
        state_from_document(&document)
    }

    fn update<T, F>(&self, slug: &Slug, update: F) -> Result<T, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> T,
    {
        let lock_path = self.lock_path(slug);
        let _lock = acquire_lock(&lock_path)
            .map_err(|error| io_error("lock", &lock_path, error.to_string()))?;
        let path = self.values_path(slug);
        let mut document = load_document(&path);
        sanitize_document(&mut document);
        let mut state = state_from_document(&document);
        let result = update(&mut state);
        merge_state(&mut document, &state);
        let encoded = toml::to_string_pretty(&document).map_err(|error| StateWriteError::Encode {
            reason: error.to_string(),
        })?;
        atomic_write_bytes(&path, encoded.as_bytes())
            .map_err(|error| io_error("write", &path, error.to_string()))?;
        Ok(result)
    }
}

fn load_document(path: &std::path::Path) -> Table {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<Table>(&text).ok())
        .unwrap_or_default()
}

fn sanitize_document(document: &mut Table) {
    remove_unless_table(document, "values");
    remove_unless_table(document, "presets");
    if let Some(Value::Table(presets)) = document.get_mut("presets") {
        presets.retain(|_, value| value.is_table());
    }

    remove_unless_table(document, "last_run");
    if let Some(Value::Table(last_run)) = document.get_mut("last_run")
        && last_run.get("values").is_some_and(|value| !value.is_table())
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
        last_run_values: document
            .get("last_run")
            .and_then(Value::as_table)
            .and_then(|last_run| last_run.get("values"))
            .and_then(Value::as_table)
            .map(string_map)
            .unwrap_or_default(),
    }
}

fn string_map(table: &Table) -> BTreeMap<String, String> {
    table
        .iter()
        .filter_map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), value.to_owned()))
        })
        .collect()
}

fn merge_state(document: &mut Table, state: &PersistedFormState) {
    set_string_map(document, "values", &state.values);

    if state.presets.is_empty() {
        document.remove("presets");
    } else {
        document.insert(
            "presets".to_owned(),
            Value::Table(
                state
                    .presets
                    .iter()
                    .map(|(name, values)| (name.clone(), Value::Table(table_from_map(values))))
                    .collect(),
            ),
        );
    }

    if state.last_run_values.is_empty() {
        if let Some(Value::Table(last_run)) = document.get_mut("last_run") {
            last_run.remove("values");
            if last_run.is_empty() {
                document.remove("last_run");
            }
        }
    } else {
        let last_run = document
            .entry("last_run".to_owned())
            .or_insert_with(|| Value::Table(Table::new()));
        let table = last_run
            .as_table_mut()
            .expect("last_run was shape-checked before merge");
        table.insert(
            "values".to_owned(),
            Value::Table(table_from_map(&state.last_run_values)),
        );
    }
}

fn set_string_map(document: &mut Table, key: &str, values: &BTreeMap<String, String>) {
    if values.is_empty() {
        document.remove(key);
    } else {
        document.insert(key.to_owned(), Value::Table(table_from_map(values)));
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
