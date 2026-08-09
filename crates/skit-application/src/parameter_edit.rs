//! Frontend-neutral hygiene for edits to hand-declared parameter rows.

use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType, ParameterValue};
use skit_i18n::{Localize, Message};
use thiserror::Error;

/// A requested parameter-row edit cannot produce a truthful form control.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ParameterEditError {
    /// An on-by-default boolean needs a distinct flag that turns it off.
    #[error("{name} is on by default, so its flag cannot turn it off")]
    BoolFlagOnByDefault {
        /// Stable field name.
        name: String,
    },
}

impl Localize for ParameterEditError {
    fn message(&self) -> Message {
        match self {
            Self::BoolFlagOnByDefault { name } => Message::new(
                "{} is on by default, so its flag could only ever turn it on again. Declare the flag that turns it OFF instead (--no-{} and the like), with default false.",
            )
            .with(name)
            .with(name),
        }
    }
}

/// Finish one edited declaration without inventing a control that cannot change the program.
///
/// Parsed, hand-edited data stays open and lossless. This function applies only at an explicit
/// edit boundary. The caller keeps the complete previous row when this function refuses it.
pub fn finish_parameter_edit(declaration: &mut ParamDecl) -> Result<(), ParameterEditError> {
    if declaration.parameter_type == ParameterType::Bool
        && declaration.delivery == ParameterDelivery::Flag
        && !declaration.flag.is_empty()
        && declaration.action.is_empty()
    {
        if declaration.default.as_ref().is_some_and(value_truthy) {
            return Err(ParameterEditError::BoolFlagOnByDefault {
                name: declaration.name.clone(),
            });
        }
        declaration.action = "store_true".to_owned();
    }
    if declaration.parameter_type != ParameterType::Bool {
        declaration.action.clear();
    }
    Ok(())
}

fn value_truthy(value: &ParameterValue) -> bool {
    match value {
        ParameterValue::String(value) => !value.is_empty(),
        ParameterValue::Integer(value) => *value != 0,
        ParameterValue::Float(value) => *value != 0.0,
        ParameterValue::Bool(value) => *value,
    }
}
