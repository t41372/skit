use std::{
    fmt::Debug,
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use skit_domain::{Entry, EntryKind, EntrySettings, StorageMode, parameters::ParameterInvariant};
use skit_i18n::Message;

use crate::{LibraryService, RepositoryError};

/// Source-file permissions captured before an entry is added.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourcePermissions {
    /// Whether the source was read-only on platforms with a portable read-only bit.
    pub readonly: bool,
    /// Unix permission bits when available.
    pub unix_mode: Option<u32>,
}

/// Exact source bytes and their intended stored filename.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EntryPayload {
    /// Byte-exact source snapshot.
    pub bytes: Vec<u8>,
    /// Filename inside `scripts/<slug>`; absent for metadata-only entries.
    pub stored_name: Option<String>,
    /// Permissions captured from the source snapshot.
    pub permissions: SourcePermissions,
}

/// Frontend-neutral request for creating one library entry.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CreateEntry {
    /// User-facing display name.
    pub name: String,
    /// Open entry-kind registry key.
    pub kind: EntryKind,
    /// Copy or reference storage policy.
    pub mode: StorageMode,
    /// Original source path or command provenance.
    pub source: String,
    /// Work-directory policy spelling.
    pub workdir: String,
    /// User-facing description.
    pub description: String,
    /// Optional byte snapshot; reference mode hashes it but never stores a copy.
    pub payload: Option<EntryPayload>,
    /// Optional settings that must commit with the new entry.
    #[serde(default)]
    pub settings: EntrySettings,
}

/// One atomic update of entry metadata and an optional stored source.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateEntry {
    /// Replacement display name.
    pub name: String,
    /// Replacement description.
    pub description: String,
    /// Replacement optional settings.
    pub settings: EntrySettings,
    /// Replacement work-directory policy.
    pub workdir: String,
    /// Replacement source bytes for a copy entry.
    pub source: Option<Vec<u8>>,
    /// Source hash held when the update started.
    pub expected_source_hash: String,
}

/// Repository-owned claim for one copy source prepared for an external editor.
pub trait ExternalCopyEdit: Debug {
    /// Return the claimed entry incarnation.
    fn entry(&self) -> &Entry;

    /// Return the authoritative stored source path passed to the editor.
    fn path(&self) -> &Path;
}

/// Locked source snapshot and metadata produced by finalizing one external edit.
#[derive(Clone, Debug, PartialEq)]
pub struct FinalizedExternalCopyEdit {
    entry: Entry,
    bytes: Vec<u8>,
}

impl FinalizedExternalCopyEdit {
    /// Build one adapter result from the same locked source read used for its hash.
    #[must_use]
    pub const fn new(entry: Entry, bytes: Vec<u8>) -> Self {
        Self { entry, bytes }
    }

    /// Return the finalized entry metadata.
    #[must_use]
    pub const fn entry(&self) -> &Entry {
        &self.entry
    }

    /// Return the exact bytes read while the finalize lock was held.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Failure from finalizing an external edit.
#[derive(Debug)]
pub enum FinalizeExternalCopyEditError {
    /// Identity, metadata, confinement, or projection failed.
    Repository(RepositoryError),
    /// The authoritative edited source could not be read while the lock was held.
    Read {
        /// Source path passed to the editor.
        path: PathBuf,
        /// Operating-system read failure.
        source: io::Error,
    },
}

impl From<RepositoryError> for FinalizeExternalCopyEditError {
    fn from(error: RepositoryError) -> Self {
        Self::Repository(error)
    }
}

/// Failure from an update whose adapter preparation must finish before the entry can change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedEntryUpdateError<E> {
    /// The entry read, identity claim, name preflight, or final repository update failed.
    Repository(RepositoryError),
    /// The host preparation failed before the repository update started.
    Preparation(E),
}

/// Identity-gated mutation port shared by CLI, Ratatui, and future GUI adapters.
pub trait EntryMutationRepository: Debug {
    /// Repository-owned external-edit claim type.
    type ExternalEdit: ExternalCopyEdit;

    /// Create a new entry atomically.
    fn create(&self, request: CreateEntry) -> Result<Entry, RepositoryError>;

    /// Verify a held entry and stamp legacy metadata with an immutable identity.
    fn claim_identity(&self, entry: &Entry) -> Result<Entry, RepositoryError>;

    /// Verify identity and name availability before a fallible external preparation.
    fn preflight_update_entry(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError>;

    /// Replace an entry description while preserving its identity.
    fn describe(&self, entry: &Entry, description: &str) -> Result<Entry, RepositoryError>;

    /// Replace typed optional settings and the work-directory policy.
    fn update_settings(
        &self,
        entry: &Entry,
        settings: &EntrySettings,
        workdir: &str,
    ) -> Result<Entry, RepositoryError>;

    /// Replace metadata and an optional stored source in one transaction.
    fn update_entry(&self, entry: &Entry, update: UpdateEntry) -> Result<Entry, RepositoryError>;

    /// Rename an entry without changing its stable slug.
    fn rename(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError>;

    /// Remove exactly the held entry incarnation.
    fn remove(&self, entry: &Entry) -> Result<String, RepositoryError>;

    /// Commit a copy-mode edit only when identity and source version still match.
    fn commit_copy_edit(
        &self,
        entry: &Entry,
        bytes: &[u8],
        expected_source_hash: &str,
    ) -> Result<Entry, RepositoryError>;

    /// Claim one authoritative copy source before an external editor starts.
    fn prepare_external_copy_edit(
        &self,
        entry: &Entry,
    ) -> Result<Self::ExternalEdit, RepositoryError>;

    /// Finalize bytes written in place by an external editor without replacing those bytes.
    fn finalize_external_copy_edit(
        &self,
        edit: &Self::ExternalEdit,
    ) -> Result<FinalizedExternalCopyEdit, FinalizeExternalCopyEditError>;
}

impl<R> LibraryService<R>
where
    R: EntryMutationRepository,
{
    /// Create one entry through the mutation port.
    pub fn add(&self, request: CreateEntry) -> Result<Entry, RepositoryError> {
        validate_settings(&request.settings, &request.workdir)?;
        self.repository.create(request)
    }

    /// Claim one held entry before a user-paced operation.
    pub fn claim_identity(&self, entry: &Entry) -> Result<Entry, RepositoryError> {
        self.repository.claim_identity(entry)
    }

    /// Update a description through the identity-gated port.
    pub fn describe(&self, entry: &Entry, description: &str) -> Result<Entry, RepositoryError> {
        self.repository.describe(entry, description)
    }

    /// Update typed settings through the identity-gated port.
    pub fn update_settings(
        &self,
        entry: &Entry,
        settings: &EntrySettings,
        workdir: &str,
    ) -> Result<Entry, RepositoryError> {
        validate_settings(settings, workdir)?;
        self.repository.update_settings(entry, settings, workdir)
    }

    /// Update metadata and an optional stored source through one transaction boundary.
    pub fn update_entry(
        &self,
        entry: &Entry,
        update: UpdateEntry,
    ) -> Result<Entry, RepositoryError> {
        validate_settings(&update.settings, &update.workdir)?;
        self.repository.update_entry(entry, update)
    }

    /// Rename one held entry.
    pub fn rename(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError> {
        self.repository.rename(entry, name)
    }

    /// Remove one held entry.
    pub fn remove(&self, entry: &Entry) -> Result<String, RepositoryError> {
        self.repository.remove(entry)
    }

    /// Commit a staged copy edit through the double compare-and-swap boundary.
    pub fn commit_copy_edit(
        &self,
        entry: &Entry,
        bytes: &[u8],
        expected_source_hash: &str,
    ) -> Result<Entry, RepositoryError> {
        self.repository
            .commit_copy_edit(entry, bytes, expected_source_hash)
    }

    /// Claim one authoritative source before a user-paced external edit.
    pub fn prepare_external_copy_edit(
        &self,
        entry: &Entry,
    ) -> Result<R::ExternalEdit, RepositoryError> {
        self.repository.prepare_external_copy_edit(entry)
    }

    /// Finalize an in-place external edit through the identity-gated port.
    pub fn finalize_external_copy_edit(
        &self,
        edit: &R::ExternalEdit,
    ) -> Result<FinalizedExternalCopyEdit, FinalizeExternalCopyEditError> {
        self.repository.finalize_external_copy_edit(edit)
    }
}

impl<R> LibraryService<R>
where
    R: EntryMutationRepository,
{
    /// Claim and preflight an entry, then prepare one external adapter.
    ///
    /// A caller can stop after preparation when the requested external effect does not need a
    /// metadata rewrite. If it commits an update, the repository repeats its identity and name
    /// checks. The early checks keep a fallible preparation from changing external state when the
    /// request is already stale or its destination name is already taken.
    pub fn prepare_entry_update<E>(
        &self,
        entry: &Entry,
        update: &UpdateEntry,
        prepare: impl FnOnce(&Entry) -> Result<(), E>,
    ) -> Result<Entry, PreparedEntryUpdateError<E>> {
        validate_settings(&update.settings, &update.workdir)
            .map_err(PreparedEntryUpdateError::Repository)?;
        let claimed = self
            .repository
            .preflight_update_entry(entry, &update.name)
            .map_err(PreparedEntryUpdateError::Repository)?;
        prepare(&claimed).map_err(PreparedEntryUpdateError::Preparation)?;
        Ok(claimed)
    }

    /// Prepare one external adapter, then commit the entry update.
    pub fn update_entry_after_preparation<E>(
        &self,
        entry: &Entry,
        update: UpdateEntry,
        prepare: impl FnOnce(&Entry) -> Result<(), E>,
    ) -> Result<Entry, PreparedEntryUpdateError<E>> {
        let claimed = self.prepare_entry_update(entry, &update, prepare)?;
        self.repository
            .update_entry(&claimed, update)
            .map_err(PreparedEntryUpdateError::Repository)
    }
}

fn validate_settings(settings: &EntrySettings, workdir: &str) -> Result<(), RepositoryError> {
    if !matches!(workdir, "origin" | "store" | "invoke") && !Path::new(workdir).is_absolute() {
        return Err(RepositoryError::InvalidMutation {
            reason: Message::new(
                "the work directory must be origin, store, invoke, or an absolute path",
            ),
        });
    }
    for parameter in &settings.parameters {
        let Some(invariant) = parameter.validate() else {
            continue;
        };
        let reason = match invariant {
            ParameterInvariant::BindingDeliveryMismatch => {
                Message::new("parameter {} has a source binding that does not match its delivery")
                    .quoted(&parameter.name)
            }
            ParameterInvariant::ChoiceWithoutChoices => {
                Message::new("choice parameter {} has no choices").quoted(&parameter.name)
            }
        };
        return Err(RepositoryError::InvalidMutation { reason });
    }
    Ok(())
}
