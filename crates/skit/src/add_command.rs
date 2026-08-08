use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::SystemTime;

use clap::Args;
use skit_core::{
    AddFileRequest, AddMode, AddPreparation, AddUseCaseError, PythonAutoAddError,
    PythonAutoAddRequest, Store, add_file, add_python_auto, format_utc_timestamp, infer_kind,
    spec_for,
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

    /// Never open an interactive review. Python dependency suggestions are accepted;
    /// newly detected managed parameters stay unmanaged.
    #[arg(long = "no-input")]
    pub(crate) no_input: bool,
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
    if kind == "python" {
        return add_python(store, args, added_at);
    }

    let request = AddFileRequest {
        source: args.path,
        name: args.name,
        kind: Some(kind),
        mode: mode(args.reference),
        description: args.description,
        workdir: None,
        interpreter: None,
        preparation: AddPreparation { added_at },
    };
    let entry = add_file(store, request).map_err(classify_add_error)?;
    print_entry_summary(&entry);
    Ok(())
}

fn add_python(store: &Store, args: AddArgs, added_at: String) -> Result<(), CliFailure> {
    let outcome = add_python_auto(
        store,
        PythonAutoAddRequest {
            source: args.path,
            name: args.name,
            mode: mode(args.reference),
            description: args.description,
            workdir: None,
            added_at,
            interactive: io::stdin().is_terminal() && io::stdout().is_terminal(),
            no_input: args.no_input,
        },
    )
    .map_err(classify_python_auto_error)?;

    print_entry_summary(&outcome.entry);
    if !outcome.dependencies.is_empty() {
        println!("  Dependencies: {}", outcome.dependencies.join(", "));
    }
    if !outcome.requires_python.is_empty() {
        println!("  Python constraint: {}", outcome.requires_python);
    }
    if !outcome.parameter_candidates.is_empty() {
        println!(
            "  Parameter candidates left unmanaged: {}",
            outcome.parameter_candidates.join(", ")
        );
    }
    Ok(())
}

fn print_entry_summary(entry: &skit_core::Entry) {
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
}

const fn mode(reference: bool) -> AddMode {
    if reference {
        AddMode::Reference
    } else {
        AddMode::Copy
    }
}

fn classify_python_auto_error(error: PythonAutoAddError) -> CliFailure {
    match error {
        PythonAutoAddError::ReviewRequired { .. } => CliFailure::usage(error.to_string()),
        PythonAutoAddError::Add(source) => classify_add_error(source),
    }
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
