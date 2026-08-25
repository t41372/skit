//! Pure form prefill and persistence policy shared by every frontend and storage adapter.
//!
//! This module owns the boundary where a value is allowed to become persistent state. Secrets are
//! excluded structurally here rather than relying on CLI, Ratatui, or a future Tauri adapter to
//! remember a masking rule. Filesystem locking and atomic TOML replacement remain storage concerns.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
};

use skit_domain::{
    Slug,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_i18n::{Localize, Message};
use thiserror::Error;

/// Exact metadata and accepted values for the most recent recorded run.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LastRunState {
    /// ISO-8601 timestamp when available.
    pub at: Option<String>,
    /// Child exit status when available.
    pub exit: Option<i64>,
    /// Exact accepted invocation values, including an explicitly empty snapshot.
    ///
    /// `None` means a raw or legacy run did not record form values.
    pub values: Option<BTreeMap<String, String>>,
}

/// Complete per-entry state owned by the Rust implementation at cutover.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PersistedFormState {
    /// Last-used non-secret values.
    pub values: BTreeMap<String, String>,
    /// Remembered argument tail.
    pub extra_args: Vec<String>,
    /// Whether the remembered tail is raw launch-menu text that should expand on replay.
    pub extra_args_raw: bool,
    /// Named presets.
    pub presets: BTreeMap<String, BTreeMap<String, String>>,
    /// Most recent run stamp and exact accepted-value snapshot.
    pub last_run: LastRunState,
}

/// A state-backed source for a preset save request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresetSnapshotSource {
    /// Definition defaults overlaid with the latest remembered explicit values.
    Prefill,
    /// The exact values accepted by the most recent non-raw run.
    LastRun,
}

/// A state read-modify-write transaction could not be committed.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StateWriteError {
    /// A filesystem operation failed.
    #[error("could not {operation} state at {path}: {reason}")]
    Io {
        /// Operation such as create, lock, write, replace, or remove.
        operation: &'static str,
        /// Affected path.
        path: String,
        /// Operating-system detail.
        reason: String,
    },
    /// The in-memory state could not be encoded as the persistence format.
    #[error("could not encode state: {reason}")]
    Encode {
        /// Serializer detail.
        reason: String,
    },
}

impl Localize for StateWriteError {
    fn message(&self) -> Message {
        match self {
            Self::Io {
                operation,
                path,
                reason,
            } => Message::new("could not {} state at {}: {}")
                .nested(Message::term(operation))
                .with(path)
                .with(reason),
            Self::Encode { reason } => Message::new("could not encode state: {}").with(reason),
        }
    }
}

/// Persistence port whose update boundary holds one adapter-defined transaction lock.
///
/// `update` is intentionally a closure rather than separate load/save methods: callers cannot
/// accidentally create a stale read-modify-write window between the two operations. The trait is
/// used through generic application services, so object safety is not required.
pub trait FormStateRepository: Debug {
    /// Load the complete known state. Missing/corrupt documents degrade to empty state.
    fn load(&self, slug: &Slug) -> PersistedFormState;

    /// Load only the run stamp needed by listing surfaces.
    ///
    /// Adapters must not construct saved values, argument tails, presets, or the last-run value
    /// snapshot on this path. A library list calls this once per row.
    fn last_run(&self, slug: &Slug) -> LastRunState;

    /// Mutate the current state while the repository holds its per-entry transaction lock.
    fn update<T, F>(&self, slug: &Slug, update: F) -> Result<T, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> T;

    /// Attempt one mutation and leave persistence byte-identical when the closure refuses.
    fn try_update<T, E, F>(&self, slug: &Slug, update: F) -> Result<Result<T, E>, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> Result<T, E>;

    /// Remove all per-entry state while holding the same transaction lock used by updates.
    fn forget(&self, slug: &Slug) -> Result<(), StateWriteError>;
}

/// Final shared state use-cases for CLI, Ratatui, and future Tauri frontends.
#[derive(Debug)]
pub struct FormStateService<R> {
    repository: R,
}

impl<R> FormStateService<R>
where
    R: FormStateRepository,
{
    /// Construct the service around one concrete state repository.
    #[must_use]
    pub const fn new(repository: R) -> Self {
        Self { repository }
    }

    /// Load the complete state for one entry.
    #[must_use]
    pub fn load(&self, slug: &Slug) -> PersistedFormState {
        self.repository.load(slug)
    }

    /// Load only the most recent run stamp for a listing or status surface.
    #[must_use]
    pub fn last_run(&self, slug: &Slug) -> LastRunState {
        self.repository.last_run(slug)
    }

    /// Save last-used values and/or the remembered argument tail.
    ///
    /// `None` means leave that surface unchanged; `Some(empty)` is an explicit clear. Existing
    /// last-used values are still scrubbed for parameters that are secret now even when no new
    /// values were supplied.
    pub fn save_last(
        &self,
        slug: &Slug,
        declarations: &[ParamDecl],
        values: Option<&BTreeMap<String, String>>,
        extra_args: Option<Vec<String>>,
        extra_args_raw: bool,
    ) -> Result<(), StateWriteError> {
        let remembered = values.map(|values| remembered_values(declarations, values));
        let secret_names = secret_names(declarations);
        self.repository.update(slug, move |state| {
            if let Some(values) = remembered {
                state.values = values;
            } else {
                strip_secret_names(&mut state.values, &secret_names);
            }
            if let Some(extra_args) = extra_args {
                state.extra_args = extra_args;
                state.extra_args_raw = !state.extra_args.is_empty() && extra_args_raw;
            }
        })
    }

    /// Save one named preset with exact current public values, including defaults and empties.
    pub fn save_preset(
        &self,
        slug: &Slug,
        name: &str,
        declarations: &[ParamDecl],
        values: &BTreeMap<String, String>,
    ) -> Result<(), StateWriteError> {
        let values = preset_values(declarations, values);
        let name = name.to_owned();
        self.repository.update(slug, move |state| {
            state.presets.insert(name, values);
        })
    }

    /// Save a preset from one state-backed snapshot in a single transaction.
    ///
    /// Returns `false` when `LastRun` has no honest value snapshot. State from releases before
    /// exact run snapshots remains usable only when it has remembered values and no run stamp.
    pub fn save_preset_from_state(
        &self,
        slug: &Slug,
        name: &str,
        declarations: &[ParamDecl],
        source: PresetSnapshotSource,
    ) -> Result<bool, StateWriteError> {
        let name = name.to_owned();
        self.repository.update(slug, move |state| {
            let values = match source {
                PresetSnapshotSource::Prefill => Some(prefill(declarations, &state.values, None)),
                PresetSnapshotSource::LastRun => state.last_run.values.clone().or_else(|| {
                    (state.last_run.at.is_none()
                        && state.last_run.exit.is_none()
                        && !state.values.is_empty())
                    .then(|| state.values.clone())
                }),
            };
            let Some(values) = values else {
                return false;
            };
            state
                .presets
                .insert(name, preset_values(declarations, &values));
            true
        })
    }

    /// Delete one named preset and report whether it existed.
    pub fn delete_preset(&self, slug: &Slug, name: &str) -> Result<bool, StateWriteError> {
        let name = name.to_owned();
        self.repository
            .update(slug, move |state| state.presets.remove(&name).is_some())
    }

    /// Retroactively scrub every persistent value surface for parameters that are secret now.
    pub fn purge_secrets(
        &self,
        slug: &Slug,
        declarations: &[ParamDecl],
    ) -> Result<BTreeSet<String>, StateWriteError> {
        self.repository
            .update(slug, |state| scrub_secrets(declarations, state))
    }

    /// Record the latest run stamp and its exact accepted-value snapshot.
    ///
    /// `values=None` records that the run had no form snapshot, as raw runs do. It clears an older
    /// snapshot so a later `--from-last` request cannot attach values from a different invocation.
    pub fn record_run(
        &self,
        slug: &Slug,
        exit: i64,
        at: &str,
        declarations: &[ParamDecl],
        values: Option<&BTreeMap<String, String>>,
    ) -> Result<(), StateWriteError> {
        let values = values.map(|values| preset_values(declarations, values));
        let at = at.to_owned();
        self.repository.update(slug, move |state| {
            state.last_run.at = Some(at);
            state.last_run.exit = Some(exit);
            state.last_run.values = values;
        })
    }

    /// Commit all state from one completed run against declarations read under the state lock.
    ///
    /// A source edit uses the same adapter lock for its secrecy commit and purge. Resolving the
    /// current declarations inside this update prevents a run that started with an older public
    /// schema from restoring plaintext after the field becomes secret.
    #[allow(clippy::too_many_arguments)]
    pub fn record_completed_run_with<E>(
        &self,
        slug: &Slug,
        exit: i64,
        at: &str,
        values: Option<&BTreeMap<String, String>>,
        extra_args: Option<Vec<String>>,
        extra_args_raw: bool,
        preset_name: Option<&str>,
        declarations: impl FnOnce() -> Result<Vec<ParamDecl>, E>,
    ) -> Result<Result<(), E>, StateWriteError> {
        let values = values.cloned();
        let at = at.to_owned();
        let preset_name = preset_name.map(str::to_owned);
        self.repository.try_update(slug, move |state| {
            if let Some(values) = values.as_ref() {
                let declarations = match declarations() {
                    Ok(declarations) => declarations,
                    Err(error) => return Err(error),
                };
                let _ = scrub_secrets(&declarations, state);
                state.values = remembered_values(&declarations, values);
                if let Some(extra_args) = extra_args {
                    state.extra_args = extra_args;
                    state.extra_args_raw = !state.extra_args.is_empty() && extra_args_raw;
                }
                if let Some(name) = preset_name {
                    state
                        .presets
                        .insert(name, preset_values(&declarations, values));
                }
                state.last_run.values = Some(preset_values(&declarations, values));
            } else {
                state.last_run.values = None;
            }
            state.last_run.at = Some(at);
            state.last_run.exit = Some(exit);
            Ok(())
        })
    }

    /// Remove all remembered state for one entry.
    pub fn forget(&self, slug: &Slug) -> Result<(), StateWriteError> {
        self.repository.forget(slug)
    }

    /// Expose the repository for composition-level inspection and focused tests.
    #[must_use]
    pub const fn repository(&self) -> &R {
        &self.repository
    }
}

/// Build the next form's values with the stable precedence:
/// definition default < last-used < selected preset.
///
/// Only fields still present in the declaration set participate, and secrets are excluded from all
/// three sources even if old state still contains plaintext from before a secrecy transition.
#[must_use]
pub fn prefill(
    declarations: &[ParamDecl],
    last_used: &BTreeMap<String, String>,
    preset: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    let active = active_public_fields(declarations);
    let mut output = BTreeMap::new();

    for declaration in declarations {
        if !declaration.secret
            && let Some(default) = declaration.default.as_ref()
        {
            output.insert(declaration.name.clone(), render_value(default));
        }
    }

    apply_known_values(&mut output, last_used, &active);
    if let Some(preset) = preset {
        apply_known_values(&mut output, preset, &active);
    }
    output
}

/// Produce the last-used map that is safe to write to persistent state.
///
/// Accepting the current definition default is not remembered, because pinning it would hide a
/// later source change. Empty values are omitted unless clearing the field is itself a delivered
/// value. Secret fields are removed before this map can reach a storage adapter.
#[must_use]
pub fn remembered_values(
    declarations: &[ParamDecl],
    submitted: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let declarations = by_name(declarations);
    submitted
        .iter()
        .filter_map(|(name, value)| {
            let declaration = declarations.get(name.as_str())?;
            if declaration.secret {
                return None;
            }
            if declaration
                .default
                .as_ref()
                .is_some_and(|default| render_value(default) == *value)
            {
                return None;
            }
            (!value.is_empty() || delivers_empty(declaration))
                .then(|| (name.clone(), value.clone()))
        })
        .collect()
}

/// Produce one named-preset value map that is safe to persist.
///
/// Presets deliberately pin submitted values verbatim, including values equal to the current
/// definition default and explicit empty strings. Removed fields and secrets are excluded.
#[must_use]
pub fn preset_values(
    declarations: &[ParamDecl],
    submitted: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let active = active_public_fields(declarations);
    submitted
        .iter()
        .filter(|(name, _)| active.contains(name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Retroactively remove values for declarations that are secret now.
///
/// Unknown keys are preserved for forward compatibility. A preset containing only newly-secret
/// values is removed entirely instead of leaving a meaningless empty preset behind. Last-run
/// snapshots are scrubbed too, so a later "save preset from last run" path cannot resurrect a value
/// that was persisted while its parameter was still public. The return set identifies names for
/// which plaintext was actually removed from at least one persistent surface.
pub fn scrub_secrets(
    declarations: &[ParamDecl],
    state: &mut PersistedFormState,
) -> BTreeSet<String> {
    let secret_names = secret_names(declarations);
    let mut removed = BTreeSet::new();

    scrub_map(&mut state.values, &secret_names, &mut removed);
    state.presets.retain(|_, values| {
        scrub_map(values, &secret_names, &mut removed);
        !values.is_empty()
    });
    if let Some(values) = state.last_run.values.as_mut() {
        scrub_map(values, &secret_names, &mut removed);
    }

    removed
}

pub(crate) fn delivers_empty(declaration: &ParamDecl) -> bool {
    declaration.default.is_some()
        && !declaration.secret
        && !declaration.degraded
        && !declaration.multiple
        && declaration.binding != ParameterBinding::Input
        && matches!(
            declaration.parameter_type,
            ParameterType::Str | ParameterType::Path
        )
        && matches!(
            declaration.delivery,
            ParameterDelivery::Inject | ParameterDelivery::Flag | ParameterDelivery::Env
        )
}

fn secret_names(declarations: &[ParamDecl]) -> BTreeSet<&str> {
    declarations
        .iter()
        .filter(|declaration| declaration.secret)
        .map(|declaration| declaration.name.as_str())
        .collect()
}

fn strip_secret_names(values: &mut BTreeMap<String, String>, secret_names: &BTreeSet<&str>) {
    values.retain(|name, _| !secret_names.contains(name.as_str()));
}

fn scrub_map(
    values: &mut BTreeMap<String, String>,
    secret_names: &BTreeSet<&str>,
    removed: &mut BTreeSet<String>,
) {
    values.retain(|name, _| {
        let secret = secret_names.contains(name.as_str());
        if secret {
            removed.insert(name.clone());
        }
        !secret
    });
}

fn active_public_fields(declarations: &[ParamDecl]) -> BTreeSet<&str> {
    declarations
        .iter()
        .filter(|declaration| !declaration.secret)
        .map(|declaration| declaration.name.as_str())
        .collect()
}

fn by_name(declarations: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    declarations
        .iter()
        .map(|declaration| (declaration.name.as_str(), declaration))
        .collect()
}

fn apply_known_values(
    output: &mut BTreeMap<String, String>,
    values: &BTreeMap<String, String>,
    active: &BTreeSet<&str>,
) {
    output.extend(
        values
            .iter()
            .filter(|(name, _)| active.contains(name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone())),
    );
}

fn render_value(value: &ParameterValue) -> String {
    match value {
        ParameterValue::String(value) => value.clone(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => render_float(*value),
        ParameterValue::Bool(value) => value.to_string(),
    }
}

fn render_float(value: f64) -> String {
    let rendered = value.to_string();
    if value.fract() == 0.0 && !rendered.contains(['e', 'E']) {
        format!("{rendered}.0")
    } else {
        rendered
    }
}
