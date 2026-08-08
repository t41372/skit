use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use serde::Serialize;

use crate::LibraryRoots;

/// One recorded run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastRun {
    pub at: String,
    pub exit: i32,
    /// Exact accepted values after secret stripping. New Rust writes always persist
    /// this table, including when it is empty, so an empty snapshot remains distinct
    /// from a legacy run stamp that never recorded values at all.
    pub values: BTreeMap<String, String>,
    /// Whether the loaded state actually contained a `last_run.values` table.
    #[serde(skip)]
    pub values_recorded: bool,
}

/// Remembered state for one library entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EntryState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub presets: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<LastRun>,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

/// State persistence failures.
#[derive(Debug)]
pub enum StateError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Encode {
        path: PathBuf,
        source: toml::ser::Error,
    },
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "cannot access {}: {source}", path.display())
            }
            Self::Encode { path, source } => {
                write!(formatter, "cannot encode {}: {source}", path.display())
            }
        }
    }
}

impl StdError for StateError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Encode { source, .. } => Some(source),
        }
    }
}

/// The state layer for remembered values, presets, and run history.
#[derive(Debug, Clone)]
pub struct StateStore {
    roots: LibraryRoots,
}

impl StateStore {
    /// Create a state store over explicit filesystem roots.
    #[must_use]
    pub fn new(roots: LibraryRoots) -> Self {
        Self { roots }
    }

    /// Return the state file for one slug.
    #[must_use]
    pub fn values_path(&self, slug: &str) -> PathBuf {
        self.roots
            .state_dir()
            .join("values")
            .join(format!("{slug}.toml"))
    }

    /// Load state. Missing, malformed, or structurally invalid sections degrade to
    /// empty values instead of breaking the whole library.
    #[must_use]
    pub fn load(&self, slug: &str) -> EntryState {
        load_document(&self.values_path(slug))
    }

    /// Save last-used values and extra arguments.
    ///
    /// `None` means keep the old section. An empty map or slice clears it. Secret
    /// names are always removed before the document reaches disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the state file cannot be locked or atomically replaced.
    pub fn save_last(
        &self,
        slug: &str,
        values: Option<&BTreeMap<String, String>>,
        extra_args: Option<&[String]>,
        secret_names: &BTreeSet<String>,
    ) -> Result<(), StateError> {
        let _lock = acquire_lock(&self.lock_path(slug))?;
        let path = self.values_path(slug);
        let mut document = load_document(&path);

        if let Some(values) = values {
            document.values = strip_secrets(values, secret_names);
        } else if !secret_names.is_empty() {
            document.values = strip_secrets(&document.values, secret_names);
        }
        if let Some(extra_args) = extra_args {
            document.extra_args = extra_args.to_vec();
        }
        save_document(&path, &document)
    }

    /// Save one named preset.
    ///
    /// # Errors
    ///
    /// Returns an error if the state file cannot be locked or atomically replaced.
    pub fn save_preset(
        &self,
        slug: &str,
        preset: &str,
        values: &BTreeMap<String, String>,
        secret_names: &BTreeSet<String>,
    ) -> Result<(), StateError> {
        let _lock = acquire_lock(&self.lock_path(slug))?;
        let path = self.values_path(slug);
        let mut document = load_document(&path);
        document
            .presets
            .insert(preset.to_owned(), strip_secrets(values, secret_names));
        save_document(&path, &document)
    }

    /// Delete one preset. Returns whether it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the state file cannot be locked or atomically replaced.
    pub fn delete_preset(&self, slug: &str, preset: &str) -> Result<bool, StateError> {
        let _lock = acquire_lock(&self.lock_path(slug))?;
        let path = self.values_path(slug);
        let mut document = load_document(&path);
        if document.presets.remove(preset).is_none() {
            return Ok(false);
        }
        save_document(&path, &document)?;
        Ok(true)
    }

    /// Remove plaintext values for parameters that became secret.
    ///
    /// # Errors
    ///
    /// Returns an error if the state file cannot be locked or atomically replaced.
    pub fn purge_secret(
        &self,
        slug: &str,
        names: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, StateError> {
        if names.is_empty() {
            return Ok(BTreeSet::new());
        }

        let _lock = acquire_lock(&self.lock_path(slug))?;
        let path = self.values_path(slug);
        let mut document = load_document(&path);
        let mut removed = BTreeSet::new();

        collect_removed(&document.values, names, &mut removed);
        document.values = strip_secrets(&document.values, names);

        let mut presets = BTreeMap::new();
        for (preset, values) in &document.presets {
            collect_removed(values, names, &mut removed);
            let cleaned = strip_secrets(values, names);
            if !cleaned.is_empty() {
                presets.insert(preset.clone(), cleaned);
            }
        }
        document.presets = presets;

        if let Some(last_run) = &mut document.last_run {
            collect_removed(&last_run.values, names, &mut removed);
            last_run.values = strip_secrets(&last_run.values, names);
        }

        save_document(&path, &document)?;
        Ok(removed)
    }

    /// Record one completed run with an exact accepted-value snapshot.
    ///
    /// `None` means the invocation contained no form-value map; it is still recorded as
    /// an exact empty snapshot. Legacy files that predate snapshots remain distinguishable
    /// on read through `LastRun::values_recorded`.
    ///
    /// # Errors
    ///
    /// Returns an error if the state file cannot be locked or atomically replaced.
    pub fn record_run(
        &self,
        slug: &str,
        exit: i32,
        at: &str,
        values: Option<&BTreeMap<String, String>>,
        secret_names: &BTreeSet<String>,
    ) -> Result<(), StateError> {
        let _lock = acquire_lock(&self.lock_path(slug))?;
        let path = self.values_path(slug);
        let mut document = load_document(&path);
        document.last_run = Some(LastRun {
            at: at.to_owned(),
            exit,
            values: values
                .map(|values| strip_secrets(values, secret_names))
                .unwrap_or_default(),
            values_recorded: true,
        });
        save_document(&path, &document)
    }

    /// Delete remembered state for one entry.
    ///
    /// # Errors
    ///
    /// Returns an error for filesystem failures other than a missing state file.
    pub fn forget(&self, slug: &str) -> Result<(), StateError> {
        let path = self.values_path(slug);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StateError::Io { path, source }),
        }
    }

    fn lock_path(&self, slug: &str) -> PathBuf {
        self.roots
            .state_dir()
            .join(".locks")
            .join(format!("{slug}.values.lock"))
    }
}

fn load_document(path: &Path) -> EntryState {
    let Ok(text) = fs::read_to_string(path) else {
        return EntryState::default();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return EntryState::default();
    };
    let Some(table) = value.as_table() else {
        return EntryState::default();
    };

    let values = table
        .get("values")
        .and_then(toml::Value::as_table)
        .map(string_table)
        .unwrap_or_default();
    let extra_args = table
        .get("extra_args")
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let presets = table
        .get("presets")
        .and_then(toml::Value::as_table)
        .map(|presets| {
            presets
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .as_table()
                        .map(|values| (name.clone(), string_table(values)))
                })
                .collect()
        })
        .unwrap_or_default();
    let last_run = table
        .get("last_run")
        .and_then(toml::Value::as_table)
        .and_then(parse_last_run);
    let extra = table
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "values" | "extra_args" | "presets" | "last_run"
            )
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    EntryState {
        values,
        extra_args,
        presets,
        last_run,
        extra,
    }
}

fn parse_last_run(table: &toml::Table) -> Option<LastRun> {
    let at = table.get("at")?.as_str()?.to_owned();
    let exit = i32::try_from(table.get("exit")?.as_integer()?).ok()?;
    let values_table = table.get("values").and_then(toml::Value::as_table);
    let values_recorded = values_table.is_some();
    let values = values_table.map(string_table).unwrap_or_default();
    Some(LastRun {
        at,
        exit,
        values,
        values_recorded,
    })
}

fn string_table(table: &toml::Table) -> BTreeMap<String, String> {
    table
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_owned())))
        .collect()
}

fn strip_secrets(
    values: &BTreeMap<String, String>,
    secret_names: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    values
        .iter()
        .filter(|(key, _)| !secret_names.contains(*key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn collect_removed(
    values: &BTreeMap<String, String>,
    names: &BTreeSet<String>,
    removed: &mut BTreeSet<String>,
) {
    removed.extend(values.keys().filter(|key| names.contains(*key)).cloned());
}

fn acquire_lock(path: &Path) -> Result<File, StateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| StateError::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| StateError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.lock().map_err(|source| StateError::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(file)
}

fn save_document(path: &Path, document: &EntryState) -> Result<(), StateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| StateError::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = toml::to_string(document).map_err(|source| StateError::Encode {
        path: path.to_owned(),
        source,
    })?;
    let mut file = AtomicWriteFile::open(path).map_err(|source| StateError::Io {
        path: path.to_owned(),
        source,
    })?;
    file.write_all(text.as_bytes())
        .map_err(|source| StateError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.commit().map_err(|source| StateError::Io {
        path: path.to_owned(),
        source,
    })
}
