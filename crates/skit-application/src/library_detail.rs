//! Stable Library detail facts every frontend renders.
//!
//! The host owns filesystem, parser, state-store, and clock work. A frontend receives one
//! immutable projection and never reconstructs these facts from partial list rows. Version 0.4
//! reads exactly the same facts for the same screen (`src/skit/tui.py:531-604`).

use std::{collections::BTreeMap, fmt::Debug, path::PathBuf};

use serde::{Deserialize, Serialize};
use skit_domain::{Entry, EntrySettings, Slug, parameters::ParamDecl};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    LibraryScan, RepositoryError,
    form_state::{FormStateRepository, LastRunState, prefill},
};

/// Storage state for the target that one entry would launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LibraryTargetState {
    /// This entry kind has no file-backed target that this version can check.
    NotApplicable,
    /// The launch target exists.
    Present,
    /// The launch target is absent at this path.
    Missing(PathBuf),
}

/// One complete storage snapshot used to project Library detail facts.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntrySnapshot {
    /// Complete entry metadata.
    pub entry: Entry,
    /// Byte-exact payload content, or `None` when the payload cannot be read.
    pub source: Option<Vec<u8>>,
    /// Current launch-target state.
    pub target: LibraryTargetState,
    /// Whether the entry's recorded original source currently exists.
    pub original_source_exists: bool,
}

/// One coherent list-and-detail snapshot for a Library refresh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibraryRefreshSnapshot {
    /// Current entry summaries and diagnostics.
    pub scan: LibraryScan,
    /// Storage facts for the same valid entries as the scan.
    pub entries: Vec<LibraryEntrySnapshot>,
}

/// Read-side storage port for one complete Library refresh.
pub trait LibraryDetailRepository: Debug {
    /// Read membership once and derive list and detail facts from the same entries.
    fn library_refresh(&self) -> Result<LibraryRefreshSnapshot, RepositoryError>;
}

/// Parser-backed form facts used by the Library detail projection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibraryFormFacts {
    /// Effective declarations in runtime order.
    pub declarations: Vec<ParamDecl>,
    /// Whether stored definitions differ from the current source.
    pub drifted: bool,
}

/// Form-analysis port used by the Library detail projection.
pub trait LibraryFormProjector: Debug {
    /// Project form facts from one byte-exact entry snapshot.
    fn project(&self, entry: &Entry, source: Option<&[u8]>) -> LibraryFormFacts;
}

/// Build one complete Library surface from application ports.
#[derive(Debug)]
pub struct LibrarySurfaceService<'a, R, S, F, E> {
    repository: &'a R,
    form_state: &'a S,
    form_projector: &'a F,
    effective_settings: E,
}

impl<'a, R, S, F, E> LibrarySurfaceService<'a, R, S, F, E> {
    /// Construct the service from storage, state, form, and source projections.
    #[must_use]
    pub const fn new(
        repository: &'a R,
        form_state: &'a S,
        form_projector: &'a F,
        effective_settings: E,
    ) -> Self {
        Self {
            repository,
            form_state,
            form_projector,
            effective_settings,
        }
    }
}

impl<R, S, F, E> LibrarySurfaceService<'_, R, S, F, E>
where
    R: LibraryDetailRepository,
    S: FormStateRepository,
    F: LibraryFormProjector,
    E: Fn(&Entry, Option<&[u8]>) -> EntrySettings,
{
    /// Build one projection against an explicit clock reading.
    pub fn load_at(
        &self,
        configured_runners: &[String],
        now: OffsetDateTime,
    ) -> Result<LibrarySurface, RepositoryError> {
        let refresh = self.repository.library_refresh()?;
        let details = refresh
            .entries
            .into_iter()
            .map(|snapshot| {
                let slug = snapshot.entry.slug.clone();
                let detail = self.entry_detail(snapshot, configured_runners, now);
                (slug, detail)
            })
            .collect();
        Ok(LibrarySurface {
            scan: refresh.scan,
            details,
        })
    }

    fn entry_detail(
        &self,
        snapshot: LibraryEntrySnapshot,
        configured_runners: &[String],
        now: OffsetDateTime,
    ) -> LibraryEntryDetail {
        let entry = &snapshot.entry;
        let kind = entry.meta.kind.as_str();
        let source = snapshot.source.as_deref();
        let settings = (self.effective_settings)(entry, source);
        let persisted = self.form_state.load(&entry.slug);
        let form = self.form_projector.project(entry, source);
        let values = prefill(&form.declarations, &persisted.values, None);
        let parameters = form
            .declarations
            .iter()
            .map(|declaration| LibraryParameterDetail {
                key: declaration.name.clone(),
                value: values.get(&declaration.name).cloned().unwrap_or_default(),
                secret: declaration.secret,
            })
            .collect();
        LibraryEntryDetail {
            added_at: entry.meta.added_at.clone(),
            template: (kind == "command" && !settings.template.is_empty())
                .then(|| settings.template.clone()),
            prompt_runner: (kind == "prompt")
                .then(|| prompt_runner(&settings.runner, configured_runners)),
            parameters,
            presets: persisted.presets.keys().cloned().collect(),
            dependencies: settings.dependencies,
            last_run: last_run(now, &persisted.last_run),
            missing_target: match snapshot.target {
                LibraryTargetState::Missing(path) => Some(path.display().to_string()),
                LibraryTargetState::NotApplicable | LibraryTargetState::Present => None,
            },
            drifted: form.drifted,
            original_file_preserved: original_file_preserved(
                entry,
                snapshot.original_source_exists,
            ),
        }
    }
}

fn prompt_runner(pin: &str, configured_runners: &[String]) -> LibraryPromptRunner {
    if pin.is_empty() {
        return LibraryPromptRunner::PickOnRunForm;
    }
    if configured_runners.iter().any(|runner| runner == pin) {
        LibraryPromptRunner::Configured(pin.to_owned())
    } else {
        LibraryPromptRunner::Missing(pin.to_owned())
    }
}

fn last_run(now: OffsetDateTime, last_run: &LastRunState) -> Option<LibraryLastRun> {
    let at = last_run.at.clone()?;
    let elapsed = OffsetDateTime::parse(&at, &Rfc3339)
        .ok()
        .map(|then| (now - then).whole_seconds());
    Some(LibraryLastRun {
        age: LibraryRunAge::from_elapsed(at.clone(), elapsed),
        at,
        exit: last_run.exit,
    })
}

fn original_file_preserved(entry: &Entry, original_source_exists: bool) -> bool {
    entry.meta.kind.as_str() != "command" && !entry.meta.source.is_empty() && original_source_exists
}

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

#[cfg(test)]
mod tests {
    use skit_domain::{EntryKind, EntryMeta};

    use super::*;

    #[test]
    fn prompt_runner_pins_are_classified_against_configured_names() {
        let configured = vec!["claude".to_owned()];
        assert_eq!(
            prompt_runner("", &configured),
            LibraryPromptRunner::PickOnRunForm
        );
        assert_eq!(
            prompt_runner("claude", &configured),
            LibraryPromptRunner::Configured("claude".to_owned())
        );
        assert_eq!(
            prompt_runner("removed", &configured),
            LibraryPromptRunner::Missing("removed".to_owned())
        );
    }

    #[test]
    fn one_clock_read_resolves_every_last_run_age_shape() {
        let now = OffsetDateTime::parse("2026-08-09T12:00:00Z", &Rfc3339).unwrap();
        let at = |value: &str| LastRunState {
            at: Some(value.to_owned()),
            exit: Some(0),
            values: None,
        };
        assert!(last_run(now, &LastRunState::default()).is_none());
        assert_eq!(
            last_run(now, &at("2026-08-09T11:59:30Z")).unwrap().age,
            LibraryRunAge::JustNow
        );
        assert_eq!(
            last_run(now, &at("2026-08-09T11:00:00Z")).unwrap().age,
            LibraryRunAge::Minutes(60)
        );
        assert_eq!(
            last_run(now, &at("2026-08-08T12:00:00Z")).unwrap().age,
            LibraryRunAge::Hours(24)
        );
        assert_eq!(
            last_run(now, &at("2026-07-09T12:00:00Z")).unwrap().age,
            LibraryRunAge::Days(31)
        );
        assert_eq!(
            last_run(now, &at("not a timestamp")).unwrap().age,
            LibraryRunAge::Raw("not a timestamp".to_owned())
        );
    }

    #[test]
    fn removal_reassurance_requires_a_noncommand_original_that_exists() {
        let entry = |kind: &str, source: &str| Entry {
            slug: Slug::parse("demo").unwrap(),
            meta: EntryMeta {
                source: source.to_owned(),
                ..EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap())
            },
        };
        assert!(original_file_preserved(
            &entry("shell", "/original/demo.sh"),
            true
        ));
        assert!(!original_file_preserved(
            &entry("command", "/original/demo.sh"),
            true
        ));
        assert!(!original_file_preserved(
            &entry("shell", "/original/demo.sh"),
            false
        ));
        assert!(!original_file_preserved(&entry("shell", ""), true));
    }
}
