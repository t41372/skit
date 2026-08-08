use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::fmt;

use crate::{FormPlan, StateError, StateStore};

/// Failures from the automation-friendly `preset save --from-last` path.
#[derive(Debug)]
pub enum PresetFromLastError {
    NoFields,
    NoRememberedValues,
    State(StateError),
}

impl fmt::Display for PresetFromLastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFields => formatter.write_str("this entry has no form fields"),
            Self::NoRememberedValues => {
                formatter.write_str("this entry has no remembered values yet — run it once first")
            }
            Self::State(source) => source.fmt(formatter),
        }
    }
}

impl StdError for PresetFromLastError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::State(source) => Some(source),
            Self::NoFields | Self::NoRememberedValues => None,
        }
    }
}

impl From<StateError> for PresetFromLastError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

/// Save a named preset from the last accepted invocation snapshot.
///
/// A current field filter is applied so removed parameters never resurrect. Definition
/// defaults are deliberately not overlaid: this operation promises history, not today's
/// form. Before run snapshots existed, explicit last-used values are accepted as a narrow
/// compatibility fallback.
///
/// # Errors
///
/// Returns an error if the entry has no fields, has never recorded usable values, or the
/// state file cannot be atomically updated.
pub fn save_preset_from_last(
    state_store: &StateStore,
    slug: &str,
    preset_name: &str,
    plan: &FormPlan,
) -> Result<BTreeMap<String, String>, PresetFromLastError> {
    if plan.fields.is_empty() {
        return Err(PresetFromLastError::NoFields);
    }

    let state = state_store.load(slug);
    let field_names = plan
        .fields
        .iter()
        .map(|field| field.key.as_str())
        .collect::<BTreeSet<_>>();
    let snapshot = if let Some(last_run) = &state.last_run {
        last_run
            .values
            .iter()
            .filter(|(key, _)| field_names.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>()
    } else if !state.values.is_empty() {
        state
            .values
            .iter()
            .filter(|(key, _)| field_names.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>()
    } else {
        return Err(PresetFromLastError::NoRememberedValues);
    };

    let secrets = plan.secret_names();
    state_store.save_preset(slug, preset_name, &snapshot, &secrets)?;
    let persisted = snapshot
        .into_iter()
        .filter(|(key, _)| !secrets.contains(key))
        .collect();
    Ok(persisted)
}
