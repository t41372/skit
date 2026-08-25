//! Define where glob expansion can occur.
//!
//! The application layer selects the values that can use glob syntax.
//! A file-system adapter returns the matches for the launch directory.
//! CLI, Ratatui, and Tauri can use the same rules.

use std::{collections::BTreeMap, fmt::Debug};

use skit_domain::parameters::ParamDecl;

use crate::{
    delivery::PreparedValue,
    tokens::{TokenContext, TokenError, expand},
};

/// Expand one path pattern in the configured launch directory.
pub trait GlobExpander: Debug {
    /// Return matches in a stable order.
    /// Return the input value when there is no match or the pattern is invalid.
    fn expand_piece(&self, piece: &str) -> Vec<String>;
}

/// Expand glob syntax only in fields that accept multiple values.
///
/// The value preparation step already used POSIX shell splitting.
/// This function does not split a value again.
#[must_use]
pub fn expand_multi_values<G: GlobExpander>(
    declarations: &[ParamDecl],
    prepared: &BTreeMap<String, PreparedValue>,
    glob: &G,
) -> BTreeMap<String, PreparedValue> {
    let multiple_names = declarations
        .iter()
        .filter(|declaration| declaration.multiple)
        .map(|declaration| declaration.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    prepared
        .iter()
        .map(|(name, value)| {
            let expanded = if multiple_names.contains(name.as_str()) {
                match value {
                    PreparedValue::Multiple(pieces) => PreparedValue::Multiple(
                        pieces
                            .iter()
                            .flat_map(|piece| glob.expand_piece(piece))
                            .collect(),
                    ),
                    PreparedValue::Scalar(value) => PreparedValue::Scalar(value.clone()),
                }
            } else {
                value.clone()
            };
            (name.clone(), expanded)
        })
        .collect()
}

/// Prepare the extra arguments for one launch.
///
/// A raw tail expands tokens first and glob syntax second.
/// Each vector item is already one argument, so this function does not split it.
/// A literal tail bypasses both expansion steps.
pub fn prepare_extra_args<G: GlobExpander>(
    extra_args: &[String],
    context: &TokenContext,
    expand_extra: bool,
    glob: &G,
) -> Result<Vec<String>, TokenError> {
    if !expand_extra {
        return Ok(extra_args.to_vec());
    }

    let mut output = Vec::new();
    for argument in extra_args {
        let expanded = expand(argument, context, true)?;
        output.extend(glob.expand_piece(&expanded));
    }
    Ok(output)
}
