mod model;
mod persistence;

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use model::StateFile;
pub use model::{Entry, EntryDraft, EntrySummary, Error, LibraryRoots, RunStamp, ScriptMeta};
use persistence::{
    acquire_lock, insert_registry_row, load_registry_document, load_registry_for_insert, read_meta,
    registry_contains_slug, remove_registry_row, set_registry_row, slugify, unique_slug,
    write_meta, write_registry_document,
};

use crate::stored_name;

/// A filesystem view of the skit library.
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

    /// List valid entries without changing registry or metadata files.
    ///
    /// Corrupt metadata is skipped. One damaged entry must not hide healthy entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the scripts directory cannot be enumerated for a reason
    /// other than it not existing.
    pub fn list(&self) -> Result<Vec<EntrySummary>, Error> {
        let scripts_dir = self.roots.data_dir().join("scripts");
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

            let Ok(meta) = read_meta(&entry_dir.join("meta.toml")) else {
                continue;
            };
            entries.push(EntrySummary {
                slug: item.file_name().to_string_lossy().into_owned(),
                name: meta.name,
                kind: meta.kind,
                mode: meta.mode,
                description: meta.description,
                source: meta.source,
                dir: entry_dir,
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
    /// Returns an error when matching metadata cannot be read or parsed, or when no
    /// matching entry exists.
    pub fn resolve(&self, query: &str) -> Result<Entry, Error> {
        let scripts_dir = self.roots.data_dir().join("scripts");
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
        let path = self
            .roots
            .state_dir()
            .join("values")
            .join(format!("{slug}.toml"));
        let text = fs::read_to_string(path).ok()?;
        toml::from_str::<StateFile>(&text).ok()?.last_run
    }

    /// Insert one complete entry under the registry lock.
    ///
    /// The caller prepares language-specific metadata before this transaction. This method
    /// owns slug allocation, filesystem collision checks, payload materialization, metadata,
    /// and the registry projection. If a write fails after the entry directory is created,
    /// the incomplete directory is removed before the error is returned.
    ///
    /// # Errors
    ///
    /// Returns an error for a duplicate display name, an unreadable registry, or a filesystem
    /// or TOML write failure.
    pub fn insert_entry(&self, draft: EntryDraft) -> Result<Entry, Error> {
        let _registry_lock = acquire_lock(&self.registry_lock_path())?;
        let mut registry = load_registry_for_insert(&self.registry_path())?;
        let (taken_slugs, taken_names) = self.filesystem_truth(&registry)?;
        if taken_names.contains(&draft.meta.name) {
            return Err(Error::NameConflict {
                name: draft.meta.name,
            });
        }

        let slug = unique_slug(&slugify(&draft.meta.name), &taken_slugs);
        let entry_dir = self.roots.data_dir().join("scripts").join(&slug);
        fs::create_dir_all(&entry_dir).map_err(|source| Error::Io {
            path: entry_dir.clone(),
            source,
        })?;

        let result = (|| {
            if let Some(payload) = &draft.payload {
                let payload_path = entry_dir.join(stored_name(&draft.meta.kind));
                write_payload(
                    &payload_path,
                    payload,
                    draft.payload_readonly,
                    draft.payload_unix_mode,
                )?;
            }
            write_meta(&entry_dir.join("meta.toml"), &draft.meta)?;
            let entry = Entry {
                slug: slug.clone(),
                meta: draft.meta,
                dir: entry_dir.clone(),
            };
            insert_registry_row(&mut registry, &entry)?;
            write_registry_document(&self.registry_path(), &registry)?;
            Ok(entry)
        })();

        if result.is_err() {
            let _ = fs::remove_dir_all(&entry_dir);
        }
        result
    }

    /// Change an entry description without changing any other metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be resolved, locked, encoded, or written.
    pub fn update_description(&self, query: &str, description: &str) -> Result<Entry, Error> {
        let initial = self.resolve(query)?;
        let _entry_lock = acquire_lock(&self.entry_lock_path(&initial.slug))?;
        let mut entry = self.resolve(&initial.slug)?;
        entry.meta.description = description.trim().to_owned();
        write_meta(&entry.dir.join("meta.toml"), &entry.meta)?;
        self.sync_registry_row(&entry)?;
        Ok(entry)
    }

    /// Change an entry display name. The slug and state path do not change.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or duplicate name, or when metadata cannot be
    /// locked, encoded, or written.
    pub fn rename(&self, query: &str, new_name: &str) -> Result<Entry, Error> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(Error::InvalidName);
        }

        let initial = self.resolve(query)?;
        let _entry_lock = acquire_lock(&self.entry_lock_path(&initial.slug))?;
        let mut entry = self.resolve(&initial.slug)?;
        let _registry_lock = acquire_lock(&self.registry_lock_path())?;

        if self
            .list()?
            .iter()
            .any(|other| other.slug != entry.slug && other.name == new_name)
        {
            return Err(Error::NameConflict {
                name: new_name.to_owned(),
            });
        }

        entry.meta.name = new_name.to_owned();
        write_meta(&entry.dir.join("meta.toml"), &entry.meta)?;
        if let Some(mut document) = load_registry_document(&self.registry_path())?
            && registry_contains_slug(&document, &entry.slug)
        {
            set_registry_row(&mut document, &entry)?;
            write_registry_document(&self.registry_path(), &document)?;
        }
        Ok(entry)
    }

    /// Remove an entry and its remembered values.
    ///
    /// A reference entry stores only metadata in the library. Its original source is
    /// outside the entry directory and is never removed here.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be resolved, locked, or removed.
    pub fn remove(&self, query: &str) -> Result<String, Error> {
        let initial = self.resolve(query)?;
        let _entry_lock = acquire_lock(&self.entry_lock_path(&initial.slug))?;
        let _registry_lock = acquire_lock(&self.registry_lock_path())?;
        let entry = self.resolve(&initial.slug)?;
        let removed_name = entry.meta.name.clone();
        let mut registry = load_registry_document(&self.registry_path())?;

        fs::remove_dir_all(&entry.dir).map_err(|source| Error::Io {
            path: entry.dir.clone(),
            source,
        })?;
        let state_path = self
            .roots
            .state_dir()
            .join("values")
            .join(format!("{}.toml", entry.slug));
        match fs::remove_file(&state_path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::Io {
                    path: state_path,
                    source,
                });
            }
        }

        if let Some(document) = &mut registry
            && remove_registry_row(document, &entry.slug)
        {
            write_registry_document(&self.registry_path(), document)?;
        }
        Ok(removed_name)
    }

    fn filesystem_truth(
        &self,
        registry: &toml::Table,
    ) -> Result<(BTreeSet<String>, BTreeSet<String>), Error> {
        let entries = registry.get("entries").and_then(toml::Value::as_table);
        let mut slugs = BTreeSet::new();
        let mut names = BTreeSet::new();
        if let Some(entries) = entries {
            for (slug, row) in entries {
                slugs.insert(slug.clone());
                if let Some(name) = row
                    .as_table()
                    .and_then(|row| row.get("name"))
                    .and_then(toml::Value::as_str)
                {
                    names.insert(name.to_owned());
                }
            }
        }

        let scripts_dir = self.roots.data_dir().join("scripts");
        let directory = match fs::read_dir(&scripts_dir) {
            Ok(directory) => directory,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok((slugs, names)),
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
            let file_type = item.file_type().map_err(|source| Error::Io {
                path: entry_dir.clone(),
                source,
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let slug = item.file_name().to_string_lossy().into_owned();
            let registered = entries.is_some_and(|entries| entries.contains_key(&slug));
            if !registered {
                let mut children = fs::read_dir(&entry_dir).map_err(|source| Error::Io {
                    path: entry_dir.clone(),
                    source,
                })?;
                if children.next().is_none() {
                    continue;
                }
            }
            slugs.insert(slug.clone());
            if registered
                && entries
                    .and_then(|entries| entries.get(&slug))
                    .and_then(toml::Value::as_table)
                    .and_then(|row| row.get("name"))
                    .and_then(toml::Value::as_str)
                    .is_some()
            {
                continue;
            }
            if let Ok(meta) = read_meta(&entry_dir.join("meta.toml")) {
                names.insert(meta.name);
            }
        }
        Ok((slugs, names))
    }

    fn entry_lock_path(&self, slug: &str) -> PathBuf {
        self.roots
            .data_dir()
            .join(".locks")
            .join(format!("{slug}.meta.lock"))
    }

    fn registry_path(&self) -> PathBuf {
        self.roots.data_dir().join("registry.toml")
    }

    fn registry_lock_path(&self) -> PathBuf {
        self.roots.data_dir().join("registry.native.lock")
    }

    fn sync_registry_row(&self, entry: &Entry) -> Result<(), Error> {
        if !self.registry_path().is_file() {
            return Ok(());
        }
        let _registry_lock = acquire_lock(&self.registry_lock_path())?;
        let Some(mut document) = load_registry_document(&self.registry_path())? else {
            return Ok(());
        };
        if !registry_contains_slug(&document, &entry.slug) {
            return Ok(());
        }
        set_registry_row(&mut document, entry)?;
        write_registry_document(&self.registry_path(), &document)
    }
}

#[cfg(unix)]
fn write_payload(
    path: &Path,
    payload: &[u8],
    readonly: bool,
    unix_mode: Option<u32>,
) -> Result<(), Error> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let requested_mode = unix_mode.unwrap_or(0o666);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true).mode(requested_mode);
    let mut file = options.open(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    file.write_all(payload).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    if let Some(mode) = unix_mode {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
            Error::Io {
                path: path.to_owned(),
                source,
            }
        })?;
    } else if readonly {
        let mut permissions = file
            .metadata()
            .map_err(|source| Error::Io {
                path: path.to_owned(),
                source,
            })?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn write_payload(
    path: &Path,
    payload: &[u8],
    readonly: bool,
    _unix_mode: Option<u32>,
) -> Result<(), Error> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(payload).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    if readonly {
        let mut permissions = file
            .metadata()
            .map_err(|source| Error::Io {
                path: path.to_owned(),
                source,
            })?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    }
    Ok(())
}
