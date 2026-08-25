//! Read storage facts for the application-owned Library projection.

use std::{fs, path::PathBuf};

use skit_application::{
    Diagnostic, DiagnosticCode, RepositoryError,
    library_detail::{
        LibraryDetailRepository, LibraryEntrySnapshot, LibraryRefreshSnapshot, LibraryTargetState,
    },
};
use skit_domain::{Entry, StorageMode};
use skit_i18n::Localize as _;

use crate::{
    FileStore,
    fs_ops::acquire_existing_lock,
    mutations::registry::Registry,
    paths::stored_filenames,
    read::{diagnostic_from, summary_from},
};

impl LibraryDetailRepository for FileStore {
    fn library_refresh(&self) -> Result<LibraryRefreshSnapshot, RepositoryError> {
        self.library_refresh_with(|_| {})
    }
}

impl FileStore {
    fn library_refresh_with(
        &self,
        mut before_source_read: impl FnMut(&Entry),
    ) -> Result<LibraryRefreshSnapshot, RepositoryError> {
        let Some(registry) = Registry::read(self.data_dir()) else {
            return Ok(LibraryRefreshSnapshot::default());
        };
        let mut refresh = LibraryRefreshSnapshot::default();
        let mut stale = Vec::new();
        for candidate in registry.row_keys() {
            let slug = match skit_domain::Slug::parse(candidate.clone()) {
                Ok(slug) => slug,
                Err(error) => {
                    refresh.scan.diagnostics.push(Diagnostic::from_message(
                        DiagnosticCode::InvalidSlug,
                        Some(candidate),
                        error.message(),
                    ));
                    continue;
                }
            };
            let meta_path = self.scripts_dir().join(slug.as_str()).join("meta.toml");
            match coherent_entry_snapshot(self, &slug, &mut before_source_read) {
                Ok((entry, metadata_bytes, snapshot)) => {
                    if !registry.matches_entry_snapshot(&entry, &meta_path, &metadata_bytes) {
                        stale.push(slug);
                    }
                    refresh.scan.entries.push(summary_from(&entry));
                    refresh.entries.push(snapshot);
                }
                Err(error) => refresh.scan.diagnostics.push(diagnostic_from(error, &slug)),
            }
        }
        if !stale.is_empty() {
            self.repair_rows(&stale);
        }
        Ok(refresh)
    }
}

fn coherent_entry_snapshot(
    store: &FileStore,
    slug: &skit_domain::Slug,
    before_source_read: &mut dyn FnMut(&Entry),
) -> Result<(Entry, Vec<u8>, LibraryEntrySnapshot), RepositoryError> {
    let lock_path = store
        .data_dir()
        .join(".locks")
        .join(format!("{}.meta.lock", slug.as_str()));
    if let Some(_lock) = existing_entry_lock(&lock_path)? {
        return read_entry_snapshot(store, slug, None);
    }

    let first = read_entry_snapshot(store, slug, Some(before_source_read))?;
    if let Some(_lock) = existing_entry_lock(&lock_path)? {
        return read_entry_snapshot(store, slug, None);
    }
    Ok(first)
}

fn existing_entry_lock(
    path: &std::path::Path,
) -> Result<Option<crate::fs_ops::FileLock>, RepositoryError> {
    acquire_existing_lock(path).map_err(|error| RepositoryError::Io {
        operation: "lock",
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

fn read_entry_snapshot(
    store: &FileStore,
    slug: &skit_domain::Slug,
    before_source_read: Option<&mut dyn FnMut(&Entry)>,
) -> Result<(Entry, Vec<u8>, LibraryEntrySnapshot), RepositoryError> {
    let (entry, metadata_bytes) = store.read_entry_snapshot(slug.clone())?;
    if let Some(before_source_read) = before_source_read {
        before_source_read(&entry);
    }
    let snapshot = entry_snapshot(store, entry.clone(), |path| fs::read(path).ok());
    Ok((entry, metadata_bytes, snapshot))
}

fn entry_snapshot(
    store: &FileStore,
    entry: Entry,
    read_source: impl FnOnce(&std::path::Path) -> Option<Vec<u8>>,
) -> LibraryEntrySnapshot {
    let source = store
        .payload_path(&entry)
        .ok()
        .and_then(|path| read_source(&path));
    let target = launch_target(store, &entry).map_or(LibraryTargetState::NotApplicable, |path| {
        if path.exists() {
            LibraryTargetState::Present
        } else {
            LibraryTargetState::Missing(path)
        }
    });
    let original_source_exists =
        !entry.meta.source.is_empty() && std::path::Path::new(&entry.meta.source).exists();
    LibraryEntrySnapshot {
        entry,
        source,
        target,
        original_source_exists,
    }
}

fn launch_target(store: &FileStore, entry: &Entry) -> Option<PathBuf> {
    let kind = entry.meta.kind.as_str();
    if !known_entry_kind(kind) || kind == "command" {
        return None;
    }
    if entry.meta.mode == StorageMode::Reference || kind == "exe" {
        return Some(if entry.meta.source.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::from(&entry.meta.source)
        });
    }
    let directory = store.entry_dir_path(&entry.slug);
    let names = stored_filenames(kind);
    let canonical = names
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.exists())
        .or_else(|| names.first().map(|name| directory.join(name)))?;
    if canonical.exists() {
        return Some(canonical);
    }
    store.payload_path(entry).ok().or(Some(canonical))
}

const fn known_entry_kind(kind: &str) -> bool {
    matches!(
        kind.as_bytes(),
        b"python"
            | b"shell"
            | b"fish"
            | b"js"
            | b"ts"
            | b"powershell"
            | b"ruby"
            | b"perl"
            | b"lua"
            | b"r"
            | b"exe"
            | b"command"
            | b"prompt"
    )
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        sync::{Arc, Barrier},
        thread,
    };

    use skit_application::{
        CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions,
    };
    use skit_domain::{EntryKind, EntrySettings};
    use tempfile::TempDir;

    use super::*;

    fn request(name: &str) -> CreateEntry {
        CreateEntry {
            name: name.to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: format!("/original/{name}.sh"),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"printf stored\n".to_vec(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        }
    }

    #[test]
    fn refresh_reports_an_invalid_registry_slug_without_hiding_valid_entries() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store.create(request("Valid")).unwrap();
        let registry_path = root.path().join("registry.toml");
        let mut document =
            toml::from_str::<toml::Table>(&fs::read_to_string(&registry_path).unwrap()).unwrap();
        let entries = document["entries"].as_table_mut().unwrap();
        let row = entries.get(entry.slug.as_str()).unwrap().clone();
        entries.insert("not a slug".to_owned(), row);
        fs::write(&registry_path, toml::to_string_pretty(&document).unwrap()).unwrap();

        let refresh = store.library_refresh().unwrap();

        assert_eq!(refresh.entries.len(), 1);
        assert_eq!(refresh.entries[0].entry.slug, entry.slug);
        assert_eq!(refresh.scan.diagnostics.len(), 1);
        assert_eq!(
            refresh.scan.diagnostics[0].code,
            DiagnosticCode::InvalidSlug
        );
        assert_eq!(
            refresh.scan.diagnostics[0].slug.as_deref(),
            Some("not a slug")
        );
    }

    #[test]
    fn refresh_keeps_a_real_existing_lock_open_error_typed() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store.create(request("Locked")).unwrap();
        let lock_path = root
            .path()
            .join(".locks")
            .join(format!("{}.meta.lock", entry.slug.as_str()));
        assert!(!lock_path.exists());
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        fs::create_dir(&lock_path).unwrap();

        let refresh = store.library_refresh().unwrap();

        #[cfg(unix)]
        {
            assert!(refresh.entries.is_empty());
            assert_eq!(refresh.scan.diagnostics.len(), 1);
            assert_eq!(refresh.scan.diagnostics[0].code, DiagnosticCode::Io);
            assert_eq!(refresh.scan.diagnostics[0].slug.as_deref(), Some("locked"));
            assert!(
                refresh.scan.diagnostics[0]
                    .message
                    .contains(&lock_path.display().to_string())
            );
        }
        #[cfg(windows)]
        {
            assert_eq!(refresh.entries.len(), 1);
            assert!(refresh.scan.diagnostics.is_empty());
        }
    }

    #[test]
    fn one_entry_snapshot_reads_the_payload_once_and_keeps_its_bytes_exact() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(CreateEntry {
                name: "Exact".to_owned(),
                kind: EntryKind::parse("shell").unwrap(),
                mode: StorageMode::Copy,
                source: "/original/exact.sh".to_owned(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"stored".to_vec(),
                    stored_name: Some("script.sh".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
        let reads = Cell::new(0);
        let expected = b"\xff\r\n";

        let snapshot = entry_snapshot(&store, entry, |_| {
            reads.set(reads.get() + 1);
            Some(expected.to_vec())
        });

        assert_eq!(reads.get(), 1);
        assert_eq!(snapshot.source.as_deref(), Some(expected.as_slice()));
    }

    #[test]
    fn refresh_never_pairs_old_metadata_with_a_concurrently_committed_new_payload() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let old_source = b"printf old\n";
        let new_source = b"printf new\n";
        let entry = store
            .create(CreateEntry {
                name: "Coherent".to_owned(),
                kind: EntryKind::parse("shell").unwrap(),
                mode: StorageMode::Copy,
                source: "/original/coherent.sh".to_owned(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: old_source.to_vec(),
                    stored_name: Some("script.sh".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
        let writer_start = Arc::new(Barrier::new(2));
        let writer_done = Arc::new(Barrier::new(2));
        let writer = {
            let store = store.clone();
            let entry = entry.clone();
            let writer_start = Arc::clone(&writer_start);
            let writer_done = Arc::clone(&writer_done);
            thread::spawn(move || {
                writer_start.wait();
                let updated = store
                    .commit_copy_edit(&entry, new_source, &entry.meta.source_hash)
                    .unwrap();
                writer_done.wait();
                updated
            })
        };
        let mut first_read = true;

        let refresh = store
            .library_refresh_with(|_| {
                if first_read {
                    first_read = false;
                    writer_start.wait();
                    writer_done.wait();
                }
            })
            .unwrap();
        let updated = writer.join().unwrap();
        let snapshot = refresh.entries.first().unwrap();
        let source = snapshot.source.as_deref().unwrap();

        assert!(source == old_source || source == new_source);
        assert_eq!(snapshot.entry.meta.source_hash, crate::content_hash(source));
        assert!(
            (source == old_source && snapshot.entry.meta.source_hash == entry.meta.source_hash)
                || (source == new_source
                    && snapshot.entry.meta.source_hash == updated.meta.source_hash)
        );

        let stable = store.library_refresh().unwrap();
        let stable = stable.entries.first().unwrap();
        assert_eq!(stable.source.as_deref(), Some(new_source.as_slice()));
        assert_eq!(stable.entry.meta.source_hash, updated.meta.source_hash);
    }
}
