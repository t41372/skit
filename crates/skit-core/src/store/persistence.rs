use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::UNIX_EPOCH;

use atomic_write_file::AtomicWriteFile;

use super::{Entry, Error, ScriptMeta, Store};
use crate::{
    DeclaredEditResult, DeclaredEdits, ParamDecl, declared_from_meta, edit_declared,
};

impl Store {
    /// Read valid declared parameter rows for one entry.
    ///
    /// Hand-edited malformed rows degrade through `declared_from_meta`; resolving the
    /// entry itself remains fallible so a missing/corrupt entry is still named.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be resolved.
    pub fn read_parameters(&self, query: &str) -> Result<Vec<ParamDecl>, Error> {
        let entry = self.resolve(query)?;
        Ok(parameter_decls(&entry))
    }

    /// Replace one entry's declared schema without changing placeholder caches or
    /// unrelated metadata. The write is serialized by the entry lock and uses the same
    /// metadata writer and registry projection as the other store mutations.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be resolved, locked, encoded, or written.
    pub fn write_parameters(&self, query: &str, decls: &[ParamDecl]) -> Result<Entry, Error> {
        let initial = self.resolve(query)?;
        let _entry_lock = acquire_lock(&self.entry_lock_path(&initial.slug))?;
        let entry = self.resolve(&initial.slug)?;
        self.write_parameters_locked(entry, decls)
    }

    /// Apply pure declared-schema edits to the latest schema under the entry lock.
    ///
    /// This closes the read/edit/write lost-update window between concurrent CLI, TUI,
    /// or future GUI mutations: edits are applied only after the lock is held and the
    /// entry has been re-read.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be resolved, locked, encoded, or written.
    pub fn edit_parameters(
        &self,
        query: &str,
        edits: &DeclaredEdits,
    ) -> Result<(Entry, DeclaredEditResult), Error> {
        let initial = self.resolve(query)?;
        let _entry_lock = acquire_lock(&self.entry_lock_path(&initial.slug))?;
        let entry = self.resolve(&initial.slug)?;
        let result = edit_declared(&parameter_decls(&entry), edits);
        let updated = self.write_parameters_locked(entry, &result.decls)?;
        Ok((updated, result))
    }

    fn write_parameters_locked(
        &self,
        mut entry: Entry,
        decls: &[ParamDecl],
    ) -> Result<Entry, Error> {
        entry.meta.parameters = if decls.is_empty() {
            None
        } else {
            Some(decls.iter().map(ParamDecl::to_meta_table).collect())
        };
        write_meta(&entry.dir.join("meta.toml"), &entry.meta)?;
        self.sync_registry_row(&entry)?;
        Ok(entry)
    }
}

fn parameter_decls(entry: &Entry) -> Vec<ParamDecl> {
    entry
        .meta
        .parameters
        .as_deref()
        .map(declared_from_meta)
        .unwrap_or_default()
}

pub(super) fn acquire_lock(path: &Path) -> Result<File, Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    file.lock().map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(file)
}

pub(super) fn read_meta(path: &Path) -> Result<ScriptMeta, Error> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| Error::InvalidMeta {
        path: path.to_owned(),
        source,
    })
}

pub(super) fn write_meta(path: &Path, meta: &ScriptMeta) -> Result<(), Error> {
    let text = toml::to_string(meta).map_err(|source| Error::EncodeToml {
        path: path.to_owned(),
        source,
    })?;
    atomic_write(path, &text)
}

pub(super) fn load_registry_for_insert(path: &Path) -> Result<toml::Table, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(empty_registry());
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    match toml::from_str::<toml::Table>(&text) {
        Ok(mut document) => {
            normalize_registry_entries(&mut document);
            Ok(document)
        }
        Err(_) => {
            let backup = path.with_file_name("registry.toml.corrupt");
            fs::rename(path, &backup).map_err(|source| Error::Io {
                path: path.to_owned(),
                source,
            })?;
            Ok(empty_registry())
        }
    }
}

pub(super) fn insert_registry_row(document: &mut toml::Table, entry: &Entry) -> Result<(), Error> {
    normalize_registry_entries(document);
    if let Some(entries) = document
        .get_mut("entries")
        .and_then(toml::Value::as_table_mut)
    {
        entries.insert(entry.slug.clone(), registry_row(entry)?);
    }
    Ok(())
}

pub(super) fn load_registry_document(path: &Path) -> Result<Option<toml::Table>, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    Ok(toml::from_str(&text).ok())
}

pub(super) fn registry_contains_slug(document: &toml::Table, slug: &str) -> bool {
    document
        .get("entries")
        .and_then(toml::Value::as_table)
        .is_some_and(|entries| entries.contains_key(slug))
}

pub(super) fn set_registry_row(document: &mut toml::Table, entry: &Entry) -> Result<(), Error> {
    let Some(entries) = document
        .get_mut("entries")
        .and_then(toml::Value::as_table_mut)
    else {
        return Ok(());
    };
    entries.insert(entry.slug.clone(), registry_row(entry)?);
    Ok(())
}

pub(super) fn remove_registry_row(document: &mut toml::Table, slug: &str) -> bool {
    document
        .get_mut("entries")
        .and_then(toml::Value::as_table_mut)
        .and_then(|entries| entries.remove(slug))
        .is_some()
}

pub(super) fn write_registry_document(path: &Path, document: &toml::Table) -> Result<(), Error> {
    let text = toml::to_string(document).map_err(|source| Error::EncodeToml {
        path: path.to_owned(),
        source,
    })?;
    atomic_write(path, &text)
}

pub(super) fn slugify(name: &str) -> String {
    let mut output = String::new();
    let mut previous_dash = false;
    for character in name.trim().to_lowercase().chars() {
        if character.is_alphanumeric() {
            output.push(character);
            previous_dash = false;
        } else if !previous_dash && !output.is_empty() {
            output.push('-');
            previous_dash = true;
        }
    }
    let slug = output.trim_matches('-');
    if slug.is_empty() {
        "script".to_owned()
    } else {
        slug.to_owned()
    }
}

pub(super) fn unique_slug(base: &str, existing: &BTreeSet<String>) -> String {
    if !existing.contains(base) {
        return base.to_owned();
    }
    let mut suffix = 2_u64;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn registry_row(entry: &Entry) -> Result<toml::Value, Error> {
    let meta_path = entry.dir.join("meta.toml");
    let metadata = fs::metadata(&meta_path).map_err(|source| Error::Io {
        path: meta_path.clone(),
        source,
    })?;
    let modified = metadata.modified().map_err(|source| Error::Io {
        path: meta_path.clone(),
        source,
    })?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::Io {
            path: meta_path.clone(),
            source: io::Error::other(error),
        })?;
    let mtime_ns = i64::try_from(duration.as_nanos()).map_err(|error| Error::Io {
        path: meta_path,
        source: io::Error::other(error),
    })?;

    let mut row = toml::Table::new();
    row.insert(
        "name".to_owned(),
        toml::Value::String(entry.meta.name.clone()),
    );
    row.insert(
        "kind".to_owned(),
        toml::Value::String(entry.meta.kind.clone()),
    );
    row.insert(
        "mode".to_owned(),
        toml::Value::String(entry.meta.mode.clone()),
    );
    row.insert(
        "description".to_owned(),
        toml::Value::String(entry.meta.description.clone()),
    );
    row.insert("mtime_ns".to_owned(), toml::Value::Integer(mtime_ns));
    if entry.meta.mode == "reference" {
        row.insert(
            "target".to_owned(),
            toml::Value::String(entry.meta.source.clone()),
        );
    }
    Ok(toml::Value::Table(row))
}

fn empty_registry() -> toml::Table {
    let mut document = toml::Table::new();
    document.insert("entries".to_owned(), toml::Value::Table(toml::Table::new()));
    document
}

fn normalize_registry_entries(document: &mut toml::Table) {
    if !document.get("entries").is_some_and(toml::Value::is_table) {
        document.insert("entries".to_owned(), toml::Value::Table(toml::Table::new()));
    }
}

fn atomic_write(path: &Path, text: &str) -> Result<(), Error> {
    let mut file = AtomicWriteFile::open(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    file.write_all(text.as_bytes())
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    file.commit().map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}
