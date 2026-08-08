//! Final frontend-neutral run-input pipeline.
//!
//! Every interactive or non-interactive frontend feeds the same raw field map and argument tail
//! through the same ordered stages: resolve ambient values, validate and split, expand filesystem
//! globs through an injected adapter, then route to argv/env/injection/template delivery surfaces.

use std::collections::BTreeMap;

use skit_domain::parameters::ParamDecl;
use thiserror::Error;

use crate::{
    delivery::{Assembly, AssemblyError, assemble},
    glob_expansion::{GlobExpander, expand_multi_values, prepare_extra_args},
    tokens::{TokenContext, TokenError},
    value_preparation::{ValuePreparationError, prepare_values},
    value_resolution::{ValueResolutionError, resolve_values},
};

/// One ordered run-input stage refused the launch material.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RunInputError {
    /// A field secret source or field token could not be resolved.
    #[error(transparent)]
    Resolution(#[from] ValueResolutionError),
    /// A required/type/choice or multi-value contract failed.
    #[error(transparent)]
    Preparation(#[from] ValuePreparationError),
    /// A token in a raw extra-argument tail could not be resolved.
    #[error("{0}")]
    ExtraToken(TokenError),
    /// Prepared shapes could not be routed to their declared delivery surface.
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
}

/// Convert raw frontend inputs into delivery-ready launch material.
pub fn assemble_run_inputs<G: GlobExpander>(
    declarations: &[ParamDecl],
    raw_values: &BTreeMap<String, String>,
    extra_args: &[String],
    expand_extra: bool,
    context: &TokenContext,
    glob: &G,
) -> Result<Assembly, RunInputError> {
    let resolved = resolve_values(declarations, raw_values, context)?;
    let prepared = prepare_values(declarations, raw_values, &resolved)?;
    let prepared = expand_multi_values(declarations, &prepared, glob);
    let extra_args = prepare_extra_args(extra_args, context, expand_extra, glob)
        .map_err(RunInputError::ExtraToken)?;
    assemble(declarations, &prepared, &extra_args).map_err(Into::into)
}
