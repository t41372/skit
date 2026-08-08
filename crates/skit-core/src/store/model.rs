use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Family, spec_for, stored_name};

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
    #[serde(default = "interpolation_enabled", skip_serializing_if = "is_true")]
    pub interpolate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<toml::Table>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

/// A complete entry that is ready for the filesystem transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryDraft {
    pub meta: ScriptMeta,
    pub payload: Option<Vec<u8>>,
    pub(crate) payload_readonly: bool,
    pub(crate) payload_unix_mode: Option<u32>,
}

impl EntryDraft {
    /// Create a draft from complete metadata and an optional copied payload.
    #[must_use]
    pub fn new(meta: ScriptMeta, payload: Option<Vec<u8>>) -> Self {
        Self {
            meta,
            payload,
            payload_readonly: false,
            payload_unix_mode: None,
        }
    }

    /// Attach the source file's permission snapshot to a copied payload.
    #[must_use]
    pub(crate) fn with_payload_permissions(
        mut self,
        readonly: bool,
        unix_mode: Option<u32>,
    ) -> Self {
        self.payload_readonly = readonly;
        self.payload_unix_mode = unix_mode;
        self
    }
}

/// A complete library entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub slug: String,
    pub meta: ScriptMeta,
    pub dir: PathBuf,
}

impl Entry {
    /// Return the source path that this entry launches.
    #[must_use]
    pub fn script_path(&self) -> PathBuf {
        script_path(
            &self.dir,
            &self.meta.kind,
            &self.meta.mode,
            &self.meta.source,
        )
    }
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
    pub dir: PathBuf,
}

impl EntrySummary {
    /// Return the source path that this entry would launch.
    #[must_use]
    pub fn script_path(&self) -> PathBuf {
        script_path(&self.dir, &self.kind, &self.mode, &self.source)
    }

    /// Report whether this version can prove that the launch target is missing.
    #[must_use]
    pub fn target_missing(&self) -> bool {
        let Some(spec) = spec_for(&self.kind) else {
            return false;
        };
        let target = if self.kind == "exe" {
            (!self.source.is_empty()).then(|| PathBuf::from(&self.source))
        } else if spec.family == Family::Template {
            None
        } else {
            Some(self.script_path())
        };
        target.is_some_and(|path| !path.exists())
    }
}

/// The run stamp stored in `state/values/<slug>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RunStamp {
    pub at: String,
    pub exit: i32,
}

#[derive(Debug, Deserialize)]
pub(super) struct StateFile {
    #[serde(default)]
    pub(super) last_run: Option<RunStamp>,
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

pub(super) fn script_path(dir: &Path, kind: &str, mode: &str, source: &str) -> PathBuf {
    if mode == "reference" {
        return PathBuf::from(source);
    }
    dir.join(stored_name(kind))
}
