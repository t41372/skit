//! Shared health facts and the inspection port used by every frontend.

use std::fmt::Debug;

use serde::{Deserialize, Serialize};

/// Availability of the Python runtime manager used by Python entries.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UvHealth {
    /// An executable was found at this path.
    Found(String),
    /// No current entry needs uv.
    NotRequired,
    /// A Python entry needs uv, but no executable was found.
    Missing,
}

/// One entry-addressable health problem.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthIssueKind {
    /// The stored launch target no longer exists.
    MissingTarget,
    /// Managed form definitions no longer match the source.
    DriftedForm,
    /// Required external commands are unavailable.
    MissingNeeds { tools: Vec<String> },
    /// A launch preflight would refuse the entry.
    LaunchBlocked { reason: String },
}

/// One selectable health issue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthIssue {
    /// Stable library address.
    pub slug: String,
    /// User-facing entry name.
    pub name: String,
    /// Typed issue detail.
    pub kind: HealthIssueKind,
}

/// Stored and active mirror state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorHealth {
    /// No mirror URLs are stored.
    Off,
    /// Stored axes are active.
    On { axes: String },
    /// Stored axes are paused by the master switch.
    Paused { axes: String },
}

/// Complete health facts collected once for all frontends.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthSnapshot {
    /// uv availability.
    pub uv: UvHealth,
    /// Number of valid registered entries.
    pub entry_count: usize,
    /// Entry-addressable issues in deterministic library order.
    pub issues: Vec<HealthIssue>,
    /// Descriptions of malformed runner rows.
    pub invalid_runner_rows: Vec<String>,
    /// Stored and active mirror state.
    pub mirror: MirrorHealth,
    /// Library location.
    pub library_path: String,
    /// Human-readable library size.
    pub library_size: String,
    /// Non-entry-specific scan diagnostics.
    pub diagnostics: Vec<String>,
}

/// Result facts from one explicit registry rebuild.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthRebuildOutcome {
    /// Entries indexed after the rebuild.
    pub entry_count: usize,
    /// Isolated directories that the rebuild skipped.
    pub problems: Vec<String>,
}

/// A refreshed report and the result of the rebuild that produced it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthRebuild {
    /// Full report collected after the registry changed.
    pub snapshot: HealthSnapshot,
    /// Rebuild-specific completion facts.
    pub outcome: HealthRebuildOutcome,
}

/// Host port that keeps doctor and interactive Health on one inspection pipeline.
pub trait HealthInspection: Debug {
    /// Adapter-owned typed failure.
    type Error;

    /// Collect one complete report without changing state.
    fn inspect(&self) -> Result<HealthSnapshot, Self::Error>;

    /// Rebuild the registry and collect a complete report from the new state.
    fn rebuild(&self) -> Result<HealthRebuild, Self::Error>;
}

/// Shared health use case for CLI, Ratatui, and future frontends.
#[derive(Debug)]
pub struct HealthService<I> {
    inspector: I,
}

impl<I> HealthService<I>
where
    I: HealthInspection,
{
    /// Construct the use case around one host adapter.
    #[must_use]
    pub const fn new(inspector: I) -> Self {
        Self { inspector }
    }

    /// Collect one complete report.
    pub fn inspect(&self) -> Result<HealthSnapshot, I::Error> {
        self.inspector.inspect()
    }

    /// Rebuild and recollect through the same inspection port.
    pub fn rebuild(&self) -> Result<HealthRebuild, I::Error> {
        self.inspector.rebuild()
    }

    /// Expose the adapter for composition-level inspection and focused tests.
    #[must_use]
    pub const fn inspector(&self) -> &I {
        &self.inspector
    }
}
