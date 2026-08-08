//! Validate resolved form values and prepare scalar/multi shapes for delivery assembly.
//!
//! This layer stays presentation- and filesystem-free. It applies the universal scalar contract,
//! reproduces the existing POSIX `shlex.split` multi-value grammar, and deliberately leaves glob
//! expansion to a later cwd/filesystem adapter.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterType, coerce_default};
use thiserror::Error;

use crate::{delivery::PreparedValue, tokens::has_tokens};

/// A form value cannot satisfy its current declaration.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ValuePreparationError {
    /// A required raw field was absent or contained only whitespace.
    #[error("{label} is required.")]
    Required {
        /// Stable field key.
        name: String,
        /// User-facing label captured at validation time.
        label: String,
    },
    /// A scalar or one member of a multi-value field failed strict typed coercion.
    #[error("parameter {name:?} has invalid {parameter_type:?} value {value:?}")]
    InvalidType {
        /// Stable field key.
        name: String,
        /// Value that failed, after token resolution when applicable.
        value: String,
        /// Declared scalar type.
        parameter_type: ParameterType,
    },
    /// A choice field did not match one of the declared values exactly.
    #[error("parameter {name:?} must be one of {choices:?}; got {value:?}")]
    InvalidChoice {
        /// Stable field key.
        name: String,
        /// Rejected value.
        value: String,
        /// Allowed values in declared order.
        choices: Vec<String>,
    },
}

/// Validate raw/resolved values and prepare their delivery shapes.
///
/// `raw_values` controls requiredness and whether type checking had to wait for token expansion.
/// `resolved_values` is the output of [`crate::value_resolution::resolve_values`]. Unknown stale
/// keys are ignored because iteration is declaration-driven.
pub fn prepare_values(
    declarations: &[ParamDecl],
    raw_values: &BTreeMap<String, String>,
    resolved_values: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, PreparedValue>, ValuePreparationError> {
    declarations
        .iter()
        .map(|declaration| {
            let raw = raw_values
                .get(&declaration.name)
                .map(String::as_str)
                .unwrap_or_default();
            let resolved = resolved_values
                .get(&declaration.name)
                .map(String::as_str)
                .unwrap_or_default();
            prepare_one(declaration, raw, resolved)
                .map(|prepared| (declaration.name.clone(), prepared))
        })
        .collect()
}

fn prepare_one(
    declaration: &ParamDecl,
    raw: &str,
    resolved: &str,
) -> Result<PreparedValue, ValuePreparationError> {
    if raw.trim().is_empty() {
        if declaration.required {
            return Err(ValuePreparationError::Required {
                name: declaration.name.clone(),
                label: label(declaration).to_owned(),
            });
        }
    } else if !declaration.degraded {
        // A token-like spelling is validated after expansion. Secrets are the exception: the
        // credential is literal data and value_resolution intentionally never expands it.
        if !has_tokens(raw) {
            validate_type(declaration, raw)?;
        } else if !declaration.secret {
            validate_type(declaration, resolved)?;
        }
    }

    if declaration.multiple {
        if resolved.is_empty() {
            Ok(PreparedValue::Multiple(Vec::new()))
        } else {
            Ok(PreparedValue::Multiple(split_multi(resolved)))
        }
    } else {
        Ok(PreparedValue::Scalar(resolved.to_owned()))
    }
}

fn validate_type(declaration: &ParamDecl, value: &str) -> Result<(), ValuePreparationError> {
    if declaration.parameter_type == ParameterType::Choice {
        if !declaration.choices.is_empty()
            && !declaration.choices.iter().any(|choice| choice == value)
        {
            return Err(ValuePreparationError::InvalidChoice {
                name: declaration.name.clone(),
                value: value.to_owned(),
                choices: declaration.choices.clone(),
            });
        }
        return Ok(());
    }

    if !matches!(
        declaration.parameter_type,
        ParameterType::Int | ParameterType::Float | ParameterType::Bool
    ) {
        return Ok(());
    }

    let pieces = if declaration.multiple {
        split_multi(value)
    } else {
        vec![value.to_owned()]
    };
    for piece in pieces {
        if coerce_default(&piece, declaration.parameter_type).is_err() {
            return Err(ValuePreparationError::InvalidType {
                name: declaration.name.clone(),
                value: piece,
                parameter_type: declaration.parameter_type,
            });
        }
    }
    Ok(())
}

fn split_multi(value: &str) -> Vec<String> {
    shlex::split(value).unwrap_or_else(|| vec![value.to_owned()])
}

fn label(declaration: &ParamDecl) -> &str {
    if declaration.prompt.is_empty() {
        &declaration.name
    } else {
        &declaration.prompt
    }
}
