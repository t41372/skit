use std::collections::{BTreeMap, BTreeSet};

use clap::Args;
use skit_core::{
    DeclaredEdits, Delivery, ParamType, StateStore, Store,
};

use crate::CliFailure;

#[derive(Debug, Args)]
pub(crate) struct ParamsArgs {
    /// Entry name or slug.
    pub(crate) name: String,

    /// Output the final entry schema as JSON.
    #[arg(long)]
    pub(crate) json: bool,

    /// Declare a parameter by hand. Repeatable.
    #[arg(long = "add", value_name = "NAME")]
    add: Vec<String>,

    /// Remove a declared parameter. Repeatable.
    #[arg(long = "rm", value_name = "NAME")]
    remove: Vec<String>,

    /// Set a parameter type: NAME=str|int|float|bool|choice|path.
    #[arg(long = "type", value_name = "NAME=TYPE")]
    types: Vec<String>,

    /// Set a declared default: NAME=VALUE.
    #[arg(long = "default", value_name = "NAME=VALUE")]
    defaults: Vec<String>,

    /// Set choice values: NAME=a,b,c.
    #[arg(long = "choices", value_name = "NAME=a,b,c")]
    choices: Vec<String>,

    /// Set delivery: NAME=env|flag|placeholder.
    #[arg(long = "deliver", value_name = "NAME=DELIVERY")]
    deliveries: Vec<String>,

    /// Set the argv flag, or NAME= for positional delivery.
    #[arg(long = "flag", value_name = "NAME=FLAG")]
    flags: Vec<String>,

    /// Mark a field required. Repeatable.
    #[arg(long = "required", value_name = "NAME")]
    required: Vec<String>,

    /// Mark a field optional. Repeatable.
    #[arg(long = "optional", value_name = "NAME")]
    optional: Vec<String>,

    /// Set field help text: NAME=TEXT.
    #[arg(long = "help-text", value_name = "NAME=TEXT")]
    help_texts: Vec<String>,

    /// Set the form label/prompt: NAME=TEXT.
    #[arg(long = "prompt", value_name = "NAME=TEXT")]
    prompts: Vec<String>,

    /// Mark a field secret. Repeatable.
    #[arg(long = "secret", value_name = "NAME")]
    secret: Vec<String>,

    /// Clear a field's secret marking. Repeatable.
    #[arg(long = "no-secret", value_name = "NAME")]
    no_secret: Vec<String>,

    /// Read a secret value from an environment variable: NAME=ENVVAR; empty clears.
    #[arg(long = "env-source", value_name = "NAME=ENVVAR")]
    env_sources: Vec<String>,
}

pub(crate) fn run(store: &Store, args: ParamsArgs) -> Result<(), CliFailure> {
    let entry = store
        .resolve(&args.name)
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    if !has_edits(&args) {
        return crate::show(store, &entry.slug, args.json).map_err(CliFailure::operational);
    }
    if !matches!(entry.meta.kind.as_str(), "command" | "exe") {
        return Err(CliFailure::usage(format!(
            "Declared parameter editing is not enabled for {} entries yet.",
            entry.meta.kind
        )));
    }

    let mut edits = build_edits(&args)?;
    if entry.meta.kind == "command" {
        edits.allowed_deliveries = vec![Delivery::Placeholder, Delivery::Env];
        edits.placeholder_names = entry
            .meta
            .params
            .as_deref()
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();
    } else {
        edits.allowed_deliveries = vec![Delivery::Flag, Delivery::Env];
    }

    let state = StateStore::new(store.roots().clone());
    let requested_secrets = edits.secret.clone();
    if !requested_secrets.is_empty() {
        state
            .purge_secret(&entry.slug, &requested_secrets)
            .map_err(|error| CliFailure::operational(error.to_string()))?;
    }

    let (updated, result) = store
        .edit_parameters(&entry.slug, &edits)
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    let final_secrets = result
        .decls
        .iter()
        .filter(|decl| decl.secret)
        .map(|decl| decl.name.clone())
        .collect::<BTreeSet<_>>();
    if !final_secrets.is_empty() {
        state
            .purge_secret(&updated.slug, &final_secrets)
            .map_err(|error| CliFailure::operational(error.to_string()))?;
    }

    for warning in result.warnings {
        eprintln!("Warning: {warning}");
    }
    crate::show(store, &updated.slug, args.json).map_err(CliFailure::operational)
}

fn has_edits(args: &ParamsArgs) -> bool {
    !args.add.is_empty()
        || !args.remove.is_empty()
        || !args.types.is_empty()
        || !args.defaults.is_empty()
        || !args.choices.is_empty()
        || !args.deliveries.is_empty()
        || !args.flags.is_empty()
        || !args.required.is_empty()
        || !args.optional.is_empty()
        || !args.help_texts.is_empty()
        || !args.prompts.is_empty()
        || !args.secret.is_empty()
        || !args.no_secret.is_empty()
        || !args.env_sources.is_empty()
}

fn build_edits(args: &ParamsArgs) -> Result<DeclaredEdits, CliFailure> {
    let mut edits = DeclaredEdits {
        add: clean_names(&args.add, "--add")?,
        remove: clean_names(&args.remove, "--rm")?,
        required: clean_names(&args.required, "--required")?.into_iter().collect(),
        optional: clean_names(&args.optional, "--optional")?.into_iter().collect(),
        secret: clean_names(&args.secret, "--secret")?.into_iter().collect(),
        no_secret: clean_names(&args.no_secret, "--no-secret")?.into_iter().collect(),
        ..DeclaredEdits::default()
    };

    for (name, value) in assignments(&args.types, "--type")? {
        let param_type = match value.as_str() {
            "str" => ParamType::String,
            "int" => ParamType::Integer,
            "float" => ParamType::Float,
            "bool" => ParamType::Boolean,
            "choice" => ParamType::Choice,
            "path" => ParamType::Path,
            _ => {
                return Err(CliFailure::usage(format!(
                    "Unknown parameter type for {name}: {value}."
                )));
            }
        };
        edits.types.insert(name, param_type);
    }
    edits.defaults = assignment_map(&args.defaults, "--default")?;
    for (name, value) in assignments(&args.choices, "--choices")? {
        edits.choices.insert(
            name,
            value
                .split(',')
                .map(str::trim)
                .filter(|choice| !choice.is_empty())
                .map(str::to_owned)
                .collect(),
        );
    }
    for (name, value) in assignments(&args.deliveries, "--deliver")? {
        let delivery = match value.as_str() {
            "env" => Delivery::Env,
            "flag" => Delivery::Flag,
            "placeholder" => Delivery::Placeholder,
            _ => {
                return Err(CliFailure::usage(format!(
                    "Unknown delivery for {name}: {value}."
                )));
            }
        };
        edits.deliveries.insert(name, delivery);
    }
    edits.flags = assignment_map(&args.flags, "--flag")?;
    edits.help = assignment_map(&args.help_texts, "--help-text")?;
    edits.prompts = assignment_map(&args.prompts, "--prompt")?;
    edits.env_sources = assignment_map(&args.env_sources, "--env-source")?;
    Ok(edits)
}

fn clean_names(values: &[String], option: &str) -> Result<Vec<String>, CliFailure> {
    values
        .iter()
        .map(|value| {
            let name = value.trim();
            if name.is_empty() {
                Err(CliFailure::usage(format!("{option} requires a name.")))
            } else {
                Ok(name.to_owned())
            }
        })
        .collect()
}

fn assignment_map(values: &[String], option: &str) -> Result<BTreeMap<String, String>, CliFailure> {
    Ok(assignments(values, option)?.into_iter().collect())
}

fn assignments(
    values: &[String],
    option: &str,
) -> Result<Vec<(String, String)>, CliFailure> {
    values
        .iter()
        .map(|value| {
            let Some((name, assigned)) = value.split_once('=') else {
                return Err(CliFailure::usage(format!(
                    "{option} expects NAME=VALUE."
                )));
            };
            let name = name.trim();
            if name.is_empty() {
                return Err(CliFailure::usage(format!(
                    "{option} expects a non-empty name."
                )));
            }
            Ok((name.to_owned(), assigned.to_owned()))
        })
        .collect()
}
