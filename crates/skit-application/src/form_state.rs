//! Pure form prefill and persistence policy shared by every frontend and storage adapter.
//!
//! This module owns the boundary where a value is allowed to become persistent state. Secrets are
//! excluded structurally here rather than relying on CLI, Ratatui, or a future Tauri adapter to
//! remember a masking rule. Filesystem locking and atomic TOML replacement remain storage concerns.

use std::collections::{BTreeMap, BTreeSet};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};

/// The value-bearing parts of per-entry form state that need secret scrubbing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PersistedFormState {
    /// Last-used values.
    pub values: BTreeMap<String, String>,
    /// Named presets.
    pub presets: BTreeMap<String, BTreeMap<String, String>>,
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
/// values is removed entirely instead of leaving a meaningless empty preset behind. The return set
/// identifies names for which plaintext was actually removed from at least one persistent surface.
pub fn scrub_secrets(
    declarations: &[ParamDecl],
    state: &mut PersistedFormState,
) -> BTreeSet<String> {
    let secret_names: BTreeSet<&str> = declarations
        .iter()
        .filter(|declaration| declaration.secret)
        .map(|declaration| declaration.name.as_str())
        .collect();
    let mut removed = BTreeSet::new();

    state.values.retain(|name, _| {
        let secret = secret_names.contains(name.as_str());
        if secret {
            removed.insert(name.clone());
        }
        !secret
    });

    state.presets.retain(|_, values| {
        values.retain(|name, _| {
            let secret = secret_names.contains(name.as_str());
            if secret {
                removed.insert(name.clone());
            }
            !secret
        });
        !values.is_empty()
    });

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
        ParameterValue::String(value)
        | ParameterValue::Choice(value)
        | ParameterValue::Path(value) => value.clone(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => render_float(*value),
        ParameterValue::Boolean(value) => value.to_string(),
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
