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

    /// Project every index row into a listing, reporting the rows that fell back to their meta.
    ///
    /// The pure half of `list`, shared with `resolve` so a name sweep never triggers the read-path
    /// self-heal (the oracle's `resolve` loads the index but never calls `_repair_rows`). A returned
    /// slug is "stale": its row could not answer (legacy, hand-broken, or a meta that changed under
    /// it) yet its meta read cleanly, so a fresh projection would repair it. A row served from the
    /// cache/legacy fast path is fresh, and a meta that would not read is doctor's, not the
    /// self-heal's -- neither is returned.
    fn scan_inner(&self) -> Result<(LibraryScan, Vec<Slug>), RepositoryError> {
        let Some(registry) = Registry::read(self.data_dir()) else {
            return Ok((LibraryScan::default(), Vec::new()));
        };
        let mut scan = LibraryScan::default();
        let mut stale = Vec::new();
        for candidate in registry.row_keys() {
            scan_registry_row(self, &registry, candidate, &mut scan, &mut stale);
        }
        Ok((scan, stale))
    }

    /// Opportunistically re-project stale index rows from their metas, under a NON-BLOCKING lock.
    ///
    /// Faithful translation of `skit.store._repair_rows` (store.py ~1006-1061). Without it a library
    /// an older skit wrote -- or one whose metas were hand edited -- would fall back to reading those
    /// metas on every listing forever; the index otherwise refreshes only on add/rename/describe or
    /// an explicit `rebuild`. Repairing on first listing makes it self-healing. Two load-bearing
    /// properties, both from the oracle:
    ///
    /// - The lock is TRY-ONLY. This runs on read paths (`list`, shell TAB completion); the blocking
    ///   lock polls forever, so a listing that used it would freeze the shell behind any process on
    ///   the lock. If the lock is busy the repair simply does not happen this time and the next
    ///   listing tries again. A read stays a read.
    /// - Rows are re-derived from their metas UNDER the lock, never written from the listing's
    ///   snapshot. Anything can commit while the listing read metas -- a rename, a describe, a
    ///   remove-then-add reusing the slug, even by an older skit whose fresh legacy row is
    ///   indistinguishable from the stale one the listing saw. Re-deriving from the meta as it is NOW
    ///   makes the newest state win no matter who wrote it.
    ///
    /// Best effort throughout: the lock, a re-read of the index, and the save can each fail without
    /// failing the read; a slug removed, or whose meta vanished or broke, since the listing is
    /// skipped; and nothing is saved unless a row actually changed. It contends on the same
    /// `registry.native.lock` writers take, so a repair never races a committing writer.
    fn repair_rows(&self, stale: &[Slug]) {
        let repair = match try_acquire_lock(&self.data_dir().join("registry.native.lock")) {
            Some(lock) => Registry::read(self.data_dir()).map(|registry| (lock, registry)),
            None => None,
        };
        let Some((_lock, mut registry)) = repair else {
            return; // busy, missing, or corrupt: the next listing tries again
        };
        let mut changed = false;
        let current = stale
            .iter()
            .filter(|slug| registry.contains(slug))
            .filter_map(|slug| {
                self.read_entry((*slug).clone())
                    .ok()
                    .map(|entry| (slug, entry))
            })
            .collect::<Vec<_>>();
        for (slug, entry) in current {
            let entry_dir = self.scripts_dir().join(slug.as_str());
            if registry
                .reproject_if_changed(&entry, &entry_dir)
                .unwrap_or(false)
            {
                changed = true;
            }
        }
        if changed {
            let _ = registry.save(); // a read must not fail because its optional write could not land
        }
    }
}

impl EntryRepository for FileStore {
    fn scan(&self) -> Result<LibraryScan, RepositoryError> {
        let (scan, stale) = self.scan_inner()?;
        if !stale.is_empty() {
            self.repair_rows(&stale);
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
            // A slug present in the registry whose meta is missing, unreadable, or corrupt resolves
            // to NotFound -- not an Io/Corrupt error. The oracle's resolve reads the meta and catches
            // _META_CORRUPTION = (OSError, TOMLDecodeError, ScriptMetaError), re-raising NotFoundError
            // (store.py resolve). A hand-edited scalar registry row is a member with no readable meta,
            // which used to crash `skit run <name>` before the chokepoint normalized it.
            return self
                .read_entry(slug)
                .map_err(|_| RepositoryError::NotFound {
                    query: query.to_owned(),
                });
        }

        let claimants = registry.name_claimants(query);
        if let [slug] = claimants.as_slice() {
            let entry = self.read_entry((*slug).clone())?;
            if entry.meta.name == query {
                return Ok(entry);
            }
        }

        let mut candidates = self
            .scan_inner()?
            .0
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
    match cached_or_authoritative_summary(Some(registry), &slug, &meta_path, || {
        store.read_entry(slug.clone())
    }) {
        Ok((summary, SummarySource::Cache)) => scan.entries.push(summary),
        // The row could not answer but its meta read cleanly: serve the meta and stage the slug so
        // the read-path self-heal can re-project the row from the truth (oracle `list_summaries`
        // stages a slug whose meta-projected row would validate).
        Ok((summary, SummarySource::Authoritative)) => {
            scan.entries.push(summary);
            stale.push(slug);
        }
        Err(error) => scan.diagnostics.push(diagnostic_from(error, &slug)),
    }
}

/// Where a listing row's summary came from, so the caller can stage only the fallbacks for repair.
enum SummarySource {
    /// The index row answered on the verified cache or legacy fast path: the row is fresh.
    Cache,
    /// The row could not answer and the meta was read authoritatively: the row is stale.
    Authoritative,
}

fn cached_or_authoritative_summary(
    registry: Option<&Registry>,
    slug: &Slug,
    meta_path: &Path,
    authoritative: impl FnOnce() -> Result<Entry, RepositoryError>,
) -> Result<(EntrySummary, SummarySource), RepositoryError> {
    if let Some(summary) = registry.and_then(|registry| registry.summary(slug, meta_path)) {
        return Ok((summary, SummarySource::Cache));
    }
    authoritative().map(|entry| (summary_from(&entry), SummarySource::Authoritative))
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
        | RepositoryError::RenameConflict { .. }
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

        for use_cache in [true, false] {
            let before = reads.get();
            let (summary, source) = cached_or_authoritative_summary(
                use_cache.then_some(&registry),
                &slug,
                &root.path().join("scripts/fast/meta.toml"),
                || {
                    reads.set(reads.get() + 1);
                    store.read_entry(slug.clone())
                },
            )
            .unwrap();

            assert_eq!(summary.description, "from the cache");
            if use_cache {
                assert!(matches!(source, SummarySource::Cache));
                assert_eq!(reads.get(), before, "the fast path parsed meta.toml");
            } else {
                assert!(matches!(source, SummarySource::Authoritative));
                assert_eq!(
                    reads.get(),
                    before + 1,
                    "the fallback did not parse meta.toml"
                );
            }
        }
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
