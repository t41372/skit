use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use skit_application::RepositoryError;
use skit_domain::{Entry, Slug, StorageMode};
use toml::{Table, Value};

use super::atomic::{atomic_write_bytes, io_error};

/// The Python-compatible, rebuildable `registry.toml` projection.
#[derive(Clone, Debug)]
pub(super) struct Registry {
    path: PathBuf,
    document: Table,
}

impl Registry {
    /// Load the current projection, backing up corrupt bytes before starting fresh.
    pub(super) fn load(data_dir: &Path) -> Result<Self, RepositoryError> {
        let path = data_dir.join("registry.toml");
        let mut document = match fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Table>(&text) {
                Ok(document) => document,
                Err(_) => {
                    backup_corrupt(&path)?;
                    Table::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Table::new(),
            Err(_) => {
                backup_corrupt(&path)?;
                Table::new()
            }
        };
        if !matches!(document.get("entries"), Some(Value::Table(_))) {
            document.insert("entries".to_owned(), Value::Table(Table::new()));
        }
        Ok(Self { path, document })
    }

    /// Whether an index row other than `excluded` already claims this display name.
    pub(super) fn name_is_taken(&self, name: &str, excluded: Option<&Slug>) -> bool {
        self.entries().iter().any(|(slug, value)| {
            excluded.is_none_or(|excluded| excluded.as_str() != slug)
                && value
                    .as_table()
                    .and_then(|row| row.get("name"))
                    .and_then(Value::as_str)
                    == Some(name)
        })
    }

    /// Whether an index row other than `excluded` already claims this slug.
    pub(super) fn slug_is_taken(&self, slug: &Slug, excluded: Option<&Slug>) -> bool {
        excluded != Some(slug) && self.entries().contains_key(slug.as_str())
    }

    /// Replace one row with a projection stamped from the current metadata file.
    pub(super) fn project(
        &mut self,
        entry: &Entry,
        entry_dir: &Path,
    ) -> Result<(), RepositoryError> {
        self.entries_mut().insert(
            entry.slug.as_str().to_owned(),
            Value::Table(row_for(entry, entry_dir)?),
        );
        Ok(())
    }

    /// Delete one row.
    pub(super) fn remove(&mut self, slug: &Slug) {
        self.entries_mut().remove(slug.as_str());
    }

    /// Persist the whole projection through the same atomic replacement discipline as metadata.
    pub(super) fn save(&self) -> Result<(), RepositoryError> {
        let text = toml::to_string_pretty(&self.document).map_err(|error| {
            RepositoryError::InvalidMutation {
                reason: format!("could not encode registry.toml: {error}"),
            }
        })?;
        atomic_write_bytes(&self.path, text.as_bytes())
    }

    fn entries(&self) -> &Table {
        self.document
            .get("entries")
            .and_then(Value::as_table)
            .expect("Registry::load normalizes entries to a table")
    }

    fn entries_mut(&mut self) -> &mut Table {
        self.document
            .get_mut("entries")
            .and_then(Value::as_table_mut)
            .expect("Registry::load normalizes entries to a table")
    }
}

fn row_for(entry: &Entry, entry_dir: &Path) -> Result<Table, RepositoryError> {
    let mut row = Table::new();
    row.insert("name".to_owned(), Value::String(entry.meta.name.clone()));
    row.insert(
        "kind".to_owned(),
        Value::String(entry.meta.kind.as_str().to_owned()),
    );
    row.insert(
        "mode".to_owned(),
        Value::String(match entry.meta.mode {
            StorageMode::Copy => "copy",
            StorageMode::Reference => "reference",
        }
        .to_owned()),
    );
    row.insert(
        "description".to_owned(),
        Value::String(entry.meta.description.clone()),
    );
    row.insert(
        "mtime_ns".to_owned(),
        Value::Integer(metadata_mtime_ns(&entry_dir.join("meta.toml"))?),
    );
    if entry.meta.mode == StorageMode::Reference {
        row.insert(
            "target".to_owned(),
            Value::String(entry.meta.source.clone()),
        );
    }
    Ok(row)
}

fn metadata_mtime_ns(path: &Path) -> Result<i64, RepositoryError> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| io_error("inspect", path, error))?;
    let nanos = modified.duration_since(UNIX_EPOCH).map_err(|error| {
        RepositoryError::InvalidMutation {
            reason: format!("metadata timestamp predates the Unix epoch: {error}"),
        }
    })?;
    i64::try_from(nanos.as_nanos()).map_err(|error| RepositoryError::InvalidMutation {
        reason: format!("metadata timestamp does not fit registry.toml: {error}"),
    })
}

fn backup_corrupt(path: &Path) -> Result<(), RepositoryError> {
    let backup = path.with_file_name(format!(
        "{}.corrupt",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("registry.toml")
    ));
    if backup.is_file() {
        fs::remove_file(&backup).map_err(|error| io_error("replace backup", &backup, error))?;
    } else if backup.exists() {
        return Err(RepositoryError::Io {
            operation: "backup",
            path: backup.display().to_string(),
            reason: "the backup path exists and is not a regular file".to_owned(),
        });
    }
    fs::rename(path, &backup).map_err(|error| io_error("backup", path, error))
}
