//! Stable Library detail facts every frontend renders.
//!
//! The host owns filesystem, parser, state-store, and clock work. A frontend receives one
//! immutable projection and never reconstructs these facts from partial list rows. Version 0.4
//! reads exactly the same facts for the same screen (`src/skit/tui.py:531-604`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skit_domain::Slug;

use crate::LibraryScan;

/// Localized relative-time shape for one recorded launch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryRunAge {
    /// Less than ninety seconds ago, including small clock skew.
    JustNow,
    /// Whole minutes ago.
    Minutes(u64),
    /// Whole hours ago.
    Hours(u64),
    /// Whole days ago.
    Days(u64),
    /// Preserve an unparseable legacy timestamp exactly.
    Raw(String),
}

impl LibraryRunAge {
    /// Apply the version 0.4 relative-time thresholds to an already parsed elapsed duration.
    #[must_use]
    pub fn from_elapsed(raw: impl Into<String>, seconds: Option<i64>) -> Self {
        let Some(seconds) = seconds else {
            return Self::Raw(raw.into());
        };
        if seconds < 90 {
            Self::JustNow
        } else if seconds < 5_400 {
            Self::Minutes(u64::try_from(seconds / 60).unwrap_or_default())
        } else if seconds < 129_600 {
            Self::Hours(u64::try_from(seconds / 3_600).unwrap_or_default())
        } else {
            Self::Days(u64::try_from(seconds / 86_400).unwrap_or_default())
        }
    }
}

/// Last launch facts shown in the Library detail pane.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibraryLastRun {
    /// ISO-8601 launch timestamp used for activity ordering.
    pub at: String,
    /// Relative presentation computed once when the host refreshes the Library.
    pub age: LibraryRunAge,
    /// Child exit status. Legacy partial state can omit it.
    pub exit: Option<i64>,
}

/// Effective parameter value shown in the Library detail summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibraryParameterDetail {
    /// Stable form key.
    pub key: String,
    /// Last nonsecret value, or the definition default when no last value exists.
    pub value: String,
    /// Mask the value and show the lock marker.
    pub secret: bool,
}

/// Prompt-runner state shown in the Library detail pane.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryPromptRunner {
    /// The launch form asks on each run.
    PickOnRunForm,
    /// The entry pins one configured runner.
    Configured(String),
    /// The entry pins a runner that is no longer configured.
    Missing(String),
}

/// Complete host-projected facts for one Library entry.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibraryEntryDetail {
    /// UTC add timestamp used as the activity fallback.
    pub added_at: String,
    /// Command-template launch material, when this entry family owns one.
    pub template: Option<String>,
    /// Prompt-runner pin state, only for prompt entries.
    pub prompt_runner: Option<LibraryPromptRunner>,
    /// Effective form values in field order.
    pub parameters: Vec<LibraryParameterDetail>,
    /// Saved preset names.
    pub presets: Vec<String>,
    /// Effective package dependencies.
    pub dependencies: Vec<String>,
    /// Most recent recorded launch.
    pub last_run: Option<LibraryLastRun>,
    /// Missing launch target path. `None` means the target is present or not file-backed.
    pub missing_target: Option<String>,
    /// Whether parser-backed form definitions differ from the current source.
    pub drifted: bool,
    /// Whether removal can truthfully reassure the user that an existing original stays intact.
    pub original_file_preserved: bool,
}

impl LibraryEntryDetail {
    /// Return the timestamp the Library sorts on.
    ///
    /// Version 0.4 keys recency on `max(last run, added at)` so a fresh add surfaces even though
    /// it has never run (`src/skit/tui.py:99-103`).
    #[must_use]
    pub fn activity_at(&self) -> &str {
        self.last_run
            .as_ref()
            .map(|run| run.at.as_str())
            .filter(|at| *at > self.added_at.as_str())
            .unwrap_or(&self.added_at)
    }
}

/// Complete host-projected Library data for one refresh.
///
/// The host owns filesystem, parser, state-store, and clock work. Frontends receive one immutable
/// projection and never reconstruct these facts from partial list rows.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibrarySurface {
    /// Current entry summaries and diagnostics.
    pub scan: LibraryScan,
    /// Complete detail facts indexed by stable entry identity.
    pub details: BTreeMap<Slug, LibraryEntryDetail>,
}
