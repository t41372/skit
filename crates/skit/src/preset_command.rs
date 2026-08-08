use std::io::{self, Write};

use clap::{Args, Subcommand};
use serde::Serialize;
use skit_core::{PresetFromLastError, StateStore, Store, plan_for_entry, save_preset_from_last};

use crate::CliFailure;

#[derive(Debug, Args)]
pub(crate) struct PresetArgs {
    #[command(subcommand)]
    command: PresetCommand,
}

#[derive(Debug, Subcommand)]
enum PresetCommand {
    /// Save a named preset. The Rust rewrite currently exposes the exact `--from-last` lane.
    Save {
        /// Entry name or slug.
        name: String,
        /// Preset name.
        preset_name: String,
        /// Capture the exact accepted values from the last run.
        #[arg(long)]
        from_last: bool,
    },
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

pub(crate) fn run(store: &Store, args: PresetArgs) -> Result<(), CliFailure> {
    let state = StateStore::new(store.roots().clone());
    match args.command {
        PresetCommand::Save {
            name,
            preset_name,
            from_last,
        } => save(store, &state, &name, &preset_name, from_last),
        PresetCommand::List { name, json } => list(store, &state, &name, json),
        PresetCommand::Delete { name, preset_name } => delete(store, &state, &name, &preset_name),
    }
}

fn save(
    store: &Store,
    state: &StateStore,
    name: &str,
    preset_name: &str,
    from_last: bool,
) -> Result<(), CliFailure> {
    if !from_last {
        return Err(CliFailure::usage(
            "Interactive preset capture is not enabled in the Rust rewrite yet; pass --from-last.",
        ));
    }
    let entry = store
        .resolve(name)
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    let plan = plan_for_entry(&entry);
    match save_preset_from_last(state, &entry.slug, preset_name, &plan) {
        Ok(_) => {
            println!("Preset \"{preset_name}\" saved for {}.", entry.meta.name);
            Ok(())
        }
        Err(PresetFromLastError::NoFields) => Err(CliFailure::usage(format!(
            "{} has no form fields, so there's nothing to save.",
            entry.meta.name
        ))),
        Err(PresetFromLastError::NoRememberedValues) => Err(CliFailure::operational(format!(
            "{} has no remembered values yet — run it once first.",
            entry.meta.name
        ))),
        Err(PresetFromLastError::State(error)) => Err(CliFailure::operational(error.to_string())),
    }
}

fn list(store: &Store, state: &StateStore, name: &str, as_json: bool) -> Result<(), CliFailure> {
    let entry = store
        .resolve(name)
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    let presets = state.load(&entry.slug).presets;
    if as_json {
        write_json(&presets).map_err(CliFailure::operational)?;
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

fn delete(
    store: &Store,
    state: &StateStore,
    name: &str,
    preset_name: &str,
) -> Result<(), CliFailure> {
    let entry = store
        .resolve(name)
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    if state
        .delete_preset(&entry.slug, preset_name)
        .map_err(|error| CliFailure::operational(error.to_string()))?
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
    Err(CliFailure::operational(format!(
        "Unknown preset \"{preset_name}\". Available: {}",
        if available.is_empty() {
            "—"
        } else {
            &available
        }
    )))
}

fn write_json(value: &impl Serialize) -> Result<(), String> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, value).map_err(|error| error.to_string())?;
    writeln!(writer).map_err(|error| error.to_string())
}
