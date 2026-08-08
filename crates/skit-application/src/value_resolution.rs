//! Resolve raw form input into per-field scalar values before validation and delivery assembly.
//!
//! User-entered secrets deliberately bypass token expansion: a credential containing braces is
//! data, not a template. An empty secret may read one explicitly configured environment source.
//! Non-secret values use the shared token engine, with placeholder delivery retaining doubled
//! braces while the other delivery channels use normal brace escapes.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterDelivery};
use thiserror::Error;

use crate::tokens::{TokenContext, TokenError, expand};

/// A raw field value could not be resolved without guessing or silently losing information.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ValueResolutionError {
    /// A secret declared an environment fallback that is absent from the launch environment.
    #[error("{name} reads from the environment variable {environment}, but it isn't set.")]
    MissingSecretEnvironment {
        /// Stable parameter name.
        name: String,
        /// Required environment variable name.
        environment: String,
    },
    /// A non-secret value contained a known token that could not be resolved.
    #[error(transparent)]
    Token(#[from] TokenError),
}

/// Resolve every declared field using only explicitly supplied launch context.
///
/// Missing submitted keys are equivalent to an empty field. Submitted keys absent from the current
/// declaration set are ignored, which prevents stale state from leaking into a newer form schema.
pub fn resolve_values(
    declarations: &[ParamDecl],
    raw_values: &BTreeMap<String, String>,
    context: &TokenContext,
) -> Result<BTreeMap<String, String>, ValueResolutionError> {
    declarations
        .iter()
        .map(|declaration| {
            let raw = raw_values
                .get(&declaration.name)
                .map(String::as_str)
                .unwrap_or_default();
            resolve_one(declaration, raw, context)
                .map(|value| (declaration.name.clone(), value))
        })
        .collect()
}

fn resolve_one(
    declaration: &ParamDecl,
    raw: &str,
    context: &TokenContext,
) -> Result<String, ValueResolutionError> {
    if declaration.secret {
        if !raw.is_empty() {
            return Ok(raw.to_owned());
        }
        if declaration.env_source.is_empty() {
            return Ok(String::new());
        }
        return context
            .env
            .get(&declaration.env_source)
            .cloned()
            .ok_or_else(|| ValueResolutionError::MissingSecretEnvironment {
                name: declaration.name.clone(),
                environment: declaration.env_source.clone(),
            });
    }

    if raw.is_empty() {
        return Ok(String::new());
    }
    expand(
        raw,
        context,
        declaration.delivery != ParameterDelivery::Placeholder,
    )
    .map_err(Into::into)
}
