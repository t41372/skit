use super::persistence::{acquire_lock, write_meta};
use super::{Entry, Error, Store};
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
            Some(ParamDecl::tables(decls))
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

trait ParamTables {
    fn tables(decls: &[ParamDecl]) -> Vec<toml::Table>;
}

impl ParamTables for ParamDecl {
    fn tables(decls: &[ParamDecl]) -> Vec<toml::Table> {
        decls.iter().map(ParamDecl::to_meta_table).collect()
    }
}
