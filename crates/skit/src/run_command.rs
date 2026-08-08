use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use clap::Args;
use skit_core::{
    AssemblyError, LaunchOptions, LaunchPlan, LaunchPlanError, Platform, PrepareRunError,
    ProgramSearch, RunError, RunRequest, StateStore, Store, format_utc_timestamp,
    load_launch_config, plan_for_entry, prepare_raw_run, prepare_run, remembered_values,
    resolve_extra_args, run_launch,
};

use crate::CliFailure;

#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    /// Entry name or slug.
    pub(crate) name: String,

    /// Set a form value explicitly. Repeatable; the last duplicate wins.
    #[arg(long = "set", value_name = "NAME=VALUE", allow_hyphen_values = true)]
    set: Vec<String>,

    /// Load a named preset before explicit --set values.
    #[arg(long, short = 'p')]
    preset: Option<String>,

    /// Save the accepted values as a preset after launch validation.
    #[arg(long = "save-preset", value_name = "NAME")]
    save_preset: Option<String>,

    /// Validate and print the masked immutable launch snapshot without spawning.
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Bypass forms, remembered values, presets, and remembered arguments.
    #[arg(long)]
    raw: bool,

    /// Never ask for missing values. This Rust slice is headless either way.
    #[arg(long = "no-input")]
    no_input: bool,

    /// Ignore and erase the remembered `--` tail before this run.
    #[arg(long = "forget-args")]
    forget_args: bool,

    /// Arguments after `--`, forwarded to the launched entry.
    #[arg(last = true)]
    extra_args: Vec<String>,
}

pub(crate) fn run(store: &Store, args: RunArgs) -> Result<(), CliFailure> {
    let _headless_contract = args.no_input;
    let entry = store
        .resolve(&args.name)
        .map_err(|error| CliFailure::coded(error.to_string(), 127))?;
    if matches!(entry.meta.kind.as_str(), "command" | "prompt") {
        return Err(CliFailure::coded(
            format!(
                "Run support for {} entries is not enabled in the Rust rewrite yet.",
                entry.meta.kind
            ),
            125,
        ));
    }
    if args.raw && (!args.set.is_empty() || args.preset.is_some() || args.save_preset.is_some()) {
        return Err(CliFailure::usage(
            "--raw cannot be combined with --set, --preset, or --save-preset.",
        ));
    }

    let explicit = parse_set_values(&args.set)?;
    let state_store = StateStore::new(store.roots().clone());
    let mut state = state_store.load(&entry.slug);
    if args.forget_args && !state.extra_args.is_empty() {
        state_store
            .save_last(&entry.slug, None, Some(&[]), &BTreeSet::new())
            .map_err(|error| CliFailure::operational(error.to_string()))?;
        state.extra_args.clear();
    }

    if let Some(preset_name) = args.save_preset.as_deref() {
        if preset_name.trim().is_empty() {
            return Err(CliFailure::usage("Preset name cannot be empty."));
        }
        if plan_for_entry(&entry).fields.is_empty() {
            return Err(CliFailure::usage(format!(
                "{} has no form fields, so there's nothing to save.",
                entry.meta.name
            )));
        }
    }

    let platform = current_platform();
    let config = load_launch_config(store.roots());
    let mut launch_options = LaunchOptions::new(
        platform,
        env::current_dir().map_err(|error| CliFailure::coded(error.to_string(), 125))?,
    );
    launch_options.js_runner = config.js_runner;
    launch_options.windows_bash = config.windows_bash;
    let programs = ProgramSearch::from_environment(platform)
        .with_fallback_path(store.roots().data_dir().join("bin"));

    if args.raw {
        let launch = prepare_raw_run(&entry, &args.extra_args, &launch_options, &programs)
            .map_err(classify_launch_error)?;
        if args.dry_run {
            return print_launch(&launch);
        }
        let code = execute_launch(&launch)?;
        record_raw_run(&state_store, &entry.slug, code)?;
        return finish_with_status(code);
    }

    let extra = resolve_extra_args(&state, &args.extra_args, args.forget_args);
    if extra.replayed {
        eprintln!(
            "Reusing remembered extra arguments: {}",
            extra.args.join(" ")
        );
    }
    let environment = unicode_environment();
    let prepared = prepare_run(
        &entry,
        RunRequest {
            state: &state,
            preset: args.preset.as_deref(),
            explicit: &explicit,
            extra_args: &extra.args,
            environment: &environment,
            launch_options: &launch_options,
        },
        &programs,
    )
    .map_err(classify_prepare_error)?;

    let secret_names = prepared.form.secret_names();
    if args.dry_run {
        if let Some(preset_name) = args.save_preset.as_deref() {
            state_store
                .save_preset(
                    &entry.slug,
                    preset_name.trim(),
                    &prepared.values,
                    &secret_names,
                )
                .map_err(|error| CliFailure::operational(error.to_string()))?;
            eprintln!(
                "Preset \"{}\" saved for {}.",
                preset_name.trim(),
                entry.meta.name
            );
        }
        return print_launch(&prepared.masked_launch);
    }

    let code = execute_launch(&prepared.launch)?;
    if !secret_names.is_empty() {
        state_store
            .purge_secret(&entry.slug, &secret_names)
            .map_err(|error| CliFailure::operational(error.to_string()))?;
    }
    let last_used = remembered_values(&prepared.form, &prepared.values);
    state_store
        .save_last(
            &entry.slug,
            Some(&last_used),
            Some(&extra.args),
            &secret_names,
        )
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    let at = format_utc_timestamp(SystemTime::now())
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    state_store
        .record_run(
            &entry.slug,
            code,
            &at,
            Some(&prepared.values),
            &secret_names,
        )
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    if let Some(preset_name) = args.save_preset.as_deref() {
        state_store
            .save_preset(
                &entry.slug,
                preset_name.trim(),
                &prepared.values,
                &secret_names,
            )
            .map_err(|error| CliFailure::operational(error.to_string()))?;
    }
    finish_with_status(code)
}

fn execute_launch(launch: &LaunchPlan) -> Result<i32, CliFailure> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&interrupted);
    ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst)).map_err(|error| {
        CliFailure::coded(format!("cannot install Ctrl-C handler: {error}"), 125)
    })?;
    run_launch(launch, &interrupted).map_err(classify_run_error)
}

fn record_raw_run(state: &StateStore, slug: &str, code: i32) -> Result<(), CliFailure> {
    let at = format_utc_timestamp(SystemTime::now())
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    state
        .record_run(slug, code, &at, None, &BTreeSet::new())
        .map_err(|error| CliFailure::operational(error.to_string()))
}

fn finish_with_status(code: i32) -> Result<(), CliFailure> {
    if code == 0 {
        Ok(())
    } else {
        Err(CliFailure::status(code))
    }
}

fn print_launch(launch: &LaunchPlan) -> Result<(), CliFailure> {
    let argv = serde_json::to_string(&launch.argv)
        .map_err(|error| CliFailure::operational(error.to_string()))?;
    println!("argv={argv}");
    println!("cwd={}", launch.cwd.display());
    if !launch.env_overlay.is_empty() {
        let environment = serde_json::to_string(&launch.env_overlay)
            .map_err(|error| CliFailure::operational(error.to_string()))?;
        println!("env={environment}");
    }
    Ok(())
}

fn parse_set_values(values: &[String]) -> Result<BTreeMap<String, String>, CliFailure> {
    let malformed = values
        .iter()
        .filter(|value| {
            value
                .split_once('=')
                .is_none_or(|(name, _)| name.trim().is_empty())
        })
        .cloned()
        .collect::<Vec<_>>();
    if !malformed.is_empty() {
        return Err(CliFailure::usage(format!(
            "Malformed --set (expected NAME=VALUE): {}",
            malformed.join(", ")
        )));
    }
    let mut output = BTreeMap::new();
    for value in values {
        if let Some((name, assigned)) = value.split_once('=') {
            output.insert(name.trim().to_owned(), assigned.to_owned());
        }
    }
    Ok(output)
}

fn classify_prepare_error(error: PrepareRunError) -> CliFailure {
    match error {
        PrepareRunError::UnknownPreset(_) | PrepareRunError::Resolve(_) => {
            CliFailure::usage(error.to_string())
        }
        PrepareRunError::Assembly(AssemblyError::InvalidValues(_))
        | PrepareRunError::Assembly(AssemblyError::MissingSecretEnvironment { .. }) => {
            CliFailure::coded(error.to_string(), 125)
        }
        PrepareRunError::Launch(source) => classify_launch_error(source),
    }
}

fn classify_launch_error(error: LaunchPlanError) -> CliFailure {
    let code = match error {
        LaunchPlanError::TargetMissing(_) => 127,
        LaunchPlanError::NotRunnable(_)
        | LaunchPlanError::MissingInterpreter(_)
        | LaunchPlanError::MissingJavaScriptRuntime(_)
        | LaunchPlanError::MissingNeeds(_) => 126,
        LaunchPlanError::UnsupportedKind(_) | LaunchPlanError::WorkingDirectoryMissing(_) => 125,
    };
    CliFailure::coded(error.to_string(), code)
}

fn classify_run_error(error: RunError) -> CliFailure {
    let code = match error {
        RunError::Spawn { .. } => 126,
        RunError::EmptyArgv | RunError::Wait { .. } => 125,
    };
    CliFailure::coded(error.to_string(), code)
}

fn unicode_environment() -> BTreeMap<String, String> {
    env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

const fn current_platform() -> Platform {
    #[cfg(windows)]
    {
        Platform::Windows
    }
    #[cfg(target_os = "macos")]
    {
        Platform::MacOs
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        Platform::Linux
    }
}
