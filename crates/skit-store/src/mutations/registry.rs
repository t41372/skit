use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use skit_application::RepositoryError;
use skit_domain::{Entry, EntryKind, EntrySummary, Slug, StorageMode};
use toml::{Table, Value};

use super::atomic::{atomic_write_bytes, io_error, try_acquire_lock};

/// The Python-compatible, rebuildable `registry.toml` projection.
#[derive(Clone, Debug)]
pub(crate) struct Registry {
    path: PathBuf,
    document: Table,
}

impl Registry {
    /// Read the current projection without changing a corrupt or unreadable file.
    pub(crate) fn read(data_dir: &Path) -> Option<Self> {
        let path = data_dir.join("registry.toml");
        let text = fs::read_to_string(&path).ok()?;
        let mut document = toml::from_str::<Table>(&text).ok()?;
        normalize_entries(&mut document);
        Some(Self { path, document })
    }

    /// Load the current projection for a writer, backing up corrupt bytes before starting fresh.
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
        normalize_entries(&mut document);
        Ok(Self { path, document })
    }

    /// Return a trusted listing row only when its shape and metadata stamp are exact.
    pub(crate) fn summary(&self, slug: &Slug, mtime_ns: i64) -> Option<EntrySummary> {
        let row = self.entries().get(slug.as_str())?.as_table()?;
        if row.get("mtime_ns")?.as_integer()? != mtime_ns {
            return None;
        }
        let name = row.get("name")?.as_str()?.to_owned();
        let kind = EntryKind::parse(row.get("kind")?.as_str()?.to_owned()).ok()?;
        let description = row.get("description")?.as_str()?.to_owned();
        let mode = match row.get("mode")?.as_str()? {
            "copy" => StorageMode::Copy,
            "reference" => StorageMode::Reference,
            _ => return None,
        };
        let target = match mode {
            StorageMode::Copy => None,
            StorageMode::Reference => Some(row.get("target")?.as_str()?.to_owned()),
        };
        Some(EntrySummary {
            slug: slug.clone(),
            name,
            kind,
            mode,
            description,
            target,
        })
    }

    /// Return canonical slugs whose rows claim this exact display name.
    pub(crate) fn name_claimants(&self, name: &str) -> Vec<Slug> {
        let mut claimants = self
            .entries()
            .iter()
            .filter_map(|(slug, value)| {
                let claimed_name = value
                    .as_table()
                    .and_then(|row| row.get("name"))
                    .and_then(Value::as_str);
                if claimed_name != Some(name) {
                    return None;
                }
                Slug::parse(slug.clone()).ok()
            })
            .collect::<Vec<_>>();
        claimants.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        claimants
    }

    /// Attempt one nonblocking, batch self-heal after a listing fell back to metadata.
    pub(crate) fn try_repair(data_dir: &Path, repairs: &[(Entry, i64)]) {
        if repairs.is_empty() {
            return;
        }
        let lock_path = data_dir.join("registry.native.lock");
        let Ok(Some(_lock)) = try_acquire_lock(&lock_path) else {
            return;
        };
        let Ok(mut registry) = Self::load(data_dir) else {
            return;
        };
        for (entry, mtime_ns) in repairs {
            registry.project_with_mtime(entry, *mtime_ns);
        }
        let _ = registry.save();
    }

    /// Return the slug of a row that already claims `name`, excluding one held entry.
    pub(super) fn name_owner(&self, name: &str, excluded: Option<&Slug>) -> Option<String> {
        self.entries().iter().find_map(|(slug, value)| {
            let belongs_to_other_entry = excluded.is_none_or(|excluded| excluded.as_str() != slug);
            let same_name = value
                .as_table()
                .and_then(|row| row.get("name"))
                .and_then(Value::as_str)
                == Some(name);
            (belongs_to_other_entry && same_name).then(|| slug.clone())
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
        let mtime_ns = metadata_mtime_ns(&entry_dir.join("meta.toml"))?;
        self.project_with_mtime(entry, mtime_ns);
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

    fn project_with_mtime(&mut self, entry: &Entry, mtime_ns: i64) {
        self.entries_mut().insert(
            entry.slug.as_str().to_owned(),
            Value::Table(row_for(entry, mtime_ns)),
        );
    }

    fn entries(&self) -> &Table {
        self.document
            .get("entries")
            .and_then(Value::as_table)
            .expect("Registry constructors normalize entries to a table")
    }

    fn entries_mut(&mut self) -> &mut Table {
        self.document
            .get_mut("entries")
            .and_then(Value::as_table_mut)
            .expect("Registry constructors normalize entries to a table")
    }
}

fn normalize_entries(document: &mut Table) {
    if !matches!(document.get("entries"), Some(Value::Table(_))) {
        document.insert("entries".to_owned(), Value::Table(Table::new()));
    }
}

fn row_for(entry: &Entry, mtime_ns: i64) -> Table {
    let mut row = Table::new();
    row.insert("name".to_owned(), Value::String(entry.meta.name.clone()));
    row.insert(
        "kind".to_owned(),
        Value::String(entry.meta.kind.as_str().to_owned()),
    );
    row.insert(
        "mode".to_owned(),
        Value::String(
            match entry.meta.mode {
                StorageMode::Copy => "copy",
                StorageMode::Reference => "reference",
            }
            .to_owned(),
        ),
    );
    row.insert(
        "description".to_owned(),
        Value::String(entry.meta.description.clone()),
    );
    row.insert("mtime_ns".to_owned(), Value::Integer(mtime_ns));
    if entry.meta.mode == StorageMode::Reference {
        row.insert(
            "target".to_owned(),
            Value::String(entry.meta.source.clone()),
        );
    }
    row
}

pub(crate) fn metadata_mtime_ns(path: &Path) -> Result<i64, RepositoryError> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| io_error("inspect", path, error))?;
    let nanos =
        modified
            .duration_since(UNIX_EPOCH)
            .map_err(|error| RepositoryError::InvalidMutation {
                reason: format!("metadata timestamp predates the Unix epoch: {error}"),
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
