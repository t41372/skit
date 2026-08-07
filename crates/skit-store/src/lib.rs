//! Filesystem and TOML adapters for skit.
//!
//! The authoritative read path is `scripts/<slug>/meta.toml`. `registry.toml` remains an
//! optimization to port only after freshness and self-heal differential tests exist.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use skit_application::{
    Diagnostic, DiagnosticCode, EntryRepository, LibraryScan, RepositoryError,
};
use skit_domain::{Entry, EntryId, EntryKind, EntryMeta, EntrySummary, Slug, StorageMode};

/// Read-only filesystem adapter for an existing skit data directory.
#[derive(Clone, Debug)]
pub struct FileStore {
    data_dir: PathBuf,
}

impl FileStore {
    /// Use the supplied skit data root (the parent of `scripts/`).
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Return the configured data root.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn scripts_dir(&self) -> PathBuf {
        self.data_dir.join("scripts")
    }

    fn read_entry(&self, slug: Slug) -> Result<Entry, RepositoryError> {
        let meta_path = self.scripts_dir().join(slug.as_str()).join("meta.toml");
        let text = fs::read_to_string(&meta_path).map_err(|error| RepositoryError::Io {
            operation: "read",
            path: meta_path.display().to_string(),
            reason: error.to_string(),
        })?;
        let raw = toml::from_str::<RawMeta>(&text).map_err(|error| RepositoryError::Corrupt {
            slug: slug.as_str().to_owned(),
            reason: error.to_string(),
        })?;
        let meta = raw
            .into_domain()
            .map_err(|reason| RepositoryError::Corrupt {
                slug: slug.as_str().to_owned(),
                reason,
            })?;
        Ok(Entry { slug, meta })
    }

    fn scan_entries(&self) -> Result<Vec<Entry>, RepositoryError> {
        let scripts_dir = self.scripts_dir();
        let reader = match fs::read_dir(&scripts_dir) {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(RepositoryError::Io {
                    operation: "scan",
                    path: scripts_dir.display().to_string(),
                    reason: error.to_string(),
                });
            }
        };

        let mut entries = Vec::new();
        for item in reader {
            let item = item.map_err(|error| RepositoryError::Io {
                operation: "scan",
                path: scripts_dir.display().to_string(),
                reason: error.to_string(),
            })?;
            let file_type = item.file_type().map_err(|error| RepositoryError::Io {
                operation: "inspect",
                path: item.path().display().to_string(),
                reason: error.to_string(),
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let candidate = item.file_name().to_string_lossy().into_owned();
            let Ok(slug) = Slug::parse(candidate) else {
                continue;
            };
            if let Ok(entry) = self.read_entry(slug) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }
}

impl EntryRepository for FileStore {
    fn scan(&self) -> Result<LibraryScan, RepositoryError> {
        let scripts_dir = self.scripts_dir();
        let reader = match fs::read_dir(&scripts_dir) {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LibraryScan::default());
            }
            Err(error) => {
                return Err(RepositoryError::Io {
                    operation: "scan",
                    path: scripts_dir.display().to_string(),
                    reason: error.to_string(),
                });
            }
        };

        let mut scan = LibraryScan::default();
        for item in reader {
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    scan.diagnostics.push(Diagnostic {
                        code: DiagnosticCode::Io,
                        slug: None,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let candidate = item.file_name().to_string_lossy().into_owned();
            let file_type = match item.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    scan.diagnostics.push(Diagnostic {
                        code: DiagnosticCode::Io,
                        slug: Some(candidate),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if !file_type.is_dir() {
                continue;
            }
            let slug = match Slug::parse(candidate.clone()) {
                Ok(slug) => slug,
                Err(error) => {
                    scan.diagnostics.push(Diagnostic {
                        code: DiagnosticCode::InvalidSlug,
                        slug: Some(candidate),
                        message: error.to_string(),
                    });
                    continue;
                }
            };

            match self.read_entry(slug.clone()) {
                Ok(entry) => scan.entries.push(summary_from(&entry)),
                Err(error) => scan.diagnostics.push(diagnostic_from(error, &slug)),
            }
        }
        Ok(scan)
    }

    fn resolve(&self, query: &str) -> Result<Entry, RepositoryError> {
        if let Ok(slug) = Slug::parse(query.to_owned()) {
            let exact_dir = self.scripts_dir().join(slug.as_str());
            if exact_dir.is_dir() {
                return self.read_entry(slug);
            }
        }

        let mut matches = self
            .scan_entries()?
            .into_iter()
            .filter(|entry| entry.meta.name == query)
            .collect::<Vec<_>>();
        match matches.len() {
            0 => Err(RepositoryError::NotFound {
                query: query.to_owned(),
            }),
            1 => Ok(matches.pop().expect("length checked")),
            _ => {
                let mut candidates = matches
                    .into_iter()
                    .map(|entry| entry.slug.as_str().to_owned())
                    .collect::<Vec<_>>();
                candidates.sort();
                Err(RepositoryError::Ambiguous {
                    query: query.to_owned(),
                    candidates,
                })
            }
        }
    }
}

fn summary_from(entry: &Entry) -> EntrySummary {
    EntrySummary {
        slug: entry.slug.clone(),
        name: entry.meta.name.clone(),
        kind: entry.meta.kind.clone(),
        mode: entry.meta.mode,
        description: entry.meta.description.clone(),
        target: if entry.meta.mode == StorageMode::Reference {
            Some(entry.meta.source.clone())
        } else {
            None
        },
    }
}

fn diagnostic_from(error: RepositoryError, slug: &Slug) -> Diagnostic {
    let code = match &error {
        RepositoryError::Io { .. } => DiagnosticCode::Io,
        RepositoryError::Corrupt { .. }
        | RepositoryError::NotFound { .. }
        | RepositoryError::Ambiguous { .. } => DiagnosticCode::CorruptMetadata,
    };
    Diagnostic {
        code,
        slug: Some(slug.as_str().to_owned()),
        message: error.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct RawMeta {
    #[serde(default = "schema_one")]
    schema: u32,
    name: String,
    kind: String,
    #[serde(default)]
    mode: StorageMode,
    #[serde(default)]
    source: String,
    #[serde(default)]
    source_hash: String,
    #[serde(default)]
    added_at: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default = "origin")]
    workdir: String,
    #[serde(default)]
    description: String,
    #[serde(flatten)]
    extra: BTreeMap<String, toml::Value>,
}

impl RawMeta {
    fn into_domain(self) -> Result<EntryMeta, String> {
        let kind = EntryKind::parse(self.kind).map_err(|error| error.to_string())?;
        let id = self
            .id
            .filter(|value| !value.is_empty())
            .map(EntryId::parse)
            .transpose()
            .map_err(|error| error.to_string())?;
        let extra = self
            .extra
            .into_iter()
            .map(|(key, value)| {
                serde_json::to_value(value)
                    .map(|value| (key, value))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(EntryMeta {
            schema: self.schema,
            name: self.name,
            kind,
            mode: self.mode,
            source: self.source,
            source_hash: self.source_hash,
            added_at: self.added_at,
            id,
            workdir: self.workdir,
            description: self.description,
            extra,
        })
    }
}

const fn schema_one() -> u32 {
    1
}

fn origin() -> String {
    "origin".to_owned()
}
