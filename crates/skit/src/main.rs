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
#[command(name = "skit", about = "A launcher and parameter manager for scripts and prompts.")]
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
        None => Err("The Ratatui workbench is not available in this development slice.".to_owned()),
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
    let data = env_root("SKIT_DATA_DIR")
        .or_else(dirs::data_local_dir)
        .map(|path| append_app(path, env::var_os("SKIT_DATA_DIR").is_none()))
        .ok_or_else(|| "Cannot find the user data directory.".to_owned())?;
    let state = env_root("SKIT_STATE_DIR")
        .or_else(dirs::state_dir)
        .or_else(dirs::data_local_dir)
        .map(|path| append_app(path, env::var_os("SKIT_STATE_DIR").is_none()))
        .ok_or_else(|| "Cannot find the user state directory.".to_owned())?;
    let config = env_root("SKIT_CONFIG_DIR")
        .or_else(dirs::config_local_dir)
        .map(|path| append_app(path, env::var_os("SKIT_CONFIG_DIR").is_none()))
        .ok_or_else(|| "Cannot find the user configuration directory.".to_owned())?;
    Ok(LibraryRoots::new(data, state, config))
}

fn env_root(name: &str) -> Option<PathBuf> {
    env::var_os(name).map(PathBuf::from)
}

fn append_app(mut path: PathBuf, append: bool) -> PathBuf {
    if append {
        path.push("skit");
    }
    path
}
