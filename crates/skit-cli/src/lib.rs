//! Rust composition root and command-line interface for skit.

#![forbid(unsafe_code)]

use std::{
    env,
    io::{self, Write},
    path::PathBuf,
};

use clap::{Parser, Subcommand};
use skit_application::{ExitClass, LibraryService, RepositoryError};
use skit_store::FileStore;
use skit_ui::LibraryState;
use thiserror::Error;

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
    /// Open the Ratatui library browser explicitly.
    Tui,
}

fn execute(cli: Cli) -> Result<(), CliError> {
    let data_dir = resolve_data_dir(cli.data_dir)?;
    let service = LibraryService::new(FileStore::new(data_dir));
    match cli.command {
        Some(Command::List { json }) => list(&service, json),
        Some(Command::Show { selector, json }) => show(&service, &selector, json),
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

fn tui(service: &LibraryService<FileStore>) -> Result<(), CliError> {
    let state = LibraryState::from_scan(service.list()?);
    skit_tui::run(state, || service.list())?;
    Ok(())
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
    #[error("could not determine the platform data directory; pass --data-dir or SKIT_DATA_DIR")]
    DataDirectoryUnavailable,
}

impl CliError {
    const fn exit_code(&self) -> u8 {
        match self {
            Self::Repository(error) => error.exit_class().code(),
            Self::Json(_) | Self::Io(_) | Self::Tui(_) | Self::DataDirectoryUnavailable => {
                ExitClass::Skit.code()
            }
        }
    }
}

const fn mode_name(mode: skit_domain::StorageMode) -> &'static str {
    match mode {
        skit_domain::StorageMode::Copy => "copy",
        skit_domain::StorageMode::Reference => "reference",
    }
}
