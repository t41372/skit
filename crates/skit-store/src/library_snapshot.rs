//! Read storage facts for the application-owned Library projection.

use std::{fs, path::PathBuf};

use skit_application::{
    RepositoryError,
    library_detail::{LibraryDetailRepository, LibraryEntrySnapshot, LibraryTargetState},
};
use skit_domain::{Entry, StorageMode};

use crate::{FileStore, paths::stored_filenames};

impl LibraryDetailRepository for FileStore {
    fn detail_snapshots(&self) -> Result<Vec<LibraryEntrySnapshot>, RepositoryError> {
        self.scan_entries()?
            .into_iter()
            .map(|entry| {
                let source = self
                    .payload_path(&entry)
                    .ok()
                    .and_then(|path| fs::read(path).ok());
                let target =
                    launch_target(self, &entry).map_or(LibraryTargetState::NotApplicable, |path| {
                        if path.exists() {
                            LibraryTargetState::Present
                        } else {
                            LibraryTargetState::Missing(path)
                        }
                    });
                let original_source_exists = !entry.meta.source.is_empty()
                    && std::path::Path::new(&entry.meta.source).exists();
                Ok(LibraryEntrySnapshot {
                    entry,
                    source,
                    target,
                    original_source_exists,
                })
            })
            .collect()
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
