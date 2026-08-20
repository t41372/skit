//! Frontend-neutral hygiene for edits to hand-declared parameter rows.

use skit_domain::parameters::{DeclaredEditWarning, ParamDecl, finish_declared_parameter_edit};
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
    match finish_declared_parameter_edit(declaration) {
        Ok(()) => Ok(()),
        Err(DeclaredEditWarning::BoolFlagOnByDefault { name }) => {
            Err(ParameterEditError::BoolFlagOnByDefault { name })
        }
        Err(warning) => unreachable!("row finalizer returned {}", warning.code()),
    }
}
