#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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

/// The schema stored in one `scripts/<slug>/meta.toml` file.
///
/// Only `name` and `kind` are required. The remaining fields use the same defaults as
/// the Python implementation. Unknown fields are retained so a newer schema does not
/// become unreadable only because this build does not know one key yet.
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
    #[serde(default)]
    pub requires_python: String,
    #[serde(default)]
    pub params: Option<Vec<String>>,
    #[serde(default)]
    pub interpreter: String,
    #[serde(default)]
    pub runner: String,
    #[serde(default = "interpolation_enabled")]
    pub interpolate: bool,
    #[serde(default)]
    pub needs: Option<Vec<String>>,
    #[serde(default)]
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
    /// Copy-mode target checks need the language registry and are added with the launch
    /// slice. A reference row can be checked from metadata alone.
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

/// Errors returned by the headless core.
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
    NotFound {
        query: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "cannot access {}: {source}", path.display()),
            Self::InvalidMeta { path, source } => {
                write!(formatter, "cannot parse {}: {source}", path.display())
            }
            Self::NotFound { query } => write!(formatter, "library entry not found: {query}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidMeta { source, .. } => Some(source),
            Self::NotFound { .. } => None,
        }
    }
}

/// A read-only view of the skit library.
///
/// Write APIs will be added only after compatibility tests define their atomicity and
/// downgrade behavior. Reads never repair or migrate old files.
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

    /// List valid entries without changing the registry or metadata files.
    ///
    /// Corrupt entry metadata is skipped. This matches the existing library behavior:
    /// one damaged entry must not make every healthy entry disappear.
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

            let meta_path = entry_dir.join("meta.toml");
            let Ok(meta) = read_meta(&meta_path) else {
                continue;
            };
            let slug = item.file_name().to_string_lossy().into_owned();
            entries.push(EntrySummary {
                slug,
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
    /// Returns an error when a matching `meta.toml` cannot be read or parsed, or when
    /// no matching entry exists.
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
        let path = self.roots.state_dir.join("values").join(format!("{slug}.toml"));
        let text = fs::read_to_string(path).ok()?;
        toml::from_str::<StateFile>(&text).ok()?.last_run
    }
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
