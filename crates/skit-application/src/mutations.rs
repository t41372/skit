use std::fmt::Debug;

use serde::{Deserialize, Serialize};
use skit_domain::{Entry, EntryKind, StorageMode};

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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
}

/// Identity-gated mutation port shared by CLI, Ratatui, and future GUI adapters.
pub trait EntryMutationRepository: Debug {
    /// Create a new entry atomically.
    fn create(&self, request: CreateEntry) -> Result<Entry, RepositoryError>;

    /// Verify a held entry and stamp legacy metadata with an immutable identity.
    fn claim_identity(&self, entry: &Entry) -> Result<Entry, RepositoryError>;

    /// Replace an entry description while preserving its identity.
    fn describe(&self, entry: &Entry, description: &str) -> Result<Entry, RepositoryError>;

    /// Rename an entry and move it to the derived slug.
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
}

impl<R> LibraryService<R>
where
    R: EntryMutationRepository,
{
    /// Create one entry through the mutation port.
    pub fn add(&self, request: CreateEntry) -> Result<Entry, RepositoryError> {
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
}
