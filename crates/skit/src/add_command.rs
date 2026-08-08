use std::path::PathBuf;
use std::time::SystemTime;

use clap::Args;
use skit_core::{
    AddFileRequest, AddMode, AddPreparation, AddUseCaseError, Store, add_file,
    format_utc_timestamp, infer_kind, spec_for,
};

use crate::CliFailure;

/// Arguments for the ordinary on-disk file intake lane.
#[derive(Debug, Args)]
pub(crate) struct AddArgs {
    /// Path to an existing script or executable.
    pub(crate) path: PathBuf,

    /// Name / alias. Defaults to the file name.
    #[arg(long, short = 'n')]
    pub(crate) name: Option<String>,

    /// Description. Inferred from a leading comment when omitted.
    #[arg(long, short = 'd')]
    pub(crate) description: Option<String>,

    /// Reference the original file instead of copying it.
    #[arg(long = "ref")]
    pub(crate) reference: bool,

    /// Force executable/program kind.
    #[arg(long)]
    pub(crate) exe: bool,

    /// Force a registered language kind for an extensionless file.
    #[arg(long)]
    pub(crate) kind: Option<String>,
}

pub(crate) fn run(store: &Store, args: AddArgs) -> Result<(), CliFailure> {
    if args.exe && args.kind.is_some() {
        return Err(CliFailure::usage("Use --kind or --exe, not both."));
    }

    let kind = if args.exe {
        "exe".to_owned()
    } else if let Some(kind) = &args.kind {
        kind.clone()
    } else {
        infer_kind(&args.path, false).to_owned()
    };
    match kind.as_str() {
        "unknown" => {
            return Err(CliFailure::usage(
                "Cannot determine the entry kind — pass --kind <language> or --exe.",
            ));
        }
        "python" => {
            return Err(CliFailure::usage(
                "Python intake is not enabled in the Rust rewrite yet; its PEP 723 and parameter onboarding must land together.",
            ));
        }
        "prompt" => {
            return Err(CliFailure::usage(
                "Prompt intake is not enabled in the Rust rewrite yet; use the dedicated prompt lane once it lands.",
            ));
        }
        "command" => {
            return Err(CliFailure::usage(
                "Command templates do not use a source-file lane.",
            ));
        }
        _ if spec_for(&kind).is_none() => {
            return Err(CliFailure::usage(format!("Unknown kind: {kind}.")));
        }
        _ => {}
    }

    let added_at = format_utc_timestamp(SystemTime::now())
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    let request = AddFileRequest {
        source: args.path,
        name: args.name,
        kind: Some(kind),
        mode: if args.reference {
            AddMode::Reference
        } else {
            AddMode::Copy
        },
        description: args.description,
        workdir: None,
        interpreter: None,
        preparation: AddPreparation { added_at },
    };
    let entry = add_file(store, request).map_err(classify_add_error)?;
    let spec = spec_for(&entry.meta.kind);
    if spec.is_some_and(|spec| spec.supports_modes) {
        println!("Added: {} ({} mode)", entry.meta.name, entry.meta.mode);
    } else {
        println!("Added: {}", entry.meta.name);
    }
    if !entry.meta.description.is_empty() {
        println!("  Description: {}", entry.meta.description);
    }
    println!("  Run it: skit run {}", entry.meta.name);
    Ok(())
}

fn classify_add_error(error: AddUseCaseError) -> CliFailure {
    let message = error.to_string();
    match error {
        AddUseCaseError::UnknownKind | AddUseCaseError::UnsupportedKind(_) => {
            CliFailure::usage(message)
        }
        AddUseCaseError::SourceNotFile(_)
        | AddUseCaseError::Io { .. }
        | AddUseCaseError::Store(_) => CliFailure::operational(message),
    }
}
