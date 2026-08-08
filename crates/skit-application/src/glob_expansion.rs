//! Filesystem-independent orchestration for glob-aware value and extra-tail expansion.
//!
//! The application layer decides *where* glob expansion is allowed, while a filesystem adapter
//! decides which paths match in the launch cwd. This keeps CLI, Ratatui, and future Tauri behavior
//! identical without leaking concrete filesystem APIs into the application crate.

use std::{collections::BTreeMap, fmt::Debug};

use skit_domain::parameters::ParamDecl;

use crate::{
    delivery::PreparedValue,
    tokens::{TokenContext, TokenError, expand},
};

/// Adapter that expands one already-tokenized path pattern relative to its configured cwd.
pub trait GlobExpander: Debug {
    /// Return deterministic matches, or the original piece when there are no matches or the
    /// pattern cannot be interpreted safely.
    fn expand_piece(&self, piece: &str) -> Vec<String>;
}

/// Expand glob syntax only inside fields explicitly declared as multi-value.
///
/// `value_preparation` has already applied POSIX shlex splitting, so this function never performs
/// another shell parse. Scalar fields, unknown stale keys, and unexpected scalar shapes on a
/// multi-value declaration pass through unchanged; shape validation remains the delivery layer's
/// responsibility.
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
                    PreparedValue::Scalar(value) => PreparedValue::Scalar(value.clone()),
                }
            } else {
                value.clone()
            };
            (name.clone(), expanded)
        })
        .collect()
}

/// Prepare a remembered or freshly entered extra-argument tail for this launch.
///
/// A raw launch-menu tail expands each stored item as one unit: token expansion first, then glob
/// expansion. It is intentionally **not** shlex-split because each vector element already denotes
/// one argument. A literal-replay tail bypasses both passes completely.
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
