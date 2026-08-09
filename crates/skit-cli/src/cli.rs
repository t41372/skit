use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File, Metadata},
    io::{self, IsTerminal as _, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::UNIX_EPOCH,
};

use clap::{
    Args, CommandFactory as _, FromArgMatches as _, Parser, Subcommand,
    error::{ContextKind, ContextValue},
};
use clap_complete::{ArgValueCandidates, CompleteEnv, CompletionCandidate, Shell, generate};
use dialoguer::{Confirm, Input, MultiSelect, Password};
use skit_application::{
    AgentInstallPlan, AgentInstallRequest, AgentRoots, AgentScope, AgentTarget, CreateEntry,
    EntryPayload, ExitClass, LibraryScan, LibraryService, RepositoryError, RepositoryOperation,
    SourcePermissions, UpdateEntry, add_workdir, detect_agent_targets,
    form_feedback::GlobCountPort,
    form_state::{FormStateService, PresetSnapshotSource, StateWriteError, prefill},
    health::{
        HealthInspection, HealthIssue, HealthIssueKind, HealthRebuild, HealthRebuildOutcome,
        HealthService, HealthSnapshot, MirrorHealth, UvHealth,
    },
    parameter_edit::finish_parameter_edit,
    payload_stored_name, plan_agent_install,
    preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesSnapshot,
    },
    prompt_selection::PromptSelectionService,
    supports_storage_modes,
    value_preparation::validate_form_value,
};
use skit_domain::{
    Entry, EntryKind, EntrySettings, EntrySummary, Slug, StorageMode,
    parameters::{
        ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
        coerce_default,
    },
};
use skit_form::{
    FormDrift, FormPlan as PreparedFormPlan, OnboardingCandidate, OnboardingParseState,
    OnboardingPlan, PreparedField, form_params, form_params_from_managed, form_plan,
    onboarding_plan,
};
use skit_i18n::{
    Locale, Localize, Message, available_locale_tags, detect_locale, format_text, kind_label,
    render as localize, requested_locale, system_locale, text,
};
use skit_language::{
    LosslessSource, UvMetadata, UvMetadataEditError, cli_params, detect_candidates,
    effective_uv_metadata_bytes, external_dependencies_at, has_uv_metadata_block_bytes, infer_kind,
    managed_params, normalize_shell_default, placeholder_params, plan_uv_metadata_edit,
    python_version_pin, read_uv_metadata, shebang_program, suggest_description,
    validate_pep440_specifiers, validate_pep508_requirement, write_managed_params,
    write_managed_params_bytes, write_uv_metadata,
};
use skit_runtime::{
    DependencyError, LaunchPaths, ProgramProbe, SystemProbe, clear_javascript_dependencies,
    managed_uv_path, resolve_javascript_runtime, resolve_launch_workdir,
};
use skit_store::{
    ConfigError, FileAgentSkillStore, FileConfigStore, FileFormStateStore, FileGlobExpander,
    FilePromptSelectionStore, FileRunnerManagementStore, PromptRunner, RunnerManagementStoreError,
    RunnerRemovalCas, expand_user_path,
};
use skit_store::{FileStore, stored_filenames};
use skit_ui::{
    Action as UiAction, AddAction, AddEffect, AddWorkflowState, DraftKind, DraftSummary,
    Effect as UiEffect, FormField, FormPurpose, FormView, HealthAction, HealthView, HostRequest,
    LibraryState, PreferencesAction, PreferencesEffect, PreferencesView, ReviewDefaults,
    RunFormContext, RunFormOptions, RunFormView, RunPathContext, RunnerManagerAction,
    RunnerManagerView, RunnerRemoveRequest, RunnerRow, RunnerRowIdentity, RunnerSaveOwner,
    RunnerSaveRequest, RunnerSaveTarget, Screen, SourceSnapshot as AddSourceSnapshot,
};
use thiserror::Error;
use unicode_width::UnicodeWidthStr as _;

use crate::run::{RunArgs, RunError};

macro_rules! humanln {
    ($message:literal $(, $value:expr)* $(,)?) => {
        println!("{}", format_text(active_locale(), $message, &[$(&$value as &dyn std::fmt::Display),*]))
    };
}

macro_rules! humanerrln {
    ($message:literal $(, $value:expr)* $(,)?) => {
        eprintln!("{}", format_text(active_locale(), $message, &[$(&$value as &dyn std::fmt::Display),*]))
    };
}

#[cfg(test)]
mod tests;

/// Run the command-line entry point and return its process status.
#[must_use]
pub fn entry() -> i32 {
    match CompleteEnv::with_factory(localized_command)
        .try_complete(env::args_os(), env::current_dir().ok().as_deref())
    {
        Ok(true) => return 0,
        Ok(false) => {}
        Err(error) => {
            print_clap_error(&error, active_locale());
            return error.exit_code();
        }
    }
    let locale = active_locale();
    let cli = match localized_command()
        .try_get_matches()
        .and_then(|matches| Cli::from_arg_matches(&matches))
    {
        Ok(cli) => cli,
        Err(error) => {
            print_clap_error(&error, locale);
            return error.exit_code();
        }
    };
    match execute(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{}", error.message().localize(locale));
            error.exit_code()
        }
    }
}

fn print_clap_error(error: &clap::Error, locale: Locale) {
    let output = localized_clap_error(error, locale);
    if error.use_stderr() {
        eprint!("{output}");
    } else {
        print!("{output}");
    }
}

fn localized_clap_error(error: &clap::Error, locale: Locale) -> String {
    let mut output = error.to_string();
    let mut literals = BTreeSet::new();
    for (kind, value) in error.context() {
        if matches!(kind, ContextKind::Usage | ContextKind::Suggested) {
            continue;
        }
        match value {
            ContextValue::String(value) => {
                literals.insert(value.clone());
            }
            ContextValue::Strings(values) => literals.extend(values.iter().cloned()),
            ContextValue::StyledStr(value) => {
                literals.insert(value.to_string());
            }
            ContextValue::StyledStrs(values) => {
                literals.extend(values.iter().map(ToString::to_string));
            }
            _ => {}
        }
    }
    let mut literals = literals
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    literals.sort_by_key(|value| std::cmp::Reverse(value.len()));
    let mut replacements = Vec::new();
    for (index, literal) in literals.into_iter().enumerate() {
        let token = format!("\u{e000}SKIT{index}\u{e001}");
        if output.contains(&literal) {
            output = output.replace(&literal, &token);
            replacements.push((token, literal));
        }
    }
    output = localize(locale, &output);
    for (token, literal) in replacements {
        output = output.replace(&token, &literal);
    }
    output
}

/// Build the command tree with each skit-authored text already translated.
///
/// Exact catalog lookups replace the whole `about` or `help` text. skit never rewrites part of a
/// Clap token such as `--help`.
fn localized_command() -> clap::Command {
    translate_command(Cli::command(), active_locale())
}

fn translate_command(command: clap::Command, locale: Locale) -> clap::Command {
    let mut command = command;
    if let Some(about) = command.get_about().map(ToString::to_string) {
        command = command.about(text(locale, &about).into_owned());
    }
    if let Some(about) = command.get_long_about().map(ToString::to_string) {
        command = command.long_about(text(locale, &about).into_owned());
    }
    command = command.mut_args(|argument| {
        let Some(help) = argument.get_help().map(|help| help.to_string()) else {
            return argument;
        };
        argument.help(text(locale, &help).into_owned())
    });
    let names = command
        .get_subcommands()
        .map(|sub| sub.get_name().to_owned())
        .collect::<Vec<_>>();
    names.into_iter().fold(command, |command, name| {
        command.mut_subcommand(name, |sub| translate_command(sub, locale))
    })
}

pub(crate) fn active_locale() -> Locale {
    if let Some(language) = env::var_os("SKIT_LANG")
        && let Some(locale) = requested_locale(language.to_str())
    {
        return locale;
    }
    if let Ok(directory) = resolve_config_dir()
        && let Ok(language) = FileConfigStore::new(directory).get("lang")
        && language != "auto"
        && let Some(locale) = requested_locale(Some(&language))
    {
        return locale;
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(language) = env::var_os(key)
            && let Some(locale) = requested_locale(language.to_str())
        {
            return locale;
        }
    }
    system_locale()
}

#[derive(Debug, Parser)]
#[command(
    name = "skit",
    about = "A script, prompt, program, and command library",
    disable_help_subcommand = true
)]
struct Cli {
    /// Show version.
    #[arg(long, short = 'V')]
    version: bool,

    /// Override the skit data directory.
    #[arg(long, global = true, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    /// Install completion for the current shell.
    #[arg(long, conflicts_with = "show_completion")]
    install_completion: bool,

    /// Print completion for the current shell.
    #[arg(long)]
    show_completion: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List entries in the library.
    List {
        /// Emit stable machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Show one entry by exact slug or exact display name.
    Show {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
        /// Emit stable machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Add one file as a copied or referenced entry.
    Add {
        /// Source file to register.
        source: Option<PathBuf>,
        /// Open entry-kind registry key.
        #[arg(long)]
        kind: Option<String>,
        /// Display name. The source stem is the default.
        #[arg(long, short = 'n')]
        name: Option<String>,
        /// Description shown in the library.
        #[arg(long, short = 'd')]
        description: Option<String>,
        /// Write a new source in the configured editor, then add it.
        #[arg(
            long,
            short = 'e',
            conflicts_with_all = ["source", "command_template", "prompt", "reference", "exe", "kind", "runner"]
        )]
        edit: bool,
        /// Reference the original instead of storing a copy.
        #[arg(long = "ref", alias = "reference")]
        reference: bool,
        /// Register a command template instead of a file.
        #[arg(
            long = "cmd",
            conflicts_with_all = ["source", "prompt", "exe", "kind", "runner", "no_interpolate"]
        )]
        command_template: Option<String>,
        /// Treat the source as a prompt entry.
        #[arg(long, conflicts_with_all = ["exe", "kind"])]
        prompt: bool,
        /// Force executable kind inference.
        #[arg(long, conflicts_with_all = ["prompt", "kind", "runner", "no_interpolate"])]
        exe: bool,
        /// Pin a prompt runner.
        #[arg(long, add = ArgValueCandidates::new(runner_candidates))]
        runner: Option<String>,
        /// Disable prompt placeholder insertion.
        #[arg(long)]
        no_interpolate: bool,
        /// Add one package dependency. Repeat for more than one value.
        #[arg(long = "dep")]
        dependencies: Option<Vec<String>>,
        /// Set the Python version constraint.
        #[arg(long)]
        python: Option<String>,
        /// Refuse interactive questions.
        #[arg(long)]
        no_input: bool,
    },
    /// Run one library entry.
    Run(RunArgs),
    /// Replace one entry description.
    Describe {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
        /// Replacement description.
        description: String,
    },
    /// Rename one entry without changing its slug.
    Rename {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
        /// Replacement display name.
        name: String,
    },
    /// Remove one entry.
    Remove {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
        /// Confirm the destructive operation.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Refuse to ask for confirmation.
        #[arg(long)]
        no_input: bool,
    },
    /// Open an entry source in the configured editor.
    Edit {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
        /// Refuse to offer creation when the entry does not exist.
        #[arg(long)]
        no_input: bool,
    },
    /// Read or edit managed and declared parameters.
    Params(Box<ParamsArgs>),
    /// Read or update dependencies and required commands.
    Deps(DepsArgs),
    /// Check runtime and library health.
    Doctor {
        /// Emit stable machine-readable output.
        #[arg(long)]
        json: bool,
        /// Rebuild the derived registry.
        #[arg(long)]
        rebuild: bool,
    },
    /// Read or set skit configuration.
    Config {
        /// Configuration key.
        key: Option<String>,
        /// Replacement value.
        value: Option<String>,
        /// Emit stable machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Manage prompt runners.
    Runner {
        #[command(subcommand)]
        command: RunnerCommand,
    },
    /// Manage named parameter presets.
    Preset {
        #[command(subcommand)]
        command: PresetCommand,
    },
    /// Install the official Agent Skill.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Open the Ratatui library browser.
    Tui,
}

#[derive(Debug, Args)]
struct DepsArgs {
    /// Entry slug or display name.
    #[arg(add = ArgValueCandidates::new(entry_candidates))]
    selector: String,
    /// Replace package dependencies. Repeat for more than one value.
    #[arg(long = "dep")]
    dependencies: Vec<String>,
    /// Clear all package dependencies.
    #[arg(long)]
    clear: bool,
    /// Replace the Python version constraint.
    #[arg(long = "python")]
    requires_python: Option<String>,
    /// Replace required external commands. Repeat for more than one value.
    #[arg(long = "need")]
    needs: Vec<String>,
    /// Clear required external commands.
    #[arg(long)]
    clear_needs: bool,
    /// Emit stable machine-readable output.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ParamsArgs {
    /// Entry slug or display name.
    #[arg(add = ArgValueCandidates::new(entry_candidates))]
    selector: String,
    /// Reconcile managed definitions with the current source.
    #[arg(long)]
    resync: bool,
    /// Manage one detected source parameter.
    #[arg(long = "manage")]
    manage: Vec<String>,
    /// Stop managing one source parameter.
    #[arg(long = "unmanage")]
    unmanage: Vec<String>,
    /// Normalize one shell constant to an environment default.
    #[arg(long = "normalize")]
    normalize: Vec<String>,
    /// Add a hand-declared parameter.
    #[arg(long = "add")]
    add: Vec<String>,
    /// Remove a declared parameter.
    #[arg(long = "rm")]
    remove: Vec<String>,
    /// Set a parameter type as NAME=TYPE.
    #[arg(long = "type")]
    parameter_types: Vec<String>,
    /// Set a default as NAME=VALUE.
    #[arg(long = "default")]
    defaults: Vec<String>,
    /// Set choices as NAME=A,B,C.
    #[arg(long)]
    choices: Vec<String>,
    /// Set delivery as NAME=DELIVERY.
    #[arg(long = "deliver", alias = "delivery")]
    delivery: Vec<String>,
    /// Set source binding as NAME=BINDING.
    #[arg(long = "binding")]
    bindings: Vec<String>,
    /// Set a flag as NAME=--FLAG. An empty flag makes the field positional.
    #[arg(long = "flag")]
    flags: Vec<String>,
    /// Allow more than one value for a field.
    #[arg(long)]
    multiple: Vec<String>,
    /// Allow only one value for a field.
    #[arg(long = "no-multiple")]
    no_multiple: Vec<String>,
    /// Repeat the flag for each value.
    #[arg(long)]
    repeat: Vec<String>,
    /// Put all values after one flag.
    #[arg(long = "no-repeat")]
    no_repeat: Vec<String>,
    /// Set an environment target as NAME=ENVVAR.
    #[arg(long = "env-target")]
    env_targets: Vec<String>,
    /// Set a boolean flag action as NAME=ACTION.
    #[arg(long = "action")]
    actions: Vec<String>,
    /// Set help text as NAME=TEXT.
    #[arg(long = "help-text")]
    help_text: Vec<String>,
    /// Set a form prompt as NAME=TEXT.
    #[arg(long = "prompt")]
    prompts: Vec<String>,
    /// Set a secret environment source as NAME=ENVVAR.
    #[arg(long = "env-source")]
    env_sources: Vec<String>,
    /// Mark fields as required.
    #[arg(long)]
    required: Vec<String>,
    /// Mark fields as optional.
    #[arg(long)]
    optional: Vec<String>,
    /// Mark fields as secret.
    #[arg(long)]
    secret: Vec<String>,
    /// Remove the secret marker from fields.
    #[arg(long = "no-secret")]
    no_secret: Vec<String>,
    /// Replace the work-directory policy.
    #[arg(long)]
    workdir: Option<String>,
    /// Replace a command template.
    #[arg(long)]
    template: Option<String>,
    /// Pin an interpreter or JavaScript runtime.
    #[arg(long)]
    interpreter: Option<String>,
    /// Pin a prompt runner. An empty value clears the pin.
    #[arg(long, add = ArgValueCandidates::new(runner_candidates))]
    runner: Option<String>,
    /// Enable prompt interpolation.
    #[arg(long, conflicts_with = "no_interpolate")]
    interpolate: bool,
    /// Disable prompt interpolation.
    #[arg(long)]
    no_interpolate: bool,
    /// Emit stable machine-readable output.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum RunnerCommand {
    /// List configured prompt runners.
    List {
        /// Emit stable machine-readable output.
        #[arg(long)]
        json: bool,
        /// Include malformed rows when supported.
        #[arg(long)]
        all: bool,
    },
    /// Add one direct argv prompt runner.
    Add {
        /// Stable runner name.
        name: String,
        /// Program and arguments. One token must contain `{{prompt}}`.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        argv: Vec<String>,
        /// Replace an existing name.
        #[arg(long)]
        force: bool,
    },
    /// Remove one configured prompt runner.
    Remove {
        /// Stable runner name.
        #[arg(add = ArgValueCandidates::new(runner_candidates))]
        name: Option<String>,
        /// Remove one malformed raw row by its zero-based index or `container`.
        #[arg(long, conflicts_with = "name")]
        row: Option<String>,
        /// Confirm removal.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Refuse to prompt.
        #[arg(long)]
        no_input: bool,
    },
}

#[derive(Debug, Subcommand)]
enum PresetCommand {
    /// Save a named preset.
    Save {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
        /// Preset name.
        name: String,
        /// Copy the exact public values from the most recent run.
        #[arg(long)]
        from_last: bool,
    },
    /// List named presets.
    List {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
        /// Emit stable machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Delete one named preset.
    Delete {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
        /// Preset name.
        #[arg(add = ArgValueCandidates::new(preset_candidates))]
        name: String,
        /// Confirm deletion.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Refuse to prompt.
        #[arg(long)]
        no_input: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// Install the bundled Agent Skill.
    Install {
        /// Agent convention: claude, codex, or agents.
        target: Option<String>,
        /// Install below this explicit directory.
        #[arg(long = "to")]
        directory: Option<PathBuf>,
        /// Use the current project instead of the user directory.
        #[arg(long)]
        project: bool,
    },
}

pub(crate) fn entry_candidates() -> Vec<CompletionCandidate> {
    resolve_data_dir(None).map_or_else(
        |_| Vec::new(),
        |directory| entry_candidates_from(&FileStore::new(directory)),
    )
}

fn entry_candidates_from(store: &FileStore) -> Vec<CompletionCandidate> {
    LibraryService::new(store.clone()).list().map_or_else(
        |_| Vec::new(),
        |scan| {
            scan.entries
                .into_iter()
                .flat_map(|entry| {
                    let help = clap::builder::StyledStr::from(format!(
                        "{} — {}",
                        entry.kind, entry.description
                    ));
                    [
                        CompletionCandidate::new(entry.slug.as_str()).help(Some(help.clone())),
                        CompletionCandidate::new(entry.name).help(Some(help)),
                    ]
                })
                .collect()
        },
    )
}

pub(crate) fn runner_candidates() -> Vec<CompletionCandidate> {
    resolve_config_dir().map_or_else(
        |_| Vec::new(),
        |directory| runner_candidates_from(&FileConfigStore::new(directory)),
    )
}

fn runner_candidates_from(store: &FileConfigStore) -> Vec<CompletionCandidate> {
    store.runners().map_or_else(
        |_| Vec::new(),
        |runners| {
            runners
                .into_iter()
                .map(|runner| CompletionCandidate::new(runner.name))
                .collect()
        },
    )
}

pub(crate) fn preset_candidates() -> Vec<CompletionCandidate> {
    resolve_state_dir().map_or_else(
        |_| Vec::new(),
        |directory| preset_candidates_from(&directory),
    )
}

fn preset_candidates_from(state_dir: &Path) -> Vec<CompletionCandidate> {
    let Ok(files) = fs::read_dir(state_dir.join("values")) else {
        return Vec::new();
    };
    let mut names = BTreeSet::new();
    for file in files.flatten() {
        let path = file.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(document) = text.parse::<toml::Table>() else {
            continue;
        };
        if let Some(presets) = document.get("presets").and_then(toml::Value::as_table) {
            names.extend(
                presets
                    .iter()
                    .filter(|(_, value)| value.is_table())
                    .map(|(name, _)| name.clone()),
            );
        }
    }
    names.into_iter().map(CompletionCandidate::new).collect()
}

fn execute(cli: Cli) -> Result<i32, CliError> {
    if cli.version {
        println!("skit {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if cli.show_completion {
        write_completion(detect_shell()?, &mut io::stdout());
        return Ok(0);
    }
    if cli.install_completion {
        let shell = detect_shell()?;
        let path = completion_path(shell)?;
        let parent = path
            .parent()
            .expect("each supported completion path has a parent");
        fs::create_dir_all(parent)?;
        let mut output = File::create(&path)?;
        write_completion(shell, &mut output);
        humanln!("Installed completion: {}", path.display());
        return Ok(0);
    }
    let data_dir = resolve_data_dir(cli.data_dir)?;
    let store = FileStore::new(data_dir);
    let service = LibraryService::new(store.clone());
    match cli.command {
        Some(Command::List { json }) => {
            list(&service, &store, json)?;
            Ok(0)
        }
        Some(Command::Show { selector, json }) => {
            show(&service, &store, &selector, json)?;
            Ok(0)
        }
        Some(Command::Add {
            source,
            kind,
            name,
            description,
            edit,
            reference,
            command_template,
            prompt,
            exe,
            runner,
            no_interpolate,
            dependencies,
            python,
            no_input,
        }) => {
            let dependencies_explicit = dependencies.is_some();
            add_command(
                &service,
                AddOptions {
                    source,
                    kind,
                    name,
                    description,
                    reference,
                    command_template,
                    prompt,
                    executable: exe,
                    runner,
                    no_interpolate,
                    dependencies: dependencies.unwrap_or_default(),
                    dependencies_explicit,
                    requires_python: python,
                    no_input,
                },
                edit,
            )?;
            Ok(0)
        }
        Some(Command::Run(args)) => run_entry(&service, &store, args),
        Some(Command::Describe {
            selector,
            description,
        }) => {
            describe(&service, &selector, &description)?;
            Ok(0)
        }
        Some(Command::Rename { selector, name }) => {
            rename(&service, &selector, &name)?;
            Ok(0)
        }
        Some(Command::Remove {
            selector,
            yes,
            no_input,
        }) => {
            remove(&service, &selector, yes, no_input)?;
            Ok(0)
        }
        Some(Command::Edit { selector, no_input }) => {
            edit(&service, &store, &selector, no_input)?;
            Ok(0)
        }
        Some(Command::Params(args)) => {
            params(&service, &store, *args)?;
            Ok(0)
        }
        Some(Command::Deps(args)) => {
            deps(&service, &store, args)?;
            Ok(0)
        }
        Some(Command::Doctor { json, rebuild }) => doctor(&service, &store, json, rebuild),
        Some(Command::Config { key, value, json }) => {
            config(key.as_deref(), value.as_deref(), json)?;
            Ok(0)
        }
        Some(Command::Runner { command }) => {
            runner(&service, command)?;
            Ok(0)
        }
        Some(Command::Preset { command }) => {
            preset(&service, &store, command)?;
            Ok(0)
        }
        Some(Command::Agent { command }) => {
            agent(command)?;
            Ok(0)
        }
        Some(Command::Tui) | None => {
            tui(&service)?;
            Ok(0)
        }
    }
}

fn add_command(
    service: &LibraryService<FileStore>,
    mut options: AddOptions,
    edit: bool,
) -> Result<(), CliError> {
    let no_input = options.no_input;
    if edit && no_input {
        return Err(CliError::Usage(Message::new(
            "--edit needs an editor; use standard input as `skit add - --name NAME`",
        )));
    }
    if edit {
        return add_draft(service, options, false);
    }
    if options.source.is_none() && options.command_template.is_none() {
        if options.prompt {
            let config_dir = resolve_config_dir()?;
            validate_prompt_runner_in(
                &FileConfigStore::new(config_dir),
                options.runner.as_deref(),
            )?;
            if !io::stdin().is_terminal() {
                options.source = Some(PathBuf::from("-"));
                return add(service, options);
            }
            if no_input {
                return Err(CliError::Usage(Message::new(
                    "a prompt body is required; pipe it to `skit add - --prompt --name NAME`",
                )));
            }
            return add_draft(service, options, true);
        }
        if no_input || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(CliError::Usage(Message::new(
                "add needs a source path, standard input as `-`, --edit, --prompt, or --cmd",
            )));
        }
        refuse_bare_add_flags(&options)?;
        let state_dir = resolve_state_dir()?;
        let config_dir = resolve_config_dir()?;
        if wants_tui_form(&config_dir)? {
            let workflow = tui_add_workflow(service.repository(), &state_dir, &config_dir)?;
            let slug = skit_tui::run_add_workflow(
                workflow,
                |effect| {
                    tui_effect(
                        service,
                        service.repository(),
                        &state_dir,
                        &config_dir,
                        effect,
                    )
                },
                active_locale(),
            )?
            .ok_or(CliError::AddCancelled)?;
            let entry = service.show(slug.as_str())?;
            print_add_summary(service.repository(), &entry)?;
            return Ok(());
        }
        return bare_add_plain(service, &config_dir);
    }
    add(service, options)
}

fn refuse_bare_add_flags(options: &AddOptions) -> Result<(), CliError> {
    let mut withheld = Vec::new();
    if options.name.as_deref().is_some_and(|name| !name.is_empty()) {
        withheld.push("--name");
    }
    if options.description.is_some() {
        withheld.push("--description");
    }
    if options.reference {
        withheld.push("--ref");
    }
    if options.executable {
        withheld.push("--exe");
    }
    if options.kind.is_some() {
        withheld.push("--kind");
    }
    if options.runner.is_some() {
        withheld.push("--runner");
    }
    if options.no_interpolate {
        withheld.push("--no-interpolate");
    }
    if options.dependencies_explicit || !options.dependencies.is_empty() {
        withheld.push("--dep");
    }
    if options.requires_python.is_some() {
        withheld.push("--python");
    }
    if withheld.is_empty() {
        return Ok(());
    }

    let needs = withheld
        .iter()
        .copied()
        .filter(|flag| !matches!(*flag, "--name" | "--description"))
        .collect::<BTreeSet<_>>();
    let lanes = [
        ("--edit", BTreeSet::from(["--dep", "--python"])),
        ("--prompt", BTreeSet::from(["--runner", "--no-interpolate"])),
        ("--cmd", BTreeSet::new()),
    ]
    .into_iter()
    .filter_map(|(lane, honored)| needs.is_subset(&honored).then_some(lane))
    .collect::<Vec<_>>();
    let flags = withheld.join(", ");
    if lanes.is_empty() {
        Err(CliError::Usage(
            Message::new(
                "{} need a source — pass the path in the same command (skit add PATH …) (nothing was added).",
            )
            .with(flags),
        ))
    } else {
        Err(CliError::Usage(
            Message::new(
                "{} need a source — pass the path in the same command (skit add PATH …), or pick a lane outright with {} (nothing was added).",
            )
            .with(flags)
            .with(lanes.join(", ")),
        ))
    }
}

fn wants_tui_form(config_dir: &Path) -> Result<bool, CliError> {
    if env::var_os("TERM").as_deref() == Some(std::ffi::OsStr::new("dumb")) {
        return Ok(false);
    }
    Ok(FileConfigStore::new(config_dir).get("form")? == "tui")
}

fn bare_add_plain(service: &LibraryService<FileStore>, config_dir: &Path) -> Result<(), CliError> {
    let locale = active_locale();
    humanln!("What would you like to add?");
    println!(
        "  1. {}",
        text(
            locale,
            "A file you already have — a script, program, or prompt"
        )
    );
    println!(
        "  2. {}",
        text(locale, "A new script, written in your editor")
    );
    println!(
        "  3. {}",
        text(locale, "A new AI-agent prompt, written in your editor")
    );
    println!(
        "  4. {}",
        text(locale, "A command template (e.g. ffmpeg -i {input})")
    );
    let choice = Input::<String>::new()
        .with_prompt(text(locale, "Which one?").into_owned())
        .default("1".to_owned())
        .validate_with(|value: &String| {
            matches!(value.trim(), "1" | "2" | "3" | "4")
                .then_some(())
                .ok_or_else(|| text(locale, "Choose a number from 1 to 4.").into_owned())
        })
        .interact_text()
        .map_err(add_dialoguer_error)?;
    match choice.trim() {
        "1" => {
            let path = add_plain_text("Path to the file")?;
            if path.trim().is_empty() {
                return Err(CliError::AddCancelled);
            }
            add(
                service,
                AddOptions {
                    source: Some(PathBuf::from(path.trim())),
                    ..empty_add_options()
                },
            )
        }
        "2" => add_plain_draft(service, config_dir, DraftKind::Script),
        "3" => add_plain_draft(service, config_dir, DraftKind::Prompt),
        "4" => {
            let template = add_plain_text("Command template")?;
            if template.trim().is_empty() {
                return Err(CliError::AddCancelled);
            }
            let name = add_plain_text("Name for the command")?;
            if name.trim().is_empty() {
                return Err(CliError::AddCancelled);
            }
            let description = add_plain_text("Description (optional)")?;
            add(
                service,
                AddOptions {
                    name: Some(name.trim().to_owned()),
                    description: Some(description.trim().to_owned()),
                    command_template: Some(template.trim().to_owned()),
                    ..empty_add_options()
                },
            )
        }
        _ => unreachable!("the dialoguer validator accepts only four choices"),
    }
}

fn add_plain_draft(
    service: &LibraryService<FileStore>,
    config_dir: &Path,
    kind: DraftKind,
) -> Result<(), CliError> {
    let name = add_plain_text("Name in skit")?;
    if name.trim().is_empty() {
        return Err(CliError::Usage(Message::new("A name is required.")));
    }
    match service.show(name.trim()) {
        Ok(_) | Err(RepositoryError::Ambiguous { .. }) => {
            return Err(CliError::Failure(
                Message::new("The name {} is already taken — pick another name.").with(name.trim()),
            ));
        }
        Err(RepositoryError::NotFound { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let Some(source) = tui_author_draft(service.repository().data_dir(), config_dir, kind)? else {
        humanln!("Nothing was written, so nothing was added.");
        return Ok(());
    };
    let path = source.path;
    let result = add(
        service,
        AddOptions {
            source: Some(path.clone()),
            name: Some(name.trim().to_owned()),
            prompt: kind == DraftKind::Prompt,
            ..empty_add_options()
        },
    );
    if result.is_ok() {
        remove_owned_draft(service.repository().data_dir(), &path)?;
    } else {
        humanerrln!("Your draft was kept at {}", path.display());
    }
    result
}

fn add_plain_text(prompt: &'static str) -> Result<String, CliError> {
    Input::<String>::new()
        .with_prompt(text(active_locale(), prompt).into_owned())
        .allow_empty(true)
        .interact_text()
        .map_err(add_dialoguer_error)
}

fn add_dialoguer_error(error: dialoguer::Error) -> CliError {
    let error = io::Error::from(error);
    if matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::UnexpectedEof
    ) {
        CliError::AddCancelled
    } else {
        CliError::Io(error)
    }
}

fn empty_add_options() -> AddOptions {
    AddOptions {
        source: None,
        kind: None,
        name: None,
        description: None,
        reference: false,
        command_template: None,
        prompt: false,
        executable: false,
        runner: None,
        no_interpolate: false,
        dependencies: Vec::new(),
        dependencies_explicit: false,
        requires_python: None,
        no_input: false,
    }
}

fn add_draft(
    service: &LibraryService<FileStore>,
    mut options: AddOptions,
    prompt: bool,
) -> Result<(), CliError> {
    let config_dir = resolve_config_dir()?;
    validate_prompt_runner_in(&FileConfigStore::new(config_dir), options.runner.as_deref())?;
    let drafts = service.repository().data_dir().join("drafts");
    fs::create_dir_all(&drafts)?;
    let suffix = if prompt { ".prompt.md" } else { "" };
    let draft = drafts.join(format!(
        "skit-{}{}",
        skit_domain::EntryId::generate().as_str(),
        suffix
    ));
    fs::write(&draft, [])?;
    open_editor(&draft)?;
    if fs::metadata(&draft)?.len() == 0 {
        return Err(CliError::Usage(
            Message::new("the draft is empty and was kept at {}").with(draft.display()),
        ));
    }
    if !prompt {
        let text =
            fs::read_to_string(&draft).map_err(|error| source_error("read", &draft, error))?;
        let shebang = text.lines().next().filter(|line| line.starts_with("#!"));
        options.kind = Some(
            infer_kind(&draft, shebang, false)
                .unwrap_or("python")
                .to_owned(),
        );
    }
    options.source = Some(draft.clone());
    options.prompt = prompt;
    let result = add(service, options);
    if result.is_ok() {
        fs::remove_file(&draft)?;
    } else {
        humanerrln!("Your draft was kept at {}", draft.display());
    }
    result
}

fn validate_prompt_runner_in(config: &FileConfigStore, name: Option<&str>) -> Result<(), CliError> {
    let Some(name) = name else {
        return Ok(());
    };
    let name = name.trim();
    if name.is_empty() {
        return Ok(());
    }
    let exists = config.runners()?.iter().any(|runner| runner.name == name);
    if exists {
        Ok(())
    } else {
        Err(CliError::Usage(
            Message::new("prompt runner {} is not configured").quoted(name),
        ))
    }
}

fn write_completion(shell: Shell, output: &mut dyn io::Write) {
    let mut command = Cli::command();
    generate(shell, &mut command, "skit", output);
}

fn detect_shell() -> Result<Shell, CliError> {
    if env::var_os("PSModulePath").is_some() {
        return Ok(Shell::PowerShell);
    }
    let name = env::var_os("SHELL")
        .and_then(|value| PathBuf::from(value).file_name().map(ToOwned::to_owned))
        .and_then(|value| value.to_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    match name.as_str() {
        "bash" => Ok(Shell::Bash),
        "elvish" => Ok(Shell::Elvish),
        "fish" => Ok(Shell::Fish),
        "pwsh" | "powershell" | "powershell.exe" | "pwsh.exe" => Ok(Shell::PowerShell),
        "zsh" => Ok(Shell::Zsh),
        _ => Err(CliError::Usage(Message::new(
            "could not detect the shell; set SHELL before completion setup",
        ))),
    }
}

fn completion_path(shell: Shell) -> Result<PathBuf, CliError> {
    let home = user_home().ok_or_else(|| {
        CliError::Usage(Message::new(
            "could not determine the home directory for completion setup",
        ))
    })?;
    let path = if shell == Shell::Bash {
        env::var_os("XDG_DATA_HOME").map_or_else(
            || {
                home.join(".local")
                    .join("share")
                    .join("bash-completion")
                    .join("completions")
                    .join("skit")
            },
            |root| {
                PathBuf::from(root)
                    .join("bash-completion")
                    .join("completions")
                    .join("skit")
            },
        )
    } else if shell == Shell::Fish {
        env::var_os("XDG_CONFIG_HOME")
            .map_or_else(|| home.join(".config"), PathBuf::from)
            .join("fish")
            .join("completions")
            .join("skit.fish")
    } else if shell == Shell::Zsh {
        env::var_os("XDG_DATA_HOME")
            .map_or_else(|| home.join(".local").join("share"), PathBuf::from)
            .join("zsh")
            .join("site-functions")
            .join("_skit")
    } else if shell == Shell::Elvish {
        env::var_os("XDG_CONFIG_HOME")
            .map_or_else(|| home.join(".config"), PathBuf::from)
            .join("elvish")
            .join("lib")
            .join("skit.elv")
    } else {
        debug_assert_eq!(shell, Shell::PowerShell);
        home.join("Documents")
            .join("PowerShell")
            .join("Completions")
            .join("_skit.ps1")
    };
    Ok(path)
}

fn user_home() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn run_entry(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    mut args: RunArgs,
) -> Result<i32, CliError> {
    if args.no_input || args.raw || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return crate::run::run(service, store, args).map_err(Into::into);
    }

    let forms = interactive_run_form(service, store, &args)?;
    let use_plain = args.plain
        || FileConfigStore::new(resolve_config_dir()?)
            .get("form")?
            .eq_ignore_ascii_case("plain");
    let values = if use_plain {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        let stdout = io::stdout();
        let mut output = stdout.lock();
        collect_plain_form(
            &forms.plain,
            active_locale(),
            &mut input,
            &mut output,
            |_| rpassword::read_password(),
        )
        .map_err(plain_form_error)?
    } else {
        // The inline run window serves the same host effects the workbench serves, so its
        // advertised chips (Ctrl+S saves a preset) work there too.
        let state_dir = resolve_state_dir()?;
        let config_dir = resolve_config_dir()?;
        skit_tui::collect_run_form(
            forms.enhanced,
            |effect| tui_effect(service, store, &state_dir, &config_dir, effect),
            active_locale(),
        )?
        .ok_or(CliError::Aborted)?
    };
    apply_interactive_run_values(&mut args, &values, &forms.baseline)?;
    args.no_input = true;
    crate::run::run(service, store, args).map_err(Into::into)
}

fn plain_form_error(error: io::Error) -> CliError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        CliError::Aborted
    } else {
        CliError::Io(error)
    }
}

struct InteractiveRunForms {
    plain: FormView,
    enhanced: RunFormView,
    baseline: BTreeMap<String, String>,
}

fn interactive_run_form(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    args: &RunArgs,
) -> Result<InteractiveRunForms, CliError> {
    let entry = service.show(&args.selector)?;
    let declarations = entry_parameters(store, &entry);
    let saved =
        FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).load(&entry.slug);
    if args.save_preset.is_some() && declarations.is_empty() {
        return Err(RunError::PresetWithoutFields.into());
    }
    let preset = match args.preset.as_deref() {
        Some(name) => Some(
            saved
                .presets
                .get(name)
                .ok_or_else(|| RunError::PresetNotFound {
                    name: name.to_owned(),
                })?,
        ),
        None => None,
    };
    let initial = prefill(&declarations, &saved.values, None);
    let baseline = prefill(&declarations, &saved.values, preset);
    let fixed_values = run_fixed_values(&declarations, &args.values)?;
    let settings = EntrySettings::from_meta(&entry.meta);
    let configured_runners = FileConfigStore::new(resolve_config_dir()?)
        .runners()?
        .into_iter()
        .map(|runner| runner.name)
        .collect::<Vec<_>>();
    let runners = if entry.meta.kind.as_str() == "prompt" && args.runner.is_none() {
        configured_runners
    } else {
        Vec::new()
    };
    let mut plain_values = baseline.clone();
    plain_values.extend(fixed_values.clone());
    let remaining = declarations
        .iter()
        .filter(|declaration| !fixed_values.contains_key(&declaration.name))
        .cloned()
        .collect::<Vec<_>>();
    let plain = plain_run_form_view(
        &entry,
        &remaining,
        &plain_values,
        &runners,
        &settings.runner,
    );
    let extra_arguments = join_editable_arguments(if !args.extra_args.is_empty() {
        &args.extra_args
    } else if args.forget_args {
        &[]
    } else {
        &saved.extra_args
    });
    let enhanced = RunFormView::from_declarations(
        entry.slug.as_str(),
        &entry.meta.name,
        &declarations,
        &initial,
        &runners,
        &settings.runner,
        &saved.presets,
        &extra_arguments,
    )
    .with_options(RunFormOptions {
        selected_preset: args.preset.clone().unwrap_or_default(),
        save_preset: args.save_preset.clone().unwrap_or_default(),
        dry_run: args.dry_run,
        include_extra: false,
        fixed_values,
    });
    Ok(InteractiveRunForms {
        plain,
        enhanced,
        baseline,
    })
}

fn run_fixed_values(
    declarations: &[ParamDecl],
    assignments: &[String],
) -> Result<BTreeMap<String, String>, CliError> {
    let names = declarations
        .iter()
        .map(|declaration| declaration.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut values = BTreeMap::new();
    for assignment in assignments {
        let Some((name, value)) = assignment.split_once('=') else {
            return Err(RunError::InvalidSet {
                value: assignment.clone(),
            }
            .into());
        };
        if name.is_empty() {
            return Err(RunError::InvalidSet {
                value: assignment.clone(),
            }
            .into());
        }
        if !names.contains(name) {
            return Err(RunError::UnknownSet {
                name: name.to_owned(),
            }
            .into());
        }
        values.insert(name.to_owned(), value.to_owned());
    }
    Ok(values)
}

fn apply_interactive_run_values(
    args: &mut RunArgs,
    values: &BTreeMap<String, String>,
    baseline: &BTreeMap<String, String>,
) -> Result<(), CliError> {
    args.values.extend(changed_form_values(values, baseline));
    if values.contains_key("_skit_preset") {
        args.preset = tui_nonempty_owned(values, "_skit_preset");
    }
    if values.contains_key("_skit_save_preset") {
        args.save_preset = tui_nonempty_owned(values, "_skit_save_preset");
    }
    if values.contains_key("_skit_runner") {
        args.runner = tui_nonempty_owned(values, "_skit_runner");
    }
    if let Some(value) = values.get("_skit_dry_run") {
        args.dry_run = tui_bool(value)?;
    }
    Ok(())
}

fn changed_form_values(
    values: &BTreeMap<String, String>,
    baseline: &BTreeMap<String, String>,
) -> Vec<String> {
    values
        .iter()
        .filter_map(|(key, value)| {
            let name = key.strip_prefix("value:")?;
            (baseline.get(name) != Some(value)).then(|| format!("{name}={value}"))
        })
        .collect()
}

fn split_editable_arguments(value: &str) -> Result<Vec<String>, CliError> {
    #[cfg(target_os = "windows")]
    {
        split_windows_arguments(value)
    }
    #[cfg(not(target_os = "windows"))]
    {
        shlex::split(value)
            .ok_or_else(|| CliError::Usage(Message::new("extra arguments have invalid quoting")))
    }
}

fn join_editable_arguments(arguments: &[String]) -> String {
    #[cfg(target_os = "windows")]
    {
        join_windows_arguments(arguments)
    }
    #[cfg(not(target_os = "windows"))]
    {
        shlex::try_join(arguments.iter().map(String::as_str)).unwrap_or_default()
    }
}

#[cfg(any(test, target_os = "windows"))]
fn split_windows_arguments(value: &str) -> Result<Vec<String>, CliError> {
    let characters = value.chars().collect::<Vec<_>>();
    let mut arguments = Vec::new();
    let mut index = 0;
    loop {
        while index < characters.len() && matches!(characters[index], ' ' | '\t') {
            index = index.saturating_add(1);
        }
        if index == characters.len() {
            break;
        }
        let mut argument = String::new();
        let mut quoted = false;
        while index < characters.len() {
            let character = characters[index];
            if matches!(character, ' ' | '\t') && !quoted {
                break;
            }
            if character == '\\' {
                let start = index;
                while index < characters.len() && characters[index] == '\\' {
                    index = index.saturating_add(1);
                }
                let backslashes = index - start;
                if index < characters.len() && characters[index] == '"' {
                    argument.extend(std::iter::repeat_n('\\', backslashes / 2));
                    if backslashes % 2 == 1 {
                        argument.push('"');
                    } else {
                        quoted = !quoted;
                    }
                    index = index.saturating_add(1);
                } else {
                    argument.extend(std::iter::repeat_n('\\', backslashes));
                }
                continue;
            }
            if character == '"' {
                if quoted && index + 1 < characters.len() && characters[index + 1] == '"' {
                    argument.push('"');
                    index = index.saturating_add(2);
                    continue;
                }
                quoted = !quoted;
                index = index.saturating_add(1);
                continue;
            }
            argument.push(character);
            index = index.saturating_add(1);
        }
        if quoted {
            return Err(CliError::Usage(Message::new(
                "extra arguments have invalid quoting",
            )));
        }
        arguments.push(argument);
    }
    Ok(arguments)
}

#[cfg(any(test, target_os = "windows"))]
fn join_windows_arguments<S: AsRef<str>>(arguments: &[S]) -> String {
    let mut command = String::new();
    for argument in arguments {
        if !command.is_empty() {
            command.push(' ');
        }
        let argument = argument.as_ref();
        let quote = argument.is_empty() || argument.contains([' ', '\t']);
        if quote {
            command.push('"');
        }
        let mut backslashes = 0;
        for character in argument.chars() {
            if character == '\\' {
                backslashes += 1;
                continue;
            }
            if character == '"' {
                command.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
            } else {
                command.extend(std::iter::repeat_n('\\', backslashes));
            }
            backslashes = 0;
            command.push(character);
        }
        command.extend(std::iter::repeat_n(
            '\\',
            if quote { backslashes * 2 } else { backslashes },
        ));
        if quote {
            command.push('"');
        }
    }
    command
}

fn collect_plain_form<R, W, F>(
    form: &FormView,
    locale: Locale,
    input: &mut R,
    output: &mut W,
    mut read_secret: F,
) -> io::Result<BTreeMap<String, String>>
where
    R: io::BufRead,
    W: io::Write,
    F: FnMut(&str) -> io::Result<String>,
{
    let mut values = BTreeMap::new();
    for field in &form.fields {
        let arguments = field
            .label_arguments
            .iter()
            .map(|value| value as &dyn std::fmt::Display)
            .collect::<Vec<_>>();
        let label = if field.translate_label {
            format_text(locale, &field.label, &arguments)
        } else {
            field.label.clone()
        };
        if field.value.is_empty() || field.secret {
            write!(output, "{label}: ")?;
        } else {
            write!(output, "{label} [{}]: ", field.value)?;
        }
        output.flush()?;
        let value = if field.secret {
            read_secret(&label)?
        } else {
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "input ended before the form was complete",
                ));
            }
            let value = line.trim_end_matches(['\r', '\n']);
            if value.is_empty() {
                field.value.clone()
            } else {
                value.to_owned()
            }
        };
        values.insert(field.key.clone(), value);
    }
    Ok(values)
}

fn list(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    json: bool,
) -> Result<(), CliError> {
    let scan = service.list()?;
    if json {
        let state = FormStateService::new(FileFormStateStore::new(resolve_state_dir()?));
        let rows = scan
            .entries
            .iter()
            .map(|entry| {
                let run = state.last_run(&entry.slug);
                serde_json::json!({
                    "name": entry.name,
                    "slug": entry.slug,
                    "kind": entry.kind,
                    "mode": entry.mode,
                    "description": entry.description,
                    "missing": summary_missing(store, entry),
                    "last_run_at": run.at,
                    "last_exit": run.exit,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string(&rows)?);
    } else {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        write_human_list(&mut output, store, &scan.entries, active_locale())?;
        let stderr = io::stderr();
        let mut errors = stderr.lock();
        for diagnostic in &scan.diagnostics {
            let detail = diagnostic.localize(active_locale());
            let warning = format_text(active_locale(), "warning: {}", &[&detail]);
            writeln!(errors, "{warning}")?;
        }
    }
    Ok(())
}

fn write_human_list<W: io::Write>(
    output: &mut W,
    store: &FileStore,
    entries: &[EntrySummary],
    locale: Locale,
) -> io::Result<()> {
    if entries.is_empty() {
        return writeln!(
            output,
            "{}",
            text(locale, "No entries yet. Add one with: skit add <path>")
        );
    }

    let rows = entries
        .iter()
        .map(|entry| {
            [
                entry.name.clone(),
                kind_label(locale, entry.kind.as_str()).into_owned(),
                list_description(store, entry, locale),
            ]
        })
        .collect::<Vec<_>>();
    let headers = [
        text(locale, "Name").into_owned(),
        text(locale, "Kind").into_owned(),
        text(locale, "Description").into_owned(),
    ];
    write_table(output, &headers, &rows)
}

fn write_table<W: io::Write, const COLUMNS: usize>(
    output: &mut W,
    headers: &[String; COLUMNS],
    rows: &[[String; COLUMNS]],
) -> io::Result<()> {
    let mut widths = std::array::from_fn(|column| {
        display_lines(&headers[column])
            .map(str::width)
            .max()
            .unwrap_or(0)
    });
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            widths[column] =
                widths[column].max(display_lines(cell).map(str::width).max().unwrap_or(0));
        }
    }

    write_border(output, '┏', '┳', '┓', '━', &widths)?;
    write_table_row(output, headers, &widths)?;
    write_border(output, '┡', '╇', '┩', '━', &widths)?;
    for row in rows {
        write_table_row(output, row, &widths)?;
    }
    write_border(output, '└', '┴', '┘', '─', &widths)
}

fn display_lines(value: &str) -> impl Iterator<Item = &str> {
    value.split('\n')
}

fn write_border<W: io::Write, const COLUMNS: usize>(
    output: &mut W,
    left: char,
    junction: char,
    right: char,
    fill: char,
    widths: &[usize; COLUMNS],
) -> io::Result<()> {
    write!(output, "{left}")?;
    for (index, width) in widths.iter().enumerate() {
        write!(output, "{}", fill.to_string().repeat(width + 2))?;
        write!(
            output,
            "{}",
            if index + 1 == widths.len() {
                right
            } else {
                junction
            }
        )?;
    }
    writeln!(output)
}

fn write_table_row<W: io::Write, const COLUMNS: usize>(
    output: &mut W,
    cells: &[String; COLUMNS],
    widths: &[usize; COLUMNS],
) -> io::Result<()> {
    let lines = cells
        .iter()
        .map(|cell| display_lines(cell).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let height = lines.iter().map(Vec::len).max().unwrap_or(1);
    for line in 0..height {
        write!(output, "│")?;
        for (column, width) in widths.iter().enumerate() {
            let value = lines[column].get(line).copied().unwrap_or("");
            let padding = width.saturating_sub(value.width());
            write!(output, " {value}{} │", " ".repeat(padding))?;
        }
        writeln!(output)?;
    }
    Ok(())
}

fn list_description(store: &FileStore, entry: &EntrySummary, locale: Locale) -> String {
    let description = if entry.description.is_empty() {
        "—".to_owned()
    } else {
        entry.description.clone()
    };
    let Some(target) = summary_target(store, entry).filter(|target| !target.exists()) else {
        return description;
    };
    let marker = format_text(locale, "⚠ missing: {}", &[&target.display()]);
    if entry.description.is_empty() {
        marker
    } else {
        format!("{description}  {marker}")
    }
}

fn show(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    selector: &str,
    json: bool,
) -> Result<(), CliError> {
    let entry = service.show(selector)?;
    let settings = effective_settings(store, &entry);
    let source = show_source_text(store, &entry)?;
    let plan = form_plan(entry.meta.kind.as_str(), &source, &settings);
    let state =
        FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).load(&entry.slug);
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if json {
        let parameter_source = plan.source.as_str();
        let fields = plan.fields.iter().map(field_json).collect::<Vec<_>>();
        let mut record = serde_json::json!({
            "name": entry.meta.name,
            "slug": entry.slug,
            "schema": entry.meta.schema,
            "id": entry.meta.id,
            "kind": entry.meta.kind,
            "mode": entry.meta.mode,
            "description": entry.meta.description,
            "source": entry.meta.source,
            "source_hash": entry.meta.source_hash,
            "added_at": entry.meta.added_at,
            "workdir": entry.meta.workdir,
            "interpreter": nonempty(&settings.interpreter),
            "missing": entry_missing(store, &entry),
            "dependencies": settings.dependencies,
            "requires_python": settings.requires_python,
            "needs": settings.needs,
            "template": nonempty(&settings.template),
            "param_source": parameter_source,
            "param_origin": parameter_origin(parameter_source),
            "degraded_reason": degradation_token(plan.degradation),
            "drift": !plan.drift.is_empty(),
            "fields": fields,
            "presets": state.presets.keys().collect::<Vec<_>>(),
            "last_run_at": state.last_run.at,
            "last_exit": state.last_run.exit,
        });
        if entry.meta.kind.as_str() == "prompt" {
            let config = FileConfigStore::new(resolve_config_dir()?);
            let runners = config
                .runners()?
                .into_iter()
                .map(|runner| runner.name)
                .collect::<Vec<_>>();
            let object = record
                .as_object_mut()
                .expect("the show record is a JSON object");
            object.insert(
                "runner".to_owned(),
                serde_json::json!(nonempty(&settings.runner)),
            );
            object.insert("runners_available".to_owned(), serde_json::json!(runners));
            object.insert(
                "interpolate".to_owned(),
                serde_json::json!(settings.interpolate),
            );
        }
        serde_json::to_writer(&mut output, &record)?;
        writeln!(output)?;
    } else {
        write_human_show(
            &mut output,
            store,
            &entry,
            &settings,
            &plan,
            &state,
            active_locale(),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_human_show<W: io::Write>(
    output: &mut W,
    store: &FileStore,
    entry: &Entry,
    settings: &EntrySettings,
    plan: &PreparedFormPlan,
    state: &skit_application::form_state::PersistedFormState,
    locale: Locale,
) -> io::Result<()> {
    writeln!(
        output,
        "{}  ({} · {})",
        entry.meta.name,
        kind_label(locale, entry.meta.kind.as_str()),
        mode_name(entry.meta.mode)
    )?;
    if !entry.meta.description.is_empty() {
        writeln!(output, "  {}", entry.meta.description)?;
    }
    if entry.meta.kind.as_str() != "command" {
        writeln!(
            output,
            "  {}",
            format_text(locale, "Source: {}", &[&entry.meta.source])
        )?;
    }
    if entry.meta.workdir != "origin" {
        writeln!(
            output,
            "  {}",
            format_text(locale, "Working directory: {}", &[&entry.meta.workdir])
        )?;
    }
    if !settings.interpreter.is_empty() {
        writeln!(
            output,
            "  {}",
            format_text(locale, "Interpreter: {}", &[&settings.interpreter])
        )?;
    }
    if let Some(target) = entry_target(store, entry).filter(|target| !target.exists()) {
        writeln!(
            output,
            "  {}",
            format_text(locale, "⚠ missing: {}", &[&target.display()])
        )?;
    }
    if !settings.dependencies.is_empty() {
        writeln!(
            output,
            "  {}",
            format_text(
                locale,
                "Dependencies: {}",
                &[&settings.dependencies.join(", ")],
            )
        )?;
    }
    if !settings.requires_python.is_empty() {
        writeln!(
            output,
            "  {}",
            format_text(
                locale,
                "Python constraint: {}",
                &[&settings.requires_python],
            )
        )?;
    }
    if !settings.needs.is_empty() {
        writeln!(
            output,
            "  {}",
            format_text(locale, "Needs: {}", &[&settings.needs.join(", ")])
        )?;
    }
    if !settings.template.is_empty() {
        writeln!(
            output,
            "  {}",
            format_text(locale, "Command template: {}", &[&settings.template])
        )?;
    }
    if entry.meta.kind.as_str() == "prompt" {
        let runner = if settings.runner.is_empty() {
            text(locale, "(asks at run time)").into_owned()
        } else {
            settings.runner.clone()
        };
        writeln!(
            output,
            "  {}",
            format_text(locale, "Runner: {}", &[&runner])
        )?;
        if !settings.interpolate {
            writeln!(
                output,
                "  {}",
                text(
                    locale,
                    "Variable insertion: off (the body travels as written)"
                )
            )?;
        }
    }
    for line in show_drift_lines(plan, &entry.meta.name, locale) {
        writeln!(output, "{line}")?;
    }
    if plan.degradation.is_some() {
        writeln!(
            output,
            "{}",
            text(
                locale,
                "skit could not model this script's own arguments; pass them after -- instead."
            )
        )?;
    }
    if plan.fields.is_empty() {
        let message = match entry.meta.kind.as_str() {
            "prompt" => "No form fields — arguments after -- go to the selected agent.",
            "command" => "No form fields — arguments after -- are appended to the command.",
            _ => "No form fields — arguments after -- pass straight through to the script.",
        };
        writeln!(output, "  {}", text(locale, message))?;
    } else {
        write_show_fields(output, &plan.fields, locale)?;
    }
    if !state.presets.is_empty() {
        writeln!(
            output,
            "  {}",
            format_text(
                locale,
                "Presets: {}",
                &[&state.presets.keys().cloned().collect::<Vec<_>>().join(", ")],
            )
        )?;
    }
    writeln!(
        output,
        "  {}",
        format_text(locale, "Run it: skit run {}", &[&entry.meta.name])
    )
}

fn write_show_fields<W: io::Write>(
    output: &mut W,
    fields: &[PreparedField],
    locale: Locale,
) -> io::Result<()> {
    let headers = [
        text(locale, "Parameter").into_owned(),
        text(locale, "Type").into_owned(),
        text(locale, "Required").into_owned(),
        text(locale, "Default").into_owned(),
        text(locale, "Choices").into_owned(),
        text(locale, "Secret").into_owned(),
        text(locale, "Help").into_owned(),
    ];
    let rows = fields
        .iter()
        .map(|prepared| {
            let field = &prepared.declaration;
            let shown_default = match &field.default {
                None => "—".to_owned(),
                Some(_) if field.secret => text(locale, "•••").into_owned(),
                Some(default) => {
                    let value = tui_parameter_value(default);
                    if value.is_empty() {
                        "—".to_owned()
                    } else {
                        value
                    }
                }
            };
            let secret = if !field.secret {
                "—".to_owned()
            } else if field.env_source.is_empty() {
                text(locale, "yes").into_owned()
            } else {
                format!("{} ← ${}", text(locale, "yes"), field.env_source)
            };
            let help = if !field.help.is_empty() {
                field.help.clone()
            } else if !field.prompt.is_empty() && field.prompt != field.name {
                field.prompt.clone()
            } else {
                "—".to_owned()
            };
            [
                field.name.clone(),
                field_type(field).to_owned(),
                if field.required {
                    text(locale, "yes").into_owned()
                } else {
                    "—".to_owned()
                },
                shown_default,
                if field.choices.is_empty() {
                    "—".to_owned()
                } else {
                    field.choices.join(", ")
                },
                secret,
                help,
            ]
        })
        .collect::<Vec<_>>();
    write_table(output, &headers, &rows)
}

const fn degradation_token(reason: Option<skit_language::DegradationReason>) -> &'static str {
    match reason {
        Some(skit_language::DegradationReason::Subcommands) => "subparsers",
        Some(_) => "dynamic",
        None => "",
    }
}

fn show_drift_lines(plan: &PreparedFormPlan, entry_name: &str, locale: Locale) -> Vec<String> {
    if let [FormDrift::PromptMissing { names }] = plan.drift.as_slice() {
        return vec![format_text(
            locale,
            "No longer in the prompt (the value would be ignored): {} — edit the body or update parameters with: skit params {}",
            &[&names.join(", "), &entry_name],
        )];
    }
    if plan.drift.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format_text(
        locale,
        "The parameter definitions for {} have drifted from the script:",
        &[&entry_name],
    )];
    for drift in &plan.drift {
        let line = match drift {
            FormDrift::Missing { declaration }
                if declaration.binding == ParameterBinding::EnvDefault =>
            {
                format_text(
                    locale,
                    "{} is no longer read from the environment (its ${...:-default} was removed or overridden by a plain assignment) — your value would be silently ignored. Re-add or resync.",
                    &[&declaration.name],
                )
            }
            FormDrift::Missing { declaration } => format_text(
                locale,
                "{}: injection target no longer exists (dropped from this run's form)",
                &[&declaration.name],
            ),
            FormDrift::TypeChanged { stored, current } => format_text(
                locale,
                "{}: type changed from {} to {} in the source (still injected — double-check the value)",
                &[
                    &stored.name,
                    &stored.parameter_type.as_str(),
                    &current.parameter_type.as_str(),
                ],
            ),
            FormDrift::Rebound { stored, .. } => format_text(
                locale,
                "{}: its prompt no longer matches a unique input/read call; falling back to position (still injected — double-check this lands on the right question, especially if it's a secret)",
                &[&stored.name],
            ),
            FormDrift::PromptMissing { names } => format_text(
                locale,
                "No longer in the prompt (the value would be ignored): {} — edit the body or update parameters with: skit params {}",
                &[&names.join(", "), &entry_name],
            ),
        };
        lines.push(format!("  {line}"));
    }
    lines.push(format_text(
        locale,
        "To refresh the definitions, run: skit params {} --resync",
        &[&entry_name],
    ));
    lines
}

fn show_source_text(store: &FileStore, entry: &Entry) -> Result<String, CliError> {
    if entry.meta.kind.as_str() == "command" || entry.meta.kind.as_str() == "exe" {
        return Ok(String::new());
    }
    let Some(path) = source_path(store, entry) else {
        return Ok(String::new());
    };
    let Ok(bytes) = fs::read(&path) else {
        return Ok(String::new());
    };
    match String::from_utf8(bytes) {
        Ok(source) => Ok(source),
        Err(error) if entry.meta.kind.as_str() == "prompt" => {
            let offset = error.utf8_error().valid_up_to();
            Err(CliError::Failure(
                Message::new("Prompt {} isn't valid UTF-8 (invalid byte at offset {}).")
                    .with(path.display())
                    .with(offset),
            ))
        }
        Err(error) => Ok(String::from_utf8_lossy(error.as_bytes()).into_owned()),
    }
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn effective_settings(store: &FileStore, entry: &Entry) -> EntrySettings {
    let mut settings = EntrySettings::from_meta(&entry.meta);
    if entry.meta.kind.as_str() == "python" && entry.meta.mode == StorageMode::Copy {
        let source = source_path(store, entry).and_then(|path| fs::read(path).ok());
        let effective = effective_uv_metadata_bytes(
            source.as_deref(),
            &UvMetadata {
                dependencies: settings.dependencies.clone(),
                requires_python: settings.requires_python.clone(),
            },
        );
        settings.dependencies = effective.dependencies;
        settings.requires_python = effective.requires_python;
    }
    settings
}

fn uv_edit_error(name: &str, error: UvMetadataEditError) -> CliError {
    match error {
        UvMetadataEditError::NonUtf8OwnBlock => CliError::Usage(
            Message::new(
                "{}'s stored copy isn't valid UTF-8, so skit can't rewrite the script's own dependency block — and that block is what uv reads. Edit it in the script itself: skit edit {}",
            )
            .with(name)
            .with(name),
        ),
        UvMetadataEditError::Language(error) => CliError::Usage(error.message()),
    }
}

fn parameter_origin(source: &str) -> &'static str {
    match source {
        "command" => "command",
        "inject" => "managed",
        "argparse" => "reader",
        "declared" => "declared",
        _ => "none",
    }
}

fn field_json(prepared: &PreparedField) -> serde_json::Value {
    let item = &prepared.declaration;
    let default = item.default.as_ref().map(tui_parameter_value);
    serde_json::json!({
        "key": item.name,
        "label": field_label(item),
        "type": field_type(item),
        "source": item.delivery.as_str(),
        "required": item.required,
        "secret": item.secret,
        "multiple": item.multiple,
        "repeat": item.repeat,
        "degraded": item.degraded,
        "choices": item.choices,
        "default": default,
        "help": item.help,
        "flag": item.flag,
        "action": field_action(item),
        "env_source": item.env_source,
        "delivers_empty": prepared.delivers_empty(),
    })
}

fn field_label(item: &ParamDecl) -> &str {
    if item.delivery == ParameterDelivery::Flag || item.prompt.is_empty() {
        &item.name
    } else {
        &item.prompt
    }
}

fn field_type(item: &ParamDecl) -> &'static str {
    if item.degraded {
        "str"
    } else {
        item.parameter_type.as_str()
    }
}

fn field_action(item: &ParamDecl) -> &str {
    if item.action.is_empty()
        && !item.degraded
        && item.delivery == ParameterDelivery::Flag
        && item.parameter_type == ParameterType::Bool
        && !item.flag.is_empty()
        && matches!(item.default, None | Some(ParameterValue::Bool(false)))
    {
        "store_true"
    } else {
        &item.action
    }
}

#[derive(Debug)]
struct AddOptions {
    source: Option<PathBuf>,
    kind: Option<String>,
    name: Option<String>,
    description: Option<String>,
    reference: bool,
    command_template: Option<String>,
    prompt: bool,
    executable: bool,
    runner: Option<String>,
    no_interpolate: bool,
    dependencies: Vec<String>,
    dependencies_explicit: bool,
    requires_python: Option<String>,
    no_input: bool,
}

fn onboard_add_source(
    kind: &str,
    mode: StorageMode,
    source_bytes: &[u8],
    entry_name: &str,
    no_input: bool,
) -> Result<Vec<u8>, CliError> {
    let source = LosslessSource::from_bytes(source_bytes);
    let plan = onboarding_plan(kind, source.normalized_text());
    if plan.parse_state == OnboardingParseState::ParserUnavailable {
        return Ok(source_bytes.to_vec());
    }

    if mode == StorageMode::Reference {
        if !print_modeled_reader_notice(&plan) {
            humanln!(
                "Reference mode never touches the original file, so parameter setup was skipped."
            );
        }
        return Ok(source_bytes.to_vec());
    }

    print_copy_onboarding_facts(&plan, entry_name);
    let candidates = plan.offered_candidates();
    if candidates.is_empty()
        || no_input
        || !io::stdin().is_terminal()
        || !io::stdout().is_terminal()
    {
        return Ok(source_bytes.to_vec());
    }

    if candidates.len() == 1 {
        humanln!(
            "Found {} parameter candidate (constants / input() calls):",
            1
        );
    } else {
        humanln!(
            "Found {} parameter candidates (constants / input() calls):",
            candidates.len()
        );
    }
    let locale = active_locale();
    let choices = candidates
        .iter()
        .map(|candidate| {
            (
                onboarding_candidate_label(locale, candidate),
                candidate.selected_by_default(),
            )
        })
        .collect::<Vec<_>>();
    let selected = MultiSelect::new()
        .with_prompt(
            text(
                locale,
                "Select the values that skit should manage (Space toggles; Enter accepts)",
            )
            .into_owned(),
        )
        .items_checked(choices)
        .interact_opt()
        .map_err(io::Error::from)?
        .ok_or(CliError::Aborted)?;
    let declarations = selected
        .into_iter()
        .map(|index| candidates[index].declaration.clone())
        .collect::<Vec<_>>();
    if declarations.is_empty() {
        return Ok(source_bytes.to_vec());
    }
    write_managed_params_bytes(kind, source_bytes, &declarations)
        .map_err(|error| CliError::Usage(error.message()))
}

fn print_copy_onboarding_facts(plan: &OnboardingPlan, entry_name: &str) {
    let modeled = print_modeled_reader_notice(plan);
    if !modeled && plan.uses_cli_framework() {
        humanln!(
            "This script parses its own arguments ({}); skit couldn't model them statically, so the run form offers an extra-arguments field.",
            plan.frameworks.join(", ")
        );
    }
    if plan.uses_argv && !plan.uses_cli_framework() {
        humanln!(
            "This script reads command-line arguments; the run form has an extra-arguments field for them."
        );
    }
    if !plan.filename_literals.is_empty() {
        let names = plan
            .filename_literals
            .iter()
            .map(|name| serde_json::to_string(name).expect("a string always serializes as JSON"))
            .collect::<Vec<_>>()
            .join(", ");
        humanln!(
            "💡 {} are written directly inside the code, so skit can't turn them into form fields. To manage one, first give it a name at the top of the script, e.g. OUTPUT = '…' (skit edit {}).",
            names,
            entry_name
        );
    }
}

fn print_modeled_reader_notice(plan: &OnboardingPlan) -> bool {
    let Some(fields) = plan
        .modeled_cli_fields()
        .filter(|fields| !fields.is_empty())
    else {
        return false;
    };
    if fields.len() == 1 {
        humanln!(
            "✓ skit read this script's own arguments ({} field). Running it opens a form — nothing to memorize.",
            fields.len()
        );
    } else {
        humanln!(
            "✓ skit read this script's own arguments ({} fields). Running it opens a form — nothing to memorize.",
            fields.len()
        );
    }
    true
}

fn onboarding_candidate_label(locale: Locale, candidate: &OnboardingCandidate) -> String {
    let declaration = &candidate.declaration;
    let secret = if declaration.secret {
        text(locale, " (secret)")
    } else {
        std::borrow::Cow::Borrowed("")
    };
    let mut label = if declaration.binding == ParameterBinding::Input {
        let ordinal = declaration.order.saturating_add(1);
        let prompt =
            serde_json::to_string(&declaration.prompt).expect("a string always serializes as JSON");
        format_text(locale, "input() #{}: {}{}", &[&ordinal, &prompt, &secret])
    } else {
        let value = declaration.default.as_ref().map_or_else(
            || text(locale, "not set").into_owned(),
            |value| serde_json::to_string(value).expect("a parameter value always serializes"),
        );
        format_text(
            locale,
            "{} ({}) = {}{}",
            &[
                &declaration.name,
                &declaration.parameter_type.as_str(),
                &value,
                &secret,
            ],
        )
    };
    if candidate.demotion.is_some() {
        label.push_str(" — ");
        label.push_str(&text(
            locale,
            "⚠ looks like a loop accumulator — probably not a parameter",
        ));
    }
    label
}

fn add(service: &LibraryService<FileStore>, options: AddOptions) -> Result<(), CliError> {
    let config_dir = resolve_config_dir()?;
    add_with_config(service, &config_dir, options)
}

fn add_with_config(
    service: &LibraryService<FileStore>,
    config_dir: &Path,
    options: AddOptions,
) -> Result<(), CliError> {
    let AddOptions {
        source,
        kind,
        name,
        description,
        reference,
        command_template,
        prompt,
        executable,
        runner,
        no_interpolate,
        dependencies,
        dependencies_explicit,
        mut requires_python,
        no_input,
    } = options;
    let dependencies_explicit = dependencies_explicit || !dependencies.is_empty();
    let mut dependencies = dependencies
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    let requires_python_explicit = requires_python.is_some();
    if prompt {
        validate_prompt_runner_in(&FileConfigStore::new(config_dir), runner.as_deref())?;
    }
    let explicit_executable = executable || kind.as_deref() == Some("exe");

    if let Some(template) = command_template {
        if template.trim().is_empty() {
            return Err(CliError::Usage(Message::new(
                "a command template cannot be empty",
            )));
        }
        if dependencies_explicit || requires_python.is_some() {
            return Err(CliError::Usage(Message::new(
                "command entries do not take package dependencies",
            )));
        }
        let kind = EntryKind::parse("command".to_owned()).expect("command kind is valid");
        let name = name.ok_or_else(|| {
            CliError::Usage(Message::new("a --cmd entry needs an explicit --name"))
        })?;
        let parameters = placeholder_params("command", &template);
        let settings = EntrySettings {
            params: parameters.iter().map(|item| item.name.clone()).collect(),
            parameters,
            template,
            ..EntrySettings::default()
        };
        let entry = service.add(CreateEntry {
            name,
            kind,
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: description.unwrap_or_default(),
            payload: None,
            settings,
        })?;
        print_add_summary(service.repository(), &entry)?;
        return Ok(());
    }

    let Some(input) = source.as_deref() else {
        return Err(CliError::Usage(Message::new(
            "add needs a source path or --cmd COMMAND",
        )));
    };
    if reference && input == Path::new("-") {
        return Err(CliError::Usage(Message::new(
            "standard input cannot be a referenced entry",
        )));
    }
    if input == Path::new("-") && (executable || kind.as_deref().is_some_and(|kind| kind == "exe"))
    {
        return Err(CliError::Usage(Message::new(
            "standard input cannot be an executable entry",
        )));
    }
    let from_stdin = input == Path::new("-");
    let (source, source_record, mut bytes, permissions, source_is_regular) = if from_stdin {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        (
            PathBuf::from("stdin"),
            String::new(),
            bytes,
            SourcePermissions::default(),
            true,
        )
    } else {
        let expanded = expand_user_path(input);
        let source = fs::canonicalize(&expanded)
            .map_err(|error| source_error("resolve", &expanded, error))?;
        let snapshot = read_source(&source, explicit_executable)?;
        let source_record = source.display().to_string();
        (
            source,
            source_record,
            snapshot.bytes,
            snapshot.permissions,
            snapshot.is_regular,
        )
    };
    let name = name.unwrap_or_else(|| source_default_name(&source));
    let mut source_text = LosslessSource::from_bytes(&bytes)
        .normalized_text()
        .to_owned();
    let shebang = source_text
        .lines()
        .next()
        .filter(|line| line.starts_with("#!"));
    let file_is_executable = permissions.unix_mode.is_some_and(|mode| mode & 0o111 != 0);
    let inferred = if prompt {
        Some("prompt")
    } else if executable {
        Some("exe")
    } else {
        infer_kind(&source, shebang, file_is_executable)
    };
    if kind.is_none() && inferred.is_none() && from_stdin && shebang.is_some() {
        return Err(CliError::Usage(Message::new(
            "The piped text's #! names no interpreter skit knows — pass --kind <language> to choose one.",
        )));
    }
    let kind = kind
        .as_deref()
        .or(inferred)
        .or(from_stdin.then_some("python"))
        .ok_or_else(|| {
            CliError::Usage(Message::new(
                "could not infer the entry kind; pass --kind KIND",
            ))
        })?;
    let kind =
        EntryKind::parse(kind.to_owned()).map_err(|error| RepositoryError::InvalidMutation {
            reason: error.message(),
        })?;
    let kind_name = kind.as_str().to_owned();
    let description = description.unwrap_or_else(|| suggest_description(&kind_name, &bytes));
    if no_interpolate && kind_name != "prompt" {
        return Err(CliError::Usage(Message::new(
            "--no-interpolate only applies to prompt entries",
        )));
    }
    let has_own_uv_metadata = kind_name == "python" && has_uv_metadata_block_bytes(&bytes);
    let uv_metadata = (kind_name == "python")
        .then(|| read_uv_metadata(&source_text))
        .flatten();
    if !dependencies_explicit && !has_own_uv_metadata && dependencies.is_empty() {
        let source_dir = (!source_record.is_empty())
            .then(|| source.parent())
            .flatten();
        dependencies = external_dependencies_at(&kind_name, &source_text, source_dir);
    }
    if !requires_python_explicit && !has_own_uv_metadata {
        requires_python = shebang
            .and_then(shebang_program)
            .and_then(python_version_pin);
    }
    if let Some(value) = &requires_python
        && matches!(value.trim(), "-" | "none")
    {
        requires_python = None;
    }
    let supports_dependencies = matches!(kind_name.as_str(), "python" | "js" | "ts");
    if dependencies_explicit && !supports_dependencies {
        return Err(CliError::Usage(
            Message::new("{} entries do not take package dependencies").with(kind_name),
        ));
    }
    if requires_python.is_some() && kind_name != "python" {
        return Err(CliError::Usage(
            Message::new("a Python constraint does not apply to {} entries").with(kind_name),
        ));
    }
    if kind_name == "python" {
        for requirement in &dependencies {
            validate_pep508_requirement(requirement)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
        if let Some(version) = requires_python.as_deref().filter(|value| !value.is_empty()) {
            validate_pep440_specifiers(version)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
    }
    if reference
        && matches!(kind_name.as_str(), "js" | "ts")
        && (dependencies_explicit || !dependencies.is_empty())
    {
        return Err(CliError::Usage(Message::new(
            "reference entries do not take managed dependencies",
        )));
    }
    if runner.is_some() && kind_name != "prompt" {
        return Err(CliError::Usage(Message::new(
            "--runner only applies to prompt entries",
        )));
    }
    if kind_name == "prompt" {
        validate_prompt_runner_in(&FileConfigStore::new(config_dir), runner.as_deref())?;
    }
    let stored_name = payload_stored_name(&kind, &source);
    let mode = if reference || kind_name == "exe" {
        StorageMode::Reference
    } else {
        StorageMode::Copy
    };
    let interpreter = shebang
        .and_then(shebang_program)
        .filter(|_| {
            !matches!(kind_name.as_str(), "python" | "prompt" | "command" | "exe")
                && infer_kind(Path::new("source"), shebang, false) == Some(kind_name.as_str())
        })
        .unwrap_or_default()
        .to_owned();
    if has_own_uv_metadata
        && let Some(metadata) = &uv_metadata
        && !metadata.dependencies.is_empty()
    {
        humanln!(
            "The script declares its own dependencies (PEP 723): {}",
            metadata.dependencies.join(", ")
        );
    }
    let mut metadata_dependencies = if has_own_uv_metadata {
        Vec::new()
    } else {
        dependencies
    };
    let mut metadata_requires_python = if has_own_uv_metadata {
        String::new()
    } else {
        requires_python.unwrap_or_default()
    };
    if kind_name == "python"
        && mode == StorageMode::Copy
        && (!metadata_dependencies.is_empty() || !metadata_requires_python.is_empty())
        && let Ok(strict_source) = String::from_utf8(bytes.clone())
    {
        let rewritten = write_uv_metadata(
            &strict_source,
            &metadata_dependencies,
            &metadata_requires_python,
        )
        .map_err(|error| CliError::Usage(error.message()))?;
        bytes = rewritten.into_bytes();
        source_text = LosslessSource::from_bytes(&bytes)
            .normalized_text()
            .to_owned();
        metadata_dependencies.clear();
        metadata_requires_python.clear();
    }
    bytes = onboard_add_source(&kind_name, mode, &bytes, &name, no_input)?;
    let payload = if kind_name == "exe" && !source_is_regular {
        None
    } else {
        Some(EntryPayload {
            bytes,
            stored_name: Some(stored_name),
            permissions,
        })
    };
    let mut settings = EntrySettings {
        dependencies: metadata_dependencies,
        requires_python: metadata_requires_python,
        interpreter,
        runner: runner.unwrap_or_default().trim().to_owned(),
        interpolate: !no_interpolate,
        ..EntrySettings::default()
    };
    if kind_name == "prompt" && settings.interpolate {
        let detected = placeholder_params("prompt", &source_text);
        settings.parameters = if detected.len() <= 30 {
            detected
        } else {
            Vec::new()
        };
        settings.params = settings
            .parameters
            .iter()
            .map(|item| item.name.clone())
            .collect();
    }
    let workdir = add_workdir(&kind, mode).to_owned();
    let entry = service.add(CreateEntry {
        name,
        kind,
        mode,
        source: source_record,
        workdir,
        description,
        payload,
        settings,
    })?;
    print_add_summary(service.repository(), &entry)?;
    Ok(())
}

fn print_add_summary(store: &FileStore, entry: &Entry) -> Result<(), CliError> {
    let settings = effective_settings(store, entry);
    let source = show_source_text(store, entry)?;
    let declarations = match entry.meta.kind.as_str() {
        "command" | "prompt" => {
            form_plan(entry.meta.kind.as_str(), &source, &settings).declarations()
        }
        "exe" => Vec::new(),
        kind => managed_params(kind, &source),
    };
    let managed = if entry.meta.kind.as_str() == "command" {
        Vec::new()
    } else {
        declarations
            .iter()
            .map(|declaration| declaration.name.clone())
            .collect::<Vec<_>>()
    };
    let secrets = declarations
        .iter()
        .filter(|declaration| declaration.secret)
        .map(|declaration| declaration.name.clone())
        .collect::<Vec<_>>();

    if entry.meta.kind.as_str() == "command" && !settings.params.is_empty() {
        humanln!(
            "Detected parameters: {} (the run form asks for them; your last values are remembered)",
            settings.params.join(", ")
        );
    }
    if supports_storage_modes(&entry.meta.kind) {
        let mode = match entry.meta.mode {
            StorageMode::Copy => "copy",
            StorageMode::Reference => "reference",
        };
        humanln!("Added: {} ({} mode)", entry.meta.name, mode);
    } else {
        humanln!("Added: {}", entry.meta.name);
    }
    if !entry.meta.description.is_empty() {
        println!(
            "  {}",
            format_text(
                active_locale(),
                "Description: {}",
                &[&entry.meta.description],
            )
        );
    }
    if !settings.dependencies.is_empty() {
        println!(
            "  {}",
            format_text(
                active_locale(),
                "Dependencies: {}",
                &[&settings.dependencies.join(", ")],
            )
        );
    }
    if !managed.is_empty() {
        println!(
            "  {}",
            format_text(
                active_locale(),
                "Managed parameters: {}",
                &[&managed.join(", ")],
            )
        );
    }
    println!(
        "  {}",
        format_text(active_locale(), "Run it: skit run {}", &[&entry.meta.name],)
    );
    if !secrets.is_empty() {
        humanln!(
            "Secret parameter values are never saved by skit: {}",
            secrets.join(", ")
        );
        if entry.meta.kind.as_str() == "prompt" {
            humanln!(
                "When this prompt runs, the selected agent receives those values as plaintext and may log or sync them."
            );
        }
    }
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
    humanln!("Description updated: {} ({})", entry.meta.name, entry.slug);
    Ok(())
}

fn rename(service: &LibraryService<FileStore>, selector: &str, name: &str) -> Result<(), CliError> {
    let held = service.show(selector)?;
    let claimed = service.claim_identity(&held)?;
    let entry = service.rename(&claimed, name)?;
    humanln!("Renamed: {} ({})", entry.meta.name, entry.slug);
    Ok(())
}

fn user_confirmed(answer: &str, default: bool) -> bool {
    let answer = answer.trim();
    if answer.is_empty() {
        return default;
    }
    matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes")
}

fn prompt_confirmation(question: &str, default: bool) -> Result<bool, CliError> {
    print!("{question}");
    io::stdout().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Err(CliError::Aborted);
    }
    Ok(user_confirmed(&answer, default))
}

fn remove(
    service: &LibraryService<FileStore>,
    selector: &str,
    yes: bool,
    no_input: bool,
) -> Result<(), CliError> {
    let held = service.show(selector)?;
    if !yes {
        if no_input || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(CliError::ConfirmationRequired);
        }
        let question = format_text(
            active_locale(),
            "Remove \"{}\"? [y/N]: ",
            &[&held.meta.name],
        );
        if !prompt_confirmation(&question, false)? {
            return Err(CliError::Aborted);
        }
    }
    let claimed = service.claim_identity(&held)?;
    let slug = claimed.slug.clone();
    let name = service.remove(&claimed)?;
    FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).forget(&slug)?;
    humanln!("Removed: {}", name);
    Ok(())
}

fn edit(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    selector: &str,
    no_input: bool,
) -> Result<(), CliError> {
    let config_dir = resolve_config_dir()?;
    edit_with_config(service, store, &config_dir, selector, no_input)
}

fn edit_with_config(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    config_dir: &Path,
    selector: &str,
    no_input: bool,
) -> Result<(), CliError> {
    let held = match service.show(selector) {
        Ok(entry) => entry,
        Err(RepositoryError::NotFound { .. }) => {
            if no_input || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return Err(CliError::Failure(
                    Message::new("no editable entry is named {}").quoted(selector),
                ));
            }
            let question = format_text(
                active_locale(),
                "No editable entry is named \"{}\". Create a script now? [Y/n]: ",
                &[&selector],
            );
            if !prompt_confirmation(&question, true)? {
                return Err(CliError::Aborted);
            }
            return add_command(
                service,
                AddOptions {
                    source: None,
                    kind: None,
                    name: Some(selector.to_owned()),
                    description: None,
                    reference: false,
                    command_template: None,
                    prompt: false,
                    executable: false,
                    runner: None,
                    no_interpolate: false,
                    dependencies: Vec::new(),
                    dependencies_explicit: false,
                    requires_python: None,
                    no_input: false,
                },
                true,
            );
        }
        Err(error) => return Err(error.into()),
    };
    if matches!(held.meta.kind.as_str(), "command" | "exe") {
        return Err(CliError::Usage(
            Message::new("entry {} does not have an editable source").with(&held.slug),
        ));
    }
    let target = source_path(store, &held).ok_or_else(|| {
        CliError::Usage(Message::new("entry {} does not have an editable source").with(&held.slug))
    })?;
    let editor = FileConfigStore::new(config_dir)
        .get("editor")
        .unwrap_or_default();
    let editor = if editor.trim().is_empty() {
        env::var("VISUAL")
            .or_else(|_| env::var("EDITOR"))
            .map_err(|_| CliError::Usage(Message::new("configure an editor before you use edit")))?
    } else {
        editor
    };
    let mut argv = shlex::split(&editor)
        .ok_or_else(|| CliError::Usage(Message::new("the editor command has invalid quoting")))?;
    if argv.is_empty() {
        return Err(CliError::Usage(Message::new("the editor command is empty")));
    }

    if held.meta.mode == StorageMode::Reference {
        let status = ProcessCommand::new(&argv[0])
            .args(&argv[1..])
            .arg(&target)
            .status()
            .map_err(|error| source_error("start editor for", &target, error))?;
        if !status.success() {
            return Err(CliError::Usage(
                Message::new("the editor exited with status {}").with(status.code().unwrap_or(1)),
            ));
        }
        return Ok(());
    }

    let original = fs::read(&target).map_err(|error| source_error("read", &target, error))?;
    let temp = tempfile::tempdir().map_err(CliError::Io)?;
    let staged = temp.path().join(
        target
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("script")),
    );
    fs::write(&staged, &original).map_err(|error| source_error("stage", &staged, error))?;
    let status = ProcessCommand::new(argv.remove(0))
        .args(argv)
        .arg(&staged)
        .status()
        .map_err(|error| source_error("start editor for", &staged, error))?;
    if !status.success() {
        return Err(CliError::Usage(
            Message::new("the editor exited with status {}").with(status.code().unwrap_or(1)),
        ));
    }
    let edited = fs::read(&staged).map_err(|error| source_error("read", &staged, error))?;
    if edited != original {
        let claimed = service.claim_identity(&held)?;
        service.commit_copy_edit(&claimed, &edited, &held.meta.source_hash)?;
        humanln!("Edited: {} ({})", held.meta.name, held.slug);
    }
    Ok(())
}

fn open_editor(target: &Path) -> Result<(), CliError> {
    open_editor_in(&resolve_config_dir()?, target)
}

fn open_editor_in(config_dir: &Path, target: &Path) -> Result<(), CliError> {
    let configured = FileConfigStore::new(config_dir)
        .get("editor")
        .unwrap_or_default();
    let editor = if configured.trim().is_empty() {
        env::var("VISUAL")
            .or_else(|_| env::var("EDITOR"))
            .map_err(|_| {
                CliError::Usage(Message::new("configure an editor before you use --edit"))
            })?
    } else {
        configured
    };
    let mut argv = shlex::split(&editor)
        .ok_or_else(|| CliError::Usage(Message::new("the editor command has invalid quoting")))?;
    if argv.is_empty() {
        return Err(CliError::Usage(Message::new("the editor command is empty")));
    }
    let status = ProcessCommand::new(argv.remove(0))
        .args(argv)
        .arg(target)
        .status()
        .map_err(|error| source_error("start editor for", target, error))?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError::Usage(
            Message::new("the editor exited with status {}").with(status.code().unwrap_or(1)),
        ))
    }
}

fn deps(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    args: DepsArgs,
) -> Result<(), CliError> {
    let held = service.show(&args.selector)?;
    let original_settings = EntrySettings::from_meta(&held.meta);
    let mut settings = original_settings.clone();
    let kind = held.meta.kind.as_str().to_owned();
    if args.clear && !args.dependencies.is_empty() {
        return Err(CliError::Usage(Message::new(
            "use --dep or --clear, not both",
        )));
    }
    if args.clear_needs && !args.needs.is_empty() {
        return Err(CliError::Usage(Message::new(
            "use --need or --clear-needs, not both",
        )));
    }
    let dependencies_edit = if args.clear {
        Some(Vec::new())
    } else if args.dependencies.is_empty() {
        None
    } else {
        Some(
            args.dependencies
                .iter()
                .map(|item| item.trim().to_owned())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>(),
        )
    };
    let python_edit = args.requires_python.clone();
    let package_change = dependencies_edit.is_some() || python_edit.is_some();
    if package_change && !matches!(kind.as_str(), "python" | "js" | "ts") {
        return Err(CliError::Usage(
            Message::new("{} does not take package dependencies; only --need applies")
                .with(held.meta.name),
        ));
    }
    if args.requires_python.is_some() && kind != "python" {
        return Err(CliError::Usage(
            Message::new("a Python constraint does not apply to {} entries").with(kind),
        ));
    }
    if package_change
        && matches!(kind.as_str(), "js" | "ts")
        && held.meta.mode == StorageMode::Reference
        && dependencies_edit
            .as_ref()
            .is_some_and(|items| !items.is_empty())
    {
        return Err(CliError::Usage(Message::new(
            "managed dependencies require copy storage",
        )));
    }
    let python_copy = kind == "python" && held.meta.mode == StorageMode::Copy;
    let source = python_copy
        .then(|| source_path(store, &held))
        .flatten()
        .and_then(|path| fs::read(path).ok());
    let stored_uv = UvMetadata {
        dependencies: settings.dependencies.clone(),
        requires_python: settings.requires_python.clone(),
    };
    let mut plan = plan_uv_metadata_edit(
        source.as_deref(),
        &stored_uv,
        dependencies_edit.clone(),
        python_edit.clone(),
    )
    .map_err(|error| uv_edit_error(&held.meta.name, error))?;
    if kind == "python" {
        for requirement in dependencies_edit.as_deref().unwrap_or_default() {
            validate_pep508_requirement(requirement)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
        if let Some(version) = python_edit.as_deref() {
            let normalized = if matches!(version.trim().to_ascii_lowercase().as_str(), "-" | "none")
            {
                ""
            } else {
                version.trim()
            };
            if !normalized.is_empty() {
                validate_pep440_specifiers(normalized)
                    .map_err(|error| CliError::Usage(error.message()))?;
            }
        }
    }
    if package_change {
        settings.dependencies.clone_from(&plan.stored.dependencies);
        settings
            .requires_python
            .clone_from(&plan.stored.requires_python);
    } else {
        // The read view uses effective values without turning source-only values into metadata.
        plan.effective = effective_uv_metadata_bytes(source.as_deref(), &stored_uv);
        plan.rewritten_source = None;
    }
    if args.clear_needs {
        settings.needs.clear();
    } else if !args.needs.is_empty() {
        settings.needs = args
            .needs
            .into_iter()
            .map(|item| item.trim().to_owned())
            .filter(|item| !item.is_empty())
            .collect();
    }

    if package_change
        && matches!(kind.as_str(), "js" | "ts")
        && dependencies_edit.as_ref().is_some_and(Vec::is_empty)
    {
        // Cleanup can fail on a locked tree. Do it before metadata so the request is retryable.
        clear_javascript_dependencies(&store.entry_dir_path(&held.slug))?;
    }

    let changed = settings != original_settings || plan.rewritten_source.is_some();
    let held = if changed {
        let claimed = service.claim_identity(&held)?;
        service.update_entry(
            &claimed,
            UpdateEntry {
                name: held.meta.name.clone(),
                description: held.meta.description.clone(),
                settings: settings.clone(),
                workdir: held.meta.workdir.clone(),
                source: plan.rewritten_source,
                expected_source_hash: held.meta.source_hash.clone(),
            },
        )?
    } else {
        held
    };
    let mut output = EntrySettings::from_meta(&held.meta);
    output.dependencies = plan.effective.dependencies;
    output.requires_python = plan.effective.requires_python;
    output.needs = settings.needs;
    write_deps(&output, args.json)
}

fn write_deps(settings: &EntrySettings, json: bool) -> Result<(), CliError> {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "dependencies": settings.dependencies,
                "requires_python": settings.requires_python,
                "needs": settings.needs,
            })
        );
    } else {
        humanln!("Dependencies: {}", settings.dependencies.join(", "));
        humanln!("Python constraint: {}", settings.requires_python);
        humanln!("Required commands: {}", settings.needs.join(", "));
    }
    Ok(())
}

fn reconcile_template_parameters(template: &str, current: &[ParamDecl]) -> Vec<ParamDecl> {
    let mut parameters = placeholder_params("command", template)
        .into_iter()
        .map(|detected| {
            current
                .iter()
                .find(|item| {
                    item.name == detected.name && item.delivery == ParameterDelivery::Placeholder
                })
                .cloned()
                .unwrap_or(detected)
        })
        .collect::<Vec<_>>();
    parameters.extend(
        current
            .iter()
            .filter(|item| item.delivery != ParameterDelivery::Placeholder)
            .cloned(),
    );
    parameters
}

fn prepare_source_management(
    kind: &str,
    mode: StorageMode,
    mut source: String,
    resync: bool,
    manage: &[String],
    unmanage: &[String],
    normalize: &[String],
) -> Result<(String, Vec<ParamDecl>), CliError> {
    if !resync && manage.is_empty() && unmanage.is_empty() && normalize.is_empty() {
        let managed = managed_params(kind, &source);
        return Ok((source, managed));
    }
    if mode == StorageMode::Reference {
        return Err(CliError::Usage(Message::new(
            "source management applies only to a stored copy",
        )));
    }
    let mut managed = managed_params(kind, &source);
    let candidates = detect_candidates(kind, &source);
    if resync {
        managed = managed
            .into_iter()
            .filter_map(|current| {
                candidates
                    .iter()
                    .find(|candidate| candidate.name == current.name)
                    .cloned()
                    .map(|mut candidate| {
                        candidate.secret = current.secret;
                        candidate.env_source = current.env_source;
                        if !current.prompt.is_empty() {
                            candidate.prompt = current.prompt;
                        }
                        candidate
                    })
            })
            .collect();
    }
    for name in manage {
        if managed.iter().any(|item| item.name == *name) {
            continue;
        }
        let candidate = candidates
            .iter()
            .find(|item| item.name == *name)
            .cloned()
            .ok_or_else(|| {
                CliError::Usage(Message::new("unknown source parameter: {}").with(name))
            })?;
        managed.push(candidate);
    }
    if !unmanage.is_empty() {
        managed.retain(|item| !unmanage.contains(&item.name));
    }
    for name in normalize {
        if kind != "shell" {
            return Err(CliError::Usage(Message::new(
                "--normalize applies only to shell entries",
            )));
        }
        source = normalize_shell_default(&source, name)
            .map_err(|error| CliError::Usage(error.message()))?;
        let normalized = detect_candidates(kind, &source)
            .into_iter()
            .find(|item| item.name == *name)
            .ok_or_else(|| CliError::Usage(Message::new("could not normalize {}").with(name)))?;
        if let Some(item) = managed.iter_mut().find(|item| item.name == *name) {
            *item = normalized;
        } else {
            managed.push(normalized);
        }
    }
    Ok((source, managed))
}

fn params(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    args: ParamsArgs,
) -> Result<(), CliError> {
    let held = service.show(&args.selector)?;
    let kind = held.meta.kind.as_str();
    let has_source_operation = args.resync
        || !args.manage.is_empty()
        || !args.unmanage.is_empty()
        || !args.normalize.is_empty();
    let has_declared_schema_operation = !args.add.is_empty()
        || !args.remove.is_empty()
        || !args.parameter_types.is_empty()
        || !args.defaults.is_empty()
        || !args.choices.is_empty()
        || !args.delivery.is_empty()
        || !args.bindings.is_empty()
        || !args.flags.is_empty()
        || !args.multiple.is_empty()
        || !args.no_multiple.is_empty()
        || !args.repeat.is_empty()
        || !args.no_repeat.is_empty()
        || !args.env_targets.is_empty()
        || !args.actions.is_empty()
        || !args.help_text.is_empty()
        || !args.required.is_empty()
        || !args.optional.is_empty();
    let has_shared_parameter_tweaks = !args.prompts.is_empty()
        || !args.env_sources.is_empty()
        || !args.secret.is_empty()
        || !args.no_secret.is_empty();
    let source_parameter_kind = matches!(
        kind,
        "python" | "shell" | "js" | "ts" | "fish" | "powershell"
    );
    if source_parameter_kind && has_declared_schema_operation {
        return Err(CliError::Usage(
            Message::new("{} manages its parameter schema in the stored source")
                .with(held.meta.name),
        ));
    }
    let has_source_schema_operation =
        has_source_operation || (source_parameter_kind && has_shared_parameter_tweaks);
    let has_metadata_schema_operation =
        !source_parameter_kind && (has_declared_schema_operation || has_shared_parameter_tweaks);
    let has_launch_policy =
        args.workdir.is_some() || args.template.is_some() || args.interpreter.is_some();
    let has_runner_policy = args.runner.is_some();
    let has_interpolation_policy = args.interpolate || args.no_interpolate;
    let exclusive_operations = [
        has_source_schema_operation,
        has_metadata_schema_operation,
        has_launch_policy,
        has_runner_policy,
        has_interpolation_policy,
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if exclusive_operations > 1 {
        return Err(CliError::Usage(Message::new(
            "run source, schema, launch, runner, and interpolation changes as separate params operations",
        )));
    }
    if !args.normalize.is_empty()
        && (args.resync
            || !args.manage.is_empty()
            || !args.unmanage.is_empty()
            || has_shared_parameter_tweaks)
    {
        return Err(CliError::Usage(Message::new(
            "--normalize must be a separate params operation",
        )));
    }
    if has_runner_policy {
        if kind != "prompt" {
            return Err(CliError::Usage(Message::new(
                "--runner only applies to prompt entries",
            )));
        }
        validate_prompt_runner_in(
            &FileConfigStore::new(resolve_config_dir()?),
            args.runner.as_deref(),
        )?;
    }
    if has_interpolation_policy && kind != "prompt" {
        return Err(CliError::Usage(Message::new(
            "--interpolate only applies to prompt entries",
        )));
    }
    if args.template.is_some() && kind != "command" {
        return Err(CliError::Usage(Message::new(
            "--template only applies to command entries",
        )));
    }
    if args.template.as_deref().is_some_and(str::is_empty) {
        return Err(CliError::Usage(Message::new(
            "a command template cannot be empty",
        )));
    }
    if args.interpreter.is_some()
        && !matches!(
            kind,
            "shell" | "fish" | "powershell" | "ruby" | "perl" | "lua" | "r" | "js" | "ts"
        )
    {
        return Err(CliError::Usage(Message::new(
            "--interpreter only applies to interpreted entries",
        )));
    }
    let mut held = held;
    let original_source = source_path(store, &held)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let mut settings = EntrySettings::from_meta(&held.meta);
    let (mut source, prepared_managed) = prepare_source_management(
        held.meta.kind.as_str(),
        held.meta.mode,
        original_source.clone(),
        args.resync,
        &args.manage,
        &args.unmanage,
        &args.normalize,
    )?;
    let mut declarations = if source_parameter_kind && has_source_schema_operation {
        form_params_from_managed(prepared_managed, &settings)
    } else {
        form_params(held.meta.kind.as_str(), &source, &settings)
    };
    for item in &settings.parameters {
        if !declarations.iter().any(|current| current.name == item.name) {
            declarations.push(item.clone());
        }
    }
    let original_declarations = declarations.clone();
    let mut tweaked_names = BTreeSet::new();
    for specification in args
        .parameter_types
        .iter()
        .chain(&args.defaults)
        .chain(&args.choices)
        .chain(&args.delivery)
        .chain(&args.bindings)
        .chain(&args.flags)
        .chain(&args.env_targets)
        .chain(&args.actions)
        .chain(&args.help_text)
        .chain(&args.prompts)
        .chain(&args.env_sources)
    {
        if let Some((name, _)) = specification.split_once('=') {
            tweaked_names.insert(name.to_owned());
        }
    }
    for name in args
        .multiple
        .iter()
        .chain(&args.no_multiple)
        .chain(&args.repeat)
        .chain(&args.no_repeat)
        .chain(&args.required)
        .chain(&args.optional)
        .chain(&args.secret)
        .chain(&args.no_secret)
    {
        tweaked_names.insert(name.clone());
    }
    let mut changed = false;

    for name in args.add {
        if declarations.iter().any(|item| item.name == name) {
            return Err(CliError::Usage(
                Message::new("parameter already exists: {}").with(name),
            ));
        }
        declarations.push(ParamDecl::new(name));
        changed = true;
    }
    if !args.remove.is_empty() {
        let before = declarations.len();
        declarations.retain(|item| !args.remove.contains(&item.name));
        changed |= declarations.len() != before;
    }
    let tweak_baseline = declarations.clone();
    for spec in args.parameter_types {
        let (name, value) = assignment(&spec, "type")?;
        parameter_mut(&mut declarations, name)?.parameter_type = parse_parameter_type(value)?;
        changed = true;
    }
    for spec in args.choices {
        let (name, value) = assignment(&spec, "choices")?;
        parameter_mut(&mut declarations, name)?.choices = value
            .split(',')
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect();
        changed = true;
    }
    for spec in args.defaults {
        let (name, value) = assignment(&spec, "default")?;
        let item = parameter_mut(&mut declarations, name)?;
        item.default = Some(
            coerce_default(value, item.parameter_type)
                .map_err(|error| CliError::Usage(error.message()))?,
        );
        changed = true;
    }
    for spec in args.delivery {
        let (name, value) = assignment(&spec, "delivery")?;
        parameter_mut(&mut declarations, name)?.delivery = parse_delivery(value)?;
        changed = true;
    }
    for spec in args.bindings {
        let (name, value) = assignment(&spec, "binding")?;
        let item = parameter_mut(&mut declarations, name)?;
        item.binding = parse_binding(value)?;
        *item = item.clone().normalized();
        changed = true;
    }
    for spec in args.flags {
        let (name, value) = assignment(&spec, "flag")?;
        parameter_mut(&mut declarations, name)?.flag = value.to_owned();
        changed = true;
    }
    changed |= set_bool(
        &mut declarations,
        &args.multiple,
        |item| &mut item.multiple,
        true,
    )?;
    changed |= set_bool(
        &mut declarations,
        &args.no_multiple,
        |item| &mut item.multiple,
        false,
    )?;
    changed |= set_bool(
        &mut declarations,
        &args.repeat,
        |item| &mut item.repeat,
        true,
    )?;
    changed |= set_bool(
        &mut declarations,
        &args.no_repeat,
        |item| &mut item.repeat,
        false,
    )?;
    for spec in args.env_targets {
        let (name, value) = assignment(&spec, "environment target")?;
        parameter_mut(&mut declarations, name)?.env_target = value.to_owned();
        changed = true;
    }
    for spec in args.actions {
        let (name, value) = assignment(&spec, "action")?;
        parameter_mut(&mut declarations, name)?.action = value.to_owned();
        changed = true;
    }
    for spec in args.help_text {
        let (name, value) = assignment(&spec, "help text")?;
        parameter_mut(&mut declarations, name)?.help = value.to_owned();
        changed = true;
    }
    for spec in args.prompts {
        let (name, value) = assignment(&spec, "prompt")?;
        let item = parameter_mut(&mut declarations, name)?;
        if source_parameter_kind && item.binding == ParameterBinding::None {
            return Err(CliError::Usage(
                Message::new("parameter {} is not managed in the stored source").with(name),
            ));
        }
        item.prompt = value.to_owned();
        changed = true;
    }
    for spec in args.env_sources {
        let (name, value) = assignment(&spec, "environment source")?;
        let item = parameter_mut(&mut declarations, name)?;
        if source_parameter_kind && item.binding == ParameterBinding::None {
            return Err(CliError::Usage(
                Message::new("parameter {} is not managed in the stored source").with(name),
            ));
        }
        item.env_source = value.to_owned();
        changed = true;
    }
    changed |= set_bool(
        &mut declarations,
        &args.required,
        |item| &mut item.required,
        true,
    )?;
    changed |= set_bool(
        &mut declarations,
        &args.optional,
        |item| &mut item.required,
        false,
    )?;
    if source_parameter_kind {
        for name in args.secret.iter().chain(&args.no_secret) {
            let item = declarations
                .iter()
                .find(|item| item.name == *name)
                .ok_or_else(|| CliError::Usage(Message::new("unknown parameter: {}").with(name)))?;
            if item.binding == ParameterBinding::None {
                return Err(CliError::Usage(
                    Message::new("parameter {} is not managed in the stored source").with(name),
                ));
            }
        }
    }
    changed |= set_bool(
        &mut declarations,
        &args.secret,
        |item| &mut item.secret,
        true,
    )?;
    changed |= set_bool(
        &mut declarations,
        &args.no_secret,
        |item| &mut item.secret,
        false,
    )?;

    for name in tweaked_names {
        let Some(previous) = tweak_baseline.iter().find(|item| item.name == name) else {
            continue;
        };
        let item = parameter_mut(&mut declarations, &name)?;
        if let Err(error) = finish_parameter_edit(item) {
            *item = previous.clone();
            eprintln!("{}", error.message().localize(active_locale()));
        }
    }
    if has_metadata_schema_operation {
        changed = declarations != original_declarations;
    }

    let mut workdir = held.meta.workdir.clone();
    if let Some(value) = args.workdir {
        workdir = value;
        changed = true;
    }
    if let Some(value) = args.template {
        settings.template = value;
        declarations = reconcile_template_parameters(&settings.template, &declarations);
        changed = true;
    }
    if let Some(value) = args.interpreter {
        settings.interpreter = value;
        changed = true;
    }
    if let Some(value) = args.runner {
        settings.runner = value.trim().to_owned();
        changed = true;
    }
    if args.interpolate || args.no_interpolate {
        settings.interpolate = args.interpolate;
        changed = true;
    }
    if source_parameter_kind && has_source_schema_operation {
        let managed = declarations
            .iter()
            .filter(|item| item.binding != ParameterBinding::None)
            .cloned()
            .collect::<Vec<_>>();
        source = write_managed_params(held.meta.kind.as_str(), &source, &managed)
            .map_err(|error| CliError::Usage(error.message()))?;
        if source != original_source {
            let claimed = service.claim_identity(&held)?;
            held = service.commit_copy_edit(&claimed, source.as_bytes(), &held.meta.source_hash)?;
        }
        if !args.secret.is_empty() {
            let state = FormStateService::new(FileFormStateStore::new(resolve_state_dir()?));
            state.purge_secrets(&held.slug, &declarations)?;
        }
    } else if changed {
        settings.parameters = declarations.clone();
        if matches!(held.meta.kind.as_str(), "command" | "prompt") {
            settings.params = declarations
                .iter()
                .filter(|item| item.delivery == ParameterDelivery::Placeholder)
                .map(|item| item.name.clone())
                .collect();
        }
        let claimed = service.claim_identity(&held)?;
        held = service.update_settings(&claimed, &settings, &workdir)?;
        if !args.secret.is_empty() {
            let state = FormStateService::new(FileFormStateStore::new(resolve_state_dir()?));
            state.purge_secrets(&held.slug, &declarations)?;
        }
    }
    write_params(&held, &source, &settings, &declarations, args.json)
}

fn write_params(
    entry: &Entry,
    source: &str,
    settings: &EntrySettings,
    declarations: &[ParamDecl],
    json: bool,
) -> Result<(), CliError> {
    if json {
        let rows = declarations
            .iter()
            .map(|item| {
                let mut row = item.to_meta_map();
                row.insert(
                    "binding".to_owned(),
                    serde_json::Value::String(item.binding.as_str().to_owned()),
                );
                row.insert(
                    "multiple".to_owned(),
                    serde_json::Value::Bool(item.multiple),
                );
                row.insert("repeat".to_owned(), serde_json::Value::Bool(item.repeat));
                row.insert(
                    "env_target".to_owned(),
                    serde_json::Value::String(item.env_target.clone()),
                );
                row.insert(
                    "action".to_owned(),
                    serde_json::Value::String(item.action.clone()),
                );
                serde_json::Value::Object(row.into_iter().collect())
            })
            .collect::<Vec<_>>();
        let managed = managed_params(entry.meta.kind.as_str(), source);
        let managed_rows = managed
            .iter()
            .map(|item| serde_json::Value::Object(item.to_block_map().into_iter().collect()))
            .collect::<Vec<_>>();
        let candidates = detect_candidates(entry.meta.kind.as_str(), source);
        let managed_names = managed
            .iter()
            .map(|item| item.name.as_str())
            .collect::<BTreeSet<_>>();
        let reader_driven = !cli_params(entry.meta.kind.as_str(), source).is_empty();
        let unmanaged = if reader_driven {
            Vec::new()
        } else if entry.meta.kind.as_str() == "prompt" && settings.interpolate {
            placeholder_params("prompt", source)
                .into_iter()
                .map(|item| item.name)
                .filter(|name| !settings.params.contains(name))
                .collect::<Vec<_>>()
        } else {
            candidates
                .iter()
                .map(|item| item.name.clone())
                .filter(|name| !managed_names.contains(name.as_str()))
                .collect::<Vec<_>>()
        };
        let current_defaults = managed
            .iter()
            .filter_map(|item| {
                let current = candidates
                    .iter()
                    .find(|candidate| candidate.name == item.name)?;
                let default = current.to_meta_map().remove("default")?;
                Some((item.name.clone(), default))
            })
            .collect::<serde_json::Map<_, _>>();
        let declared = settings
            .parameters
            .iter()
            .map(|item| serde_json::Value::Object(item.to_meta_map().into_iter().collect()))
            .collect::<Vec<_>>();
        let state =
            FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).load(&entry.slug);
        let mut record = serde_json::json!({
            "params": managed_rows,
            "parameters": rows,
            "current_defaults": current_defaults,
            "last_values": state.values,
            "unmanaged": unmanaged,
            "placeholders": settings.params,
            "declared": declared,
        });
        if entry.meta.kind.as_str() == "prompt" {
            let object = record
                .as_object_mut()
                .expect("the params record is a JSON object");
            object.insert(
                "runner".to_owned(),
                serde_json::json!(nonempty(&settings.runner)),
            );
            object.insert(
                "interpolate".to_owned(),
                serde_json::json!(settings.interpolate),
            );
        }
        println!("{record}");
    } else {
        let state =
            FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).load(&entry.slug);
        let candidates = detect_candidates(entry.meta.kind.as_str(), source);
        let reader_driven = !cli_params(entry.meta.kind.as_str(), source).is_empty();
        let declared_names = declarations
            .iter()
            .map(|item| item.name.as_str())
            .collect::<BTreeSet<_>>();
        let unmanaged = if reader_driven {
            Vec::new()
        } else {
            candidates
                .iter()
                .map(|item| item.name.clone())
                .filter(|name| !declared_names.contains(name.as_str()))
                .collect::<Vec<_>>()
        };
        for item in declarations {
            humanln!("Parameter: {}", item.name);
            humanln!("Type: {}", item.parameter_type.as_str());
            humanln!("Delivery: {}", item.delivery.as_str());
            if let Some(default) = candidates
                .iter()
                .find(|candidate| candidate.name == item.name)
                .and_then(|candidate| candidate.default.as_ref())
                .or(item.default.as_ref())
            {
                humanln!("Current default: {}", tui_parameter_value(default));
            }
            if let Some(value) = state.values.get(&item.name) {
                humanln!("Last value: {}", value);
            }
            if !item.choices.is_empty() {
                humanln!("Choices: {}", item.choices.join(", "));
            }
            if !item.prompt.is_empty() {
                humanln!("Prompt: {}", item.prompt);
            }
            if !item.help.is_empty() {
                humanln!("Help: {}", item.help);
            }
            if !item.env_source.is_empty() {
                humanln!("Environment source: {}", item.env_source);
            }
            if item.secret {
                humanln!("Secret: yes");
            }
        }
        if !unmanaged.is_empty() {
            humanln!("Unmanaged candidates: {}", unmanaged.join(", "));
        }
        if entry.meta.mode == StorageMode::Reference {
            humanln!("Source management is not available for a reference entry.");
        }
        if entry.meta.kind.as_str() == "prompt" {
            humanln!(
                "Prompt runner: {}",
                if settings.runner.is_empty() {
                    text(active_locale(), "not set").into_owned()
                } else {
                    settings.runner.clone()
                }
            );
            humanln!(
                "Interpolation: {}",
                text(
                    active_locale(),
                    if settings.interpolate { "on" } else { "off" }
                )
            );
        }
    }
    Ok(())
}

/// One validated `runner remove` target.
#[derive(Debug)]
enum RunnerSelection {
    /// Remove by stable runner name.
    Name(String),
    /// Remove one raw row by its zero-based index.
    Row(usize),
    /// Repair a malformed prompt or runner-list container.
    Container,
}

impl RunnerSelection {
    fn label(&self, locale: Locale) -> String {
        match self {
            Self::Name(name) => name.clone(),
            Self::Row(row) => Message::new("row {}").with(row).localize(locale),
            Self::Container => text(locale, "container").into_owned(),
        }
    }
}

fn parse_binding(value: &str) -> Result<ParameterBinding, CliError> {
    match value {
        "const" => Ok(ParameterBinding::Const),
        "input" => Ok(ParameterBinding::Input),
        "envdefault" => Ok(ParameterBinding::EnvDefault),
        "none" => Ok(ParameterBinding::None),
        _ => Err(CliError::Usage(
            Message::new("unknown parameter binding: {}").with(value),
        )),
    }
}

fn assignment<'a>(value: &'a str, field: &'static str) -> Result<(&'a str, &'a str), CliError> {
    value
        .split_once('=')
        .filter(|(name, _)| !name.is_empty())
        .ok_or_else(|| {
            CliError::Usage(Message::new("{} needs NAME=VALUE").nested(Message::term(field)))
        })
}

fn parameter_mut<'a>(
    declarations: &'a mut [ParamDecl],
    name: &str,
) -> Result<&'a mut ParamDecl, CliError> {
    declarations
        .iter_mut()
        .find(|item| item.name == name)
        .ok_or_else(|| CliError::Usage(Message::new("unknown parameter: {}").with(name)))
}

fn parse_parameter_type(value: &str) -> Result<ParameterType, CliError> {
    match value {
        "str" => Ok(ParameterType::Str),
        "int" => Ok(ParameterType::Int),
        "float" => Ok(ParameterType::Float),
        "bool" => Ok(ParameterType::Bool),
        "choice" => Ok(ParameterType::Choice),
        "path" => Ok(ParameterType::Path),
        _ => Err(CliError::Usage(
            Message::new("unknown parameter type: {}").with(value),
        )),
    }
}

fn parse_delivery(value: &str) -> Result<ParameterDelivery, CliError> {
    match value {
        "inject" => Ok(ParameterDelivery::Inject),
        "env" => Ok(ParameterDelivery::Env),
        "flag" => Ok(ParameterDelivery::Flag),
        "placeholder" => Ok(ParameterDelivery::Placeholder),
        _ => Err(CliError::Usage(
            Message::new("unknown parameter delivery: {}").with(value),
        )),
    }
}

fn set_bool(
    declarations: &mut [ParamDecl],
    names: &[String],
    field: impl Fn(&mut ParamDecl) -> &mut bool,
    value: bool,
) -> Result<bool, CliError> {
    for name in names {
        *field(parameter_mut(declarations, name)?) = value;
    }
    Ok(!names.is_empty())
}

fn config(key: Option<&str>, value: Option<&str>, json: bool) -> Result<(), CliError> {
    let store = FileConfigStore::new(resolve_config_dir()?);
    config_in(&store, key, value, json)
}

fn config_in(
    store: &FileConfigStore,
    key: Option<&str>,
    value: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    match (key, value) {
        (Some(key), Some(value)) => {
            if let Some(recovery) = store.set_with_recovery(key, value)? {
                humanerrln!(
                    "skit could not parse {}. skit backed up the file to {} before this change. Recover missing settings from the backup.",
                    recovery.path.display(),
                    recovery.backup_path.display(),
                );
            }
            let value = store.get(key)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&BTreeMap::from([(key, value.as_str())]))?
                );
            } else {
                humanln!("Set: {}={}", key, value);
            }
        }
        (Some(key), None) => {
            let value = store.get(key)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&BTreeMap::from([(key, value.as_str())]))?
                );
            } else {
                println!("{value}");
            }
        }
        (None, None) => {
            let settings = store.settings()?;
            if json {
                println!("{}", serde_json::to_string(&settings)?);
            } else {
                for (key, value) in settings {
                    println!("{key}={value}");
                }
            }
        }
        (None, Some(_)) => {
            return Err(CliError::Usage(Message::new(
                "a configuration value needs a key",
            )));
        }
    }
    Ok(())
}

fn runner(service: &LibraryService<FileStore>, command: RunnerCommand) -> Result<(), CliError> {
    let store = FileConfigStore::new(resolve_config_dir()?);
    match command {
        RunnerCommand::List { json, all } => {
            store.ensure_runners_seeded()?;
            if all {
                let rows = store.runner_rows()?;
                if json {
                    let output = rows
                        .into_iter()
                        .map(|row| {
                            serde_json::json!({
                                "row": row.index,
                                "name": row.name,
                                "argv": row.argv,
                                "reason": row.reason,
                                "descriptor": row.descriptor,
                                "valid": row.reason.is_none(),
                            })
                        })
                        .collect::<Vec<_>>();
                    println!("{}", serde_json::to_string(&output)?);
                } else {
                    let locale = active_locale();
                    if rows.is_empty() {
                        humanln!(
                            "No agents are configured. Add one with: skit runner add mycli -- mycli run {{prompt}}"
                        );
                        return Ok(());
                    }
                    // Version 0.4 prints a four-column table here (`src/skit/cli.py:3306-3319`).
                    // The raw index is zero-based (`src/skit/config.py:687` `enumerate`), and a
                    // malformed enclosing value shows `container` in its place.
                    let table = rows
                        .into_iter()
                        .map(|row| {
                            let index = row.index.map_or_else(
                                || text(locale, "container").into_owned(),
                                |index| index.to_string(),
                            );
                            let status = row
                                .localized_reason(locale)
                                .unwrap_or_else(|| text(locale, "valid").into_owned());
                            let name = row
                                .name
                                .clone()
                                .unwrap_or_else(|| row.localized_descriptor(locale));
                            let command = row
                                .argv
                                .as_deref()
                                .map(runner_command_text)
                                .unwrap_or_default();
                            [index, name, command, status]
                        })
                        .collect::<Vec<_>>();
                    let headers = [
                        text(locale, "Row").into_owned(),
                        text(locale, "Runner").into_owned(),
                        text(locale, "Command").into_owned(),
                        text(locale, "Status").into_owned(),
                    ];
                    let stdout = io::stdout();
                    let mut output = stdout.lock();
                    write_table(&mut output, &headers, &table)?;
                }
                return Ok(());
            }
            let runners = store.runners()?;
            if json {
                let rows = runners
                    .into_iter()
                    .map(|runner| serde_json::json!({"name": runner.name, "argv": runner.argv}))
                    .collect::<Vec<_>>();
                println!("{}", serde_json::to_string(&rows)?);
            } else {
                if runners.is_empty() {
                    humanln!(
                        "No agents are configured. Add one with: skit runner add mycli -- mycli run {{prompt}}"
                    );
                }
                let amp_seeded = runners.iter().any(|runner| {
                    runner.name == "amp" && runner.argv == ["amp", "-x", "{{prompt}}"]
                });
                if !runners.is_empty() {
                    // Version 0.4 prints Runner/Command as a table (`src/skit/cli.py:3336-3342`).
                    let locale = active_locale();
                    let table = runners
                        .iter()
                        .map(|runner| [runner.name.clone(), runner_command_text(&runner.argv)])
                        .collect::<Vec<_>>();
                    let headers = [
                        text(locale, "Runner").into_owned(),
                        text(locale, "Command").into_owned(),
                    ];
                    let stdout = io::stdout();
                    let mut output = stdout.lock();
                    write_table(&mut output, &headers, &table)?;
                }
                if amp_seeded {
                    humanln!(
                        "The built-in amp preset uses amp -x and runs the prompt once; it does not open an interactive session."
                    );
                }
            }
        }
        RunnerCommand::Add { name, argv, force } => {
            let command = runner_command_text(&argv);
            let existed = store.set_runner(
                PromptRunner {
                    name: name.clone(),
                    argv,
                },
                force,
            )?;
            if existed {
                humanln!("Runner {} updated: {}", name, command);
            } else {
                humanln!("Runner {} added: {}", name, command);
            }
        }
        RunnerCommand::Remove {
            name,
            row,
            yes,
            no_input,
        } => {
            // Version 0.4 strips the row value before it compares (`src/skit/cli.py:3457`),
            // so a padded `--row " 5 "` selects row 5 there and must do the same here.
            let row = row.as_deref().map(str::trim);
            let selection = match (name.as_deref(), row) {
                (Some(name), None) if !name.trim().is_empty() => {
                    RunnerSelection::Name(name.trim().to_owned())
                }
                (Some(_), None) => {
                    return Err(CliError::Usage(Message::new(
                        "a prompt runner needs a name",
                    )));
                }
                (None, Some("container")) => RunnerSelection::Container,
                (None, Some(row)) => {
                    let index = row.parse::<usize>().map_err(|_| {
                        CliError::Usage(Message::new(
                            "--row must be a non-negative index or 'container'.",
                        ))
                    })?;
                    RunnerSelection::Row(index)
                }
                _ => {
                    return Err(CliError::Usage(Message::new(
                        "runner remove needs a name or --row INDEX",
                    )));
                }
            };
            store.ensure_runners_seeded()?;
            let rows = store.runner_rows()?;
            let mut targets = match &selection {
                RunnerSelection::Name(name) => rows
                    .iter()
                    .filter(|row| row.name.as_deref() == Some(name.as_str()))
                    .cloned()
                    .collect::<Vec<_>>(),
                RunnerSelection::Row(index) => rows
                    .iter()
                    .filter(|row| row.index == Some(*index))
                    .cloned()
                    .collect::<Vec<_>>(),
                RunnerSelection::Container => rows
                    .iter()
                    .filter(|row| row.index.is_none())
                    .cloned()
                    .collect::<Vec<_>>(),
            };
            if targets.is_empty() {
                return match &selection {
                    RunnerSelection::Name(name) => {
                        let configured = rows
                            .iter()
                            .filter(|row| row.reason.is_none())
                            .filter_map(|row| row.name.as_deref())
                            .collect::<Vec<_>>()
                            .join(", ");
                        Err(CliError::Failure(
                            Message::new("Unknown runner: {}. Configured runners: {}")
                                .with(name)
                                .with(if configured.is_empty() {
                                    "—".to_owned()
                                } else {
                                    configured
                                }),
                        ))
                    }
                    RunnerSelection::Row(row) => Err(CliError::Failure(
                        Message::new(
                            "Unknown runner row: {}. Inspect with: skit runner list --all",
                        )
                        .with(row),
                    )),
                    RunnerSelection::Container => Err(CliError::Failure(Message::new(
                        "Unknown runner row: container. Inspect with: skit runner list --all",
                    ))),
                };
            }
            if !matches!(&selection, RunnerSelection::Name(_)) && targets[0].reason.is_none() {
                let name = targets[0].name.as_deref().unwrap_or_default();
                return Err(CliError::Usage(
                    Message::new(
                        "Runner row {} is valid. Remove the agent by name instead: skit runner remove {}",
                    )
                    .with(selection.label(active_locale()))
                    .with(name),
                ));
            }
            let target = selection.label(active_locale());
            if let RunnerSelection::Name(name) = &selection {
                let pinned = prompt_runner_pin_count(service, name)?;
                if pinned == 1 {
                    humanln!(
                        "1 prompt pins this runner and will need another runner before it can run again."
                    );
                } else if pinned > 1 {
                    humanln!(
                        "{} prompts pin this runner and will need another runner before they can run again.",
                        pinned
                    );
                }
            }
            if !yes {
                if no_input || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                    return Err(CliError::Usage(Message::new(
                        "Confirmation is required; pass --yes to remove the runner.",
                    )));
                }
                let question = match &selection {
                    RunnerSelection::Name(name) => {
                        format_text(active_locale(), "Remove the agent \"{}\"? [y/N]: ", &[name])
                    }
                    RunnerSelection::Row(_) => format_text(
                        active_locale(),
                        "Remove runner row {} (\"{}\")? [y/N]: ",
                        &[&target, &targets[0].localized_descriptor(active_locale())],
                    ),
                    RunnerSelection::Container => text(
                        active_locale(),
                        "Remove the malformed prompt runner container? [y/N]: ",
                    )
                    .into_owned(),
                };
                if !prompt_confirmation(&question, false)? {
                    return Err(CliError::Aborted);
                }
            }
            let removed = match &selection {
                RunnerSelection::Name(name) => store.remove_runner_if_unchanged(name, &targets)?,
                RunnerSelection::Row(_) | RunnerSelection::Container => {
                    store.remove_runner_row_if_unchanged(&targets.remove(0))?
                }
            };
            if !removed {
                return Err(CliError::Failure(Message::new(
                    "The runner row changed before it could be removed; inspect again.",
                )));
            }
            match selection {
                RunnerSelection::Name(name) => humanln!("Runner {} removed.", name),
                RunnerSelection::Row(row) => humanln!("Malformed runner row {} removed.", row),
                RunnerSelection::Container => {
                    humanln!("Malformed prompt runner container removed.")
                }
            }
        }
    }
    Ok(())
}

fn runner_command_text(arguments: &[String]) -> String {
    join_editable_arguments(arguments)
}

fn prompt_runner_pin_count(
    service: &LibraryService<FileStore>,
    runner: &str,
) -> Result<usize, CliError> {
    let scan = service.list()?;
    let mut count = 0;
    for summary in scan
        .entries
        .iter()
        .filter(|summary| summary.kind.as_str() == "prompt")
    {
        match service.show(summary.slug.as_str()) {
            Ok(entry) if EntrySettings::from_meta(&entry.meta).runner == runner => {
                count += 1;
            }
            Ok(_) | Err(RepositoryError::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(count)
}

fn preset(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    command: PresetCommand,
) -> Result<(), CliError> {
    let state = FormStateService::new(FileFormStateStore::new(resolve_state_dir()?));
    match command {
        PresetCommand::Save {
            selector,
            name,
            from_last,
        } => {
            let entry = service.show(&selector)?;
            let declarations = entry_parameters(store, &entry);
            if declarations.is_empty() {
                return Err(CliError::Usage(
                    Message::new("{} has no form fields, so there's nothing to save.")
                        .with(&entry.meta.name),
                ));
            }
            let interactive = !from_last && io::stdin().is_terminal() && io::stdout().is_terminal();
            let saved = if interactive {
                let current = state.load(&entry.slug);
                let initial = prefill(&declarations, &current.values, None);
                let values = collect_preset_values(&declarations, &initial)?;
                let mut secret_names = declarations
                    .iter()
                    .filter(|declaration| declaration.secret)
                    .map(|declaration| declaration.name.clone())
                    .collect::<Vec<_>>();
                secret_names.sort();
                if !secret_names.is_empty() {
                    humanln!(
                        "Secret values are never stored in presets; skipped: {}",
                        secret_names.join(", "),
                    );
                }
                state.save_preset(&entry.slug, &name, &declarations, &values)?;
                true
            } else {
                let source = if from_last {
                    PresetSnapshotSource::LastRun
                } else {
                    PresetSnapshotSource::Prefill
                };
                state.save_preset_from_state(&entry.slug, &name, &declarations, source)?
            };
            if !saved {
                return Err(CliError::Failure(
                    Message::new("{} has no remembered values yet — run it once first.")
                        .with(entry.meta.name),
                ));
            }
            humanln!("Preset \"{}\" saved for {}.", name, entry.meta.name);
        }
        PresetCommand::List { selector, json } => {
            let entry = service.show(&selector)?;
            let presets = state.load(&entry.slug).presets;
            if json {
                println!("{}", serde_json::json!(presets));
            } else if presets.is_empty() {
                humanln!(
                    "No presets for {} yet. Create one with: skit run {} --save-preset <preset>",
                    entry.meta.name,
                    entry.meta.name,
                );
            } else {
                for (name, values) in presets {
                    let values = values
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("  {name}: {values}");
                }
            }
        }
        PresetCommand::Delete {
            selector,
            name,
            yes: _,
            no_input: _,
        } => {
            let entry = service.show(&selector)?;
            if !state.delete_preset(&entry.slug, &name)? {
                let available = state
                    .load(&entry.slug)
                    .presets
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(CliError::Failure(
                    Message::new("Unknown preset \"{}\". Available: {}")
                        .with(name)
                        .with(if available.is_empty() {
                            "—".to_owned()
                        } else {
                            available
                        }),
                ));
            }
            humanln!("Preset \"{}\" deleted from {}.", name, entry.meta.name);
        }
    }
    Ok(())
}

fn collect_preset_values(
    declarations: &[ParamDecl],
    initial: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, CliError> {
    let locale = active_locale();
    declarations
        .iter()
        .map(|declaration| {
            if !declaration.help.is_empty() {
                eprintln!("  {}", declaration.help);
            }
            if declaration.degraded {
                eprintln!(
                    "  {}",
                    text(locale, "Leave empty to use the script's own default.")
                );
            }
            if declaration.binding == ParameterBinding::Input {
                eprintln!(
                    "  {}",
                    text(
                        locale,
                        "Leave empty and the script will ask you in the terminal.",
                    )
                );
            }
            let label = if declaration.prompt.is_empty() {
                declaration.name.clone()
            } else {
                declaration.prompt.clone()
            };
            let default = initial.get(&declaration.name).cloned().unwrap_or_default();
            let value = collect_preset_value(declaration, &label, &default, locale)?;
            Ok((declaration.name.clone(), value))
        })
        .collect()
}

fn collect_preset_value(
    declaration: &ParamDecl,
    label: &str,
    default: &str,
    locale: Locale,
) -> Result<String, CliError> {
    if declaration.parameter_type == ParameterType::Bool {
        let checked = coerce_default(default, ParameterType::Bool)
            .ok()
            .and_then(|value| match value {
                ParameterValue::Bool(value) => Some(value),
                _ => None,
            })
            .unwrap_or(false);
        return Confirm::new()
            .with_prompt(label)
            .default(checked)
            .interact_opt()
            .map_err(dialoguer_error)?
            .map(|value| value.to_string())
            .ok_or(CliError::Aborted);
    }

    if declaration.secret {
        if !declaration.env_source.is_empty() {
            eprintln!(
                "  {}",
                format_text(
                    locale,
                    "Enter to read it from the environment variable {}.",
                    &[&declaration.env_source],
                )
            );
        }
        return Password::new()
            .with_prompt(label)
            .allow_empty_password(!declaration.required)
            .validate_with(|value: &String| {
                validate_form_value(declaration, value)
                    .map_err(|error| error.message().localize(locale))
            })
            .interact()
            .map_err(dialoguer_error);
    }

    let prompt =
        if declaration.parameter_type == ParameterType::Choice && !declaration.choices.is_empty() {
            format!("{} ({})", label, declaration.choices.join("/"))
        } else {
            label.to_owned()
        };
    let mut input = Input::<String>::new()
        .with_prompt(prompt)
        .allow_empty(!declaration.required)
        .validate_with(|value: &String| {
            validate_form_value(declaration, value)
                .map_err(|error| error.message().localize(locale))
        });
    if !default.is_empty() {
        input = input.default(default.to_owned());
    } else if declaration.required
        && declaration.parameter_type == ParameterType::Choice
        && let Some(first) = declaration.choices.first()
    {
        input = input.default(first.clone());
    }
    input
        .interact_text()
        .map(|value| value.trim().to_owned())
        .map_err(dialoguer_error)
}

fn dialoguer_error(error: dialoguer::Error) -> CliError {
    let error = io::Error::from(error);
    if matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::UnexpectedEof
    ) {
        CliError::Aborted
    } else {
        CliError::Io(error)
    }
}

fn doctor(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    json: bool,
    rebuild: bool,
) -> Result<i32, CliError> {
    let state_location = resolve_state_dir()?;
    let config_location = resolve_config_dir()?;
    let config = FileConfigStore::new(&config_location);
    let health = HealthService::new(CliHealthInspector::new(service, store, &config_location));
    let (snapshot, rebuilt_entries, rebuild_diagnostics) = if rebuild {
        let rebuilt = health.rebuild()?;
        (
            rebuilt.snapshot,
            Some(rebuilt.outcome.entry_count),
            rebuilt.outcome.problems,
        )
    } else {
        (health.inspect()?, None, Vec::new())
    };
    let uv = match &snapshot.uv {
        UvHealth::Found(path) => Some(path.clone()),
        UvHealth::Missing | UvHealth::NotRequired => None,
    };
    let mut missing = Vec::new();
    let mut drift = Vec::new();
    let mut needs_missing = BTreeMap::<String, Vec<String>>::new();
    let mut launch_blocked = BTreeMap::<String, String>::new();
    for issue in &snapshot.issues {
        match &issue.kind {
            HealthIssueKind::MissingTarget => missing.push(issue.name.clone()),
            HealthIssueKind::DriftedForm => drift.push(issue.name.clone()),
            HealthIssueKind::MissingNeeds { tools } => {
                needs_missing.insert(issue.name.clone(), tools.clone());
            }
            HealthIssueKind::LaunchBlocked { reason } => {
                launch_blocked.insert(issue.name.clone(), reason.clone());
            }
        }
    }
    let bad_runners = snapshot.invalid_runner_rows.clone();
    let mirror = config.mirror()?;
    let scripts = PathBuf::from(&snapshot.library_path);
    let size = directory_size(&scripts);
    let code = usize::from(matches!(snapshot.uv, UvHealth::Missing)) as i32;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "uv": uv,
                "entries": snapshot.entry_count,
                "missing": missing,
                "drift": drift,
                "needs_missing": needs_missing,
                "launch_blocked": launch_blocked,
                "runner_rows_invalid": bad_runners,
                "rebuilt": rebuilt_entries,
                "rebuild_problems": rebuild_diagnostics,
                "mirror": {
                    "enabled": mirror.enabled,
                    "pypi": mirror.pypi,
                    "python_install": mirror.python_install,
                    "uv_binary": mirror.uv_binary,
                    "npm": mirror.npm,
                },
                "location": scripts,
                "size_bytes": size,
                "state_location": state_location,
                "config_location": config_location,
                "diagnostics": snapshot.diagnostics,
            })
        );
    } else {
        match &snapshot.uv {
            UvHealth::Found(path) => humanln!("OK uv: {}", path),
            UvHealth::Missing => humanln!("ERROR uv: not found"),
            UvHealth::NotRequired => humanln!("OK uv: not required"),
        }
        humanln!("Entries: {}", snapshot.entry_count);
        humanln!("Library: {} ({} bytes)", scripts.display(), size);
        humanln!("State: {}", state_location.display());
        humanln!("Config: {}", config_location.display());
        if let Some(count) = rebuilt_entries {
            humanln!("Registry rebuilt: {}", count);
        }
        for name in missing {
            humanln!("WARN {}: the launch target is gone from disk", name);
        }
        for name in drift {
            humanln!(
                "WARN {}: form definitions are out of sync; run: skit params {} --resync",
                name,
                name
            );
        }
        for (name, tools) in needs_missing {
            humanln!(
                "WARN {}: missing external commands: {}",
                name,
                tools.join(", ")
            );
        }
        for (name, reason) in launch_blocked {
            humanln!("WARN {}: a run would refuse to start: {}", name, reason);
        }
        if !bad_runners.is_empty() {
            humanln!("WARN malformed prompt runners: {}", bad_runners.join(", "));
        }
        for diagnostic in rebuild_diagnostics {
            humanln!("WARN {}", diagnostic);
        }
    }
    Ok(code)
}

fn doctor_entry_drifted(store: &FileStore, entry: &Entry) -> bool {
    let Some(path) = source_path(store, entry) else {
        return false;
    };
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let source = if entry.meta.kind.as_str() == "prompt" {
        let Ok(source) = String::from_utf8(bytes) else {
            return false;
        };
        source
    } else {
        LosslessSource::from_bytes(&bytes)
            .normalized_text()
            .to_owned()
    };
    let settings = EntrySettings::from_meta(&entry.meta);
    !form_plan(entry.meta.kind.as_str(), &source, &settings)
        .drift
        .is_empty()
}

fn doctor_launch_block<P: ProgramProbe>(
    entry: &Entry,
    settings: &EntrySettings,
    config: &FileConfigStore,
    probe: &P,
) -> Result<Option<Message>, CliError> {
    if !matches!(entry.meta.workdir.as_str(), "invoke" | "store" | "origin") {
        let path = Path::new(&entry.meta.workdir);
        if !path.is_absolute() {
            return Ok(Some(
                Message::new("custom working directory must be absolute: {}")
                    .with(&entry.meta.workdir),
            ));
        }
        if !probe.is_dir(path) {
            return Ok(Some(
                Message::new("working directory does not exist: {}").with(path.display()),
            ));
        }
    }
    let required = match entry.meta.kind.as_str() {
        "python" => Some(interpreter_name(settings, "uv")),
        "shell" => Some(if settings.interpreter.is_empty() {
            let configured = config.get("shell.bash_path")?;
            if configured.is_empty() {
                "bash".to_owned()
            } else {
                configured
            }
        } else {
            settings.interpreter.clone()
        }),
        "fish" => Some(interpreter_name(settings, "fish")),
        "powershell" => Some(interpreter_name(settings, "pwsh")),
        "ruby" => Some(interpreter_name(settings, "ruby")),
        "perl" => Some(interpreter_name(settings, "perl")),
        "lua" => Some(interpreter_name(settings, "lua")),
        "r" => Some(interpreter_name(settings, "Rscript")),
        "command" => Some(if cfg!(windows) { "cmd.exe" } else { "sh" }.to_owned()),
        "js" | "ts" => match resolve_javascript_runtime(settings, probe) {
            Ok(_) => None,
            Err(error) => return Ok(Some(error.message())),
        },
        "prompt" if !settings.runner.is_empty() => {
            let runner = config
                .runners()?
                .into_iter()
                .find(|runner| runner.name == settings.runner);
            let Some(runner) = runner else {
                return Ok(Some(
                    Message::new("prompt runner {} is not configured").with(&settings.runner),
                ));
            };
            runner.argv.first().cloned()
        }
        "prompt" | "exe" => None,
        kind => return Ok(Some(Message::new("unknown entry kind: {}").with(kind))),
    };
    Ok(required.and_then(|name| {
        probe
            .find_program(&name)
            .is_none()
            .then(|| Message::new("required program was not found: {}").with(name))
    }))
}

fn interpreter_name(settings: &EntrySettings, default: &str) -> String {
    if settings.interpreter.is_empty() {
        default.to_owned()
    } else {
        settings.interpreter.clone()
    }
}

fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !metadata.is_dir() {
        return 0;
    }
    directory_contents_size(path)
}

fn directory_contents_size(path: &Path) -> u64 {
    fs::read_dir(path).map_or(0, |items| {
        items.filter_map(Result::ok).fold(0_u64, |total, item| {
            let item_path = item.path();
            let size = fs::symlink_metadata(&item_path).map_or(0, |metadata| {
                if metadata.is_file() {
                    metadata.len()
                } else if metadata.is_dir() {
                    directory_contents_size(&item_path)
                } else if metadata.file_type().is_symlink() {
                    fs::metadata(&item_path)
                        .ok()
                        .filter(|target| target.is_file())
                        .map_or(0, |target| target.len())
                } else {
                    0
                }
            });
            total.saturating_add(size)
        })
    })
}

fn agent(command: AgentCommand) -> Result<(), CliError> {
    match command {
        AgentCommand::Install {
            target,
            directory,
            project,
        } => {
            let roots = AgentRoots {
                home: user_home(),
                cwd: env::current_dir().map_err(CliError::Io)?,
            };
            let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
            let plan = plan_agent_install(
                &AgentInstallRequest {
                    target,
                    directory,
                    project,
                    interactive,
                },
                &roots,
                Path::is_dir,
            )
            .map_err(|error| {
                if error.exit_class() == ExitClass::Failure {
                    CliError::Failure(error.message())
                } else {
                    CliError::Usage(error.message())
                }
            })?;
            let skills_dir = match plan {
                AgentInstallPlan::Ready { skills_dir } => skills_dir,
                AgentInstallPlan::Choose { candidates } => {
                    let Some(target) = pick_agent_target(&candidates)? else {
                        humanln!("Cancelled — nothing was written.");
                        return Ok(());
                    };
                    target.skills_dir()
                }
            };
            let path = FileAgentSkillStore
                .install(&skills_dir, include_bytes!("../../../skills/skit/SKILL.md"))?;
            humanln!("Installed Agent Skill: {}", path.display());
        }
    }
    Ok(())
}

fn pick_agent_target(candidates: &[AgentTarget]) -> Result<Option<AgentTarget>, CliError> {
    humanln!("Agent directories on this machine:");
    for (index, target) in candidates.iter().enumerate() {
        let scope = text(
            active_locale(),
            match target.scope {
                AgentScope::User => "user",
                AgentScope::Project => "project",
            },
        );
        println!(
            "  {}. {} ({})  →  {}",
            index.saturating_add(1),
            target.name,
            scope,
            target.skills_dir().display()
        );
    }
    let target = loop {
        let question = format_text(
            active_locale(),
            "Install where? [1-{}] (1): ",
            &[&candidates.len()],
        );
        print!("{question}");
        io::stdout().flush()?;
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer)? == 0 {
            return Err(CliError::Aborted);
        }
        let answer = answer.trim();
        let choice = if answer.is_empty() {
            Some(1)
        } else {
            answer.parse::<usize>().ok()
        };
        if let Some(choice) = choice.filter(|choice| (1..=candidates.len()).contains(choice)) {
            break candidates[choice.saturating_sub(1)].clone();
        }
        humanerrln!("Choose a number from 1 to {}.", candidates.len());
    };
    let question = format_text(
        active_locale(),
        "Write the skill into {}? [Y/n] ",
        &[&target.skills_dir().display()],
    );
    if prompt_confirmation(&question, true)? {
        Ok(Some(target))
    } else {
        Ok(None)
    }
}

fn entry_parameters(store: &FileStore, entry: &Entry) -> Vec<ParamDecl> {
    let settings = EntrySettings::from_meta(&entry.meta);
    let source = source_path(store, entry)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    form_params(entry.meta.kind.as_str(), &source, &settings)
}

fn settings_parameters(store: &FileStore, entry: &Entry) -> Vec<ParamDecl> {
    let settings = EntrySettings::from_meta(&entry.meta);
    let mut parameters = entry_parameters(store, entry);
    for parameter in settings.parameters {
        if !parameters
            .iter()
            .any(|current| current.name == parameter.name)
        {
            parameters.push(parameter);
        }
    }
    parameters
}

fn source_path(store: &FileStore, entry: &Entry) -> Option<PathBuf> {
    store.payload_path(entry).ok()
}

fn entry_missing(store: &FileStore, entry: &Entry) -> bool {
    entry_target(store, entry).is_some_and(|path| !path.exists())
}

fn entry_target(store: &FileStore, entry: &Entry) -> Option<PathBuf> {
    let kind = entry.meta.kind.as_str();
    if !known_entry_kind(kind) || kind == "command" {
        return None;
    }
    if entry.meta.mode == StorageMode::Reference || kind == "exe" {
        return Some(if entry.meta.source.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::from(&entry.meta.source)
        });
    }
    let canonical = copy_summary_target(store, &entry.slug, kind)?;
    if canonical.exists() {
        return Some(canonical);
    }
    store.payload_path(entry).ok().or(Some(canonical))
}

fn summary_missing(store: &FileStore, entry: &EntrySummary) -> bool {
    summary_target(store, entry).is_some_and(|path| !path.exists())
}

fn summary_target(store: &FileStore, entry: &EntrySummary) -> Option<PathBuf> {
    let kind = entry.kind.as_str();
    if !known_entry_kind(kind) || kind == "command" {
        return None;
    }
    match entry.mode {
        StorageMode::Reference => entry.target.as_ref().map(|target| {
            if target.is_empty() {
                PathBuf::from(".")
            } else {
                PathBuf::from(target)
            }
        }),
        StorageMode::Copy => copy_summary_target(store, &entry.slug, kind),
    }
}

fn copy_summary_target(store: &FileStore, slug: &Slug, kind: &str) -> Option<PathBuf> {
    let directory = store.entry_dir_path(slug);
    let names = stored_filenames(kind);
    names
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.exists())
        .or_else(|| names.first().map(|name| directory.join(name)))
}

fn known_entry_kind(kind: &str) -> bool {
    matches!(
        kind,
        "python"
            | "shell"
            | "fish"
            | "js"
            | "ts"
            | "powershell"
            | "ruby"
            | "perl"
            | "lua"
            | "r"
            | "exe"
            | "command"
            | "prompt"
    )
}

fn tui(service: &LibraryService<FileStore>) -> Result<(), CliError> {
    let store = service.repository();
    let state_dir = resolve_state_dir()?;
    let config_dir = resolve_config_dir()?;
    let scan = service.list()?;
    let rerunnable = tui_rerunnable(&scan, &state_dir);
    let mut state = LibraryState::from_scan(scan);
    let _ = state.update(UiAction::ReplaceRerunnable(rerunnable));
    skit_tui::run(
        state,
        |effect| tui_effect(service, store, &state_dir, &config_dir, effect),
        active_locale(),
    )
    .map_err(CliError::from)
}

fn tui_effect(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
    effect: UiEffect,
) -> Result<UiAction, CliError> {
    match effect {
        UiEffect::None | UiEffect::Quit => Ok(UiAction::ClearStatus),
        UiEffect::Reload => {
            let scan = service.list()?;
            let rerunnable = tui_rerunnable(&scan, state_dir);
            Ok(UiAction::Replace { scan, rerunnable })
        }
        UiEffect::Rerun { selector } => tui_rerun(service, store, state_dir, config_dir, &selector),
        UiEffect::Open { request, selector } => Ok(UiAction::Present(tui_open(
            service, store, state_dir, config_dir, request, selector,
        )?)),
        UiEffect::Preferences(effect) => tui_preferences_effect(service, config_dir, effect),
        UiEffect::CountRunGlob {
            field,
            value,
            request,
            ..
        } => {
            let count = FileGlobExpander::new(&request.cwd).count_matches(&request);
            Ok(UiAction::SetRunGlobCount {
                field,
                value,
                count,
            })
        }
        UiEffect::SaveRunPreset {
            selector,
            name,
            mut values,
            secret_names,
        } => {
            let entry = service.show(&selector)?;
            let declarations = entry_parameters(store, &entry);
            refuse_empty_preset_schema(&declarations)?;
            values.retain(|key, _| !secret_names.contains(key));
            let state = FormStateService::new(FileFormStateStore::new(state_dir));
            state.save_preset(&entry.slug, &name, &declarations, &values)?;
            let presets = state.load(&entry.slug).presets;
            Ok(UiAction::RunPresetSaved {
                message: format_text(active_locale(), "Preset \"{}\" saved.", &[&name]),
                name,
                presets,
            })
        }
        UiEffect::HealthRebuild => {
            let rebuilt = HealthService::new(CliHealthInspector::new(service, store, config_dir))
                .rebuild()?;
            Ok(UiAction::Health(HealthAction::Rebuilt {
                snapshot: Box::new(rebuilt.snapshot),
                outcome: rebuilt.outcome,
            }))
        }
        UiEffect::SaveRunner { request, owner } => {
            tui_save_runner(service, config_dir, request, owner)
        }
        UiEffect::RemoveRunner(request) => tui_remove_runner(service, config_dir, request),
        UiEffect::RefreshPreferencesAfterRunners => {
            let Screen::Preferences(preferences) = tui_preferences_screen(config_dir)? else {
                unreachable!("the preferences builder always returns Preferences")
            };
            Ok(UiAction::RunnerManagerClosed { preferences })
        }
        UiEffect::Add(effects) => tui_add_effect(service, store, state_dir, config_dir, effects),
        UiEffect::Edit { selector } => {
            edit_with_config(service, store, config_dir, &selector, true)?;
            Ok(tui_complete(service, state_dir, "Source saved")?)
        }
        UiEffect::Remove { selector } => {
            remove(service, &selector, true, true)?;
            Ok(tui_complete(service, state_dir, "Entry removed")?)
        }
        UiEffect::Submit {
            purpose,
            selector,
            values,
        } => tui_submit(
            service, store, state_dir, config_dir, purpose, selector, &values,
        ),
    }
}

fn tui_add_effect(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
    effects: Vec<AddEffect>,
) -> Result<UiAction, CliError> {
    let locale = active_locale();
    let mut warnings = Vec::new();
    for effect in effects {
        match effect {
            AddEffect::InspectSource { request, path } => {
                let result = tui_add_source(store.data_dir(), &path)
                    .map_err(|error| error.message().localize(locale));
                return Ok(UiAction::Add(AddAction::SourceInspected {
                    request,
                    result,
                }));
            }
            AddEffect::AuthorDraft { request, kind } => {
                let result = tui_author_draft(store.data_dir(), config_dir, kind)
                    .map_err(|error| error.message().localize(locale));
                return Ok(UiAction::Add(AddAction::DraftEdited { request, result }));
            }
            AddEffect::DeleteDraft { request, path } => {
                let result = remove_owned_draft(store.data_dir(), &path)
                    .map_err(|error| error.message().localize(locale));
                return Ok(UiAction::Add(AddAction::DraftDeleted { request, result }));
            }
            AddEffect::EditSource { request, path } => {
                let result = open_editor_in(config_dir, &path)
                    .and_then(|()| tui_add_source(store.data_dir(), &path))
                    .map_err(|error| error.message().localize(locale));
                return Ok(UiAction::Add(AddAction::SourceEdited { request, result }));
            }
            AddEffect::Commit {
                request,
                entry,
                source,
            } => {
                let result = source
                    .as_ref()
                    .map_or(Ok(()), |expected| {
                        verify_tui_add_source(store.data_dir(), expected)
                    })
                    .and_then(|()| service.add(*entry).map_err(CliError::from))
                    .map(|created| created.slug.as_str().to_owned())
                    .map_err(|error| error.message().localize(locale));
                return Ok(UiAction::Add(AddAction::CommitFinished { request, result }));
            }
            AddEffect::ConsumeDraft(path) => {
                if let Err(error) = remove_owned_draft(store.data_dir(), &path) {
                    warnings.push(error.message().localize(locale));
                }
            }
            AddEffect::DraftKept(_) => {}
            AddEffect::RememberRunner(name) => {
                if let Err(error) =
                    PromptSelectionService::new(FilePromptSelectionStore::new(state_dir))
                        .remember_runner(&name)
                {
                    warnings.push(error.message().localize(locale));
                }
            }
            AddEffect::Complete(raw_slug) => {
                let mut message = text(locale, "Entry added").into_owned();
                for warning in &warnings {
                    message.push('\n');
                    message.push_str(&format_text(locale, "warning: {}", &[warning]));
                }
                let Ok(slug) = Slug::parse(raw_slug) else {
                    return Ok(UiAction::Complete {
                        scan: None,
                        rerunnable: None,
                        message,
                    });
                };
                let scan = match service.list() {
                    Ok(scan) => scan,
                    Err(error) => {
                        message.push('\n');
                        message.push_str(&format_text(
                            locale,
                            "warning: {}",
                            &[&error.message().localize(locale)],
                        ));
                        return Ok(UiAction::Complete {
                            scan: None,
                            rerunnable: None,
                            message,
                        });
                    }
                };
                let rerunnable = tui_rerunnable(&scan, state_dir);
                return Ok(UiAction::AddCompleted {
                    scan,
                    rerunnable,
                    slug,
                    message,
                });
            }
            AddEffect::Cancel => return Ok(UiAction::AddCancelled),
        }
    }
    Ok(UiAction::ClearStatus)
}

fn tui_add_source(data_dir: &Path, input: &Path) -> Result<AddSourceSnapshot, CliError> {
    let expanded = expand_user_path(input);
    let path =
        fs::canonicalize(&expanded).map_err(|error| source_error("resolve", &expanded, error))?;
    let metadata = fs::metadata(&path).map_err(|error| source_error("inspect", &path, error))?;
    let is_directory = metadata.is_dir();
    let (bytes, permissions, is_regular) = if metadata.is_file() {
        let mut file = File::open(&path).map_err(|error| source_error("open", &path, error))?;
        let metadata = file
            .metadata()
            .map_err(|error| source_error("inspect", &path, error))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| source_error("read", &path, error))?;
        (bytes, source_permissions(&metadata), true)
    } else {
        (Vec::new(), source_permissions(&metadata), false)
    };
    let is_draft = is_owned_draft(data_dir, &path);
    Ok(AddSourceSnapshot {
        source_record: path.display().to_string(),
        path,
        bytes,
        permissions,
        is_regular,
        is_directory,
        is_draft,
    })
}

fn verify_tui_add_source(data_dir: &Path, expected: &AddSourceSnapshot) -> Result<(), CliError> {
    let current = tui_add_source(data_dir, &expected.path)?;
    if &current == expected {
        Ok(())
    } else {
        Err(CliError::Failure(Message::new(
            "source changed while the add review was open; review it again",
        )))
    }
}

fn tui_author_draft(
    data_dir: &Path,
    config_dir: &Path,
    kind: DraftKind,
) -> Result<Option<AddSourceSnapshot>, CliError> {
    let drafts_dir = create_owned_drafts_dir(data_dir)?;
    let (suffix, starter) = match kind {
        DraftKind::Script => (".py", b"#!/usr/bin/env python3\n".to_vec()),
        DraftKind::Prompt => (
            ".prompt.md",
            format!("{}\n\n", text(active_locale(), "# New prompt")).into_bytes(),
        ),
    };
    let mut staged = tempfile::Builder::new()
        .prefix("skit-new-")
        .suffix(suffix)
        .tempfile_in(&drafts_dir)
        .map_err(|error| source_error("create", &drafts_dir, error))?;
    staged
        .write_all(&starter)
        .map_err(|error| source_error("write", staged.path(), error))?;
    staged
        .flush()
        .map_err(|error| source_error("write", staged.path(), error))?;
    let path = staged
        .keep()
        .map_err(|error| source_error("keep", &drafts_dir, error.error))?
        .1;

    if let Err(error) = open_editor_in(config_dir, &path) {
        if fs::read(&path).ok().as_deref() == Some(starter.as_slice()) {
            let _ = fs::remove_file(&path);
        }
        return Err(error);
    }
    let edited = fs::read(&path).map_err(|error| source_error("read", &path, error))?;
    let unchanged = std::str::from_utf8(&edited).is_ok_and(|text| {
        let text = text.trim();
        text.is_empty() || std::str::from_utf8(&starter).is_ok_and(|starter| text == starter.trim())
    });
    if unchanged {
        fs::remove_file(&path).map_err(|error| source_error("remove", &path, error))?;
        return Ok(None);
    }
    tui_add_source(data_dir, &path).map(Some)
}

fn is_owned_draft(data_dir: &Path, path: &Path) -> bool {
    let Some(drafts_dir) = existing_owned_drafts_dir(data_dir) else {
        return false;
    };
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("skit-"))
        && path
            .parent()
            .and_then(|parent| fs::canonicalize(parent).ok())
            .is_some_and(|parent| parent == drafts_dir)
}

fn existing_owned_drafts_dir(data_dir: &Path) -> Option<PathBuf> {
    let raw = data_dir.join("drafts");
    let metadata = fs::symlink_metadata(&raw).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    let data_dir = fs::canonicalize(data_dir).ok()?;
    let drafts_dir = fs::canonicalize(raw).ok()?;
    (drafts_dir.parent() == Some(data_dir.as_path())).then_some(drafts_dir)
}

fn create_owned_drafts_dir(data_dir: &Path) -> Result<PathBuf, CliError> {
    fs::create_dir_all(data_dir).map_err(|error| source_error("create", data_dir, error))?;
    let raw = data_dir.join("drafts");
    match fs::symlink_metadata(&raw) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&raw).map_err(|error| source_error("create", &raw, error))?;
        }
        Err(error) => return Err(source_error("inspect", &raw, error)),
    }
    existing_owned_drafts_dir(data_dir).ok_or_else(|| {
        CliError::Failure(
            Message::new("skit's drafts path is not an owned directory: {}").with(raw.display()),
        )
    })
}

fn remove_owned_draft(data_dir: &Path, path: &Path) -> Result<(), CliError> {
    if !is_owned_draft(data_dir, path) {
        return Err(CliError::Failure(Message::new(
            "refusing to remove a file outside skit's drafts directory",
        )));
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(source_error("remove", path, error)),
    }
}

fn tui_preferences_effect(
    service: &LibraryService<FileStore>,
    config_dir: &Path,
    effect: PreferencesEffect,
) -> Result<UiAction, CliError> {
    match effect {
        PreferencesEffect::None | PreferencesEffect::Close | PreferencesEffect::ConfirmDiscard => {
            Ok(UiAction::ClearStatus)
        }
        PreferencesEffect::Save(change) => {
            let requested_language = change.settings.get("lang").cloned();
            if let Err(error) = change.validate_files(expanded_preference_path_is_file) {
                return Ok(UiAction::Preferences(PreferencesAction::ValidationFailed(
                    error,
                )));
            }
            if let Err(error) = FileConfigStore::new(config_dir).set_many(&change.settings) {
                return Ok(UiAction::SetStatus(format_text(
                    active_locale(),
                    "Error: {}",
                    &[&error.message().localize(active_locale())],
                )));
            }
            let locale = requested_language
                .as_deref()
                .filter(|language| !language.is_empty() && *language != "auto")
                .map_or_else(active_locale, |language| detect_locale(Some(language)));
            Ok(UiAction::PreferencesSaved {
                locale: locale.tag().to_owned(),
                message: "Preferences saved".to_owned(),
            })
        }
        PreferencesEffect::ManageAgents => {
            Ok(UiAction::Present(tui_runners_screen(service, config_dir)?))
        }
        PreferencesEffect::DiscoverAgentSkillTargets => {
            let roots = AgentRoots {
                home: user_home(),
                cwd: env::current_dir().map_err(CliError::Io)?,
            };
            Ok(UiAction::Preferences(
                PreferencesAction::PresentAgentSkillTargets(detect_agent_targets(
                    &roots,
                    Path::is_dir,
                )),
            ))
        }
        PreferencesEffect::InstallAgentSkill { skills_dir } => {
            match FileAgentSkillStore
                .install(&skills_dir, include_bytes!("../../../skills/skit/SKILL.md"))
            {
                Ok(path) => Ok(UiAction::Preferences(
                    PreferencesAction::AgentSkillInstalled {
                        message: format_text(
                            active_locale(),
                            "Installed the skit Agent Skill: {}",
                            &[&path.display()],
                        ),
                    },
                )),
                Err(error) => Ok(UiAction::SetStatus(format_text(
                    active_locale(),
                    "Error: {}",
                    &[&error.message().localize(active_locale())],
                ))),
            }
        }
    }
}

fn expanded_preference_path_is_file(path: &Path) -> bool {
    expand_user_path(path).is_file()
}

fn tui_rerunnable(scan: &LibraryScan, state_dir: &Path) -> Vec<Slug> {
    let state = FormStateService::new(FileFormStateStore::new(state_dir));
    scan.entries
        .iter()
        .filter_map(|entry| {
            let last_run = state.last_run(&entry.slug);
            (last_run.at.is_some() || last_run.exit.is_some() || last_run.values.is_some())
                .then(|| entry.slug.clone())
        })
        .collect()
}

fn tui_rerun(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
    selector: &str,
) -> Result<UiAction, CliError> {
    let entry = service.show(selector)?;
    let saved = FormStateService::new(FileFormStateStore::new(state_dir)).load(&entry.slug);
    if saved.last_run.at.is_none()
        && saved.last_run.exit.is_none()
        && saved.last_run.values.is_none()
    {
        return Ok(UiAction::SetStatus(format_text(
            active_locale(),
            "{} hasn't run yet — press Enter to fill the form first.",
            &[&entry.meta.name],
        )));
    }
    if entry.meta.kind.as_str() == "prompt"
        && EntrySettings::from_meta(&entry.meta).runner.is_empty()
    {
        return Ok(UiAction::Present(tui_open(
            service,
            store,
            state_dir,
            config_dir,
            HostRequest::Run,
            Some(selector.to_owned()),
        )?));
    }

    let result = crate::run::run_with_roots(
        service,
        store,
        state_dir,
        config_dir,
        RunArgs {
            selector: selector.to_owned(),
            values: Vec::new(),
            preset: None,
            save_preset: None,
            runner: None,
            dry_run: false,
            no_input: true,
            plain: true,
            raw: false,
            forget_args: false,
            extra_args: Vec::new(),
        },
    );
    match result {
        Ok(_) if FileConfigStore::new(config_dir).get("after_run")? == "exit" => Ok(UiAction::Quit),
        Ok(exit) => tui_complete(
            service,
            state_dir,
            &format_text(
                active_locale(),
                "Run finished with exit status {}",
                &[&exit],
            ),
        ),
        Err(RunError::Inputs(skit_application::run_inputs::RunInputError::Preparation(_))) => {
            Ok(UiAction::Present(tui_open(
                service,
                store,
                state_dir,
                config_dir,
                HostRequest::Run,
                Some(selector.to_owned()),
            )?))
        }
        Err(error) => Ok(UiAction::SetStatus(format_text(
            active_locale(),
            "Error: {}",
            &[&error.message().localize(active_locale())],
        ))),
    }
}

fn tui_open(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
    request: HostRequest,
    selector: Option<String>,
) -> Result<Screen, CliError> {
    match request {
        HostRequest::Run => {
            let entry = service.show(tui_selector(&selector)?)?;
            let settings = EntrySettings::from_meta(&entry.meta);
            let source = crate::run::source_text(store, &entry, &settings)?;
            let plan = form_plan(entry.meta.kind.as_str(), &source, &settings);
            let context = tui_run_context(store, &entry)?;
            let saved = FormStateService::new(FileFormStateStore::new(state_dir)).load(&entry.slug);
            let runners = if entry.meta.kind.as_str() == "prompt" {
                FileConfigStore::new(config_dir)
                    .runners()?
                    .into_iter()
                    .map(|runner| runner.name)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            Ok(tui_run_form(
                &entry,
                &plan,
                &saved.values,
                &runners,
                &settings.runner,
                &saved.presets,
                &join_editable_arguments(&saved.extra_args),
                context,
                active_locale(),
            ))
        }
        HostRequest::Add => tui_add_screen(store, state_dir, config_dir),
        HostRequest::Settings => {
            let entry = service.show(tui_selector(&selector)?)?;
            Ok(tui_settings_form(store, &entry))
        }
        HostRequest::Preferences => tui_preferences_screen(config_dir),
        HostRequest::Health => tui_health_screen(service, store, config_dir),
        HostRequest::Runners => tui_runners_screen(service, config_dir),
        HostRequest::Presets => {
            let entry = service.show(tui_selector(&selector)?)?;
            let saved = FormStateService::new(FileFormStateStore::new(state_dir)).load(&entry.slug);
            Ok(tui_presets_form(
                &entry,
                &saved.presets.keys().cloned().collect::<Vec<_>>(),
            ))
        }
        HostRequest::Rename => {
            let entry = service.show(tui_selector(&selector)?)?;
            Ok(Screen::Form(FormView {
                purpose: FormPurpose::Rename,
                title: "Rename {}".to_owned(),
                title_arguments: vec![entry.meta.name.clone()],
                translate_title: true,
                selector: Some(entry.slug.as_str().to_owned()),
                fields: vec![FormField::text("name", "Name", entry.meta.name)],
                focused: 0,
                submit_label: "Rename".to_owned(),
            }))
        }
    }
}

fn tui_selector(selector: &Option<String>) -> Result<&str, CliError> {
    selector
        .as_deref()
        .ok_or_else(|| CliError::Usage(Message::new("select an entry first")))
}

#[allow(clippy::too_many_arguments)]
fn tui_run_form(
    entry: &Entry,
    plan: &PreparedFormPlan,
    saved: &BTreeMap<String, String>,
    runners: &[String],
    runner_default: &str,
    presets: &BTreeMap<String, BTreeMap<String, String>>,
    extra_arguments: &str,
    context: RunFormContext,
    locale: Locale,
) -> Screen {
    let mut form = RunFormView::from_declarations(
        entry.slug.as_str(),
        &entry.meta.name,
        &plan.declarations(),
        saved,
        runners,
        runner_default,
        presets,
        extra_arguments,
    )
    .with_context(context);
    form.drift_lines = show_drift_lines(plan, &entry.meta.name, locale);
    let degradation = degradation_token(plan.degradation);
    form.degraded_reason = (!degradation.is_empty()).then(|| degradation.to_owned());
    Screen::Run(Box::new(form))
}

fn tui_run_context(store: &FileStore, entry: &Entry) -> Result<RunFormContext, CliError> {
    let tokens = crate::run::token_context();
    let invoke_cwd = PathBuf::from(&tokens.cwd);
    let script = if entry.meta.kind.as_str() == "command" {
        PathBuf::new()
    } else {
        store.payload_path(entry)?
    };
    let paths = LaunchPaths {
        script,
        entry_dir: store.entry_dir_path(&entry.slug),
        invoke_cwd: invoke_cwd.clone(),
    };
    let workdir = resolve_launch_workdir(entry, &paths, &SystemProbe)
        .map_err(RunError::from)?
        .display()
        .to_string();
    Ok(RunFormContext {
        entry_kind: entry.meta.kind.as_str().to_owned(),
        path: Some(RunPathContext {
            workdir,
            invoke_cwd: invoke_cwd.display().to_string(),
        }),
        tokens,
    })
}

fn plain_run_form_view(
    entry: &Entry,
    declarations: &[ParamDecl],
    saved: &BTreeMap<String, String>,
    runners: &[String],
    runner_default: &str,
) -> FormView {
    let mut fields = declarations
        .iter()
        .map(|parameter| {
            let label = if parameter.prompt.is_empty() {
                parameter.name.clone()
            } else {
                parameter.prompt.clone()
            };
            let value = if parameter.secret {
                String::new()
            } else {
                saved.get(&parameter.name).cloned().unwrap_or_default()
            };
            if parameter.secret {
                FormField::secret_raw(format!("value:{}", parameter.name), label, value)
            } else {
                FormField::text_raw(format!("value:{}", parameter.name), label, value)
            }
        })
        .collect::<Vec<_>>();
    if !runners.is_empty() {
        fields.push(tui_options_field(
            "_skit_runner",
            "Prompt runner",
            "Prompt runner choices: {}",
            runners,
            if runners.iter().any(|name| name == runner_default) {
                runner_default.to_owned()
            } else {
                runners.first().cloned().unwrap_or_default()
            },
        ));
    }
    FormView {
        purpose: FormPurpose::Run,
        title: "Run {}".to_owned(),
        title_arguments: vec![entry.meta.name.clone()],
        translate_title: true,
        selector: Some(entry.slug.as_str().to_owned()),
        fields,
        focused: 0,
        submit_label: "Run".to_owned(),
    }
}

fn tui_add_screen(
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
) -> Result<Screen, CliError> {
    Ok(Screen::Add(Box::new(tui_add_workflow(
        store, state_dir, config_dir,
    )?)))
}

fn tui_add_workflow(
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
) -> Result<AddWorkflowState, CliError> {
    let runner_names = FileConfigStore::new(config_dir)
        .runners()?
        .into_iter()
        .map(|runner| runner.name)
        .collect();
    let last_runner =
        PromptSelectionService::new(FilePromptSelectionStore::new(state_dir)).last_runner();
    let defaults = ReviewDefaults {
        runner_names,
        last_runner: (!last_runner.is_empty()).then_some(last_runner),
        ..ReviewDefaults::default()
    };
    Ok(AddWorkflowState::new(tui_drafts(store.data_dir())).with_review_defaults(defaults))
}

fn tui_drafts(data_dir: &Path) -> Vec<DraftSummary> {
    let Some(drafts_dir) = existing_owned_drafts_dir(data_dir) else {
        return Vec::new();
    };
    let Ok(items) = fs::read_dir(&drafts_dir) else {
        return Vec::new();
    };
    items
        .filter_map(Result::ok)
        .filter(|item| item.file_name().to_string_lossy().starts_with("skit-"))
        .filter_map(|item| {
            let metadata = item.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let modified = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |value| value.as_nanos().min(u128::from(u64::MAX)) as u64);
            Some(DraftSummary {
                path: item.path(),
                modified,
            })
        })
        .collect()
}

fn tui_settings_form(store: &FileStore, entry: &Entry) -> Screen {
    let settings = effective_settings(store, entry);
    let mut fields = vec![
        FormField::text("name", "Name", &entry.meta.name),
        FormField::multiline("description", "Description", &entry.meta.description),
        FormField::text("workdir", "Working directory", &entry.meta.workdir),
        FormField::text("interpreter", "Interpreter", settings.interpreter),
        FormField::text("runner", "Prompt runner", settings.runner),
        FormField::text(
            "dependencies",
            "Package dependencies",
            settings.dependencies.join("\n"),
        ),
        FormField::text("python", "Python constraint", settings.requires_python),
        FormField::text("needs", "Required commands", settings.needs.join(", ")),
        FormField::multiline("template", "Command template", settings.template),
        FormField::text(
            "interpolate",
            "Prompt interpolation (true or false)",
            settings.interpolate.to_string(),
        ),
        FormField::text(
            "source:resync",
            "Resync managed source parameters (true or false)",
            "false",
        ),
        FormField::text("source:manage", "Manage source parameters", ""),
        FormField::text("source:unmanage", "Stop managing source parameters", ""),
        FormField::text("source:normalize", "Normalize shell parameters", ""),
        FormField::text("parameter:add", "Add parameters", ""),
        FormField::text("parameter:remove", "Remove parameters", ""),
    ];
    for (index, parameter) in settings_parameters(store, entry).iter().enumerate() {
        let prefix = format!("parameter:{index}");
        let subject = &parameter.name;
        fields.extend([
            FormField::text_with_arguments(
                format!("{prefix}:name"),
                "Parameter {} name",
                vec![index.to_string()],
                subject,
            ),
            FormField::text_with_arguments(
                format!("{prefix}:binding"),
                "{} source binding",
                vec![subject.clone()],
                parameter.binding.as_str(),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:delivery"),
                "{} delivery",
                vec![subject.clone()],
                parameter.delivery.as_str(),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:type"),
                "{} type",
                vec![subject.clone()],
                parameter.parameter_type.as_str(),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:default"),
                "{} default",
                vec![subject.clone()],
                parameter
                    .default
                    .as_ref()
                    .map_or_else(String::new, tui_parameter_value),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:choices"),
                "{} choices",
                vec![subject.clone()],
                parameter.choices.join(", "),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:required"),
                "{} is required",
                vec![subject.clone()],
                parameter.required.to_string(),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:multiple"),
                "{} takes multiple values",
                vec![subject.clone()],
                parameter.multiple.to_string(),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:repeat"),
                "{} repeats its flag",
                vec![subject.clone()],
                parameter.repeat.to_string(),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:prompt"),
                "{} prompt",
                vec![subject.clone()],
                &parameter.prompt,
            ),
            FormField::multiline_with_arguments(
                format!("{prefix}:help"),
                "{} help",
                vec![subject.clone()],
                &parameter.help,
            ),
            FormField::text_with_arguments(
                format!("{prefix}:secret"),
                "{} is secret",
                vec![subject.clone()],
                parameter.secret.to_string(),
            ),
            FormField::text_with_arguments(
                format!("{prefix}:env_source"),
                "{} secret environment source",
                vec![subject.clone()],
                &parameter.env_source,
            ),
            FormField::text_with_arguments(
                format!("{prefix}:env_target"),
                "{} environment target",
                vec![subject.clone()],
                &parameter.env_target,
            ),
            FormField::text_with_arguments(
                format!("{prefix}:flag"),
                "{} flag",
                vec![subject.clone()],
                &parameter.flag,
            ),
            FormField::text_with_arguments(
                format!("{prefix}:action"),
                "{} flag action",
                vec![subject.clone()],
                &parameter.action,
            ),
        ]);
    }
    Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: "Settings for {}".to_owned(),
        title_arguments: vec![entry.meta.name.clone()],
        translate_title: true,
        selector: Some(entry.slug.as_str().to_owned()),
        fields,
        focused: 0,
        submit_label: "Save".to_owned(),
    })
}

fn tui_preferences_screen(config_dir: &Path) -> Result<Screen, CliError> {
    let config = FileConfigStore::new(config_dir);
    let settings = config.settings()?;
    let setting = |key: &str| settings.get(key).cloned().unwrap_or_default();
    let mirror = config.mirror()?;
    let snapshot = PreferencesSnapshot {
        language: setting("lang"),
        available_languages: available_locale_tags()
            .iter()
            .map(|tag| (*tag).to_owned())
            .collect(),
        effective_language: active_locale().tag().to_owned(),
        editor: setting("editor"),
        editor_fallback: env::var("VISUAL")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| env::var("EDITOR").ok().filter(|value| !value.is_empty())),
        form: match setting("form").as_str() {
            "plain" => InteractiveFormChoice::Plain,
            _ => InteractiveFormChoice::Tui,
        },
        after_run: match setting("after_run").as_str() {
            "stay" => AfterRunChoice::Stay,
            _ => AfterRunChoice::Exit,
        },
        javascript: match setting("js.runner").as_str() {
            "deno" => JavascriptChoice::Deno,
            "bun" => JavascriptChoice::Bun,
            "node" => JavascriptChoice::Node,
            _ => JavascriptChoice::Automatic,
        },
        bash_path: cfg!(target_os = "windows").then(|| setting("shell.bash_path")),
        runner_names: config
            .runners()?
            .into_iter()
            .map(|runner| runner.name)
            .collect(),
        mirror: MirrorConfiguration {
            enabled: mirror.enabled,
            pypi: mirror.pypi,
            python_install: mirror.python_install,
            uv_binary: mirror.uv_binary,
            npm: mirror.npm,
        },
    };
    Ok(Screen::Preferences(Box::new(PreferencesView::new(
        PreferencesDraft::from_snapshot(snapshot),
    ))))
}

fn tui_health_screen(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    config_dir: &Path,
) -> Result<Screen, CliError> {
    let snapshot =
        HealthService::new(CliHealthInspector::new(service, store, config_dir)).inspect()?;
    Ok(Screen::Health(Box::new(HealthView::new(snapshot))))
}

struct CliHealthInspector<'a> {
    service: &'a LibraryService<FileStore>,
    store: &'a FileStore,
    config_dir: &'a Path,
}

impl std::fmt::Debug for CliHealthInspector<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CliHealthInspector")
            .field("data_dir", &self.store.data_dir())
            .field("config_dir", &self.config_dir)
            .finish_non_exhaustive()
    }
}

impl<'a> CliHealthInspector<'a> {
    const fn new(
        service: &'a LibraryService<FileStore>,
        store: &'a FileStore,
        config_dir: &'a Path,
    ) -> Self {
        Self {
            service,
            store,
            config_dir,
        }
    }

    fn collect(&self) -> Result<HealthSnapshot, CliError> {
        let scan = self.service.list()?;
        let mut entries = Vec::with_capacity(scan.entries.len());
        for summary in &scan.entries {
            match self.service.show(summary.slug.as_str()) {
                Ok(entry) => entries.push(entry),
                Err(RepositoryError::NotFound { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let probe = SystemProbe;
        let private_uv = managed_uv_path(self.store.data_dir());
        let uv_path = probe
            .find_program("uv")
            .or_else(|| probe.is_executable(&private_uv).then_some(private_uv));
        let uv = match uv_path {
            Some(path) => UvHealth::Found(path.display().to_string()),
            None => UvHealth::Missing,
        };
        let config = FileConfigStore::new(self.config_dir);
        let mut missing = Vec::new();
        let mut drifted = Vec::new();
        let mut needs_missing = Vec::new();
        let mut launch_blocked = Vec::new();
        for entry in &entries {
            if entry_missing(self.store, entry) {
                missing.push(HealthIssue {
                    slug: entry.slug.as_str().to_owned(),
                    name: entry.meta.name.clone(),
                    kind: HealthIssueKind::MissingTarget,
                });
            }
            if doctor_entry_drifted(self.store, entry) {
                drifted.push(HealthIssue {
                    slug: entry.slug.as_str().to_owned(),
                    name: entry.meta.name.clone(),
                    kind: HealthIssueKind::DriftedForm,
                });
            }
            let mut settings = EntrySettings::from_meta(&entry.meta);
            if entry.meta.kind.as_str() == "python"
                && settings.interpreter.is_empty()
                && let UvHealth::Found(path) = &uv
            {
                settings.interpreter.clone_from(path);
            }
            let absent = settings
                .needs
                .iter()
                .filter(|name| probe.find_program(name).is_none())
                .cloned()
                .collect::<Vec<_>>();
            if !absent.is_empty() {
                needs_missing.push(HealthIssue {
                    slug: entry.slug.as_str().to_owned(),
                    name: entry.meta.name.clone(),
                    kind: HealthIssueKind::MissingNeeds { tools: absent },
                });
            } else if known_entry_kind(entry.meta.kind.as_str())
                && !entry_missing(self.store, entry)
                && let Some(reason) = doctor_launch_block(entry, &settings, &config, &probe)?
            {
                launch_blocked.push(HealthIssue {
                    slug: entry.slug.as_str().to_owned(),
                    name: entry.meta.name.clone(),
                    kind: HealthIssueKind::LaunchBlocked {
                        reason: reason.localize(active_locale()),
                    },
                });
            }
        }
        let issues = missing
            .into_iter()
            .chain(drifted)
            .chain(needs_missing)
            .chain(launch_blocked)
            .collect();
        let invalid_runner_rows = config
            .runner_rows()?
            .into_iter()
            .filter(|row| row.reason.is_some())
            .map(|row| row.localized_descriptor(active_locale()))
            .collect();
        let mirrors = config.mirror()?;
        let mirror_settings = config.settings()?;
        let pypi = &mirror_settings["mirror.pypi"];
        let github = &mirror_settings["mirror.github"];
        let npm = &mirror_settings["mirror.npm"];
        let mirror_axes = format!("pypi={pypi} · github={github} · npm={npm}");
        let mirror = if [pypi, github, npm].into_iter().all(|axis| axis == "off") {
            MirrorHealth::Off
        } else if mirrors.enabled {
            MirrorHealth::On { axes: mirror_axes }
        } else {
            MirrorHealth::Paused { axes: mirror_axes }
        };
        let scripts = self.store.data_dir().join("scripts");
        let size = directory_size(&scripts);
        Ok(HealthSnapshot {
            uv,
            entry_count: scan.entries.len(),
            issues,
            invalid_runner_rows,
            mirror,
            library_path: scripts.display().to_string(),
            library_size: health_size_text(size),
            diagnostics: scan
                .diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.localize(active_locale()))
                .collect(),
        })
    }
}

impl HealthInspection for CliHealthInspector<'_> {
    type Error = CliError;

    fn inspect(&self) -> Result<HealthSnapshot, Self::Error> {
        self.collect()
    }

    fn rebuild(&self) -> Result<HealthRebuild, Self::Error> {
        let problems = self
            .service
            .list()?
            .diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.localize(active_locale()))
            .collect::<Vec<_>>();
        let entry_count = self.store.rebuild_registry()?;
        Ok(HealthRebuild {
            snapshot: self.collect()?,
            outcome: HealthRebuildOutcome {
                entry_count,
                problems,
            },
        })
    }
}

fn health_size_text(size: u64) -> String {
    if size < 1024 {
        return format!("{size} B");
    }
    let mut value = size as f64 / 1024.0;
    for unit in ["KB", "MB", "GB"] {
        if value < 1024.0 || unit == "GB" {
            return format!("{value:.1} {unit}");
        }
        value /= 1024.0;
    }
    unreachable!("the final size unit always returns")
}

fn tui_runners_screen(
    service: &LibraryService<FileStore>,
    config_dir: &Path,
) -> Result<Screen, CliError> {
    Ok(Screen::Runners(Box::new(RunnerManagerView::new(
        tui_runner_rows(service, config_dir)?,
    ))))
}

fn tui_runner_rows(
    service: &LibraryService<FileStore>,
    config_dir: &Path,
) -> Result<Vec<RunnerRow>, CliError> {
    let config = FileConfigStore::new(config_dir);
    config.ensure_runners_seeded()?;
    let rows = config.runner_rows()?;
    let mut pinned = BTreeMap::<String, usize>::new();
    for summary in service
        .list()?
        .entries
        .into_iter()
        .filter(|summary| summary.kind.as_str() == "prompt")
    {
        let entry = match service.show(summary.slug.as_str()) {
            Ok(entry) => entry,
            Err(RepositoryError::NotFound { .. }) => continue,
            Err(error) => return Err(error.into()),
        };
        let runner = EntrySettings::from_meta(&entry.meta).runner;
        if !runner.is_empty() {
            *pinned.entry(runner).or_default() += 1;
        }
    }
    let identities = rows
        .iter()
        .map(|row| RunnerRowIdentity {
            index: row.index,
            snapshot_token: row.snapshot_token(),
        })
        .collect::<Vec<_>>();
    Ok(rows
        .iter()
        .zip(&identities)
        .map(|(row, identity)| {
            let key_identities = row.name.as_ref().map_or_else(Vec::new, |name| {
                rows.iter()
                    .zip(&identities)
                    .filter(|(candidate, _)| candidate.name.as_ref() == Some(name))
                    .map(|(_, identity)| identity.clone())
                    .collect()
            });
            RunnerRow {
                identity: identity.clone(),
                name: row.name.clone(),
                argv: row.argv.clone(),
                reason: row.reason.clone(),
                descriptor: row.localized_descriptor(active_locale()),
                key_identities,
                pinned_count: row
                    .name
                    .as_ref()
                    .and_then(|name| pinned.get(name))
                    .copied()
                    .unwrap_or(0),
            }
        })
        .collect())
}

fn tui_save_runner(
    service: &LibraryService<FileStore>,
    config_dir: &Path,
    request: RunnerSaveRequest,
    owner: RunnerSaveOwner,
) -> Result<UiAction, CliError> {
    let config = FileConfigStore::new(config_dir);
    let runner = PromptRunner {
        name: request.name.clone(),
        argv: request.argv.clone(),
    };
    let updated = !matches!(request.target, RunnerSaveTarget::New);
    let result = match &request.target {
        RunnerSaveTarget::New => config.set_runner(runner, false).map(|_| true),
        RunnerSaveTarget::Named { expected, .. } => {
            let current = config.runner_rows()?;
            let Some(expected) = resolve_runner_rows(&current, expected) else {
                return Ok(tui_runner_save_failure(
                    owner,
                    text(
                        active_locale(),
                        "The runner row changed before it could be saved; inspect again.",
                    )
                    .into_owned(),
                ));
            };
            config.set_runner_if_unchanged(runner, &expected)
        }
        RunnerSaveTarget::RawRow { expected } => {
            let current = config.runner_rows()?;
            let Some(expected) = resolve_runner_row(&current, expected) else {
                return Ok(tui_runner_save_failure(
                    owner,
                    text(
                        active_locale(),
                        "The runner row changed before it could be saved; inspect again.",
                    )
                    .into_owned(),
                ));
            };
            config.replace_runner_row_if_unchanged(runner, expected)
        }
    };
    let saved = match result {
        Ok(saved) => saved,
        Err(error) => {
            return Ok(tui_runner_save_failure(
                owner,
                error.message().localize(active_locale()),
            ));
        }
    };
    if !saved {
        return Ok(tui_runner_save_failure(
            owner,
            text(
                active_locale(),
                "The runner row changed before it could be saved; inspect again.",
            )
            .into_owned(),
        ));
    }
    let template = if updated {
        "Runner {} updated: {}"
    } else {
        "Runner {} added: {}"
    };
    let message = format_text(
        active_locale(),
        template,
        &[&request.name, &runner_command_text(&request.argv)],
    );
    Ok(match owner {
        RunnerSaveOwner::Manager => UiAction::Runners(RunnerManagerAction::MutationSucceeded {
            rows: tui_runner_rows(service, config_dir)?,
            selected_name: Some(request.name),
            message,
        }),
        RunnerSaveOwner::Editor(owner) => UiAction::RunnerEditorSaved {
            owner,
            name: request.name,
            message,
        },
    })
}

fn tui_runner_save_failure(owner: RunnerSaveOwner, message: String) -> UiAction {
    match owner {
        RunnerSaveOwner::Manager => UiAction::Runners(RunnerManagerAction::MutationFailed(message)),
        RunnerSaveOwner::Editor(owner) => UiAction::RunnerEditorSaveFailed { owner, message },
    }
}

fn tui_remove_runner(
    service: &LibraryService<FileStore>,
    config_dir: &Path,
    request: RunnerRemoveRequest,
) -> Result<UiAction, CliError> {
    let config = FileConfigStore::new(config_dir);
    let current = config.runner_rows()?;
    match &request {
        RunnerRemoveRequest::Named {
            name,
            expected,
            expected_pinned_count,
        } => {
            let Some(expected) = resolve_runner_rows(&current, expected) else {
                return Ok(UiAction::Runners(RunnerManagerAction::MutationFailed(
                    text(
                        active_locale(),
                        "The runner row changed before it could be removed; inspect again.",
                    )
                    .into_owned(),
                )));
            };
            let management =
                FileRunnerManagementStore::new(service.repository().data_dir(), config_dir);
            match management.remove_named_if_unchanged(
                name,
                &expected,
                *expected_pinned_count,
            ) {
                Ok(RunnerRemovalCas::Removed) => {
                    Ok(UiAction::Runners(RunnerManagerAction::MutationSucceeded {
                        rows: tui_runner_rows(service, config_dir)?,
                        selected_name: None,
                        message: format_text(active_locale(), "Runner {} removed.", &[name]),
                    }))
                }
                Ok(RunnerRemovalCas::RowsChanged) => Ok(UiAction::Runners(
                    RunnerManagerAction::MutationFailed(
                        text(
                            active_locale(),
                            "The runner row changed before it could be removed; inspect again.",
                        )
                        .into_owned(),
                    ),
                )),
                Ok(RunnerRemovalCas::PinsChanged { .. }) => Ok(UiAction::Runners(
                    RunnerManagerAction::MutationFailed(
                        text(
                            active_locale(),
                            "The prompt pins changed before the runner could be removed; inspect again.",
                        )
                        .into_owned(),
                    ),
                )),
                Err(error) => Ok(UiAction::Runners(RunnerManagerAction::MutationFailed(
                    match error {
                        RunnerManagementStoreError::Library(error) => {
                            error.message().localize(active_locale())
                        }
                        RunnerManagementStoreError::Config(error) => {
                            error.message().localize(active_locale())
                        }
                    },
                ))),
            }
        }
        RunnerRemoveRequest::RawRow { expected } => {
            let Some(row) = resolve_runner_row(&current, expected) else {
                return Ok(UiAction::Runners(RunnerManagerAction::MutationFailed(
                    text(
                        active_locale(),
                        "The runner row changed before it could be removed; inspect again.",
                    )
                    .into_owned(),
                )));
            };
            let message = row.index.map_or_else(
                || {
                    text(
                        active_locale(),
                        "Malformed prompt runner container removed.",
                    )
                    .into_owned()
                },
                |index| {
                    format_text(
                        active_locale(),
                        "Malformed runner row {} removed.",
                        &[&index],
                    )
                },
            );
            match config.remove_runner_row_if_unchanged(row) {
                Ok(true) => Ok(UiAction::Runners(RunnerManagerAction::MutationSucceeded {
                    rows: tui_runner_rows(service, config_dir)?,
                    selected_name: None,
                    message,
                })),
                Ok(false) => Ok(UiAction::Runners(RunnerManagerAction::MutationFailed(
                    text(
                        active_locale(),
                        "The runner row changed before it could be removed; inspect again.",
                    )
                    .into_owned(),
                ))),
                Err(error) => Ok(UiAction::Runners(RunnerManagerAction::MutationFailed(
                    error.message().localize(active_locale()),
                ))),
            }
        }
    }
}

fn resolve_runner_rows(
    rows: &[skit_store::PromptRunnerRow],
    identities: &[RunnerRowIdentity],
) -> Option<Vec<skit_store::PromptRunnerRow>> {
    let resolved = identities
        .iter()
        .map(|identity| resolve_runner_row(rows, identity).cloned())
        .collect::<Option<Vec<_>>>()?;
    let unique = resolved
        .iter()
        .map(|row| row.index)
        .collect::<BTreeSet<_>>();
    (unique.len() == resolved.len()).then_some(resolved)
}

fn resolve_runner_row<'a>(
    rows: &'a [skit_store::PromptRunnerRow],
    identity: &RunnerRowIdentity,
) -> Option<&'a skit_store::PromptRunnerRow> {
    rows.iter()
        .find(|row| row.index == identity.index && row.snapshot_token() == identity.snapshot_token)
}

fn tui_presets_form(entry: &Entry, presets: &[String]) -> Screen {
    Screen::Form(FormView {
        purpose: FormPurpose::Presets,
        title: "Presets for {}: {}".to_owned(),
        title_arguments: vec![entry.meta.name.clone(), presets.join(", ")],
        translate_title: true,
        selector: Some(entry.slug.as_str().to_owned()),
        fields: vec![
            FormField::text("name", "Preset name", ""),
            FormField::text("action", "Action (save or delete)", "save"),
        ],
        focused: 0,
        submit_label: "Apply".to_owned(),
    })
}

fn tui_options_field(
    key: &str,
    label: &str,
    options_label: &str,
    options: &[String],
    value: impl Into<String>,
) -> FormField {
    if options.is_empty() {
        FormField::text(key, label, value)
    } else {
        FormField::text_with_arguments(key, options_label, vec![options.join(", ")], value)
    }
}

fn tui_split_list(value: &str) -> Vec<String> {
    value
        .split(|character: char| character == ',' || character.is_whitespace())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn tui_dependency_list(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn tui_parameter_value(value: &ParameterValue) -> String {
    match value {
        ParameterValue::String(value) => value.clone(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => serde_json::Number::from_f64(*value)
            .expect("parameter defaults are finite")
            .to_string(),
        ParameterValue::Bool(value) => value.to_string(),
    }
}

fn tui_declarations_from_values(
    values: &BTreeMap<String, String>,
) -> Result<Vec<ParamDecl>, CliError> {
    let mut declarations = Vec::new();
    for index in 0.. {
        let prefix = format!("parameter:{index}");
        let name_key = format!("{prefix}:name");
        let Some(name) = values.get(&name_key) else {
            break;
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(CliError::Usage(
                Message::new("parameter {} needs a name").with(index),
            ));
        }
        if declarations
            .iter()
            .any(|item: &ParamDecl| item.name == name)
        {
            return Err(CliError::Usage(
                Message::new("duplicate parameter: {}").with(name),
            ));
        }
        let get = |field: &str| {
            values
                .get(&format!("{prefix}:{field}"))
                .map_or("", String::as_str)
                .trim()
        };
        let mut declaration = ParamDecl::new(name);
        declaration.binding = match get("binding") {
            "" | "none" => ParameterBinding::None,
            "const" => ParameterBinding::Const,
            "input" => ParameterBinding::Input,
            "envdefault" => ParameterBinding::EnvDefault,
            value => {
                return Err(CliError::Usage(
                    Message::new("unknown parameter binding: {}").with(value),
                ));
            }
        };
        declaration.delivery = parse_delivery(if get("delivery").is_empty() {
            "flag"
        } else {
            get("delivery")
        })?;
        declaration.parameter_type = parse_parameter_type(if get("type").is_empty() {
            "str"
        } else {
            get("type")
        })?;
        if !get("default").is_empty() {
            declaration.default = Some(
                coerce_default(get("default"), declaration.parameter_type)
                    .map_err(|error| CliError::Usage(error.message()))?,
            );
        }
        declaration.choices = get("choices")
            .split(',')
            .map(str::trim)
            .filter(|choice| !choice.is_empty())
            .map(str::to_owned)
            .collect();
        declaration.required = tui_bool(get("required"))?;
        declaration.multiple = tui_bool(get("multiple"))?;
        declaration.repeat = tui_bool(get("repeat"))?;
        declaration.prompt = get("prompt").to_owned();
        declaration.help = get("help").to_owned();
        declaration.secret = tui_bool(get("secret"))?;
        declaration.env_source = get("env_source").to_owned();
        declaration.env_target = get("env_target").to_owned();
        declaration.flag = get("flag").to_owned();
        declaration.action = get("action").to_owned();
        if declaration.validate().is_some() {
            return Err(CliError::Usage(
                Message::new("parameter {} has incompatible settings").with(name),
            ));
        }
        declarations.push(declaration);
    }
    Ok(declarations)
}

fn tui_submit(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
    purpose: FormPurpose,
    selector: Option<String>,
    values: &BTreeMap<String, String>,
) -> Result<UiAction, CliError> {
    match purpose {
        FormPurpose::Run => tui_submit_run(
            service,
            store,
            state_dir,
            config_dir,
            tui_selector(&selector)?,
            values,
        ),
        FormPurpose::Add => {
            let source = tui_value(values, "source");
            let template = tui_value(values, "template");
            let kind = tui_value(values, "kind");
            add_with_config(
                service,
                config_dir,
                AddOptions {
                    source: (!source.is_empty()).then(|| PathBuf::from(source)),
                    kind: (!kind.is_empty()).then_some(kind.to_owned()),
                    name: tui_nonempty_owned(values, "name"),
                    description: tui_nonempty_owned(values, "description"),
                    reference: tui_value(values, "mode").eq_ignore_ascii_case("reference"),
                    command_template: (!template.is_empty()).then_some(template.to_owned()),
                    prompt: kind == "prompt",
                    executable: kind == "exe",
                    runner: tui_nonempty_owned(values, "runner"),
                    no_interpolate: false,
                    dependencies: tui_dependency_list(tui_value(values, "dependencies")),
                    dependencies_explicit: !tui_value(values, "dependencies").is_empty(),
                    requires_python: tui_nonempty_owned(values, "python"),
                    no_input: false,
                },
            )?;
            tui_complete(service, state_dir, "Entry added")
        }
        FormPurpose::Settings => {
            tui_submit_settings(service, store, state_dir, tui_selector(&selector)?, values)?;
            tui_complete(service, state_dir, "Settings saved")
        }
        FormPurpose::Preferences => {
            let config = FileConfigStore::new(config_dir);
            config.set_many(values)?;
            tui_complete(service, state_dir, "Preferences saved")
        }
        FormPurpose::Runners => {
            let config = FileConfigStore::new(config_dir);
            let name = tui_required(values, "name")?;
            if tui_bool(tui_value(values, "remove"))? {
                if !config.remove_runner(name)? {
                    return Err(CliError::Usage(
                        Message::new("unknown prompt runner: {}").with(name),
                    ));
                }
            } else {
                let argv = shlex::split(tui_required(values, "argv")?).ok_or_else(|| {
                    CliError::Usage(Message::new("the runner arguments have invalid quoting"))
                })?;
                config.set_runner(
                    PromptRunner {
                        name: name.to_owned(),
                        argv,
                    },
                    true,
                )?;
            }
            tui_complete(service, state_dir, "Prompt runners saved")
        }
        FormPurpose::Presets => {
            let selector = tui_selector(&selector)?;
            let entry = service.show(selector)?;
            let declarations = entry_parameters(store, &entry);
            let state = FormStateService::new(FileFormStateStore::new(state_dir));
            let name = tui_required(values, "name")?;
            if tui_value(values, "action").eq_ignore_ascii_case("delete") {
                if !state.delete_preset(&entry.slug, name)? {
                    return Err(CliError::Usage(
                        Message::new("unknown preset: {}").with(name),
                    ));
                }
            } else {
                refuse_empty_preset_schema(&declarations)?;
                let current = state.load(&entry.slug);
                state.save_preset(&entry.slug, name, &declarations, &current.values)?;
            }
            tui_complete(service, state_dir, "Presets saved")
        }
        FormPurpose::Rename => {
            rename(
                service,
                tui_selector(&selector)?,
                tui_required(values, "name")?,
            )?;
            tui_complete(service, state_dir, "Entry renamed")
        }
    }
}

fn refuse_empty_preset_schema(declarations: &[ParamDecl]) -> Result<(), CliError> {
    if declarations.is_empty() {
        Err(CliError::Usage(Message::new(
            "cannot save a preset because the entry has no form fields",
        )))
    } else {
        Ok(())
    }
}

fn tui_submit_run(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
    selector: &str,
    values: &BTreeMap<String, String>,
) -> Result<UiAction, CliError> {
    let entry = service.show(selector)?;
    let saved = FormStateService::new(FileFormStateStore::new(state_dir)).load(&entry.slug);
    let run_values = changed_form_values(values, &saved.values);
    let extra_args = split_editable_arguments(tui_value(values, "_skit_args"))?;
    let exit = crate::run::run_with_roots(
        service,
        store,
        state_dir,
        config_dir,
        RunArgs {
            selector: selector.to_owned(),
            values: run_values,
            preset: tui_nonempty_owned(values, "_skit_preset"),
            save_preset: tui_nonempty_owned(values, "_skit_save_preset"),
            runner: tui_nonempty_owned(values, "_skit_runner"),
            dry_run: tui_bool(tui_value(values, "_skit_dry_run"))?,
            no_input: true,
            plain: true,
            raw: false,
            forget_args: false,
            extra_args,
        },
    )?;
    if FileConfigStore::new(config_dir).get("after_run")? == "exit" {
        Ok(UiAction::Quit)
    } else {
        tui_complete(
            service,
            state_dir,
            &format!("Run finished with exit status {exit}"),
        )
    }
}

fn tui_submit_settings(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    selector: &str,
    values: &BTreeMap<String, String>,
) -> Result<(), CliError> {
    let entry = service.show(selector)?;
    let name = tui_required(values, "name")?;
    let description = tui_value(values, "description");
    let stored_settings = EntrySettings::from_meta(&entry.meta);
    let baseline_settings = effective_settings(store, &entry);
    let mut settings = stored_settings.clone();
    settings.interpreter = tui_value(values, "interpreter").to_owned();
    if !settings.interpreter.is_empty()
        && !matches!(
            entry.meta.kind.as_str(),
            "shell" | "fish" | "powershell" | "ruby" | "perl" | "lua" | "r" | "js" | "ts"
        )
    {
        return Err(CliError::Usage(Message::new(
            "the entry does not use a pinnable interpreter",
        )));
    }
    settings.runner = tui_value(values, "runner").trim().to_owned();
    let submitted_dependencies = tui_dependency_list(tui_value(values, "dependencies"));
    let submitted_python = tui_value(values, "python").to_owned();
    if !submitted_dependencies.is_empty()
        && !matches!(entry.meta.kind.as_str(), "python" | "js" | "ts")
    {
        return Err(CliError::Usage(Message::new(
            "package dependencies apply only to Python and JavaScript entries",
        )));
    }
    if !submitted_python.is_empty() && entry.meta.kind.as_str() != "python" {
        return Err(CliError::Usage(Message::new(
            "a Python constraint applies only to Python entries",
        )));
    }
    let dependencies_edit = (submitted_dependencies != baseline_settings.dependencies)
        .then_some(submitted_dependencies.clone());
    let python_edit =
        (submitted_python != baseline_settings.requires_python).then_some(submitted_python.clone());
    if entry.meta.kind.as_str() == "python" {
        for requirement in dependencies_edit.as_deref().unwrap_or_default() {
            validate_pep508_requirement(requirement)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
        if let Some(version) = python_edit.as_deref() {
            let normalized = version.trim();
            if !normalized.is_empty()
                && !matches!(normalized.to_ascii_lowercase().as_str(), "-" | "none")
            {
                validate_pep440_specifiers(normalized)
                    .map_err(|error| CliError::Usage(error.message()))?;
            }
        }
    } else {
        settings.dependencies = submitted_dependencies;
        settings.requires_python = submitted_python;
    }
    settings.needs = tui_split_list(tui_value(values, "needs"));
    let previous_template = settings.template.clone();
    settings.template = tui_value(values, "template").to_owned();
    settings.interpolate = tui_bool(tui_value(values, "interpolate"))?;
    let mut declarations = tui_declarations_from_values(values)?;
    let removed = tui_split_list(tui_value(values, "parameter:remove"));
    declarations.retain(|parameter| !removed.contains(&parameter.name));
    for name in tui_split_list(tui_value(values, "parameter:add")) {
        if declarations.iter().any(|parameter| parameter.name == name) {
            return Err(CliError::Usage(
                Message::new("parameter already exists: {}").with(name),
            ));
        }
        declarations.push(ParamDecl::new(name));
    }
    if entry.meta.kind.as_str() == "command" && settings.template != previous_template {
        declarations = reconcile_template_parameters(&settings.template, &declarations);
    }
    let original_source = source_path(store, &entry).and_then(|path| fs::read(path).ok());
    let source_view = original_source.as_deref().map(LosslessSource::from_bytes);
    let source_interface_names = original_source
        .as_ref()
        .zip(source_view.as_ref())
        .map_or_else(BTreeSet::new, |source| {
            managed_params(entry.meta.kind.as_str(), source.1.normalized_text())
                .into_iter()
                .chain(cli_params(
                    entry.meta.kind.as_str(),
                    source.1.normalized_text(),
                ))
                .map(|parameter| parameter.name)
                .collect()
        });
    settings.parameters = declarations
        .iter()
        .filter(|parameter| !source_interface_names.contains(&parameter.name))
        .cloned()
        .collect();
    if matches!(entry.meta.kind.as_str(), "command" | "prompt") {
        settings.params = settings
            .parameters
            .iter()
            .filter(|parameter| parameter.delivery == ParameterDelivery::Placeholder)
            .map(|parameter| parameter.name.clone())
            .collect();
    }
    let mut rewritten_source = None;
    if entry.meta.kind.as_str() == "python" && entry.meta.mode == StorageMode::Copy {
        let plan = plan_uv_metadata_edit(
            original_source.as_deref(),
            &UvMetadata {
                dependencies: stored_settings.dependencies.clone(),
                requires_python: stored_settings.requires_python.clone(),
            },
            dependencies_edit,
            python_edit,
        )
        .map_err(|error| uv_edit_error(&entry.meta.name, error))?;
        settings.dependencies = plan.stored.dependencies;
        settings.requires_python = plan.stored.requires_python;
        rewritten_source = plan.rewritten_source;
    } else if entry.meta.kind.as_str() == "python" {
        let plan = plan_uv_metadata_edit(
            None,
            &UvMetadata {
                dependencies: stored_settings.dependencies.clone(),
                requires_python: stored_settings.requires_python.clone(),
            },
            dependencies_edit,
            python_edit,
        )
        .map_err(|error| uv_edit_error(&entry.meta.name, error))?;
        settings.dependencies = plan.stored.dependencies;
        settings.requires_python = plan.stored.requires_python;
    }
    let source_requested = tui_bool(tui_value(values, "source:resync"))?
        || !tui_value(values, "source:manage").is_empty()
        || !tui_value(values, "source:unmanage").is_empty()
        || !tui_value(values, "source:normalize").is_empty();
    if let Some(original_bytes) = original_source.as_deref() {
        let mut working = rewritten_source
            .take()
            .unwrap_or_else(|| original_bytes.to_vec());
        let view = LosslessSource::from_bytes(&working);
        let original_text = view.normalized_text().to_owned();
        let (rewritten, mut managed) = prepare_source_management(
            entry.meta.kind.as_str(),
            entry.meta.mode,
            original_text.clone(),
            tui_bool(tui_value(values, "source:resync"))?,
            &tui_split_list(tui_value(values, "source:manage")),
            &tui_split_list(tui_value(values, "source:unmanage")),
            &tui_split_list(tui_value(values, "source:normalize")),
        )?;
        for parameter in &mut managed {
            if let Some(submitted) = declarations
                .iter()
                .find(|submitted| submitted.name == parameter.name)
            {
                if submitted.binding == ParameterBinding::None {
                    return Err(CliError::Usage(
                        Message::new("use source:unmanage to remove the source binding for {}")
                            .with(&submitted.name),
                    ));
                }
                *parameter = submitted.clone();
            }
        }
        let source_text_changed = rewritten != original_text;
        if source_text_changed {
            working = view.restore_bytes(&rewritten);
        }
        let before_managed = managed_params(entry.meta.kind.as_str(), &rewritten);
        if source_requested || source_text_changed || managed != before_managed {
            working = write_managed_params_bytes(entry.meta.kind.as_str(), &working, &managed)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
        if working != original_bytes {
            rewritten_source = Some(working);
        }
    } else if source_requested {
        return Err(CliError::Usage(Message::new(
            "the stored source is not valid UTF-8",
        )));
    }
    let claimed = service.claim_identity(&entry)?;
    let entry = service.update_entry(
        &claimed,
        UpdateEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            settings: settings.clone(),
            workdir: tui_value(values, "workdir").to_owned(),
            source: rewritten_source,
            expected_source_hash: entry.meta.source_hash.clone(),
        },
    )?;
    let state = FormStateService::new(FileFormStateStore::new(state_dir));
    state.purge_secrets(&entry.slug, &declarations)?;
    Ok(())
}

fn tui_complete(
    service: &LibraryService<FileStore>,
    state_dir: &Path,
    message: &str,
) -> Result<UiAction, CliError> {
    let scan = service.list()?;
    let rerunnable = tui_rerunnable(&scan, state_dir);
    Ok(UiAction::Complete {
        scan: Some(scan),
        rerunnable: Some(rerunnable),
        message: message.to_owned(),
    })
}

fn tui_value<'a>(values: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    values.get(key).map_or("", String::as_str).trim()
}

fn tui_nonempty_owned(values: &BTreeMap<String, String>, key: &str) -> Option<String> {
    let value = tui_value(values, key);
    (!value.is_empty()).then(|| value.to_owned())
}

fn tui_required<'a>(values: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, CliError> {
    let value = tui_value(values, key);
    if value.is_empty() {
        Err(CliError::Usage(Message::new("{} is required").with(key)))
    } else {
        Ok(value)
    }
}

fn tui_bool(value: &str) -> Result<bool, CliError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "" | "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(CliError::Usage(
            Message::new("invalid Boolean value {}; use true or false").quoted(value),
        )),
    }
}

#[derive(Debug)]
struct SourceSnapshot {
    bytes: Vec<u8>,
    permissions: SourcePermissions,
    is_regular: bool,
}

fn read_source(path: &Path, allow_non_regular: bool) -> Result<SourceSnapshot, CliError> {
    let metadata = fs::metadata(path).map_err(|error| source_error("inspect", path, error))?;
    if allow_non_regular && !metadata.is_file() {
        return Ok(SourceSnapshot {
            bytes: Vec::new(),
            permissions: source_permissions(&metadata),
            is_regular: false,
        });
    }
    let mut file = File::open(path).map_err(|error| source_error("open", path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| source_error("inspect", path, error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| source_error("read", path, error))?;
    Ok(SourceSnapshot {
        bytes,
        permissions: source_permissions(&metadata),
        is_regular: metadata.is_file(),
    })
}

fn source_default_name(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("script")
        .to_owned()
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

fn resolve_state_dir() -> Result<PathBuf, CliError> {
    if let Some(path) = env::var_os("SKIT_STATE_DIR") {
        return Ok(PathBuf::from(path));
    }
    platform_state_dir().ok_or(CliError::DirectoryUnavailable("state"))
}

fn resolve_config_dir() -> Result<PathBuf, CliError> {
    if let Some(path) = env::var_os("SKIT_CONFIG_DIR") {
        return Ok(PathBuf::from(path));
    }
    platform_config_dir().ok_or(CliError::DirectoryUnavailable("configuration"))
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .map(PathBuf::from)
        .map(|path| path.join("skit"))
}

#[cfg(target_os = "windows")]
fn platform_state_dir() -> Option<PathBuf> {
    platform_data_dir()
}

#[cfg(target_os = "windows")]
fn platform_config_dir() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("skit"))
}

#[cfg(target_os = "macos")]
fn platform_state_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).map(|path| {
        path.join("Library")
            .join("Application Support")
            .join("skit")
    })
}

#[cfg(target_os = "macos")]
fn platform_config_dir() -> Option<PathBuf> {
    platform_state_dir()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_state_dir() -> Option<PathBuf> {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .map(|path| path.join("skit"))
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".local").join("state").join("skit"))
        })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_config_dir() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .map(|path| path.join("skit"))
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".config").join("skit"))
        })
}

#[cfg(not(any(unix, target_os = "windows")))]
fn platform_state_dir() -> Option<PathBuf> {
    None
}

#[cfg(not(any(unix, target_os = "windows")))]
fn platform_config_dir() -> Option<PathBuf> {
    None
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
    #[error(transparent)]
    Run(#[from] RunError),
    #[error(transparent)]
    Dependencies(#[from] DependencyError),
    #[error("could not encode JSON output: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not write output: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Tui(#[from] skit_tui::TuiError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    State(#[from] StateWriteError),
    #[error("{0}")]
    Usage(Message),
    #[error("{0}")]
    Failure(Message),
    #[error("confirmation is required; pass --yes to remove the entry")]
    ConfirmationRequired,
    #[error("operation cancelled")]
    Aborted,
    #[error("Cancelled — nothing was added.")]
    AddCancelled,
    #[error("could not {operation} {path}: {source}")]
    Source {
        operation: &'static str,
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("could not determine the platform data directory; pass --data-dir or SKIT_DATA_DIR")]
    DataDirectoryUnavailable,
    #[error("could not determine the platform {0} directory; set the matching SKIT_*_DIR variable")]
    DirectoryUnavailable(&'static str),
}

impl Localize for CliError {
    fn message(&self) -> Message {
        match self {
            Self::Repository(error) => error.message(),
            Self::Run(error) => error.message(),
            Self::Dependencies(error) => error.message(),
            Self::Json(error) => Message::new("could not encode JSON output: {}").with(error),
            Self::Io(error) => Message::new("could not write output: {}").with(error),
            Self::Tui(error) => error.message(),
            Self::Config(error) => error.message(),
            Self::State(error) => error.message(),
            Self::Usage(message) => message.clone(),
            Self::Failure(message) => message.clone(),
            Self::ConfirmationRequired => {
                Message::new("confirmation is required; pass --yes to remove the entry")
            }
            Self::Aborted => Message::new("operation cancelled"),
            Self::AddCancelled => Message::new("Cancelled — nothing was added."),
            Self::Source {
                operation,
                path,
                source,
            } => Message::new("could not {} {}: {}")
                .nested(Message::term(operation))
                .with(path)
                .with(source),
            Self::DataDirectoryUnavailable => Message::new(
                "could not determine the platform data directory; pass --data-dir or SKIT_DATA_DIR",
            ),
            Self::DirectoryUnavailable(name) => Message::new(
                "could not determine the platform {} directory; set the matching SKIT_*_DIR variable",
            )
            .with(name),
        }
    }
}

impl CliError {
    const fn exit_code(&self) -> i32 {
        match self {
            Self::Repository(error) => error.exit_class(RepositoryOperation::Manage).code() as i32,
            Self::Run(error) => error.exit_code(),
            Self::Dependencies(_) => ExitClass::Skit.code() as i32,
            Self::ConfirmationRequired | Self::Usage(_) => ExitClass::Usage.code() as i32,
            Self::Failure(_) => ExitClass::Failure.code() as i32,
            Self::Aborted | Self::AddCancelled => ExitClass::Aborted.code() as i32,
            Self::Config(error) if error.is_usage() => ExitClass::Usage.code() as i32,
            Self::Config(_) => ExitClass::Failure.code() as i32,
            Self::Json(_)
            | Self::Io(_)
            | Self::Tui(_)
            | Self::State(_)
            | Self::DataDirectoryUnavailable
            | Self::DirectoryUnavailable(_) => ExitClass::Skit.code() as i32,
            Self::Source { .. } => ExitClass::Failure.code() as i32,
        }
    }
}

const fn mode_name(mode: StorageMode) -> &'static str {
    match mode {
        StorageMode::Copy => "copy",
        StorageMode::Reference => "reference",
    }
}
