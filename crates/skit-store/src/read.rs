//! Read-side filesystem and TOML adapter.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use skit_application::{Diagnostic, DiagnosticCode, EntryRepository, LibraryScan, RepositoryError};
use skit_domain::{Entry, EntryId, EntryKind, EntryMeta, EntrySummary, Slug, StorageMode};
use skit_i18n::Localize;

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
            let item = strict_directory_item(item, &scripts_dir)?;
            let file_type = strict_file_type(item.file_type(), &item.path())?;
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
            scan_directory_item(self, registry.as_ref(), item, &mut scan, &mut repairs);
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
                entry_with_name(entry, query)
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

fn strict_directory_item(
    item: io::Result<fs::DirEntry>,
    directory: &Path,
) -> Result<fs::DirEntry, RepositoryError> {
    item.map_err(|error| RepositoryError::Io {
        operation: "scan",
        path: directory.display().to_string(),
        reason: error.to_string(),
    })
}

fn strict_file_type(
    file_type: io::Result<fs::FileType>,
    path: &Path,
) -> Result<fs::FileType, RepositoryError> {
    file_type.map_err(|error| RepositoryError::Io {
        operation: "inspect",
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

fn record_file_type(
    candidate: String,
    file_type: io::Result<fs::FileType>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<fs::FileType> {
    match file_type {
        Ok(file_type) => Some(file_type),
        Err(error) => {
            let error = RepositoryError::Io {
                operation: "inspect",
                path: candidate.clone(),
                reason: error.to_string(),
            };
            diagnostics.push(Diagnostic::from_message(
                DiagnosticCode::Io,
                Some(candidate),
                error.message(),
            ));
            None
        }
    }
}

fn scan_directory_item(
    store: &FileStore,
    registry: Option<&Registry>,
    item: io::Result<fs::DirEntry>,
    scan: &mut LibraryScan,
    repairs: &mut Vec<(Entry, i64)>,
) {
    let item = match item {
        Ok(item) => item,
        Err(error) => {
            let error = RepositoryError::Io {
                operation: "scan",
                path: store.scripts_dir().display().to_string(),
                reason: error.to_string(),
            };
            scan.diagnostics.push(Diagnostic::from_message(
                DiagnosticCode::Io,
                None,
                error.message(),
            ));
            return;
        }
    };
    let candidate = item.file_name().to_string_lossy().into_owned();
    if !record_file_type(candidate.clone(), item.file_type(), &mut scan.diagnostics)
        .is_some_and(|file_type| file_type.is_dir())
    {
        return;
    }
    let slug = match Slug::parse(candidate.clone()) {
        Ok(slug) => slug,
        Err(error) => {
            scan.diagnostics.push(Diagnostic::from_message(
                DiagnosticCode::InvalidSlug,
                Some(candidate),
                error.message(),
            ));
            return;
        }
    };
    // The mtime only stamps the registry cache. An unusable one costs the cache,
    // never the entry.
    let mtime_ns = metadata_mtime_ns(&item.path().join("meta.toml")).ok();
    if let Some(summary) = mtime_ns
        .and_then(|mtime_ns| registry.and_then(|registry| registry.summary(&slug, mtime_ns)))
    {
        scan.entries.push(summary);
        return;
    }
    match store.read_entry(slug.clone()) {
        Ok(entry) => {
            scan.entries.push(summary_from(&entry));
            if let Some(mtime_ns) = mtime_ns {
                repairs.push((entry, mtime_ns));
            }
        }
        Err(error) => scan.diagnostics.push(diagnostic_from(error, &slug)),
    }
}

fn entry_with_name(entry: Entry, query: &str) -> Result<Entry, RepositoryError> {
    if entry.meta.name == query {
        Ok(entry)
    } else {
        Err(RepositoryError::NotFound {
            query: query.to_owned(),
        })
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
        RepositoryError::Io { .. } | RepositoryError::Rollback { .. } => DiagnosticCode::Io,
        RepositoryError::NotFound { .. }
        | RepositoryError::Ambiguous { .. }
        | RepositoryError::Conflict { .. }
        | RepositoryError::InvalidMutation { .. }
        | RepositoryError::StaleEntry { .. }
        | RepositoryError::SourceChanged { .. }
        | RepositoryError::Corrupt { .. } => DiagnosticCode::CorruptMetadata,
    };
    Diagnostic::from_message(code, Some(slug.as_str().to_owned()), error.message())
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

#[cfg(test)]
mod tests {
    use std::io;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn directory_error_adapters_keep_strict_and_best_effort_policies_separate() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("scripts"), "file").unwrap();
        assert!(matches!(
            FileStore::new(root.path()).scan_entries(),
            Err(RepositoryError::Io {
                operation: "scan",
                ..
            })
        ));

        assert!(matches!(
            strict_directory_item(Err(io::Error::other("iteration failed")), root.path()),
            Err(RepositoryError::Io {
                operation: "scan",
                ..
            })
        ));
        assert!(matches!(
            strict_file_type(Err(io::Error::other("inspect failed")), root.path()),
            Err(RepositoryError::Io {
                operation: "inspect",
                ..
            })
        ));

        let mut scan = LibraryScan::default();
        let mut repairs = Vec::new();
        scan_directory_item(
            &FileStore::new(root.path()),
            None,
            Err(io::Error::other("iteration failed")),
            &mut scan,
            &mut repairs,
        );
        assert_eq!(scan.diagnostics[0].code, DiagnosticCode::Io);
        assert_eq!(scan.diagnostics[0].slug, None);

        let scan_root = TempDir::new().unwrap();
        fs::create_dir_all(scan_root.path().join("scripts/Upper")).unwrap();
        assert!(
            FileStore::new(scan_root.path())
                .scan_entries()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn inspection_errors_and_changed_names_become_typed_results() {
        let mut diagnostics = Vec::new();
        assert!(
            record_file_type(
                "demo".to_owned(),
                Err(io::Error::other("inspect failed")),
                &mut diagnostics,
            )
            .is_none()
        );
        assert_eq!(diagnostics[0].code, DiagnosticCode::Io);
        assert_eq!(diagnostics[0].slug.as_deref(), Some("demo"));

        let entry = Entry {
            slug: Slug::parse("demo").unwrap(),
            meta: EntryMeta::minimal("Current", EntryKind::parse("command").unwrap()),
        };
        assert!(entry_with_name(entry.clone(), "Old").is_err());
        assert_eq!(
            entry_with_name(entry, "Current").unwrap().meta.name,
            "Current"
        );
    }
}
