use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;

use crate::forms::{FormField, FormPlan, is_bool_text, shellish_split};
use crate::{Delivery, ParamType, validate_values};

/// Delivery-ready material shared by CLI, Ratatui, and a future GUI frontend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assembly {
    pub args: Vec<String>,
    pub masked_args: Vec<String>,
    pub inject_values: BTreeMap<String, String>,
    pub masked_inject: BTreeMap<String, String>,
    pub command_values: BTreeMap<String, String>,
    pub masked_command_values: BTreeMap<String, String>,
    pub env_values: BTreeMap<String, String>,
    pub masked_env: BTreeMap<String, String>,
}

/// A form could not be turned into delivery material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblyError {
    InvalidValues(BTreeMap<String, String>),
    MissingSecretEnvironment { field: String, variable: String },
}

impl fmt::Display for AssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValues(errors) => {
                let rendered = errors
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(formatter, "invalid form values: {rendered}")
            }
            Self::MissingSecretEnvironment { field, variable } => write!(
                formatter,
                "{field} reads from the environment variable {variable}, but it is not set"
            ),
        }
    }
}

impl StdError for AssemblyError {}

/// Route already-resolved form values into argv, environment, injection, and command
/// placeholder channels. This stage does not spawn a process and never writes secrets.
///
/// # Errors
///
/// Returns an error when typed validation fails or a secret field names an environment
/// source that is absent.
pub fn assemble_delivery(
    plan: &FormPlan,
    values: &BTreeMap<String, String>,
    extra_args: &[String],
    environment: &BTreeMap<String, String>,
) -> Result<Assembly, AssemblyError> {
    let errors = validate_values(plan, values);
    if !errors.is_empty() {
        return Err(AssemblyError::InvalidValues(errors));
    }

    let mut output = Assembly::default();
    let mut positionals = Vec::new();
    let mut flags = Vec::new();
    let mut masked_positionals = Vec::new();
    let mut masked_flags = Vec::new();

    for field in &plan.fields {
        let raw = values.get(&field.key).map_or("", String::as_str);
        let value = resolve_secret(field, raw, environment)?;
        match field.delivery {
            Delivery::Inject => assemble_inject(field, &value, &mut output),
            Delivery::Env => assemble_env(field, &value, &mut output),
            Delivery::Placeholder => assemble_placeholder(field, &value, &mut output),
            Delivery::Flag => assemble_flag(
                field,
                &value,
                &mut positionals,
                &mut flags,
                &mut masked_positionals,
                &mut masked_flags,
            ),
        }
    }

    output.args = positionals;
    output.args.extend(flags);
    output.args.extend(extra_args.iter().cloned());
    output.masked_args = masked_positionals;
    output.masked_args.extend(masked_flags);
    output.masked_args.extend(extra_args.iter().cloned());
    Ok(output)
}

fn resolve_secret(
    field: &FormField,
    raw: &str,
    environment: &BTreeMap<String, String>,
) -> Result<String, AssemblyError> {
    if !field.secret || !raw.is_empty() {
        return Ok(raw.to_owned());
    }
    if field.env_source.is_empty() {
        return Ok(String::new());
    }
    environment.get(&field.env_source).cloned().ok_or_else(|| {
        AssemblyError::MissingSecretEnvironment {
            field: field.label.clone(),
            variable: field.env_source.clone(),
        }
    })
}

fn assemble_inject(field: &FormField, value: &str, output: &mut Assembly) {
    if value.is_empty() && !field.delivers_empty() {
        return;
    }
    output
        .inject_values
        .insert(field.key.clone(), value.to_owned());
    output.masked_inject.insert(
        field.key.clone(),
        if field.secret && !value.is_empty() {
            "•••".to_owned()
        } else {
            value.to_owned()
        },
    );
}

fn assemble_env(field: &FormField, value: &str, output: &mut Assembly) {
    if value.is_empty() && !field.delivers_empty() {
        return;
    }
    let target = if field.env_target.is_empty() {
        field.key.clone()
    } else {
        field.env_target.clone()
    };
    output.env_values.insert(target.clone(), value.to_owned());
    output.masked_env.insert(
        target,
        if field.secret && !value.is_empty() {
            "•••".to_owned()
        } else {
            value.to_owned()
        },
    );
}

fn assemble_placeholder(field: &FormField, value: &str, output: &mut Assembly) {
    output
        .command_values
        .insert(field.key.clone(), value.to_owned());
    output.masked_command_values.insert(
        field.key.clone(),
        if field.secret && !value.is_empty() {
            "•••".to_owned()
        } else {
            value.to_owned()
        },
    );
}

fn assemble_flag(
    field: &FormField,
    value: &str,
    positionals: &mut Vec<String>,
    flags: &mut Vec<String>,
    masked_positionals: &mut Vec<String>,
    masked_flags: &mut Vec<String>,
) {
    if field.param_type == ParamType::Boolean {
        if field.flag.is_empty() {
            return;
        }
        let fired = bool_value(value);
        if (field.action == "store_true" && fired) || (field.action == "store_false" && !fired) {
            flags.push(field.flag.clone());
            masked_flags.push(field.flag.clone());
        }
        return;
    }
    if value.is_empty() && !field.delivers_empty() {
        return;
    }

    let pieces = if field.multiple {
        shellish_split(value)
    } else {
        vec![value.to_owned()]
    };
    let masked_pieces = if field.secret && !value.is_empty() {
        vec!["•••".to_owned()]
    } else {
        pieces.clone()
    };

    if field.flag.is_empty() {
        positionals.extend(pieces);
        masked_positionals.extend(masked_pieces);
    } else if field.repeat {
        for piece in pieces {
            flags.push(field.flag.clone());
            flags.push(piece);
        }
        for piece in masked_pieces {
            masked_flags.push(field.flag.clone());
            masked_flags.push(piece);
        }
    } else {
        flags.push(field.flag.clone());
        flags.extend(pieces);
        masked_flags.push(field.flag.clone());
        masked_flags.extend(masked_pieces);
    }
}

fn bool_value(value: &str) -> bool {
    is_bool_text(value)
        && matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "y" | "on"
        )
}
