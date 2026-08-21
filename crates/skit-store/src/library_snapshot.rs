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
    mutations::registry::Registry,
    paths::stored_filenames,
    read::{diagnostic_from, summary_from},
};

impl LibraryDetailRepository for FileStore {
    fn library_refresh(&self) -> Result<LibraryRefreshSnapshot, RepositoryError> {
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
            match self.read_entry_snapshot(slug.clone()) {
                Ok((entry, metadata_bytes)) => {
                    if !registry.matches_entry_snapshot(&entry, &meta_path, &metadata_bytes) {
                        stale.push(slug);
                    }
                    refresh.scan.entries.push(summary_from(&entry));
                    refresh
                        .entries
                        .push(entry_snapshot(self, entry, |path| fs::read(path).ok()));
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
    use std::cell::Cell;

    use skit_application::{
        CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions,
    };
    use skit_domain::{EntryKind, EntrySettings};
    use tempfile::TempDir;

    use super::*;

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
}
