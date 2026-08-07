//! Read-side filesystem and TOML adapter.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use skit_application::{Diagnostic, DiagnosticCode, EntryRepository, LibraryScan, RepositoryError};
use skit_domain::{Entry, EntryId, EntryKind, EntryMeta, EntrySummary, Slug, StorageMode};

use crate::mutations::registry::{Registry, metadata_mtime_ns};

/// Filesystem adapter for an existing skit data directory.
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

    pub(crate) fn scripts_dir(&self) -> PathBuf {
        self.data_dir.join("scripts")
    }

    pub(crate) fn read_entry(&self, slug: Slug) -> Result<Entry, RepositoryError> {
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

    pub(crate) fn scan_entries(&self) -> Result<Vec<Entry>, RepositoryError> {
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

        let registry = Registry::read(self.data_dir());
        let mut repairs = Vec::new();
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
            let meta_path = item.path().join("meta.toml");
            let mtime_ns = match metadata_mtime_ns(&meta_path) {
                Ok(mtime_ns) => mtime_ns,
                Err(error) => {
                    scan.diagnostics.push(diagnostic_from(error, &slug));
                    continue;
                }
            };
            if let Some(summary) = registry
                .as_ref()
                .and_then(|registry| registry.summary(&slug, mtime_ns))
            {
                scan.entries.push(summary);
                continue;
            }

            match self.read_entry(slug.clone()) {
                Ok(entry) => {
                    scan.entries.push(summary_from(&entry));
                    repairs.push((entry, mtime_ns));
                }
                Err(error) => scan.diagnostics.push(diagnostic_from(error, &slug)),
            }
        }
        Registry::try_repair(self.data_dir(), &repairs);
        Ok(scan)
    }

    fn resolve(&self, query: &str) -> Result<Entry, RepositoryError> {
        if let Ok(slug) = Slug::parse(query.to_owned()) {
            let exact_dir = self.scripts_dir().join(slug.as_str());
            if exact_dir.is_dir() {
                return self.read_entry(slug);
            }
        }

        if let Some(registry) = Registry::read(self.data_dir()) {
            let claimants = registry.name_claimants(query);
            if let [slug] = claimants.as_slice() {
                let entry = self.read_entry((*slug).clone())?;
                if entry.meta.name == query {
                    return Ok(entry);
                }
            }
        }

        let mut candidates = self
            .scan()?
            .entries
            .into_iter()
            .filter(|entry| entry.name == query)
            .map(|entry| entry.slug)
            .collect::<Vec<_>>();
        match candidates.len() {
            0 => Err(RepositoryError::NotFound {
                query: query.to_owned(),
            }),
            1 => {
                let slug = candidates.pop().expect("length checked");
                let entry = self.read_entry(slug)?;
                if entry.meta.name == query {
                    Ok(entry)
                } else {
                    Err(RepositoryError::NotFound {
                        query: query.to_owned(),
                    })
                }
            }
            _ => {
                let mut candidates = candidates
                    .into_iter()
                    .map(|slug| slug.as_str().to_owned())
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
    let code = match error {
        RepositoryError::Io { .. } => DiagnosticCode::Io,
        RepositoryError::NotFound { .. }
        | RepositoryError::Ambiguous { .. }
        | RepositoryError::Conflict { .. }
        | RepositoryError::InvalidMutation { .. }
        | RepositoryError::StaleEntry { .. }
        | RepositoryError::SourceChanged { .. }
        | RepositoryError::Corrupt { .. } => DiagnosticCode::CorruptMetadata,
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
