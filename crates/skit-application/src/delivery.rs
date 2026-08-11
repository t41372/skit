//! Frontend-neutral routing of prepared parameter values to execution surfaces.
//!
//! This layer deliberately does not read the filesystem, process environment, clock, or terminal.
//! Token expansion, globbing, secret-source resolution, and validation prepare values before this
//! boundary; this module only turns those prepared values into the exact argv/env/injection/template
//! shapes a launcher consumes and the independently masked shapes a frontend may display.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};
use skit_i18n::{Localize, Message};
use thiserror::Error;

use crate::form_state::delivers_empty;

const MASK: &str = "•••";

/// One value after token/glob preparation but before routing to a runtime delivery channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedValue {
    /// A scalar form value.
    Scalar(String),
    /// Already-split values for a multi-value flag or positional.
    Multiple(Vec<String>),
}

/// Delivery-ready execution material plus independently masked transparency surfaces.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Assembly {
    /// Child argv tail, including caller-supplied extra arguments.
    pub args: Vec<String>,
    /// `args` with secret values masked for dry-run/transparency output.
    pub masked_args: Vec<String>,
    /// Values to inject into a temporary source representation.
    pub inject_values: BTreeMap<String, String>,
    /// Values used to fill command/prompt placeholders.
    pub command_values: BTreeMap<String, String>,
    /// Placeholder values with secrets masked.
    pub masked_command_values: BTreeMap<String, String>,
    /// Environment overlay keyed by the target variable name.
    pub env_values: BTreeMap<String, String>,
    /// Environment overlay with secrets masked.
    pub masked_env: BTreeMap<String, String>,
    /// Injection transparency rows; an explicit empty string renders as `''`.
    pub display: Vec<(String, String)>,
}

/// A prepared value shape cannot be represented by its declaration.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AssemblyError {
    /// Multiple pieces reached a scalar field. Guessing a join would change user intent.
    #[error("parameter {name:?} received multiple values but is not a multi-value flag")]
    UnexpectedMultiple {
        /// Parameter name whose prepared shape was invalid.
        name: String,
    },
}

impl Localize for AssemblyError {
    fn message(&self) -> Message {
        match self {
            Self::UnexpectedMultiple { name } => {
                Message::new("parameter {} received multiple values but is not a multi-value flag")
                    .quoted(name)
            }
        }
    }
}

/// Route prepared field values into child-process delivery channels.
///
/// Positionals are emitted before option flags, matching the Python compatibility contract, and
/// caller-supplied extra arguments remain last. Real execution values and masked display values are
/// built in parallel so a secret never has to be reconstructed from already-rendered output.
pub fn assemble(
    declarations: &[ParamDecl],
    values: &BTreeMap<String, PreparedValue>,
    extra_args: &[String],
) -> Result<Assembly, AssemblyError> {
    let mut output = Assembly::default();
    let mut positionals = Vec::new();
    let mut masked_positionals = Vec::new();
    let mut flags = Vec::new();
    let mut masked_flags = Vec::new();

    for declaration in declarations {
        match declaration.delivery {
            ParameterDelivery::Flag => route_flag(
                declaration,
                values.get(&declaration.name),
                &mut positionals,
                &mut masked_positionals,
                &mut flags,
                &mut masked_flags,
            )?,
            ParameterDelivery::Inject => {
                let value = scalar_value(declaration, values.get(&declaration.name))?;
                if !value.is_empty() || delivers_empty(declaration) {
                    output
                        .inject_values
                        .insert(declaration.name.clone(), value.to_owned());
                    output.display.push((
                        declaration.name.clone(),
                        display_value(declaration.secret, value),
                    ));
                }
            }
            ParameterDelivery::Env => {
                let value = scalar_value(declaration, values.get(&declaration.name))?;
                if !value.is_empty() || delivers_empty(declaration) {
                    let target = declaration.env_var().to_owned();
                    output.env_values.insert(target.clone(), value.to_owned());
                    output
                        .masked_env
                        .insert(target, masked_value(declaration.secret, value));
                }
            }
            ParameterDelivery::Placeholder => {
                let value = scalar_value(declaration, values.get(&declaration.name))?;
                output
                    .command_values
                    .insert(declaration.name.clone(), value.to_owned());
                output.masked_command_values.insert(
                    declaration.name.clone(),
                    masked_value(declaration.secret, value),
                );
            }
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

/// Build the localized-safe semantic lines that disclose a launch before it starts.
///
/// The command is already masked by the runtime planner. Injection rows use the independently
/// masked display values from [`Assembly`], so a frontend never has to reconstruct secrets from
/// execution data.
#[must_use]
pub fn transparency_messages(assembly: &Assembly, command: &str) -> Vec<Message> {
    let mut messages = Vec::new();
    if !assembly.display.is_empty() {
        let pairs = assembly
            .display
            .iter()
            .map(|(name, value)| format!("{name} = {value}"))
            .collect::<Vec<_>>()
            .join(", ");
        messages.push(Message::new("→ inject: {}").with(pairs));
        messages.push(Message::new(
            "  (written to a temporary copy, deleted after the run; your original file is untouched)",
        ));
    }
    messages.push(Message::new("→ {}").with(command));
    messages
}

fn route_flag(
    declaration: &ParamDecl,
    prepared: Option<&PreparedValue>,
    positionals: &mut Vec<String>,
    masked_positionals: &mut Vec<String>,
    flags: &mut Vec<String>,
    masked_flags: &mut Vec<String>,
) -> Result<(), AssemblyError> {
    if declaration.parameter_type == ParameterType::Bool {
        let value = scalar_value(declaration, prepared)?;
        let fired = truthy(value);
        if !declaration.flag.is_empty()
            && ((declaration.action == "store_true" && fired)
                || (declaration.action == "store_false" && !fired))
        {
            flags.push(declaration.flag.clone());
            masked_flags.push(declaration.flag.clone());
        }
        return Ok(());
    }

    let pieces = flag_pieces(declaration, prepared)?;
    if pieces.is_empty()
        || (pieces.len() == 1 && pieces[0].is_empty() && !delivers_empty(declaration))
    {
        return Ok(());
    }
    let masked_pieces = pieces
        .iter()
        .map(|value| masked_value(declaration.secret, value))
        .collect::<Vec<_>>();

    if declaration.flag.is_empty() {
        positionals.extend(pieces);
        masked_positionals.extend(masked_pieces);
    } else if declaration.repeat {
        for (piece, masked_piece) in pieces.into_iter().zip(masked_pieces) {
            flags.push(declaration.flag.clone());
            flags.push(piece);
            masked_flags.push(declaration.flag.clone());
            masked_flags.push(masked_piece);
        }
    } else {
        flags.push(declaration.flag.clone());
        flags.extend(pieces);
        masked_flags.push(declaration.flag.clone());
        masked_flags.extend(masked_pieces);
    }
    Ok(())
}

fn scalar_value<'a>(
    declaration: &ParamDecl,
    prepared: Option<&'a PreparedValue>,
) -> Result<&'a str, AssemblyError> {
    match prepared {
        Some(PreparedValue::Scalar(value)) => Ok(value),
        Some(PreparedValue::Multiple(_)) => Err(AssemblyError::UnexpectedMultiple {
            name: declaration.name.clone(),
        }),
        None => Ok(""),
    }
}

fn flag_pieces(
    declaration: &ParamDecl,
    prepared: Option<&PreparedValue>,
) -> Result<Vec<String>, AssemblyError> {
    match prepared {
        Some(PreparedValue::Multiple(values)) if declaration.multiple => Ok(values.clone()),
        Some(PreparedValue::Multiple(_)) => Err(AssemblyError::UnexpectedMultiple {
            name: declaration.name.clone(),
        }),
        Some(PreparedValue::Scalar(value)) => {
            if declaration.multiple && value.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![value.clone()])
            }
        }
        None => Ok(Vec::new()),
    }
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "y" | "on"
    )
}

fn masked_value(secret: bool, value: &str) -> String {
    if secret && !value.is_empty() {
        MASK.to_owned()
    } else {
        value.to_owned()
    }
}

fn display_value(secret: bool, value: &str) -> String {
    if secret {
        MASK.to_owned()
    } else if value.is_empty() {
        "''".to_owned()
    } else {
        value.to_owned()
    }
}
