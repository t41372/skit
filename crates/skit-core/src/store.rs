use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};

/// The filesystem roots that skit owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoots {
    data_dir: PathBuf,
    state_dir: PathBuf,
    config_dir: PathBuf,
}

impl LibraryRoots {
    /// Create explicit roots for the data, state, and configuration layers.
    pub fn new(
        data_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        config_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            data_dir: data_dir.into(),
            state_dir: state_dir.into(),
            config_dir: config_dir.into(),
        }
    }

    /// Return the data directory.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Return the state directory.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Return the configuration directory.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }
}

fn schema_v1() -> u32 {
    1
}

fn copy_mode() -> String {
    "copy".to_owned()
}

fn origin_workdir() -> String {
    "origin".to_owned()
}

fn interpolation_enabled() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// The schema stored in one `scripts/<slug>/meta.toml` file.
///
/// Only `name` and `kind` are required when reading. The other fields use the same
/// defaults as the Python implementation. Unknown fields are retained so a newer
/// schema does not become unreadable only because this build does not know one key.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ScriptMeta {
    #[serde(default = "schema_v1")]
    pub schema: u32,
    pub name: String,
    pub kind: String,
    #[serde(default = "copy_mode")]
    pub mode: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_hash: String,
    #[serde(default)]
    pub added_at: String,
    #[serde(default = "origin_workdir")]
    pub workdir: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub requires_python: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub interpreter: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub runner: String,
    #[serde(
        default = "interpolation_enabled",
        skip_serializing_if = "is_true"
    )]
    pub interpolate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<toml::Table>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

/// A complete library entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub slug: String,
    pub meta: ScriptMeta,
    pub dir: PathBuf,
}

/// The fields needed by library listings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySummary {
    pub slug: String,
    pub name: String,
    pub kind: String,
    pub mode: String,
    pub description: String,
    pub source: String,
}

impl EntrySummary {
    /// Report whether a referenced launch target is missing.
    ///
    /// Copy-mode checks need the language registry and are added with the launch slice.
    #[must_use]
    pub fn target_missing(&self) -> bool {
        self.mode == "reference" && !self.source.is_empty() && !Path::new(&self.source).exists()
    }
}

/// The run stamp stored in `state/values/<slug>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RunStamp {
    pub at: String,
    pub exit: i32,
}

#[derive(Debug, Deserialize)]
struct StateFile {
    #[serde(default)]
    last_run: Option<RunStamp>,
}

/// Errors returned by the headless store.
#[derive(Debug)]
pub enum Error {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    InvalidMeta {
        path: PathBuf,
        source: toml::de::Error,
    },
    EncodeToml {
        path: PathBuf,
        source: toml::ser::Error,
    },
    InvalidName,
    NameConflict {
        name: String,
    },
    NotFound {
        query: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "cannot access {}: {source}", path.display())
            }
            Self::InvalidMeta { path, source } => {
                write!(formatter, "cannot parse {}: {source}", path.display())
            }
            Self::EncodeToml { path, source } => {
                write!(formatter, "cannot encode {}: {source}", path.display())
            }
            Self::InvalidName => write!(formatter, "a name is required"),
            Self::NameConflict { name } => write!(formatter, "the name is already in use: {name}"),
            Self::NotFound { query } => write!(formatter, "library entry not found: {query}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidMeta { source, .. } => Some(source),
            Self::EncodeToml { source, .. } => Some(source),
            Self::InvalidName | Self::NameConflict { .. } | Self::NotFound { .. } => None,
        }
    }
}

/// A filesystem view of the skit library.
#[derive(Debug, Clone)]
pub struct Store {
    roots: LibraryRoots,
}

impl Store {
    /// Create a store over explicit filesystem roots.
    #[must_use]
    pub fn new(roots: LibraryRoots) -> Self {
        Self { roots }
    }

    /// Return the owned filesystem roots.
    #[must_use]
    pub fn roots(&self) -> &LibraryRoots {
        &self.roots
    }

    /// List valid entries without changing registry or metadata files.
    ///
    /// Corrupt metadata is skipped. One damaged entry must not hide healthy entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the scripts directory cannot be enumerated for a reason
    /// other than it not existing.
    pub fn list(&self) -> Result<Vec<EntrySummary>, Error> {
        let scripts_dir = self.roots.data_dir.join("scripts");
        let directory = match fs::read_dir(&scripts_dir) {
            Ok(directory) => directory,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::Io {
                    path: scripts_dir,
                    source,
                });
            }
        };

        let mut entries = Vec::new();
        for item in directory {
            let item = item.map_err(|source| Error::Io {
                path: scripts_dir.clone(),
                source,
            })?;
            let entry_dir = item.path();
            let file_type = item.file_type().map_err(|source| Error::Io {
                path: entry_dir.clone(),
                source,
            })?;
            if !file_type.is_dir() {
                continue;
            }

            let Ok(meta) = read_meta(&entry_dir.join("meta.toml")) else {
                continue;
            };
            entries.push(EntrySummary {
                slug: item.file_name().to_string_lossy().into_owned(),
                name: meta.name,
                kind: meta.kind,
                mode: meta.mode,
                description: meta.description,
                source: meta.source,
            });
        }

        entries.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.slug.cmp(&right.slug))
        });
        Ok(entries)
    }

    /// Resolve an entry by exact slug first, then by exact display name.
    ///
    /// # Errors
    ///
    /// Returns an error when matching metadata cannot be read or parsed, or when no
    /// matching entry exists.
    pub fn resolve(&self, query: &str) -> Result<Entry, Error> {
        let scripts_dir = self.roots.data_dir.join("scripts");
        let direct_dir = scripts_dir.join(query);
        if direct_dir.is_dir() {
            let meta = read_meta(&direct_dir.join("meta.toml"))?;
            return Ok(Entry {
                slug: query.to_owned(),
                meta,
                dir: direct_dir,
            });
        }

        let directory = match fs::read_dir(&scripts_dir) {
            Ok(directory) => directory,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(Error::NotFound {
                    query: query.to_owned(),
                });
            }
            Err(source) => {
                return Err(Error::Io {
                    path: scripts_dir,
                    source,
                });
            }
        };

        for item in directory {
            let item = item.map_err(|source| Error::Io {
                path: scripts_dir.clone(),
                source,
            })?;
            let entry_dir = item.path();
            if !entry_dir.is_dir() {
                continue;
            }
            let Ok(meta) = read_meta(&entry_dir.join("meta.toml")) else {
                continue;
            };
            if meta.name == query {
                return Ok(Entry {
                    slug: item.file_name().to_string_lossy().into_owned(),
                    meta,
                    dir: entry_dir,
                });
            }
        }

        Err(Error::NotFound {
            query: query.to_owned(),
        })
    }

    /// Read the last-run stamp for one entry.
    ///
    /// Missing or corrupt state is treated as no history, as in the Python version.
    #[must_use]
    pub fn last_run(&self, slug: &str) -> Option<RunStamp> {
        let path = self
            .roots
            .state_dir
            .join("values")
            .join(format!("{slug}.toml"));
        let text = fs::read_to_string(path).ok()?;
        toml::from_str::<StateFile>(&text).ok()?.last_run
    }

    /// Change an entry description without changing any other metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be resolved, locked, encoded, or written.
    pub fn update_description(&self, query: &str, description: &str) -> Result<Entry, Error> {
        let initial = self.resolve(query)?;
        let _entry_lock = acquire_lock(&self.entry_lock_path(&initial.slug))?;
        let mut entry = self.resolve(&initial.slug)?;
        entry.meta.description = description.trim().to_owned();
        write_meta(&entry.dir.join("meta.toml"), &entry.meta)?;
        self.sync_registry_row(&entry)?;
        Ok(entry)
    }

    /// Change an entry display name. The slug and state path do not change.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or duplicate name, or when metadata cannot be
    /// locked, encoded, or written.
    pub fn rename(&self, query: &str, new_name: &str) -> Result<Entry, Error> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(Error::InvalidName);
        }

        let initial = self.resolve(query)?;
        let _entry_lock = acquire_lock(&self.entry_lock_path(&initial.slug))?;
        let mut entry = self.resolve(&initial.slug)?;
        let _registry_lock = acquire_lock(&self.registry_lock_path())?;

        if self
            .list()?
            .iter()
            .any(|other| other.slug != entry.slug && other.name == new_name)
        {
            return Err(Error::NameConflict {
                name: new_name.to_owned(),
            });
        }

        entry.meta.name = new_name.to_owned();
        write_meta(&entry.dir.join("meta.toml"), &entry.meta)?;
        if let Some(mut document) = load_registry_document(&self.registry_path())?
            && registry_contains_slug(&document, &entry.slug)
        {
            set_registry_row(&mut document, &entry)?;
            write_registry_document(&self.registry_path(), &document)?;
        }
        Ok(entry)
    }

    /// Remove an entry and its remembered values.
    ///
    /// A reference entry stores only metadata in the library. Its original source is
    /// outside the entry directory and is never removed here.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be resolved, locked, or removed.
    pub fn remove(&self, query: &str) -> Result<String, Error> {
        let initial = self.resolve(query)?;
        let _entry_lock = acquire_lock(&self.entry_lock_path(&initial.slug))?;
        let _registry_lock = acquire_lock(&self.registry_lock_path())?;
        let entry = self.resolve(&initial.slug)?;
        let removed_name = entry.meta.name.clone();
        let mut registry = load_registry_document(&self.registry_path())?;

        fs::remove_dir_all(&entry.dir).map_err(|source| Error::Io {
            path: entry.dir.clone(),
            source,
        })?;
        let state_path = self
            .roots
            .state_dir
            .join("values")
            .join(format!("{}.toml", entry.slug));
        match fs::remove_file(&state_path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::Io {
                    path: state_path,
                    source,
                });
            }
        }

        if let Some(document) = &mut registry
            && remove_registry_row(document, &entry.slug)
        {
            write_registry_document(&self.registry_path(), document)?;
        }
        Ok(removed_name)
    }

    fn entry_lock_path(&self, slug: &str) -> PathBuf {
        self.roots
            .data_dir
            .join(".locks")
            .join(format!("{slug}.meta.lock"))
    }

    fn registry_path(&self) -> PathBuf {
        self.roots.data_dir.join("registry.toml")
    }

    fn registry_lock_path(&self) -> PathBuf {
        self.roots.data_dir.join("registry.native.lock")
    }

    fn sync_registry_row(&self, entry: &Entry) -> Result<(), Error> {
        if !self.registry_path().is_file() {
            return Ok(());
        }
        let _registry_lock = acquire_lock(&self.registry_lock_path())?;
        let Some(mut document) = load_registry_document(&self.registry_path())? else {
            return Ok(());
        };
        if !registry_contains_slug(&document, &entry.slug) {
            return Ok(());
        }
        set_registry_row(&mut document, entry)?;
        write_registry_document(&self.registry_path(), &document)
    }
}

fn acquire_lock(path: &Path) -> Result<File, Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
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
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    file.lock().map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(file)
}

fn read_meta(path: &Path) -> Result<ScriptMeta, Error> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| Error::InvalidMeta {
        path: path.to_owned(),
        source,
    })
}

fn write_meta(path: &Path, meta: &ScriptMeta) -> Result<(), Error> {
    let text = toml::to_string(meta).map_err(|source| Error::EncodeToml {
        path: path.to_owned(),
        source,
    })?;
    atomic_write(path, &text)
}

fn load_registry_document(path: &Path) -> Result<Option<toml::Table>, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    Ok(toml::from_str(&text).ok())
}

fn registry_contains_slug(document: &toml::Table, slug: &str) -> bool {
    document
        .get("entries")
        .and_then(toml::Value::as_table)
        .is_some_and(|entries| entries.contains_key(slug))
}

fn set_registry_row(document: &mut toml::Table, entry: &Entry) -> Result<(), Error> {
    let Some(entries) = document
        .get_mut("entries")
        .and_then(toml::Value::as_table_mut)
    else {
        return Ok(());
    };
    entries.insert(entry.slug.clone(), registry_row(entry)?);
    Ok(())
}

fn remove_registry_row(document: &mut toml::Table, slug: &str) -> bool {
    document
        .get_mut("entries")
        .and_then(toml::Value::as_table_mut)
        .and_then(|entries| entries.remove(slug))
        .is_some()
}

fn registry_row(entry: &Entry) -> Result<toml::Value, Error> {
    let meta_path = entry.dir.join("meta.toml");
    let metadata = fs::metadata(&meta_path).map_err(|source| Error::Io {
        path: meta_path.clone(),
        source,
    })?;
    let modified = metadata.modified().map_err(|source| Error::Io {
        path: meta_path.clone(),
        source,
    })?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::Io {
            path: meta_path.clone(),
            source: io::Error::other(error),
        })?;
    let mtime_ns = i64::try_from(duration.as_nanos()).map_err(|error| Error::Io {
        path: meta_path,
        source: io::Error::other(error),
    })?;

    let mut row = toml::Table::new();
    row.insert("name".to_owned(), toml::Value::String(entry.meta.name.clone()));
    row.insert("kind".to_owned(), toml::Value::String(entry.meta.kind.clone()));
    row.insert("mode".to_owned(), toml::Value::String(entry.meta.mode.clone()));
    row.insert(
        "description".to_owned(),
        toml::Value::String(entry.meta.description.clone()),
    );
    row.insert("mtime_ns".to_owned(), toml::Value::Integer(mtime_ns));
    if entry.meta.mode == "reference" {
        row.insert(
            "target".to_owned(),
            toml::Value::String(entry.meta.source.clone()),
        );
    }
    Ok(toml::Value::Table(row))
}

fn write_registry_document(path: &Path, document: &toml::Table) -> Result<(), Error> {
    let text = toml::to_string(document).map_err(|source| Error::EncodeToml {
        path: path.to_owned(),
        source,
    })?;
    atomic_write(path, &text)
}

fn atomic_write(path: &Path, text: &str) -> Result<(), Error> {
    let mut file = AtomicWriteFile::open(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    file.write_all(text.as_bytes()).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    file.commit().map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}
