#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde::Serialize;
use skit_core::{StateStore, Store, discover_roots};

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
    /// Remove a registered entry. The original source file stays unchanged.
    Remove {
        /// Entry name or slug.
        name: String,
        /// Skip confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Rename an entry. Presets, remembered values, and history stay with it.
    Rename {
        /// Entry name or slug.
        name: String,
        /// New display name.
        new_name: String,
    },
    /// Set the description shown in the library and `skit list`.
    Describe {
        /// Entry name or slug.
        name: String,
        /// New description. Use an empty string to clear it.
        text: String,
    },
    /// Manage named parameter presets for an entry.
    Preset {
        #[command(subcommand)]
        command: PresetCommand,
    },
}

#[derive(Debug, Subcommand)]
enum PresetCommand {
    /// List an entry's saved presets.
    List {
        /// Entry name or slug.
        name: String,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Delete a named preset from an entry.
    Delete {
        /// Entry name or slug.
        name: String,
        /// Preset name.
        preset_name: String,
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

    let store = Store::new(discover_roots().map_err(|error| error.to_string())?);
    match cli.command {
        Some(Command::List { json }) => list(&store, json),
        Some(Command::Remove { name, yes }) => remove(&store, &name, yes),
        Some(Command::Rename { name, new_name }) => rename(&store, &name, &new_name),
        Some(Command::Describe { name, text }) => describe(&store, &name, &text),
        Some(Command::Preset { command }) => preset(&store, command),
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

fn rename(store: &Store, name: &str, new_name: &str) -> Result<(), String> {
    let entry = store
        .rename(name, new_name)
        .map_err(|error| error.to_string())?;
    println!("Renamed to {}.", entry.meta.name);
    Ok(())
}

fn describe(store: &Store, name: &str, text: &str) -> Result<(), String> {
    let entry = store
        .update_description(name, text)
        .map_err(|error| error.to_string())?;
    if entry.meta.description.is_empty() {
        println!("Description cleared for {}.", entry.meta.name);
    } else {
        println!("Description updated for {}.", entry.meta.name);
    }
    Ok(())
}

fn remove(store: &Store, name: &str, yes: bool) -> Result<(), String> {
    if !yes {
        let entry = store.resolve(name).map_err(|error| error.to_string())?;
        let question = if entry.meta.source.is_empty() {
            format!("Remove \"{}\"?", entry.meta.name)
        } else {
            format!(
                "Remove \"{}\"? Your original file will not be deleted.",
                entry.meta.name
            )
        };
        if !confirm(&question)? {
            return Err("Aborted!".to_owned());
        }
    }

    let removed = store.remove(name).map_err(|error| error.to_string())?;
    println!("Removed: {removed}");
    Ok(())
}

fn confirm(question: &str) -> Result<bool, String> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    write!(writer, "{question} [y/N]: ").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())?;

    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn preset(store: &Store, command: PresetCommand) -> Result<(), String> {
    let state = StateStore::new(store.roots().clone());
    match command {
        PresetCommand::List { name, json } => preset_list(store, &state, &name, json),
        PresetCommand::Delete { name, preset_name } => {
            preset_delete(store, &state, &name, &preset_name)
        }
    }
}

fn preset_list(store: &Store, state: &StateStore, name: &str, as_json: bool) -> Result<(), String> {
    let entry = store.resolve(name).map_err(|error| error.to_string())?;
    let presets = state.load(&entry.slug).presets;
    if as_json {
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        serde_json::to_writer(&mut writer, &presets).map_err(|error| error.to_string())?;
        writeln!(writer).map_err(|error| error.to_string())?;
        return Ok(());
    }

    if presets.is_empty() {
        println!(
            "No presets for {} yet. Create one with: skit run {} --save-preset <preset>",
            entry.meta.name, entry.meta.name
        );
        return Ok(());
    }

    for (preset_name, values) in presets {
        let pairs = values
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {preset_name}: {pairs}");
    }
    Ok(())
}

fn preset_delete(
    store: &Store,
    state: &StateStore,
    name: &str,
    preset_name: &str,
) -> Result<(), String> {
    let entry = store.resolve(name).map_err(|error| error.to_string())?;
    if state
        .delete_preset(&entry.slug, preset_name)
        .map_err(|error| error.to_string())?
    {
        println!("Preset \"{preset_name}\" deleted from {}.", entry.meta.name);
        return Ok(());
    }

    let available = state
        .load(&entry.slug)
        .presets
        .into_keys()
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Unknown preset \"{preset_name}\". Available: {}",
        if available.is_empty() {
            "—"
        } else {
            &available
        }
    ))
}
