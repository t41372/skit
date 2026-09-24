//! The outcome that one status line reports, and how long the line stays.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long a receipt or a note stays before it clears itself.
pub const TRANSIENT_STATUS_TIME: Duration = Duration::from_secs(4);

/// The outcome that one status line reports.
///
/// A frontend shows the kind with a glyph and a color: no glyph for information, `✓` for a
/// success, `⚠` for a warning, and `✗` for an error. The kind also sets how long the line stays.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusKind {
    /// Neutral information, such as a hint about the next step.
    #[default]
    Info,
    /// An operation that finished.
    Success,
    /// An operation that finished with a problem, or a request that needs a change first.
    Warning,
    /// An operation that failed.
    Error,
}

impl StatusKind {
    /// How long a status line of this kind stays before it clears itself.
    ///
    /// A note or a receipt clears after [`TRANSIENT_STATUS_TIME`], and the footer then shows its
    /// usual line again, for example the entry count. A warning or an error has no time limit: it
    /// stays until the user presses a key or clicks, because the user must read it to act.
    #[must_use]
    pub const fn display_time(self) -> Option<Duration> {
        match self {
            Self::Info | Self::Success => Some(TRANSIENT_STATUS_TIME),
            Self::Warning | Self::Error => None,
        }
    }

    /// Report whether this is the default kind, which serialization leaves out.
    ///
    /// Serde's `skip_serializing_if` passes the field by reference.
    #[must_use]
    pub const fn is_info(&self) -> bool {
        matches!(self, Self::Info)
    }
}
