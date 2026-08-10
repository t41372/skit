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

use crate::{fs_ops::try_acquire_lock, mutations::registry::Registry};

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

    /// Read every readable entry with its complete metadata in one directory pass.
    ///
    /// A composition root that needs whole entries for the whole library must use this instead of
    /// resolving each slug: [`EntryRepository::resolve`] re-reads the registry every call, so a
    /// per-entry loop is quadratic. A corrupt entry is skipped, never fatal.
    pub fn scan_entries(&self) -> Result<Vec<Entry>, RepositoryError> {
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

    fn repair_registry_rows(&self, slugs: &[Slug]) {
        let lock_path = self.data_dir.join("registry.native.lock");
        let Ok(Some(_lock)) = try_acquire_lock(&lock_path) else {
            return;
        };
        let Some(mut registry) = Registry::read(self.data_dir()) else {
            return;
        };
        let mut changed = false;
        for slug in slugs {
            if !registry.contains(slug) {
                continue;
            }
            let Ok(entry) = self.read_entry(slug.clone()) else {
                continue;
            };
            let entry_dir = self.scripts_dir().join(slug.as_str());
            if let Ok(repaired) = registry.repair_existing(&entry, &entry_dir) {
                changed |= repaired;
            }
        }
        if changed {
            let _ = registry.save();
        }
    }
}

impl EntryRepository for FileStore {
    fn scan(&self) -> Result<LibraryScan, RepositoryError> {
        let Some(registry) = Registry::read(self.data_dir()) else {
            return Ok(LibraryScan::default());
        };
        let mut scan = LibraryScan::default();
        let mut stale = Vec::new();
        for candidate in registry.row_keys() {
            scan_registry_row(self, &registry, candidate, &mut scan, &mut stale);
        }
        if !stale.is_empty() {
            self.repair_registry_rows(&stale);
        }
        Ok(scan)
    }

    fn resolve(&self, query: &str) -> Result<Entry, RepositoryError> {
        let registry =
            Registry::read(self.data_dir()).ok_or_else(|| RepositoryError::NotFound {
                query: query.to_owned(),
            })?;
        if let Ok(slug) = Slug::parse(query.to_owned())
            && registry.contains(&slug)
        {
            return self.read_entry(slug);
        }

        let claimants = registry.name_claimants(query);
        if let [slug] = claimants.as_slice() {
            let entry = self.read_entry((*slug).clone())?;
            if entry.meta.name == query {
                return Ok(entry);
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

fn scan_registry_row(
    store: &FileStore,
    registry: &Registry,
    candidate: String,
    scan: &mut LibraryScan,
    stale: &mut Vec<Slug>,
) {
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
    let meta_path = store.scripts_dir().join(slug.as_str()).join("meta.toml");
    if let Some(summary) = registry.summary(&slug, &meta_path) {
        scan.entries.push(summary);
        return;
    }
    match store.read_entry(slug.clone()) {
        Ok(entry) => {
            scan.entries.push(summary_from(&entry));
            stale.push(slug);
        }
        Err(error) => scan.diagnostics.push(diagnostic_from(error, &slug)),
    }
}

fn cached_or_authoritative_summary(
    registry: Option<&Registry>,
    slug: &Slug,
    meta_path: &Path,
    authoritative: impl FnOnce() -> Result<Entry, RepositoryError>,
) -> Result<EntrySummary, RepositoryError> {
    if let Some(summary) = registry.and_then(|registry| registry.summary(slug, meta_path)) {
        return Ok(summary);
    }
    authoritative().map(|entry| summary_from(&entry))
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
        RepositoryError::Io { .. }
        | RepositoryError::Rollback { .. }
        | RepositoryError::RemovalIncomplete { .. } => DiagnosticCode::Io,
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
    use std::{cell::Cell, io};

    use skit_application::{CreateEntry, EntryMutationRepository};
    use skit_domain::EntrySettings;
    use tempfile::TempDir;

    use super::*;

    #[test]
    #[cfg(any(unix, windows))]
    fn a_verified_cache_hit_does_not_call_the_authoritative_reader() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        store
            .create(CreateEntry {
                name: "Fast".to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: "from the cache".to_owned(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        let registry = Registry::read(root.path()).unwrap();
        let slug = Slug::parse("fast").unwrap();
        let reads = Cell::new(0_u32);

        let summary = cached_or_authoritative_summary(
            Some(&registry),
            &slug,
            &root.path().join("scripts/fast/meta.toml"),
            || {
                reads.set(reads.get() + 1);
                store.read_entry(slug.clone())
            },
        )
        .unwrap();

        assert_eq!(summary.description, "from the cache");
        assert_eq!(reads.get(), 0, "the fast path parsed meta.toml");
    }

    #[test]
    fn directory_error_adapters_keep_writer_scans_strict() {
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

        let scan_root = TempDir::new().unwrap();
        fs::create_dir_all(scan_root.path().join("scripts/Upper")).unwrap();
        assert!(
            FileStore::new(scan_root.path())
                .scan_entries()
                .unwrap()
                .is_empty()
        );
    }

    fn registry_table(root: &TempDir) -> toml::Table {
        toml::from_str(&fs::read_to_string(root.path().join("registry.toml")).unwrap()).unwrap()
    }

    fn registry_row<'a>(document: &'a toml::Table, slug: &str) -> &'a toml::Table {
        document
            .get("entries")
            .and_then(toml::Value::as_table)
            .and_then(|entries| entries.get(slug))
            .and_then(toml::Value::as_table)
            .unwrap()
    }

    #[test]
    fn test_repair_skips_an_entry_removed_meanwhile() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(CreateEntry {
                name: "legacy".to_owned(),
                kind: EntryKind::parse("future-kind").unwrap(),
                mode: StorageMode::Reference,
                source: "/original/legacy.tool".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        let slug = entry.slug.clone();

        store.remove(&entry).unwrap();
        store.repair_registry_rows(std::slice::from_ref(&slug));

        let document = registry_table(&root);
        assert!(
            document
                .get("entries")
                .and_then(toml::Value::as_table)
                .is_none_or(|entries| !entries.contains_key(slug.as_str()))
        );
    }

    #[test]
    fn test_repair_keeps_a_rename_that_landed_meanwhile() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(CreateEntry {
                name: "before".to_owned(),
                kind: EntryKind::parse("future-kind").unwrap(),
                mode: StorageMode::Reference,
                source: "/original/tool".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        let renamed = store.rename(&entry, "after").unwrap();

        store.repair_registry_rows(std::slice::from_ref(&renamed.slug));

        let document = registry_table(&root);
        let row = registry_row(&document, renamed.slug.as_str());
        assert_eq!(row.get("name").and_then(toml::Value::as_str), Some("after"));
        assert_eq!(
            row.get("mode").and_then(toml::Value::as_str),
            Some("reference")
        );
    }

    #[test]
    fn test_repair_adopts_a_slug_reused_by_an_older_skit_meanwhile() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(CreateEntry {
                name: "deploy".to_owned(),
                kind: EntryKind::parse("future-kind").unwrap(),
                mode: StorageMode::Reference,
                source: "/original/old.tool".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        let meta_path = root
            .path()
            .join("scripts")
            .join(entry.slug.as_str())
            .join("meta.toml");
        fs::write(
            &meta_path,
            concat!(
                "schema = 1\n",
                "name = \"deploy\"\n",
                "kind = \"shell\"\n",
                "mode = \"copy\"\n",
                "source = \"/original/new.sh\"\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-08-10T00:00:00Z\"\n",
                "workdir = \"invoke\"\n",
                "description = \"\"\n",
            ),
        )
        .unwrap();
        let registry_path = root.path().join("registry.toml");
        let mut document = registry_table(&root);
        let rows = document
            .get_mut("entries")
            .and_then(toml::Value::as_table_mut)
            .unwrap();
        rows.insert(
            entry.slug.as_str().to_owned(),
            toml::Value::Table(toml::Table::from_iter([
                ("name".to_owned(), toml::Value::String("deploy".to_owned())),
                ("kind".to_owned(), toml::Value::String("shell".to_owned())),
                (
                    "description".to_owned(),
                    toml::Value::String(String::new()),
                ),
            ])),
        );
        fs::write(&registry_path, toml::to_string_pretty(&document).unwrap()).unwrap();

        store.repair_registry_rows(std::slice::from_ref(&entry.slug));

        let repaired = registry_table(&root);
        let row = registry_row(&repaired, entry.slug.as_str());
        assert_eq!(row.get("kind").and_then(toml::Value::as_str), Some("shell"));
        assert_eq!(row.get("mode").and_then(toml::Value::as_str), Some("copy"));
        assert!(row.get("target").is_none());
    }

    #[test]
    fn test_repair_skips_a_meta_that_broke_or_went_unrepresentable_meanwhile() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let corrupt = store
            .create(CreateEntry {
                name: "corrupt".to_owned(),
                kind: EntryKind::parse("future-kind").unwrap(),
                mode: StorageMode::Reference,
                source: "/original/corrupt.tool".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        let sideways = store
            .create(CreateEntry {
                name: "sideways".to_owned(),
                kind: EntryKind::parse("future-kind").unwrap(),
                mode: StorageMode::Reference,
                source: "/original/sideways.tool".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        let before = fs::read(root.path().join("registry.toml")).unwrap();
        fs::write(
            root.path()
                .join("scripts")
                .join(corrupt.slug.as_str())
                .join("meta.toml"),
            "not [ toml",
        )
        .unwrap();
        let sideways_meta = root
            .path()
            .join("scripts")
            .join(sideways.slug.as_str())
            .join("meta.toml");
        let mut document = toml::from_str::<toml::Table>(&fs::read_to_string(&sideways_meta).unwrap())
            .unwrap();
        document.insert(
            "mode".to_owned(),
            toml::Value::String("sideways".to_owned()),
        );
        fs::write(sideways_meta, toml::to_string_pretty(&document).unwrap()).unwrap();

        store.repair_registry_rows(&[corrupt.slug, sideways.slug]);

        assert_eq!(fs::read(root.path().join("registry.toml")).unwrap(), before);
    }

    #[test]
    fn changed_names_become_typed_results() {
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
