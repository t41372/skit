#![forbid(unsafe_code)]

mod add_command;
mod preset_command;

use std::io::{self, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde::Serialize;
use skit_core::{
    Entry, Family, FormField, StateStore, Store, discover_roots, plan_for_entry, spec_for,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
pub(crate) struct CliFailure {
    message: String,
    code: u8,
}

impl CliFailure {
    pub(crate) fn operational(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }

    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }
}

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
    /// Add an existing script or executable to the library.
    Add(add_command::AddArgs),
    /// List every registered entry.
    List {
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show metadata and the complete machine-facing parameter schema.
    Show {
        /// Entry name or slug.
        name: String,
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
    Preset(preset_command::PresetArgs),
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

#[derive(Debug, Serialize)]
struct ShowField {
    key: String,
    label: String,
    #[serde(rename = "type")]
    type_name: &'static str,
    source: &'static str,
    required: bool,
    secret: bool,
    multiple: bool,
    repeat: bool,
    degraded: bool,
    choices: Vec<String>,
    default: Option<String>,
    help: String,
    flag: String,
    action: String,
    env_source: String,
    delivers_empty: bool,
}

#[derive(Debug, Serialize)]
struct ShowRow {
    name: String,
    slug: String,
    kind: String,
    mode: String,
    description: String,
    source: String,
    workdir: String,
    interpreter: Option<String>,
    missing: bool,
    dependencies: Vec<String>,
    requires_python: String,
    needs: Vec<String>,
    template: Option<String>,
    param_source: &'static str,
    param_origin: &'static str,
    degraded_reason: String,
    drift: bool,
    fields: Vec<ShowField>,
    presets: Vec<String>,
    last_run_at: Option<String>,
    last_exit: Option<i32>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("{}", failure.message);
            ExitCode::from(failure.code)
        }
    }
}

fn run() -> Result<(), CliFailure> {
    let cli = Cli::parse();
    if cli.version {
        println!("skit {VERSION}");
        return Ok(());
    }

    let store =
        Store::new(discover_roots().map_err(|error| CliFailure::operational(error.to_string()))?);
    match cli.command {
        Some(Command::Add(args)) => add_command::run(&store, args),
        Some(Command::List { json }) => list(&store, json).map_err(CliFailure::operational),
        Some(Command::Show { name, json }) => {
            show(&store, &name, json).map_err(CliFailure::operational)
        }
        Some(Command::Remove { name, yes }) => {
            remove(&store, &name, yes).map_err(CliFailure::operational)
        }
        Some(Command::Rename { name, new_name }) => {
            rename(&store, &name, &new_name).map_err(CliFailure::operational)
        }
        Some(Command::Describe { name, text }) => {
            describe(&store, &name, &text).map_err(CliFailure::operational)
        }
        Some(Command::Preset(args)) => preset_command::run(&store, args),
        None => skit_tui::run(&store).map_err(|error| CliFailure::operational(error.to_string())),
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
        write_json(&rows)?;
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

fn show(store: &Store, name: &str, as_json: bool) -> Result<(), String> {
    let entry = store.resolve(name).map_err(|error| error.to_string())?;
    let plan = plan_for_entry(&entry);
    let state_store = StateStore::new(store.roots().clone());
    let state = state_store.load(&entry.slug);
    let last_run_at = state.last_run.as_ref().map(|run| run.at.clone());
    let last_exit = state.last_run.as_ref().map(|run| run.exit);
    let mut presets = state.presets.keys().cloned().collect::<Vec<_>>();
    presets.sort();

    if !as_json {
        println!(
            "{}  ({} · {})",
            entry.meta.name, entry.meta.kind, entry.meta.mode
        );
        if !entry.meta.description.is_empty() {
            println!("  {}", entry.meta.description);
        }
        if !entry.meta.source.is_empty() {
            println!("  Source: {}", entry.meta.source);
        }
        if plan.fields.is_empty() {
            println!("  No form fields");
        } else {
            for field in &plan.fields {
                println!("  {}\t{}\t{}", field.key, field.type_name(), field.source());
            }
        }
        println!("  Run it: skit run {}", entry.meta.name);
        return Ok(());
    }

    let row = ShowRow {
        name: entry.meta.name.clone(),
        slug: entry.slug.clone(),
        kind: entry.meta.kind.clone(),
        mode: entry.meta.mode.clone(),
        description: entry.meta.description.clone(),
        source: entry.meta.source.clone(),
        workdir: entry.meta.workdir.clone(),
        interpreter: nonempty(&entry.meta.interpreter),
        missing: target_missing(&entry),
        dependencies: entry.meta.dependencies.clone().unwrap_or_default(),
        requires_python: entry.meta.requires_python.clone(),
        needs: entry.meta.needs.clone().unwrap_or_default(),
        template: nonempty(&entry.meta.template),
        param_source: plan.source.as_str(),
        param_origin: plan.source.origin(),
        degraded_reason: plan.degraded_reason,
        drift: plan.drift,
        fields: plan.fields.iter().map(show_field).collect(),
        presets,
        last_run_at,
        last_exit,
    };
    write_json(&row)
}

fn show_field(field: &FormField) -> ShowField {
    ShowField {
        key: field.key.clone(),
        label: field.label.clone(),
        type_name: field.type_name(),
        source: field.source(),
        required: field.required,
        secret: field.secret,
        multiple: field.multiple,
        repeat: field.repeat,
        degraded: field.degraded,
        choices: field.choices.clone(),
        default: field.default.clone(),
        help: field.help.clone(),
        flag: field.flag.clone(),
        action: field.action.clone(),
        env_source: field.env_source.clone(),
        delivers_empty: field.delivers_empty(),
    }
}

fn target_missing(entry: &Entry) -> bool {
    let Some(spec) = spec_for(&entry.meta.kind) else {
        return false;
    };
    spec.family != Family::Template && !entry.script_path().exists()
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn write_json(value: &impl Serialize) -> Result<(), String> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, value).map_err(|error| error.to_string())?;
    writeln!(writer).map_err(|error| error.to_string())
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
