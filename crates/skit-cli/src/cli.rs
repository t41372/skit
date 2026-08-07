use std::{
    env,
    fs::{self, File, Metadata},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};
use skit_application::{
    CreateEntry, EntryPayload, ExitClass, LibraryService, RepositoryError, SourcePermissions,
};
use skit_domain::{EntryKind, StorageMode};
use skit_store::FileStore;
use skit_ui::LibraryState;
use thiserror::Error;

#[cfg(test)]
mod tests;

/// Run the real command-line entry point and return its stable process status.
#[must_use]
pub fn entry() -> u8 {
    let cli = Cli::parse();
    match execute(cli) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            error.exit_code()
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "skit",
    version,
    about = "A script, prompt, program, and command library",
    disable_help_subcommand = true
)]
struct Cli {
    /// Override the skit data directory (the parent of scripts/ and registry.toml).
    #[arg(long, global = true, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List entries in the library.
    List {
        /// Emit the stable machine-readable contract.
        #[arg(long)]
        json: bool,
    },
    /// Show one entry by exact slug or exact display name.
    Show {
        /// Entry slug or display name.
        selector: String,
        /// Emit the stable machine-readable contract.
        #[arg(long)]
        json: bool,
    },
    /// Add one file as a copied or referenced entry.
    Add {
        /// Source file to register.
        source: PathBuf,
        /// Open entry-kind registry key.
        #[arg(long)]
        kind: String,
        /// Display name; defaults to the source stem.
        #[arg(long)]
        name: Option<String>,
        /// Reference the original instead of storing a copy.
        #[arg(long)]
        reference: bool,
    },
    /// Replace one entry's description.
    Describe {
        /// Entry slug or display name.
        selector: String,
        /// Replacement description.
        description: String,
    },
    /// Rename one entry and derive its new slug.
    Rename {
        /// Entry slug or display name.
        selector: String,
        /// Replacement display name.
        name: String,
    },
    /// Remove one entry.
    Remove {
        /// Entry slug or display name.
        selector: String,
        /// Confirm the destructive operation.
        #[arg(long)]
        yes: bool,
    },
    /// Open the Ratatui library browser explicitly.
    Tui,
}

fn execute(cli: Cli) -> Result<(), CliError> {
    let data_dir = resolve_data_dir(cli.data_dir)?;
    let service = LibraryService::new(FileStore::new(data_dir));
    match cli.command {
        Some(Command::List { json }) => list(&service, json),
        Some(Command::Show { selector, json }) => show(&service, &selector, json),
        Some(Command::Add {
            source,
            kind,
            name,
            reference,
        }) => add(&service, &source, &kind, name, reference),
        Some(Command::Describe {
            selector,
            description,
        }) => describe(&service, &selector, &description),
        Some(Command::Rename { selector, name }) => rename(&service, &selector, &name),
        Some(Command::Remove { selector, yes }) => remove(&service, &selector, yes),
        Some(Command::Tui) | None => tui(&service),
    }
}

fn list(service: &LibraryService<FileStore>, json: bool) -> Result<(), CliError> {
    let scan = service.list()?;
    if json {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        serde_json::to_writer(&mut output, &scan)?;
        writeln!(output)?;
    } else {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        for entry in &scan.entries {
            writeln!(
                output,
                "{}\t{}\t{}",
                entry.name, entry.kind, entry.description
            )?;
        }
        let stderr = io::stderr();
        let mut errors = stderr.lock();
        for diagnostic in &scan.diagnostics {
            writeln!(errors, "warning: {}", diagnostic.message)?;
        }
    }
    Ok(())
}

fn show(service: &LibraryService<FileStore>, selector: &str, json: bool) -> Result<(), CliError> {
    let entry = service.show(selector)?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if json {
        serde_json::to_writer(&mut output, &entry)?;
        writeln!(output)?;
    } else {
        writeln!(output, "{} ({})", entry.meta.name, entry.slug)?;
        writeln!(output, "kind: {}", entry.meta.kind)?;
        writeln!(output, "mode: {}", mode_name(entry.meta.mode))?;
        if !entry.meta.description.is_empty() {
            writeln!(output, "{}", entry.meta.description)?;
        }
    }
    Ok(())
}

fn add(
    service: &LibraryService<FileStore>,
    source: &Path,
    kind: &str,
    name: Option<String>,
    reference: bool,
) -> Result<(), CliError> {
    let source =
        fs::canonicalize(source).map_err(|error| source_error("resolve", source, error))?;
    let (bytes, permissions) = read_source(&source)?;
    let name = name.unwrap_or_else(|| source_default_name(&source));
    let kind =
        EntryKind::parse(kind.to_owned()).map_err(|error| RepositoryError::InvalidMutation {
            reason: error.to_string(),
        })?;
    let stored_name = stored_name(kind.as_str(), &source);
    let mode = if reference {
        StorageMode::Reference
    } else {
        StorageMode::Copy
    };
    let entry = service.add(CreateEntry {
        name,
        kind,
        mode,
        source: source.display().to_string(),
        workdir: if reference { "origin" } else { "invoke" }.to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes,
            stored_name: Some(stored_name),
            permissions,
        }),
    })?;
    println!("Added: {} ({})", entry.meta.name, entry.slug);
    Ok(())
}

fn describe(
    service: &LibraryService<FileStore>,
    selector: &str,
    description: &str,
) -> Result<(), CliError> {
    let held = service.show(selector)?;
    let claimed = service.claim_identity(&held)?;
    let entry = service.describe(&claimed, description)?;
    println!("Described: {} ({})", entry.meta.name, entry.slug);
    Ok(())
}

fn rename(service: &LibraryService<FileStore>, selector: &str, name: &str) -> Result<(), CliError> {
    let held = service.show(selector)?;
    let claimed = service.claim_identity(&held)?;
    let entry = service.rename(&claimed, name)?;
    println!("Renamed: {} ({})", entry.meta.name, entry.slug);
    Ok(())
}

fn remove(service: &LibraryService<FileStore>, selector: &str, yes: bool) -> Result<(), CliError> {
    if !yes {
        return Err(CliError::ConfirmationRequired);
    }
    let held = service.show(selector)?;
    let claimed = service.claim_identity(&held)?;
    let name = service.remove(&claimed)?;
    println!("Removed: {name}");
    Ok(())
}

fn tui(service: &LibraryService<FileStore>) -> Result<(), CliError> {
    let state = LibraryState::from_scan(service.list()?);
    skit_tui::run(state, || service.list())?;
    Ok(())
}

fn read_source(path: &Path) -> Result<(Vec<u8>, SourcePermissions), CliError> {
    let mut file = File::open(path).map_err(|error| source_error("open", path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| source_error("inspect", path, error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| source_error("read", path, error))?;
    Ok((bytes, source_permissions(&metadata)))
}

fn source_default_name(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("script")
        .to_owned()
}

fn stored_name(kind: &str, source: &Path) -> String {
    match kind {
        "python" => "script.py".to_owned(),
        "shell" => "script.sh".to_owned(),
        "js" => "script.js".to_owned(),
        "ts" => "script.ts".to_owned(),
        "fish" => "script.fish".to_owned(),
        "powershell" => "script.ps1".to_owned(),
        "ruby" => "script.rb".to_owned(),
        "perl" => "script.pl".to_owned(),
        "lua" => "script.lua".to_owned(),
        "r" => "script.R".to_owned(),
        _ => source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("script")
            .to_owned(),
    }
}

fn source_error(operation: &'static str, path: &Path, source: io::Error) -> CliError {
    CliError::Source {
        operation,
        path: path.display().to_string(),
        source,
    }
}

#[cfg(unix)]
fn source_permissions(metadata: &Metadata) -> SourcePermissions {
    use std::os::unix::fs::PermissionsExt as _;

    SourcePermissions {
        readonly: metadata.permissions().readonly(),
        unix_mode: Some(metadata.permissions().mode() & 0o777),
    }
}

#[cfg(not(unix))]
fn source_permissions(metadata: &Metadata) -> SourcePermissions {
    SourcePermissions {
        readonly: metadata.permissions().readonly(),
        unix_mode: None,
    }
}

fn resolve_data_dir(override_dir: Option<PathBuf>) -> Result<PathBuf, CliError> {
    if let Some(path) = override_dir {
        return Ok(path);
    }
    if let Some(path) = env::var_os("SKIT_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    platform_data_dir().ok_or(CliError::DataDirectoryUnavailable)
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .map(PathBuf::from)
        .map(|path| path.join("skit"))
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).map(|path| {
        path.join("Library")
            .join("Application Support")
            .join("skit")
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_data_dir() -> Option<PathBuf> {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .map(|path| path.join("skit"))
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".local").join("share").join("skit"))
        })
}

#[cfg(not(any(unix, target_os = "windows")))]
fn platform_data_dir() -> Option<PathBuf> {
    None
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("could not encode JSON output: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not write output: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Tui(#[from] skit_tui::TuiError),
    #[error("confirmation is required; pass --yes to remove the entry")]
    ConfirmationRequired,
    #[error("could not {operation} {path}: {source}")]
    Source {
        operation: &'static str,
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("could not determine the platform data directory; pass --data-dir or SKIT_DATA_DIR")]
    DataDirectoryUnavailable,
}

impl CliError {
    const fn exit_code(&self) -> u8 {
        match self {
            Self::Repository(error) => error.exit_class().code(),
            Self::ConfirmationRequired => ExitClass::Usage.code(),
            Self::Json(_)
            | Self::Io(_)
            | Self::Tui(_)
            | Self::Source { .. }
            | Self::DataDirectoryUnavailable => ExitClass::Skit.code(),
        }
    }
}

const fn mode_name(mode: StorageMode) -> &'static str {
    match mode {
        StorageMode::Copy => "copy",
        StorageMode::Reference => "reference",
    }
}
