#![forbid(unsafe_code)]

use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde::Serialize;
use skit_core::{LibraryRoots, Store};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Parser)]
#[command(
    name = "skit",
    about = "A launcher and parameter manager for scripts and prompts."
)]
struct Cli {
    #[arg(long, short = 'V', global = true, help = "Show version")]
    version: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List every registered entry.
    List {
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Serialize)]
struct ListRow {
    name: String,
    slug: String,
    kind: String,
    mode: String,
    description: String,
    missing: bool,
    last_run_at: Option<String>,
    last_exit: Option<i32>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    if cli.version {
        println!("skit {VERSION}");
        return Ok(());
    }

    let store = Store::new(resolve_roots()?);
    match cli.command {
        Some(Command::List { json }) => list(&store, json),
        None => skit_tui::run(&store).map_err(|error| error.to_string()),
    }
}

fn list(store: &Store, as_json: bool) -> Result<(), String> {
    let entries = store.list().map_err(|error| error.to_string())?;
    if as_json {
        let rows = entries
            .into_iter()
            .map(|entry| {
                let missing = entry.target_missing();
                let last = store.last_run(&entry.slug);
                ListRow {
                    name: entry.name,
                    slug: entry.slug,
                    kind: entry.kind,
                    mode: entry.mode,
                    description: entry.description,
                    missing,
                    last_run_at: last.as_ref().map(|run| run.at.clone()),
                    last_exit: last.map(|run| run.exit),
                }
            })
            .collect::<Vec<_>>();
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        serde_json::to_writer(&mut writer, &rows).map_err(|error| error.to_string())?;
        writeln!(writer).map_err(|error| error.to_string())?;
        return Ok(());
    }

    if entries.is_empty() {
        println!("No entries yet. Add one with: skit add <path>");
        return Ok(());
    }
    println!("Name\tKind\tDescription");
    for entry in entries {
        let description = if entry.description.is_empty() {
            "—"
        } else {
            &entry.description
        };
        println!("{}\t{}\t{description}", entry.name, entry.kind);
    }
    Ok(())
}

fn resolve_roots() -> Result<LibraryRoots, String> {
    if let (Some(data), Some(state), Some(config)) = (
        env_root("SKIT_DATA_DIR"),
        env_root("SKIT_STATE_DIR"),
        env_root("SKIT_CONFIG_DIR"),
    ) {
        return Ok(LibraryRoots::new(data, state, config));
    }

    #[cfg(target_os = "windows")]
    {
        let root = env_root("LOCALAPPDATA")
            .ok_or_else(|| "Cannot find the user data directory.".to_owned())?;
        return Ok(LibraryRoots::new(
            env_root("SKIT_DATA_DIR").unwrap_or_else(|| root.join("skit")),
            env_root("SKIT_STATE_DIR").unwrap_or_else(|| root.join("skit")),
            env_root("SKIT_CONFIG_DIR").unwrap_or_else(|| root.join("skit")),
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let home = env_root("HOME").ok_or_else(|| "Cannot find the home directory.".to_owned())?;
        let root = home
            .join("Library")
            .join("Application Support")
            .join("skit");
        return Ok(LibraryRoots::new(
            env_root("SKIT_DATA_DIR").unwrap_or_else(|| root.clone()),
            env_root("SKIT_STATE_DIR").unwrap_or_else(|| root.clone()),
            env_root("SKIT_CONFIG_DIR").unwrap_or(root),
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let home = env_root("HOME").ok_or_else(|| "Cannot find the home directory.".to_owned())?;
        let data = env_root("SKIT_DATA_DIR").unwrap_or_else(|| {
            env_root("XDG_DATA_HOME")
                .unwrap_or_else(|| home.join(".local/share"))
                .join("skit")
        });
        let state = env_root("SKIT_STATE_DIR").unwrap_or_else(|| {
            env_root("XDG_STATE_HOME")
                .unwrap_or_else(|| home.join(".local/state"))
                .join("skit")
        });
        let config = env_root("SKIT_CONFIG_DIR").unwrap_or_else(|| {
            env_root("XDG_CONFIG_HOME")
                .unwrap_or_else(|| home.join(".config"))
                .join("skit")
        });
        Ok(LibraryRoots::new(data, state, config))
    }
}

fn env_root(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
