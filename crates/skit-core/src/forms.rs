use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::params::{
    Binding, Delivery, ParamDecl, ParamDefault, ParamType, declared_from_meta,
    synthesized_placeholder,
};
use crate::{Entry, EntryState, spec_for};

/// Where a form plan came from. Parser-backed sources can extend this enum without
/// changing the renderer-facing field model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlanSource {
    #[default]
    None,
    Declared,
    Command,
}

impl PlanSource {
    /// Stable machine-facing spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Declared => "declared",
            Self::Command => "command",
        }
    }

    /// Stable machine-facing origin token used by `show --json`.
    #[must_use]
    pub const fn origin(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Command => "command",
            Self::None => "none",
        }
    }
}

/// One renderer-independent field in the run form.
#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    pub key: String,
    pub label: String,
    pub param_type: ParamType,
    pub delivery: Delivery,
    pub choices: Vec<String>,
    pub default: Option<String>,
    pub help: String,
    pub required: bool,
    pub secret: bool,
    pub env_source: String,
    pub degraded: bool,
    pub multiple: bool,
    pub repeat: bool,
    pub flag: String,
    pub action: String,
    pub env_target: String,
    pub input_binding: bool,
    pub empty_uses_default: bool,
}

impl FormField {
    /// Project a universal parameter declaration into the renderer model.
    #[must_use]
    pub fn from_decl(decl: &ParamDecl) -> Self {
        let mut action = decl.action.clone();
        if action.is_empty()
            && !decl.degraded
            && decl.param_type == ParamType::Boolean
            && decl.delivery == Delivery::Flag
            && !decl.flag.is_empty()
            && !default_truthy(decl.default.as_ref())
        {
            action = "store_true".to_owned();
        }
        Self {
            key: decl.name.clone(),
            label: if decl.prompt.is_empty() {
                decl.name.clone()
            } else {
                decl.prompt.clone()
            },
            param_type: if decl.degraded {
                ParamType::String
            } else {
                decl.param_type
            },
            delivery: decl.delivery,
            choices: decl.choices.clone(),
            default: decl.default.as_ref().map(default_text),
            help: decl.help.clone(),
            required: decl.required,
            secret: decl.secret,
            env_source: decl.env_source.clone(),
            degraded: decl.degraded,
            multiple: decl.multiple,
            repeat: decl.repeat,
            flag: decl.flag.clone(),
            action,
            env_target: decl.env_target.clone(),
            input_binding: decl.binding == Binding::Input,
            empty_uses_default: false,
        }
    }

    /// Stable value-source token used by machine interfaces.
    #[must_use]
    pub const fn source(&self) -> &'static str {
        self.delivery.as_str()
    }

    /// Stable type token used by machine interfaces.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.param_type.as_str()
    }

    /// Whether clearing this field deliberately delivers an empty string instead of
    /// falling back to the script's own default.
    #[must_use]
    pub fn delivers_empty(&self) -> bool {
        self.default.is_some()
            && !self.secret
            && !self.degraded
            && !self.multiple
            && !self.input_binding
            && !self.empty_uses_default
            && matches!(self.param_type, ParamType::String | ParamType::Path)
            && matches!(
                self.delivery,
                Delivery::Inject | Delivery::Flag | Delivery::Env
            )
    }
}

/// A complete renderer-independent form description.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormPlan {
    pub source: PlanSource,
    pub fields: Vec<FormField>,
    pub degraded_reason: String,
    pub drift: bool,
}

impl FormPlan {
    /// Names whose values may never be persisted.
    #[must_use]
    pub fn secret_names(&self) -> BTreeSet<String> {
        self.fields
            .iter()
            .filter(|field| field.secret)
            .map(|field| field.key.clone())
            .collect()
    }
}

/// Build the parser-free portion of an entry's form plan.
///
/// Command placeholders and hand-declared `[[parameters]]` rows need no language
/// parser, so they are available even when parser-backed capabilities are absent.
#[must_use]
pub fn plan_for_entry(entry: &Entry) -> FormPlan {
    let Some(spec) = spec_for(&entry.meta.kind) else {
        return FormPlan::default();
    };
    let declared = entry
        .meta
        .parameters
        .as_deref()
        .map(declared_from_meta)
        .unwrap_or_default();

    if spec.placeholder_params && spec.stored_name.is_empty() {
        return command_plan(&entry.meta.params, &declared);
    }

    let fields = declared
        .iter()
        .filter(|decl| matches!(decl.delivery, Delivery::Flag | Delivery::Env))
        .map(FormField::from_decl)
        .collect::<Vec<_>>();
    if fields.is_empty() {
        FormPlan::default()
    } else {
        FormPlan {
            source: PlanSource::Declared,
            fields,
            ..FormPlan::default()
        }
    }
}

fn command_plan(placeholders: &Option<Vec<String>>, declared: &[ParamDecl]) -> FormPlan {
    let names = placeholders.as_deref().unwrap_or_default();
    let placeholder_set = names.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let by_name = declared
        .iter()
        .map(|decl| (decl.name.as_str(), decl))
        .collect::<BTreeMap<_, _>>();
    let mut resolved = Vec::new();
    for name in names {
        if let Some(decl) = by_name.get(name.as_str())
            && decl.delivery == Delivery::Placeholder
        {
            resolved.push((*decl).clone());
        } else {
            resolved.push(synthesized_placeholder(name));
        }
    }
    resolved.extend(
        declared
            .iter()
            .filter(|decl| {
                decl.delivery == Delivery::Env && !placeholder_set.contains(decl.name.as_str())
            })
            .cloned(),
    );
    FormPlan {
        source: PlanSource::Command,
        fields: resolved.iter().map(FormField::from_decl).collect(),
        ..FormPlan::default()
    }
}

/// Merge definition defaults, last-used values, and a named preset. Secret fields
/// are never prefilled even if stale plaintext exists in an older state file.
#[must_use]
pub fn prefill(
    plan: &FormPlan,
    state: &EntryState,
    preset: Option<&str>,
) -> BTreeMap<String, String> {
    let keys = plan
        .fields
        .iter()
        .map(|field| field.key.as_str())
        .collect::<BTreeSet<_>>();
    let secret = plan
        .fields
        .iter()
        .filter(|field| field.secret)
        .map(|field| field.key.as_str())
        .collect::<BTreeSet<_>>();
    let mut output = BTreeMap::new();
    for field in &plan.fields {
        if let Some(default) = &field.default
            && !field.secret
        {
            output.insert(field.key.clone(), default.clone());
        }
    }
    output.extend(
        state
            .values
            .iter()
            .filter(|(key, _)| keys.contains(key.as_str()) && !secret.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    if let Some(preset_name) = preset
        && let Some(values) = state.presets.get(preset_name)
    {
        output.extend(
            values
                .iter()
                .filter(|(key, _)| keys.contains(key.as_str()) && !secret.contains(key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    output
}

/// A bad explicit value key. Explicit input must never be silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError {
    pub key: String,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown parameter: {}", self.key)
    }
}

impl std::error::Error for ResolveError {}

/// Resolve one run's values using `default < last-used < preset < explicit`.
///
/// Explicit values may include secrets: they participate in this in-memory run but
/// persistence remains the state layer's responsibility.
///
/// # Errors
///
/// Returns an error if an explicit key is not present in the current form plan.
pub fn resolve_values(
    plan: &FormPlan,
    state: &EntryState,
    preset: Option<&str>,
    explicit: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ResolveError> {
    let keys = plan
        .fields
        .iter()
        .map(|field| field.key.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(key) = explicit.keys().find(|key| !keys.contains(key.as_str())) {
        return Err(ResolveError { key: key.clone() });
    }
    let mut output = prefill(plan, state, preset);
    output.extend(explicit.clone());
    Ok(output)
}

/// Validate one resolved value map. The returned map is keyed by field name and is
/// empty when the form can submit.
#[must_use]
pub fn validate_values(
    plan: &FormPlan,
    values: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    for field in &plan.fields {
        let value = values.get(&field.key).map_or("", String::as_str);
        if let Some(error) = validate_value(field, value) {
            errors.insert(field.key.clone(), error);
        }
    }
    errors
}

fn validate_value(field: &FormField, value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return field
            .required
            .then(|| format!("{} is required.", field.label));
    }
    let pieces = if field.multiple {
        shellish_split(value)
    } else {
        vec![value.to_owned()]
    };
    match field.param_type {
        ParamType::Integer => pieces.iter().find_map(|piece| {
            piece.parse::<i64>().err().map(|_| {
                format!(
                    "{} needs a whole number — you typed {value:?}.",
                    field.label
                )
            })
        }),
        ParamType::Float => pieces.iter().find_map(|piece| match piece.parse::<f64>() {
            Ok(number) if number.is_finite() => None,
            Ok(_) | Err(_) => Some(format!(
                "{} needs a number — you typed {value:?}.",
                field.label
            )),
        }),
        ParamType::Boolean => pieces.iter().find_map(|piece| {
            (!is_bool_text(piece))
                .then(|| format!("{} needs on or off — you typed {value:?}.", field.label))
        }),
        ParamType::Choice
            if !field.choices.is_empty() && !field.choices.iter().any(|v| v == value) =>
        {
            Some(format!(
                "{} must be one of: {}",
                field.label,
                field.choices.join(", ")
            ))
        }
        ParamType::String | ParamType::Choice | ParamType::Path => None,
    }
}

pub(crate) fn shellish_split(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match (quote, character) {
            (None, '\\') => escaped = true,
            (None, '\'' | '"') => quote = Some(character),
            (Some(active), current_char) if active == current_char => quote = None,
            (None, current_char) if current_char.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            (_, current_char) => current.push(current_char),
        }
    }
    if escaped || quote.is_some() {
        return vec![value.to_owned()];
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

pub(crate) fn is_bool_text(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "y" | "on" | "false" | "0" | "no" | "n" | "off"
    )
}

fn default_text(default: &ParamDefault) -> String {
    match default {
        ParamDefault::Boolean(value) => {
            if *value {
                "true".to_owned()
            } else {
                "false".to_owned()
            }
        }
        _ => default.to_string(),
    }
}

fn default_truthy(default: Option<&ParamDefault>) -> bool {
    match default {
        Some(ParamDefault::Boolean(value)) => *value,
        Some(ParamDefault::Integer(value)) => *value != 0,
        Some(ParamDefault::Float(value)) => *value != 0.0,
        Some(ParamDefault::String(value)) => {
            is_bool_text(value)
                && !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "false" | "0" | "no" | "n" | "off"
                )
        }
        None => false,
    }
}
