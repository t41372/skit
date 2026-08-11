//! UI-independent use cases and ports for skit.

#![forbid(unsafe_code)]

mod agent_skill;
pub mod delivery;
pub mod form_feedback;
pub mod form_state;
pub mod glob_expansion;
pub mod health;
pub mod library_detail;
mod mutations;
pub mod parameter_edit;
pub mod path_insertion;
mod payload_policy;
pub mod preferences;
pub mod prompt_selection;
pub mod run_inputs;
pub mod runner_management;
pub mod tokens;
pub mod value_preparation;
pub mod value_resolution;

use std::fmt::Debug;

pub use agent_skill::{
    AgentInstallError, AgentInstallPlan, AgentInstallRequest, AgentRoots, AgentScope, AgentTarget,
    detect_agent_targets, plan_agent_install,
};
pub use mutations::{
    CreateEntry, EntryMutationRepository, EntryPayload, SourcePermissions, UpdateEntry,
};
pub use payload_policy::{
    add_workdir, canonical_stored_filename, payload_stored_name, supports_storage_modes,
};
use serde::{Deserialize, Serialize};
use skit_domain::{Entry, EntrySummary};
use skit_i18n::{Locale, Localize, Message};
use thiserror::Error;

/// Stable non-child exit classifications used by every frontend.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum ExitClass {
    /// A requested operation failed without a child-process status.
    Failure = 1,
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

/// The use-case boundary that interprets a repository failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryOperation {
    /// Inspect or mutate the library without launching an entry.
    Manage,
    /// Resolve an entry as the target of `run`.
    Launch,
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
    /// Typed presentation detail. Machine serialization keeps `message` in English.
    #[serde(skip)]
    presentation: Option<Message>,
}

impl Diagnostic {
    /// Create a diagnostic from a typed user-visible message.
    #[must_use]
    pub fn from_message(code: DiagnosticCode, slug: Option<String>, presentation: Message) -> Self {
        Self {
            code,
            slug,
            message: presentation.localize(Locale::En),
            presentation: Some(presentation),
        }
    }

    /// Create a diagnostic whose detail is adapter-owned plain text.
    #[must_use]
    pub const fn plain(code: DiagnosticCode, slug: Option<String>, message: String) -> Self {
        Self {
            code,
            slug,
            message,
            presentation: None,
        }
    }

    /// Return the human-readable detail in the selected locale.
    #[must_use]
    pub fn localize(&self, locale: Locale) -> String {
        self.presentation
            .as_ref()
            .map_or_else(|| self.message.clone(), |message| message.localize(locale))
    }
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
        reason: Message,
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
    /// A primary write and its recovery both failed.
    #[error("rollback at {path} failed after {primary}: {rollback}")]
    Rollback {
        /// Affected path.
        path: String,
        /// Failure that started recovery.
        primary: Box<Self>,
        /// Failure from the recovery attempt.
        rollback: Box<Self>,
    },
    /// Membership was removed, but some entry files remain for an explicit recovery.
    #[error(
        "{name} was removed from the library, but its files could not be fully deleted: {path}"
    )]
    RemovalIncomplete {
        /// Display name of the removed entry.
        name: String,
        /// Entry directory that remains available to delete or rebuild.
        path: String,
    },
}

impl RepositoryError {
    /// Map a repository failure to the v0.4 process contract for one use case.
    #[must_use]
    pub const fn exit_class(&self, operation: RepositoryOperation) -> ExitClass {
        match (operation, self) {
            (RepositoryOperation::Launch, Self::NotFound { .. }) => ExitClass::NotFound,
            (
                RepositoryOperation::Launch,
                Self::Ambiguous { .. } | Self::Conflict { .. } | Self::InvalidMutation { .. },
            ) => ExitClass::Usage,
            (
                RepositoryOperation::Launch,
                Self::StaleEntry { .. }
                | Self::SourceChanged { .. }
                | Self::Corrupt { .. }
                | Self::Io { .. }
                | Self::Rollback { .. }
                | Self::RemovalIncomplete { .. },
            ) => ExitClass::Skit,
            (RepositoryOperation::Manage, Self::InvalidMutation { .. }) => ExitClass::Usage,
            (RepositoryOperation::Manage, _) => ExitClass::Failure,
        }
    }
}

impl Localize for RepositoryError {
    fn message(&self) -> Message {
        match self {
            Self::NotFound { query } => Message::new("entry not found: {}").with(query),
            Self::Ambiguous { query, candidates } => {
                Message::new("entry name {} is ambiguous; use one of these slugs: {}")
                    .quoted(query)
                    .with(format!("{candidates:?}"))
            }
            Self::Conflict { name, slug } => Message::new("entry {} already exists at slug {}")
                .quoted(name)
                .quoted(slug),
            Self::InvalidMutation { reason } => {
                Message::new("invalid entry mutation: {}").nested(reason.clone())
            }
            Self::StaleEntry { slug } => {
                Message::new("entry {} changed while this operation was underway").quoted(slug)
            }
            Self::SourceChanged {
                slug,
                expected,
                actual,
            } => Message::new(
                "entry {} source changed while this edit was underway (expected {}, found {})",
            )
            .quoted(slug)
            .with(expected)
            .with(actual),
            Self::Corrupt { slug, reason } => Message::new("entry {} has corrupt metadata: {}")
                .quoted(slug)
                .with(reason),
            Self::Io {
                operation,
                path,
                reason,
            } => Message::new("could not {} {}: {}")
                .nested(Message::term(operation))
                .with(path)
                .with(reason),
            Self::Rollback {
                path,
                primary,
                rollback,
            } => Message::new("rollback at {} failed after {}: {}")
                .with(path)
                .nested(primary.message())
                .nested(rollback.message()),
            Self::RemovalIncomplete { name, path } => Message::new(
                "{} was removed from the library, but its files couldn't be fully deleted: {} — close any program using them, then delete the folder (or run `skit doctor --rebuild` to restore the entry and retry).",
            )
            .with(name)
            .with(path),
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
        scan.entries
            .sort_by(|left, right| left.slug.cmp(&right.slug));
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
