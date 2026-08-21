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
pub mod path_completion;
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
    CreateEntry, EntryMutationRepository, EntryPayload, PreparedEntryUpdateError,
    SourcePermissions, UpdateEntry,
};
pub use payload_policy::{
    add_workdir, canonical_stored_filename, payload_stored_name, supports_storage_modes,
};
use serde::{Deserialize, Serialize};
use skit_domain::{Entry, EntrySummary};
use skit_i18n::{Locale, Localize, Message};
use thiserror::Error;

/// Stable identity of one filesystem object captured by a host adapter.
///
/// Source bytes and permissions remain separate transaction facts. This value identifies the file
/// incarnation so a same-bytes replacement cannot inherit an earlier destructive claim.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceIdentity(SourceIdentityKind);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "platform")]
enum SourceIdentityKind {
    Unix {
        device: u64,
        inode: u64,
        change_time_seconds: i64,
        change_time_nanoseconds: i64,
    },
    Windows {
        volume_serial_number: u64,
        #[serde(with = "u128_decimal")]
        file_index: u128,
        creation_time: u64,
    },
}

mod u128_decimal {
    use serde::{Deserialize, Deserializer, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Encoded {
        Legacy(u64),
        Decimal(String),
    }

    pub(super) fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Encoded::deserialize(deserializer)? {
            Encoded::Legacy(value) => Ok(u128::from(value)),
            Encoded::Decimal(value) => value.parse().map_err(serde::de::Error::custom),
        }
    }
}

impl SourceIdentity {
    /// Construct an identity from Unix `stat` values captured from an open file.
    #[must_use]
    pub const fn unix(
        device: u64,
        inode: u64,
        change_time_seconds: i64,
        change_time_nanoseconds: i64,
    ) -> Self {
        Self(SourceIdentityKind::Unix {
            device,
            inode,
            change_time_seconds,
            change_time_nanoseconds,
        })
    }

    /// Construct an identity from Windows file metadata captured from an open file.
    #[must_use]
    pub const fn windows(volume_serial_number: u64, file_index: u128, creation_time: u64) -> Self {
        Self(SourceIdentityKind::Windows {
            volume_serial_number,
            file_index,
            creation_time,
        })
    }

    /// Report whether two observations name the same filesystem object.
    ///
    /// A rename can update Unix change time. Cleanup uses this narrower comparison only after it
    /// has atomically moved a fully verified claim into skit's private quarantine.
    #[must_use]
    pub fn same_file(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (
                SourceIdentityKind::Unix {
                    device: left_device,
                    inode: left_inode,
                    ..
                },
                SourceIdentityKind::Unix {
                    device: right_device,
                    inode: right_inode,
                    ..
                },
            ) => left_device == right_device && left_inode == right_inode,
            (
                SourceIdentityKind::Windows {
                    volume_serial_number: left_volume,
                    file_index: left_index,
                    creation_time: left_creation,
                    ..
                },
                SourceIdentityKind::Windows {
                    volume_serial_number: right_volume,
                    file_index: right_index,
                    creation_time: right_creation,
                    ..
                },
            ) => {
                left_volume == right_volume
                    && left_index == right_index
                    && left_creation == right_creation
            }
            _ => false,
        }
    }
}

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
    /// A create would collide with an existing entry's display name.
    #[error("The name {name} is already taken — pick another name.")]
    Conflict {
        /// Requested display name.
        name: String,
    },
    /// A rename would collide with an existing entry's display name.
    #[error("The name {name} is already taken.")]
    RenameConflict {
        /// Requested display name.
        name: String,
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
        "{name} was removed from the library, but its files couldn't be fully deleted: {path} — close any program using them, then delete the folder (or run `skit doctor --rebuild` to restore the entry and retry)."
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
                Self::Ambiguous { .. }
                | Self::Conflict { .. }
                | Self::RenameConflict { .. }
                | Self::InvalidMutation { .. },
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
            Self::Conflict { name } => {
                Message::new("The name {} is already taken — pick another name.").with(name)
            }
            Self::RenameConflict { name } => {
                Message::new("The name {} is already taken.").with(name)
            }
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

#[cfg(test)]
mod source_identity_tests {
    use super::{SourceIdentity, SourceIdentityKind};

    #[test]
    fn source_identity_is_typed_by_platform_and_roundtrips() {
        let unix = SourceIdentity(SourceIdentityKind::Unix {
            device: 7,
            inode: 11,
            change_time_seconds: 13,
            change_time_nanoseconds: 17,
        });
        let windows = SourceIdentity(SourceIdentityKind::Windows {
            volume_serial_number: 19,
            file_index: 23,
            creation_time: 29,
        });
        assert_ne!(unix, windows);

        for identity in [unix, windows] {
            let bytes = serde_json::to_vec(&identity).unwrap();
            assert_eq!(
                serde_json::from_slice::<SourceIdentity>(&bytes).unwrap(),
                identity
            );
        }
    }

    #[test]
    fn every_identity_component_participates_in_equality() {
        let baseline = SourceIdentity(SourceIdentityKind::Unix {
            device: 1,
            inode: 2,
            change_time_seconds: 3,
            change_time_nanoseconds: 4,
        });
        for different in [
            SourceIdentity(SourceIdentityKind::Unix {
                device: 9,
                inode: 2,
                change_time_seconds: 3,
                change_time_nanoseconds: 4,
            }),
            SourceIdentity(SourceIdentityKind::Unix {
                device: 1,
                inode: 9,
                change_time_seconds: 3,
                change_time_nanoseconds: 4,
            }),
            SourceIdentity(SourceIdentityKind::Unix {
                device: 1,
                inode: 2,
                change_time_seconds: 9,
                change_time_nanoseconds: 4,
            }),
            SourceIdentity(SourceIdentityKind::Unix {
                device: 1,
                inode: 2,
                change_time_seconds: 3,
                change_time_nanoseconds: 9,
            }),
        ] {
            assert_ne!(baseline, different);
        }
    }

    #[test]
    fn same_file_ignores_rename_time_but_not_platform_file_id() {
        let before = SourceIdentity::unix(1, 2, 3, 4);
        let renamed = SourceIdentity::unix(1, 2, 30, 40);
        let replacement = SourceIdentity::unix(1, 9, 30, 40);
        assert!(before.same_file(&renamed));
        assert!(!before.same_file(&replacement));
        assert!(!before.same_file(&SourceIdentity::windows(1, 2, 3)));
        assert!(!SourceIdentity::windows(1, 2, 3).same_file(&SourceIdentity::windows(1, 2, 4)));
    }

    #[test]
    fn windows_identity_keeps_the_complete_stable_file_id() {
        let high_bit = 1_u128 << 96;
        let identity = SourceIdentity::windows(7, high_bit + 11, 13);

        assert_eq!(
            serde_json::to_value(&identity).unwrap(),
            serde_json::json!({
                "platform": "windows",
                "volume_serial_number": 7,
                "file_index": (high_bit + 11).to_string(),
                "creation_time": 13,
            })
        );
        assert!(identity.same_file(&SourceIdentity::windows(7, high_bit + 11, 13)));
        assert!(!identity.same_file(&SourceIdentity::windows(7, 11, 13)));

        let legacy = serde_json::json!({
            "platform": "windows",
            "volume_serial_number": 7,
            "file_index": 11,
            "creation_time": 13,
        });
        assert_eq!(
            serde_json::from_value::<SourceIdentity>(legacy).unwrap(),
            SourceIdentity::windows(7, 11, 13)
        );
    }
}
