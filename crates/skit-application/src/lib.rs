//! UI-independent use cases and ports for skit.

#![forbid(unsafe_code)]

pub mod delivery;
pub mod form_state;
mod mutations;
pub mod tokens;

use std::fmt::Debug;

pub use mutations::{CreateEntry, EntryMutationRepository, EntryPayload, SourcePermissions};
use serde::{Deserialize, Serialize};
use skit_domain::{Entry, EntrySummary};
use thiserror::Error;

/// Stable non-child exit classifications used by every frontend.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum ExitClass {
    /// Command-line shape or deterministic selection failure.
    Usage = 2,
    /// skit-side operational or data failure.
    Skit = 125,
    /// A target exists but cannot be executed or resolved.
    NotExecutable = 126,
    /// The requested library entry or launch target is missing.
    NotFound = 127,
    /// An interactive operation was cancelled.
    Aborted = 130,
}

impl ExitClass {
    /// Return the process exit code.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// A machine-stable diagnostic category produced during a best-effort library scan.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    /// An entry directory name is not a canonical slug.
    InvalidSlug,
    /// An entry's `meta.toml` could not be read or validated.
    CorruptMetadata,
    /// A filesystem operation failed while scanning one entry.
    Io,
}

/// A non-fatal problem isolated to one entry during listing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    /// Machine-readable category.
    pub code: DiagnosticCode,
    /// Entry address when it could be identified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Human-readable detail for logs and terminal output.
    pub message: String,
}

/// A library listing plus per-entry diagnostics.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibraryScan {
    /// Valid entries.
    pub entries: Vec<EntrySummary>,
    /// Problems that did not justify hiding valid siblings.
    pub diagnostics: Vec<Diagnostic>,
}

/// Failures from an entry repository port.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RepositoryError {
    /// No entry matched the selector.
    #[error("entry not found: {query}")]
    NotFound {
        /// Selector supplied by the caller.
        query: String,
    },
    /// More than one display name matched, so choosing would require a guess.
    #[error("entry name {query:?} is ambiguous; use one of these slugs: {candidates:?}")]
    Ambiguous {
        /// Ambiguous display name.
        query: String,
        /// Deterministic candidate slug list.
        candidates: Vec<String>,
    },
    /// A create or rename would collide with an existing entry.
    #[error("entry {name:?} already exists at slug {slug:?}")]
    Conflict {
        /// Requested display name.
        name: String,
        /// Conflicting stable address.
        slug: String,
    },
    /// A requested mutation could not satisfy the storage contract.
    #[error("invalid entry mutation: {reason}")]
    InvalidMutation {
        /// Stable, user-facing refusal detail.
        reason: String,
    },
    /// A held entry now resolves to a different incarnation.
    #[error("entry {slug:?} changed while this operation was underway")]
    StaleEntry {
        /// Address that changed owners.
        slug: String,
    },
    /// A staged copy edit no longer matches the source version it started from.
    #[error(
        "entry {slug:?} source changed while this edit was underway (expected {expected}, found {actual})"
    )]
    SourceChanged {
        /// Address of the edited entry.
        slug: String,
        /// Source digest captured when editing started.
        expected: String,
        /// Digest of the current stored bytes.
        actual: String,
    },
    /// A selected entry exists but its authoritative metadata is corrupt.
    #[error("entry {slug:?} has corrupt metadata: {reason}")]
    Corrupt {
        /// Entry address.
        slug: String,
        /// Parser or validation detail.
        reason: String,
    },
    /// A filesystem operation failed.
    #[error("could not {operation} {path}: {reason}")]
    Io {
        /// Operation name, such as `read` or `scan`.
        operation: &'static str,
        /// Affected path.
        path: String,
        /// Operating-system detail.
        reason: String,
    },
}

impl RepositoryError {
    /// Map a repository failure to the process contract without consulting localized text.
    #[must_use]
    pub const fn exit_class(&self) -> ExitClass {
        match self {
            Self::NotFound { .. } => ExitClass::NotFound,
            Self::Ambiguous { .. } | Self::Conflict { .. } | Self::InvalidMutation { .. } => {
                ExitClass::Usage
            }
            Self::StaleEntry { .. }
            | Self::SourceChanged { .. }
            | Self::Corrupt { .. }
            | Self::Io { .. } => ExitClass::Skit,
        }
    }
}

/// Read-side persistence port shared by every frontend.
pub trait EntryRepository: Debug {
    /// Scan every entry, isolating per-entry corruption as diagnostics.
    fn scan(&self) -> Result<LibraryScan, RepositoryError>;

    /// Resolve an exact slug first, then an exact display name.
    fn resolve(&self, query: &str) -> Result<Entry, RepositoryError>;
}

/// Application use cases shared by CLI, Ratatui, and the future Tauri adapter.
#[derive(Debug)]
pub struct LibraryService<R> {
    repository: R,
}

impl<R> LibraryService<R>
where
    R: EntryRepository,
{
    /// Construct the service around a concrete repository adapter.
    #[must_use]
    pub const fn new(repository: R) -> Self {
        Self { repository }
    }

    /// List entries in a deterministic, locale-independent order.
    pub fn list(&self) -> Result<LibraryScan, RepositoryError> {
        let mut scan = self.repository.scan()?;
        scan.entries.sort_by_cached_key(|entry| {
            (entry.name.to_lowercase(), entry.slug.as_str().to_owned())
        });
        scan.diagnostics.sort_by(|left, right| {
            left.slug
                .as_deref()
                .unwrap_or_default()
                .cmp(right.slug.as_deref().unwrap_or_default())
                .then_with(|| left.message.cmp(&right.message))
        });
        Ok(scan)
    }

    /// Resolve one entry without adding frontend policy.
    pub fn show(&self, query: &str) -> Result<Entry, RepositoryError> {
        self.repository.resolve(query)
    }

    /// Expose the adapter for composition-level inspection and focused tests.
    #[must_use]
    pub const fn repository(&self) -> &R {
        &self.repository
    }
}
