use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File, Metadata},
    io::{self, IsTerminal as _, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use clap::{Args, CommandFactory as _, FromArgMatches as _, Parser, Subcommand};
use clap_complete::{ArgValueCandidates, CompleteEnv, CompletionCandidate, Shell, generate};
use skit_application::{
    CreateEntry, EntryPayload, EntryRepository as _, ExitClass, LibraryService, RepositoryError,
    SourcePermissions, UpdateEntry,
    form_state::{FormStateService, StateWriteError, prefill},
};
use skit_domain::{
    Entry, EntryKind, EntrySettings, EntrySummary, StorageMode,
    parameters::{
        ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
        coerce_default,
    },
};
use skit_form::form_params;
use skit_i18n::{Locale, Localize, Message, detect_locale, format_text, render as localize, text};
use skit_language::{
    cli_params, detect_candidates, external_dependencies_at, infer_kind, managed_params,
    normalize_shell_default, placeholder_params, python_version_pin, read_uv_metadata,
    shebang_program, validate_pep440_specifiers, validate_pep508_requirement, write_managed_params,
    write_uv_metadata,
};
use skit_runtime::{
    DependencyError, ProgramProbe, SystemProbe, clear_javascript_dependencies, managed_uv_path,
    resolve_javascript_runtime,
};
use skit_store::{ConfigError, FileConfigStore, FileFormStateStore, PromptRunner};
use skit_store::{FileStore, stored_filename};
use skit_ui::{
    Action as UiAction, Effect as UiEffect, FormField, FormPurpose, FormView, HostRequest,
    LibraryState, ReportItem, ReportView, Screen,
};
use thiserror::Error;

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
            let _ = error.print();
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
            // Clap composes this report. Only whole framework words change.
            let output = localize(locale, &error.to_string());
            if error.use_stderr() {
                eprint!("{output}");
            } else {
                print!("{output}");
            }
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
        command = command.about(text(locale, &about).to_owned());
    }
    if let Some(about) = command.get_long_about().map(ToString::to_string) {
        command = command.long_about(text(locale, &about).to_owned());
    }
    command = command.mut_args(|argument| {
        let Some(help) = argument.get_help().map(|help| help.to_string()) else {
            return argument;
        };
        argument.help(text(locale, &help).to_owned())
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
    if let Some(language) = env::var_os("SKIT_LANG") {
        return detect_locale(language.to_str());
    }
    if let Ok(directory) = resolve_config_dir()
        && let Ok(language) = FileConfigStore::new(directory).get("lang")
        && !language.is_empty()
        && language != "auto"
    {
        return detect_locale(Some(&language));
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(language) = env::var_os(key) {
            return detect_locale(language.to_str());
        }
    }
    Locale::En
}

#[derive(Debug, Parser)]
#[command(
    name = "skit",
    version,
    about = "A script, prompt, program, and command library",
    disable_help_subcommand = true
)]
struct Cli {
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
    /// Rename one entry and derive its new slug.
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
        /// Remove one malformed raw row by its one-based index.
        #[arg(long, conflicts_with = "name")]
        row: Option<usize>,
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
                    description: description.unwrap_or_default(),
                    reference,
                    command_template,
                    prompt,
                    executable: exe,
                    runner,
                    no_interpolate,
                    dependencies: dependencies.unwrap_or_default(),
                    dependencies_explicit,
                    requires_python: python,
                },
                edit,
                no_input,
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
            runner(command)?;
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
    no_input: bool,
) -> Result<(), CliError> {
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
            validate_prompt_runner(options.runner.as_deref())?;
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
        let form = tui_add_form_view();
        let values = skit_tui::collect_form(form, active_locale())?.ok_or(CliError::Aborted)?;
        let source = tui_nonempty_owned(&values, "source").map(PathBuf::from);
        let template = tui_nonempty_owned(&values, "template");
        return add(
            service,
            AddOptions {
                source,
                kind: tui_nonempty_owned(&values, "kind"),
                name: tui_nonempty_owned(&values, "name"),
                description: tui_value(&values, "description").to_owned(),
                reference: tui_value(&values, "mode").eq_ignore_ascii_case("reference"),
                command_template: template,
                prompt: tui_value(&values, "kind").eq_ignore_ascii_case("prompt"),
                executable: tui_value(&values, "kind").eq_ignore_ascii_case("exe"),
                runner: tui_nonempty_owned(&values, "runner"),
                no_interpolate: false,
                dependencies: tui_dependency_list(tui_value(&values, "dependencies")),
                dependencies_explicit: !tui_value(&values, "dependencies").is_empty(),
                requires_python: tui_nonempty_owned(&values, "python"),
            },
        );
    }
    add(service, options)
}

fn add_draft(
    service: &LibraryService<FileStore>,
    mut options: AddOptions,
    prompt: bool,
) -> Result<(), CliError> {
    validate_prompt_runner(options.runner.as_deref())?;
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

fn validate_prompt_runner(name: Option<&str>) -> Result<(), CliError> {
    let config_dir = resolve_config_dir()?;
    validate_prompt_runner_in(&FileConfigStore::new(config_dir), name)
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

    let (form, baseline) = interactive_run_form(service, store, &args)?;
    let use_plain = args.plain
        || FileConfigStore::new(resolve_config_dir()?)
            .get("form")?
            .eq_ignore_ascii_case("plain");
    let values = if use_plain {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        let stdout = io::stdout();
        let mut output = stdout.lock();
        collect_plain_form(&form, active_locale(), &mut input, &mut output, |_| {
            rpassword::read_password()
        })
        .map_err(plain_form_error)?
    } else {
        skit_tui::collect_form(form, active_locale())?.ok_or(CliError::Aborted)?
    };
    apply_interactive_run_values(&mut args, &values, &baseline)?;
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

fn interactive_run_form(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    args: &RunArgs,
) -> Result<(FormView, BTreeMap<String, String>), CliError> {
    let entry = service.show(&args.selector)?;
    let declarations = entry_parameters(store, &entry);
    let saved =
        FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).load(&entry.slug);
    let preset = args
        .preset
        .as_deref()
        .and_then(|name| saved.presets.get(name));
    let initial = prefill(&declarations, &saved.values, preset);
    let settings = EntrySettings::from_meta(&entry.meta);
    let mut runners = FileConfigStore::new(resolve_config_dir()?)
        .runners()?
        .into_iter()
        .map(|runner| runner.name)
        .collect::<Vec<_>>();
    if !settings.runner.is_empty() {
        runners.retain(|name| name != &settings.runner);
        runners.insert(0, settings.runner);
    }
    let mut form = tui_run_form_view(
        &entry,
        &declarations,
        &initial,
        &runners,
        &saved.presets.keys().cloned().collect::<Vec<_>>(),
    );
    for value in &args.values {
        if let Some((name, value)) = value.split_once('=') {
            set_form_value(&mut form, &format!("value:{name}"), value);
        }
    }
    set_form_value(
        &mut form,
        "_skit_preset",
        args.preset.as_deref().unwrap_or_default(),
    );
    set_form_value(
        &mut form,
        "_skit_save_preset",
        args.save_preset.as_deref().unwrap_or_default(),
    );
    set_form_value(
        &mut form,
        "_skit_runner",
        args.runner.as_deref().unwrap_or_else(|| {
            if entry.meta.kind.as_str() == "prompt" {
                runners.first().map_or("", String::as_str)
            } else {
                ""
            }
        }),
    );
    set_form_value(
        &mut form,
        "_skit_args",
        &join_editable_arguments(&args.extra_args),
    );
    set_form_value(&mut form, "_skit_dry_run", &args.dry_run.to_string());
    Ok((form, initial))
}

fn set_form_value(form: &mut FormView, key: &str, value: &str) {
    if let Some(field) = form.fields.iter_mut().find(|field| field.key == key) {
        field.value = value.to_owned();
    }
}

fn apply_interactive_run_values(
    args: &mut RunArgs,
    values: &BTreeMap<String, String>,
    baseline: &BTreeMap<String, String>,
) -> Result<(), CliError> {
    args.values = changed_form_values(values, baseline);
    args.preset = tui_nonempty_owned(values, "_skit_preset");
    args.save_preset = tui_nonempty_owned(values, "_skit_save_preset");
    args.runner = tui_nonempty_owned(values, "_skit_runner");
    args.dry_run = tui_bool(tui_value(values, "_skit_dry_run"))?;
    args.extra_args = split_editable_arguments(tui_value(values, "_skit_args"))?;
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
            (!value.is_empty() && baseline.get(name) != Some(value))
                .then(|| format!("{name}={value}"))
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn split_editable_arguments(value: &str) -> Result<Vec<String>, CliError> {
    split_windows_arguments(value)
}

#[cfg(not(target_os = "windows"))]
fn split_editable_arguments(value: &str) -> Result<Vec<String>, CliError> {
    shlex::split(value)
        .ok_or_else(|| CliError::Usage(Message::new("extra arguments have invalid quoting")))
}

#[cfg(target_os = "windows")]
fn join_editable_arguments(arguments: &[String]) -> String {
    join_windows_arguments(arguments)
}

#[cfg(not(target_os = "windows"))]
fn join_editable_arguments(arguments: &[String]) -> String {
    shlex::try_join(arguments.iter().map(String::as_str)).unwrap_or_default()
}

#[cfg(any(test, target_os = "windows"))]
fn split_windows_arguments(value: &str) -> Result<Vec<String>, CliError> {
    let characters = value.chars().collect::<Vec<_>>();
    let mut arguments = Vec::new();
    let mut index = 0;
    loop {
        while index < characters.len() && matches!(characters[index], ' ' | '\t') {
            index += 1;
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
                    index += 1;
                }
                let backslashes = index - start;
                if index < characters.len() && characters[index] == '"' {
                    argument.extend(std::iter::repeat_n('\\', backslashes / 2));
                    if backslashes % 2 == 1 {
                        argument.push('"');
                    } else {
                        quoted = !quoted;
                    }
                    index += 1;
                } else {
                    argument.extend(std::iter::repeat_n('\\', backslashes));
                }
                continue;
            }
            if character == '"' {
                quoted = !quoted;
                index += 1;
                continue;
            }
            argument.push(character);
            index += 1;
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
                let run = state.load(&entry.slug).last_run;
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
        for entry in &scan.entries {
            let row = format!("{}\t{}\t{}", entry.name, entry.kind, entry.description);
            writeln!(output, "{row}")?;
        }
        let stderr = io::stderr();
        let mut errors = stderr.lock();
        for diagnostic in &scan.diagnostics {
            let warning = format_text(active_locale(), "warning: {}", &[&diagnostic.message]);
            writeln!(errors, "{warning}")?;
        }
    }
    Ok(())
}

fn show(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    selector: &str,
    json: bool,
) -> Result<(), CliError> {
    let entry = service.show(selector)?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if json {
        let settings = effective_settings(store, &entry);
        let source = show_source_text(store, &entry)?;
        let declarations = form_params(entry.meta.kind.as_str(), &source, &settings);
        let state =
            FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).load(&entry.slug);
        let parameter_source = parameter_source(entry.meta.kind.as_str(), &source, &declarations);
        let fields = declarations.iter().map(field_json).collect::<Vec<_>>();
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
            "degraded_reason": declarations.iter().any(|item| item.degraded)
                .then_some("dynamic"),
            "drift": doctor_entry_drifted(store, &entry),
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
        let settings = effective_settings(store, &entry);
        let source = show_source_text(store, &entry)?;
        let declarations = form_params(entry.meta.kind.as_str(), &source, &settings);
        let state =
            FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).load(&entry.slug);
        writeln!(output, "{} ({})", entry.meta.name, entry.slug)?;
        let kind = format_text(active_locale(), "Kind: {}", &[&entry.meta.kind]);
        writeln!(output, "{kind}")?;
        let mode = format_text(
            active_locale(),
            "Storage mode: {}",
            &[&mode_name(entry.meta.mode)],
        );
        writeln!(output, "{mode}")?;
        if !entry.meta.description.is_empty() {
            writeln!(output, "{}", entry.meta.description)?;
        }
        humanln!("Source: {}", entry.meta.source);
        humanln!("Work directory: {}", entry.meta.workdir);
        humanln!(
            "Missing: {}",
            text(
                active_locale(),
                if entry_missing(store, &entry) {
                    "yes"
                } else {
                    "no"
                }
            )
        );
        humanln!(
            "Drift: {}",
            text(
                active_locale(),
                if doctor_entry_drifted(store, &entry) {
                    "yes"
                } else {
                    "no"
                }
            )
        );
        if !settings.interpreter.is_empty() {
            humanln!("Interpreter: {}", settings.interpreter);
        }
        if !settings.dependencies.is_empty() {
            humanln!("Dependencies: {}", settings.dependencies.join(", "));
        }
        if !settings.requires_python.is_empty() {
            humanln!("Python constraint: {}", settings.requires_python);
        }
        if !settings.needs.is_empty() {
            humanln!("Required commands: {}", settings.needs.join(", "));
        }
        if !settings.template.is_empty() {
            humanln!("Template: {}", settings.template);
        }
        if entry.meta.kind.as_str() == "prompt" {
            humanln!(
                "Prompt runner: {}",
                if settings.runner.is_empty() {
                    text(active_locale(), "not set").to_owned()
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
        if !declarations.is_empty() {
            humanln!("Parameters:");
            for field in &declarations {
                humanln!(
                    "  {} ({}, {})",
                    field.name,
                    field.parameter_type.as_str(),
                    field.delivery.as_str()
                );
            }
        }
        if !state.presets.is_empty() {
            humanln!(
                "Presets: {}",
                state.presets.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }
        humanln!("Run: skit run {}", entry.slug);
    }
    Ok(())
}

fn show_source_text(store: &FileStore, entry: &Entry) -> Result<String, CliError> {
    if entry.meta.kind.as_str() == "command" || entry.meta.kind.as_str() == "exe" {
        return Ok(String::new());
    }
    if entry.meta.kind.as_str() == "prompt" {
        let path = if entry.meta.mode == StorageMode::Copy {
            store
                .entry_dir_path(&entry.slug)
                .join(stored_filename("prompt").expect("prompt has a stored file name"))
        } else {
            PathBuf::from(&entry.meta.source)
        };
        let bytes = fs::read(&path).map_err(|error| source_error("read", &path, error))?;
        return String::from_utf8(bytes).map_err(|_| CliError::SourceEncoding {
            path: path.display().to_string(),
        });
    }
    let Some(path) = source_path(store, entry) else {
        return Ok(String::new());
    };
    Ok(fs::read(&path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default())
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn effective_settings(store: &FileStore, entry: &Entry) -> EntrySettings {
    let mut settings = EntrySettings::from_meta(&entry.meta);
    if entry.meta.kind.as_str() == "python"
        && entry.meta.mode == StorageMode::Copy
        && let Some(metadata) = source_path(store, entry)
            .and_then(|path| fs::read_to_string(path).ok())
            .as_deref()
            .and_then(read_uv_metadata)
    {
        settings.dependencies = metadata.dependencies;
        settings.requires_python = metadata.requires_python;
    }
    settings
}

fn parameter_source(kind: &str, source: &str, declarations: &[ParamDecl]) -> &'static str {
    if matches!(kind, "command" | "prompt") {
        "command"
    } else if declarations
        .iter()
        .any(|item| item.binding != skit_domain::parameters::ParameterBinding::None)
    {
        "inject"
    } else if !cli_params(kind, source).is_empty() {
        "argparse"
    } else if declarations.is_empty() {
        "none"
    } else {
        "declared"
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

fn field_json(item: &ParamDecl) -> serde_json::Value {
    serde_json::json!({
        "key": item.name,
        "label": if item.prompt.is_empty() { &item.name } else { &item.prompt },
        "type": item.parameter_type.as_str(),
        "source": item.delivery.as_str(),
        "required": item.required,
        "secret": item.secret,
        "multiple": item.multiple,
        "repeat": item.repeat,
        "degraded": item.degraded,
        "choices": item.choices,
        "default": item.default,
        "help": item.help,
        "flag": item.flag,
        "action": item.action,
        "env_source": item.env_source,
        "delivers_empty": item.default.is_some()
            && !item.secret
            && !item.multiple
            && matches!(item.parameter_type, ParameterType::Str | ParameterType::Path)
            && matches!(item.delivery, ParameterDelivery::Inject | ParameterDelivery::Flag | ParameterDelivery::Env),
    })
}

#[derive(Debug)]
struct AddOptions {
    source: Option<PathBuf>,
    kind: Option<String>,
    name: Option<String>,
    description: String,
    reference: bool,
    command_template: Option<String>,
    prompt: bool,
    executable: bool,
    runner: Option<String>,
    no_interpolate: bool,
    dependencies: Vec<String>,
    dependencies_explicit: bool,
    requires_python: Option<String>,
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
            description,
            payload: None,
            settings,
        })?;
        humanln!("Added: {} ({})", entry.meta.name, entry.slug);
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
    let (source, source_record, mut bytes, permissions) = if input == Path::new("-") {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        (
            PathBuf::from("stdin"),
            String::new(),
            bytes,
            SourcePermissions::default(),
        )
    } else {
        let source =
            fs::canonicalize(input).map_err(|error| source_error("resolve", input, error))?;
        let (bytes, permissions) = read_source(&source)?;
        let source_record = source.display().to_string();
        (source, source_record, bytes, permissions)
    };
    let name = name.unwrap_or_else(|| source_default_name(&source));
    let mut source_text = String::from_utf8_lossy(&bytes).into_owned();
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
    let kind = kind.as_deref().or(inferred).ok_or_else(|| {
        CliError::Usage(Message::new(
            "could not infer the entry kind; pass --kind KIND",
        ))
    })?;
    let kind =
        EntryKind::parse(kind.to_owned()).map_err(|error| RepositoryError::InvalidMutation {
            reason: error.message(),
        })?;
    let kind_name = kind.as_str().to_owned();
    if no_interpolate && kind_name != "prompt" {
        return Err(CliError::Usage(Message::new(
            "--no-interpolate only applies to prompt entries",
        )));
    }
    let uv_metadata = (kind_name == "python")
        .then(|| read_uv_metadata(&source_text))
        .flatten();
    if !dependencies_explicit {
        if let Some(metadata) = &uv_metadata
            && !metadata.dependencies.is_empty()
        {
            dependencies = metadata.dependencies.clone();
        } else if dependencies.is_empty() {
            let source_dir = (!source_record.is_empty())
                .then(|| source.parent())
                .flatten();
            dependencies = external_dependencies_at(&kind_name, &source_text, source_dir);
        }
    }
    if !requires_python_explicit {
        requires_python = uv_metadata
            .as_ref()
            .map(|metadata| metadata.requires_python.clone())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                shebang
                    .and_then(shebang_program)
                    .and_then(python_version_pin)
            });
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
    let stored_name = stored_name(&kind_name, &source);
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
    let mut metadata_dependencies = dependencies;
    let mut metadata_requires_python = requires_python.unwrap_or_default();
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
        source_text = String::from_utf8_lossy(&bytes).into_owned();
        metadata_dependencies.clear();
        metadata_requires_python.clear();
    }
    let payload = Some(EntryPayload {
        bytes,
        stored_name: Some(stored_name),
        permissions,
    });
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
    let entry = service.add(CreateEntry {
        name,
        kind,
        mode,
        source: source_record,
        workdir: if mode == StorageMode::Reference {
            "origin"
        } else {
            "invoke"
        }
        .to_owned(),
        description,
        payload,
        settings,
    })?;
    humanln!("Added: {} ({})", entry.meta.name, entry.slug);
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
                return Err(CliError::Usage(
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
                    description: String::new(),
                    reference: false,
                    command_template: None,
                    prompt: false,
                    executable: false,
                    runner: None,
                    no_interpolate: false,
                    dependencies: Vec::new(),
                    dependencies_explicit: false,
                    requires_python: None,
                },
                true,
                false,
            );
        }
        Err(error) => return Err(error.into()),
    };
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
    let configured = FileConfigStore::new(resolve_config_dir()?)
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
    let mut held = service.show(&args.selector)?;
    let mut settings = EntrySettings::from_meta(&held.meta);
    let kind = held.meta.kind.as_str().to_owned();
    let had_legacy_package_metadata =
        !settings.dependencies.is_empty() || !settings.requires_python.is_empty();
    let package_change =
        !args.dependencies.is_empty() || args.clear || args.requires_python.is_some();
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
        && !args.clear
    {
        return Err(CliError::Usage(Message::new(
            "managed dependencies require copy storage",
        )));
    }
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
    let mut effective_dependencies = settings.dependencies.clone();
    let mut effective_python = settings.requires_python.clone();
    let python_copy = kind == "python" && held.meta.mode == StorageMode::Copy;
    let source = python_copy
        .then(|| source_path(store, &held))
        .flatten()
        .and_then(|path| fs::read_to_string(path).ok());
    if let Some(metadata) = source.as_deref().and_then(read_uv_metadata) {
        effective_dependencies = metadata.dependencies;
        effective_python = metadata.requires_python;
    }
    if args.clear {
        effective_dependencies.clear();
    } else if !args.dependencies.is_empty() {
        effective_dependencies = args
            .dependencies
            .iter()
            .map(|item| item.trim().to_owned())
            .filter(|item| !item.is_empty())
            .collect();
    }
    if let Some(version) = &args.requires_python {
        effective_python = if matches!(version.trim(), "-" | "none") {
            String::new()
        } else {
            version.trim().to_owned()
        };
    }
    if package_change && kind == "python" {
        for requirement in &effective_dependencies {
            validate_pep508_requirement(requirement)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
        if !effective_python.is_empty() {
            validate_pep440_specifiers(&effective_python)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
    }
    if package_change && python_copy {
        let source = source.ok_or_else(|| {
            CliError::Usage(Message::new("the Python stored copy is not readable UTF-8"))
        })?;
        let rewritten = write_uv_metadata(&source, &effective_dependencies, &effective_python)
            .map_err(|error| CliError::Usage(error.message()))?;
        if rewritten != source {
            let claimed = service.claim_identity(&held)?;
            held =
                service.commit_copy_edit(&claimed, rewritten.as_bytes(), &held.meta.source_hash)?;
        }
        settings.dependencies.clear();
        settings.requires_python.clear();
    } else if package_change {
        settings.dependencies = effective_dependencies.clone();
        settings.requires_python = effective_python.clone();
    }
    let needs_changed = !args.needs.is_empty() || args.clear_needs;
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
    let metadata_changed = needs_changed
        || (package_change && !python_copy)
        || (package_change && python_copy && had_legacy_package_metadata);
    if metadata_changed {
        let claimed = service.claim_identity(&held)?;
        held = service.update_settings(&claimed, &settings, &held.meta.workdir)?;
    }
    if package_change && matches!(kind.as_str(), "js" | "ts") && effective_dependencies.is_empty() {
        clear_javascript_dependencies(&store.entry_dir_path(&held.slug))?;
    }
    let mut output = EntrySettings::from_meta(&held.meta);
    output.dependencies = effective_dependencies;
    output.requires_python = effective_python;
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
) -> Result<String, CliError> {
    if !resync && manage.is_empty() && unmanage.is_empty() && normalize.is_empty() {
        return Ok(source);
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
    let rewritten = write_managed_params(kind, &source, &managed)
        .map_err(|error| CliError::Usage(error.message()))?;
    Ok(rewritten)
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
    let mut source = prepare_source_management(
        held.meta.kind.as_str(),
        held.meta.mode,
        original_source.clone(),
        args.resync,
        &args.manage,
        &args.unmanage,
        &args.normalize,
    )?;
    let mut settings = EntrySettings::from_meta(&held.meta);
    let mut declarations = form_params(held.meta.kind.as_str(), &source, &settings);
    for item in &settings.parameters {
        if !declarations.iter().any(|current| current.name == item.name) {
            declarations.push(item.clone());
        }
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
                    text(active_locale(), "not set").to_owned()
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
    /// Remove one raw row by its one-based index.
    Row(usize),
}

impl RunnerSelection {
    fn label(&self) -> String {
        match self {
            Self::Name(name) => name.clone(),
            Self::Row(row) => format!("row {row}"),
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

fn assignment<'a>(value: &'a str, field: &str) -> Result<(&'a str, &'a str), CliError> {
    value
        .split_once('=')
        .filter(|(name, _)| !name.is_empty())
        .ok_or_else(|| CliError::Usage(Message::new("{} needs NAME=VALUE").with(field)))
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
            store.set(key, value)?;
            if json {
                println!("{}", serde_json::json!({"key": key, "value": value}));
            } else {
                humanln!("Set: {}={}", key, value);
            }
        }
        (Some(key), None) => {
            let value = store.get(key)?;
            if json {
                println!("{}", serde_json::json!({"key": key, "value": value}));
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

fn runner(command: RunnerCommand) -> Result<(), CliError> {
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
                    for row in rows {
                        let status = row.reason.as_deref().unwrap_or("valid");
                        println!("{}\t{}\t{}", row.index, row.descriptor, status);
                    }
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
                for runner in runners {
                    println!("{}\t{}", runner.name, runner.argv.join(" "));
                }
            }
        }
        RunnerCommand::Add { name, argv, force } => {
            store.set_runner(
                PromptRunner {
                    name: name.clone(),
                    argv,
                },
                force,
            )?;
            humanln!("Added runner: {}", name);
        }
        RunnerCommand::Remove {
            name,
            row,
            yes,
            no_input,
        } => {
            let selection = match (name.as_deref(), row) {
                (Some(name), None) => RunnerSelection::Name(name.to_owned()),
                (None, Some(row)) => RunnerSelection::Row(row),
                _ => {
                    return Err(CliError::Usage(Message::new(
                        "runner remove needs a name or --row INDEX",
                    )));
                }
            };
            let target = selection.label();
            if !yes {
                if no_input || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                    return Err(CliError::ConfirmationRequiredFor("runner removal"));
                }
                let question =
                    format_text(active_locale(), "Remove runner \"{}\"? [y/N]: ", &[&target]);
                if !prompt_confirmation(&question, false)? {
                    return Err(CliError::Aborted);
                }
            }
            let removed = match selection {
                RunnerSelection::Name(name) => store.remove_runner(&name)?,
                RunnerSelection::Row(row) => store.remove_runner_row(row)?,
            };
            if !removed {
                return Err(CliError::Usage(
                    Message::new("unknown prompt runner: {}").with(target),
                ));
            }
            humanln!("Removed runner: {}", target);
        }
    }
    Ok(())
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
            refuse_empty_preset_schema(&declarations)?;
            let current = state.load(&entry.slug);
            let values = if from_last {
                &current.last_run.values
            } else {
                &current.values
            };
            state.save_preset(&entry.slug, &name, &declarations, values)?;
            humanln!("Saved preset: {}", name);
        }
        PresetCommand::List { selector, json } => {
            let entry = service.show(&selector)?;
            let presets = state.load(&entry.slug).presets;
            if json {
                println!("{}", serde_json::json!({"presets": presets}));
            } else {
                for name in presets.keys() {
                    println!("{name}");
                }
            }
        }
        PresetCommand::Delete {
            selector,
            name,
            yes,
            no_input,
        } => {
            if !yes {
                if no_input || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                    return Err(CliError::ConfirmationRequiredFor("preset deletion"));
                }
                let question =
                    format_text(active_locale(), "Delete preset \"{}\"? [y/N]: ", &[&name]);
                if !prompt_confirmation(&question, false)? {
                    return Err(CliError::Aborted);
                }
            }
            let entry = service.show(&selector)?;
            if !state.delete_preset(&entry.slug, &name)? {
                return Err(CliError::Usage(
                    Message::new("unknown preset: {}").with(name),
                ));
            }
            humanln!("Deleted preset: {}", name);
        }
    }
    Ok(())
}

fn doctor(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    json: bool,
    rebuild: bool,
) -> Result<i32, CliError> {
    let before = service.list()?;
    let rebuild_problems = if rebuild {
        before
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let rebuilt_entries = rebuild.then(|| store.rebuild_registry()).transpose()?;
    let scan = service.list()?;
    let state_location = resolve_state_dir()?;
    let config_location = resolve_config_dir()?;
    let config = FileConfigStore::new(&config_location);
    let probe = SystemProbe;
    let private_uv = managed_uv_path(store.data_dir());
    let uv = probe
        .find_program("uv")
        .or_else(|| probe.is_executable(&private_uv).then_some(private_uv));
    let entries = scan
        .entries
        .iter()
        .filter_map(|summary| service.show(summary.slug.as_str()).ok())
        .collect::<Vec<_>>();
    let missing = scan
        .entries
        .iter()
        .filter(|entry| summary_missing(store, entry))
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    let drift = entries
        .iter()
        .filter(|entry| doctor_entry_drifted(store, entry))
        .map(|entry| entry.meta.name.clone())
        .collect::<Vec<_>>();
    let mut needs_missing = BTreeMap::<String, Vec<String>>::new();
    let mut launch_blocked = BTreeMap::<String, String>::new();
    for entry in &entries {
        let mut settings = EntrySettings::from_meta(&entry.meta);
        if entry.meta.kind.as_str() == "python"
            && settings.interpreter.is_empty()
            && let Some(path) = &uv
        {
            settings.interpreter = path.display().to_string();
        }
        let absent = settings
            .needs
            .iter()
            .filter(|name| probe.find_program(name).is_none())
            .cloned()
            .collect::<Vec<_>>();
        if !absent.is_empty() {
            needs_missing.insert(entry.meta.name.clone(), absent);
        } else if !missing.contains(&entry.meta.name)
            && let Some(reason) = doctor_launch_block(entry, &settings, &config, &probe)?
        {
            launch_blocked.insert(entry.meta.name.clone(), reason);
        }
    }
    let bad_runners = config.invalid_runner_rows()?;
    let mirror = config.mirror()?;
    let scripts = store.data_dir().join("scripts");
    let size = directory_size(&scripts);
    let uv_required = entries.is_empty()
        || entries
            .iter()
            .any(|entry| entry.meta.kind.as_str() == "python");
    let code = if uv.is_some() || !uv_required { 0 } else { 1 };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "uv": uv,
                "entries": scan.entries.len(),
                "missing": missing,
                "drift": drift,
                "needs_missing": needs_missing,
                "launch_blocked": launch_blocked,
                "runner_rows_invalid": bad_runners,
                "rebuilt": rebuilt_entries,
                "rebuild_problems": rebuild_problems,
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
                "diagnostics": scan.diagnostics,
            })
        );
    } else {
        match uv {
            Some(path) => humanln!("OK uv: {}", path.display()),
            None => humanln!("ERROR uv: not found"),
        }
        humanln!("Entries: {}", scan.entries.len());
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
        for problem in rebuild_problems {
            humanln!("WARN {}", problem);
        }
    }
    Ok(code)
}

fn doctor_entry_drifted(store: &FileStore, entry: &Entry) -> bool {
    let Some(path) = source_path(store, entry) else {
        return false;
    };
    let Ok(source) = fs::read_to_string(path) else {
        return false;
    };
    let settings = EntrySettings::from_meta(&entry.meta);
    if entry.meta.kind.as_str() == "prompt" {
        if !settings.interpolate {
            return false;
        }
        let fresh = placeholder_params("prompt", &source)
            .into_iter()
            .map(|parameter| parameter.name)
            .collect::<Vec<_>>();
        return settings.params.iter().any(|name| !fresh.contains(name));
    }
    let managed = managed_params(entry.meta.kind.as_str(), &source);
    if managed.is_empty() {
        return false;
    }
    let detected = detect_candidates(entry.meta.kind.as_str(), &source);
    managed.iter().any(|parameter| {
        !detected.iter().any(|candidate| {
            candidate.name == parameter.name && candidate.binding == parameter.binding
        })
    })
}

fn doctor_launch_block<P: ProgramProbe>(
    entry: &Entry,
    settings: &EntrySettings,
    config: &FileConfigStore,
    probe: &P,
) -> Result<Option<String>, CliError> {
    if !matches!(entry.meta.workdir.as_str(), "invoke" | "store" | "origin") {
        let path = Path::new(&entry.meta.workdir);
        if !path.is_absolute() {
            return Ok(Some(
                "the custom working directory is not absolute".to_owned(),
            ));
        }
        if !probe.is_dir(path) {
            return Ok(Some(format!(
                "working directory does not exist: {}",
                path.display()
            )));
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
            Err(error) => return Ok(Some(error.to_string())),
        },
        "prompt" if !settings.runner.is_empty() => {
            let runner = config
                .runners()?
                .into_iter()
                .find(|runner| runner.name == settings.runner);
            let Some(runner) = runner else {
                return Ok(Some(format!(
                    "prompt runner is not configured: {}",
                    settings.runner
                )));
            };
            runner.argv.first().cloned()
        }
        "prompt" | "exe" => None,
        kind => return Ok(Some(format!("unknown entry kind: {kind}"))),
    };
    Ok(required.and_then(|name| {
        probe
            .find_program(&name)
            .is_none()
            .then(|| format!("required program was not found: {name}"))
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
    if metadata.is_file() || metadata.file_type().is_symlink() {
        return metadata.len();
    }
    fs::read_dir(path).map_or(0, |items| {
        items
            .filter_map(Result::ok)
            .map(|item| directory_size(&item.path()))
            .fold(0_u64, u64::saturating_add)
    })
}

fn agent(command: AgentCommand) -> Result<(), CliError> {
    match command {
        AgentCommand::Install {
            target,
            directory,
            project,
        } => {
            let path = if let Some(directory) = directory {
                directory.join("skit").join("SKILL.md")
            } else {
                let target = match target {
                    Some(target) => target,
                    None => detect_agent_target(project)?,
                };
                agent_root(&target, project)?
                    .join("skills")
                    .join("skit")
                    .join("SKILL.md")
            };
            let parent = path.parent().expect("each Agent Skill path has a parent");
            fs::create_dir_all(parent).map_err(|error| source_error("create", parent, error))?;
            fs::write(&path, include_bytes!("../../../skills/skit/SKILL.md"))
                .map_err(|error| source_error("write", &path, error))?;
            humanln!("Installed Agent Skill: {}", path.display());
        }
    }
    Ok(())
}

fn detect_agent_target(project: bool) -> Result<String, CliError> {
    let base = if project {
        env::current_dir().map_err(CliError::Io)?
    } else {
        env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            CliError::Usage(Message::new("could not determine the user directory"))
        })?
    };
    let detected = [
        ("claude", ".claude"),
        ("codex", ".codex"),
        ("agents", ".agents"),
    ]
    .into_iter()
    .filter_map(|(target, directory)| base.join(directory).is_dir().then_some(target))
    .collect::<Vec<_>>();
    match detected.as_slice() {
        [target] => Ok((*target).to_owned()),
        [] => Err(CliError::Usage(Message::new(
            "select an agent convention or use --to; no agent directory exists",
        ))),
        _ => Err(CliError::Usage(Message::new(
            "select an agent convention or use --to; more than one agent directory exists",
        ))),
    }
}

fn agent_root(target: &str, project: bool) -> Result<PathBuf, CliError> {
    if project {
        let current = env::current_dir().map_err(CliError::Io)?;
        return match target {
            "claude" => Ok(current.join(".claude")),
            "codex" => Ok(current.join(".codex")),
            "agents" => Ok(current.join(".agents")),
            _ => Err(CliError::Usage(
                Message::new("unknown agent convention: {}").with(target),
            )),
        };
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::Usage(Message::new("could not determine the user directory")))?;
    match target {
        "claude" => Ok(home.join(".claude")),
        "codex" => Ok(home.join(".codex")),
        "agents" => Ok(home.join(".agents")),
        _ => Err(CliError::Usage(
            Message::new("unknown agent convention: {}").with(target),
        )),
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
    match entry.meta.kind.as_str() {
        "command" => false,
        _ => source_path(store, entry).is_none_or(|path| !path.is_file()),
    }
}

fn summary_missing(store: &FileStore, entry: &EntrySummary) -> bool {
    if entry.kind.as_str() == "command" {
        return false;
    }
    if let Some(target) = &entry.target {
        return !Path::new(target).is_file();
    }
    store
        .resolve(entry.slug.as_str())
        .ok()
        .and_then(|resolved| store.payload_path(&resolved).ok())
        .is_none_or(|path| !path.is_file())
}

fn tui(service: &LibraryService<FileStore>) -> Result<(), CliError> {
    let state = LibraryState::from_scan(service.list()?);
    let store = service.repository();
    let state_dir = resolve_state_dir()?;
    let config_dir = resolve_config_dir()?;
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
        UiEffect::Reload => Ok(UiAction::Replace(service.list()?)),
        UiEffect::Open { request, selector } => Ok(UiAction::Present(tui_open(
            service, store, state_dir, config_dir, request, selector,
        )?)),
        UiEffect::Edit { selector } => {
            edit_with_config(service, store, config_dir, &selector, true)?;
            Ok(tui_complete(service, "Source saved")?)
        }
        UiEffect::Remove { selector } => {
            remove(service, &selector, true, true)?;
            Ok(tui_complete(service, "Entry removed")?)
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
            let declarations = entry_parameters(store, &entry);
            let saved = FormStateService::new(FileFormStateStore::new(state_dir)).load(&entry.slug);
            let settings = EntrySettings::from_meta(&entry.meta);
            let mut runners = FileConfigStore::new(config_dir)
                .runners()?
                .into_iter()
                .map(|runner| runner.name)
                .collect::<Vec<_>>();
            if !settings.runner.is_empty() {
                runners.retain(|name| name != &settings.runner);
                runners.insert(0, settings.runner);
            }
            Ok(tui_run_form(
                &entry,
                &declarations,
                &saved.values,
                &runners,
                &saved.presets.keys().cloned().collect::<Vec<_>>(),
            ))
        }
        HostRequest::Add => Ok(tui_add_form()),
        HostRequest::Settings => {
            let entry = service.show(tui_selector(&selector)?)?;
            Ok(tui_settings_form(store, &entry))
        }
        HostRequest::Preferences => tui_preferences_form(config_dir),
        HostRequest::Health => tui_health_report(service, store),
        HostRequest::Runners => tui_runners_form(config_dir),
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

fn tui_run_form(
    entry: &Entry,
    declarations: &[ParamDecl],
    saved: &BTreeMap<String, String>,
    runners: &[String],
    presets: &[String],
) -> Screen {
    Screen::Form(tui_run_form_view(
        entry,
        declarations,
        saved,
        runners,
        presets,
    ))
}

fn tui_run_form_view(
    entry: &Entry,
    declarations: &[ParamDecl],
    saved: &BTreeMap<String, String>,
    runners: &[String],
    presets: &[String],
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
    fields.extend([
        tui_options_field("_skit_preset", "Preset", "Preset choices: {}", presets, ""),
        FormField::text("_skit_save_preset", "Save as preset", ""),
        tui_options_field(
            "_skit_runner",
            "Prompt runner",
            "Prompt runner choices: {}",
            runners,
            runners.first().cloned().unwrap_or_default(),
        ),
        FormField::text("_skit_args", "Extra arguments", ""),
        FormField::text("_skit_dry_run", "Dry run (true or false)", "false"),
    ]);
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

fn tui_add_form() -> Screen {
    Screen::Form(tui_add_form_view())
}

fn tui_add_form_view() -> FormView {
    FormView {
        purpose: FormPurpose::Add,
        title: "Add an entry".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: None,
        fields: vec![
            FormField::text("source", "Source path", ""),
            FormField::text("name", "Name", ""),
            FormField::text("kind", "Kind", ""),
            FormField::multiline("description", "Description", ""),
            FormField::text("mode", "Storage mode (copy or reference)", "copy"),
            FormField::multiline("template", "Command template", ""),
            FormField::text("runner", "Prompt runner", ""),
            FormField::text("dependencies", "Package dependencies", ""),
            FormField::text("python", "Python constraint", ""),
        ],
        focused: 0,
        submit_label: "Add".to_owned(),
    }
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

fn tui_preferences_form(config_dir: &Path) -> Result<Screen, CliError> {
    let settings = FileConfigStore::new(config_dir).settings()?;
    Ok(Screen::Form(FormView {
        purpose: FormPurpose::Preferences,
        title: "Preferences".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: None,
        fields: settings
            .into_iter()
            .map(|(key, value)| tui_preference_field(key, value))
            .collect(),
        focused: 0,
        submit_label: "Save".to_owned(),
    }))
}

fn tui_preference_field(key: String, value: String) -> FormField {
    let label = match key.as_str() {
        "lang" => "Language",
        "editor" => "Editor command",
        "form" => "Form style",
        "after_run" => "After run",
        "shell.bash_path" => "Bash path",
        "js.runner" => "JavaScript runtime",
        "mirror" => "Mirror",
        "mirror.pypi" => "PyPI mirror",
        "mirror.github" => "GitHub mirror",
        "mirror.npm" => "npm mirror",
        _ => return FormField::text_raw(key.clone(), key, value),
    };
    FormField::text(key, label, value)
}

fn tui_health_report(
    service: &LibraryService<FileStore>,
    store: &FileStore,
) -> Result<Screen, CliError> {
    let scan = service.list()?;
    let missing = scan
        .entries
        .iter()
        .filter(|entry| summary_missing(store, entry))
        .count();
    let mut items = vec![
        ReportItem {
            status: "ok".to_owned(),
            label: "Entries".to_owned(),
            translate_label: true,
            detail: scan.entries.len().to_string(),
            translate_detail: false,
        },
        ReportItem {
            status: if missing == 0 { "ok" } else { "error" }.to_owned(),
            label: "Missing targets".to_owned(),
            translate_label: true,
            detail: missing.to_string(),
            translate_detail: false,
        },
        ReportItem {
            status: "ok".to_owned(),
            label: "Data directory".to_owned(),
            translate_label: true,
            detail: store.data_dir().display().to_string(),
            translate_detail: false,
        },
    ];
    items.extend(scan.diagnostics.into_iter().map(|diagnostic| ReportItem {
        status: "error".to_owned(),
        label: diagnostic.slug.unwrap_or_else(|| "Library".to_owned()),
        translate_label: false,
        detail: diagnostic.message,
        translate_detail: false,
    }));
    Ok(Screen::Report(ReportView {
        title: "Health".to_owned(),
        items,
    }))
}

fn tui_runners_form(config_dir: &Path) -> Result<Screen, CliError> {
    let runners = FileConfigStore::new(config_dir)
        .runners()?
        .into_iter()
        .map(|runner| format!("{}={}", runner.name, runner.argv.join(" ")))
        .collect::<Vec<_>>();
    Ok(Screen::Form(FormView {
        purpose: FormPurpose::Runners,
        title: "Prompt runners: {}".to_owned(),
        title_arguments: vec![runners.join("; ")],
        translate_title: true,
        selector: None,
        fields: vec![
            FormField::text("name", "Runner name", ""),
            FormField::text("argv", "Arguments", ""),
            FormField::text("remove", "Remove (true or false)", "false"),
        ],
        focused: 0,
        submit_label: "Save".to_owned(),
    }))
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
        ParameterValue::Float(value) => value.to_string(),
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
                    description: tui_value(values, "description").to_owned(),
                    reference: tui_value(values, "mode").eq_ignore_ascii_case("reference"),
                    command_template: (!template.is_empty()).then_some(template.to_owned()),
                    prompt: kind == "prompt",
                    executable: kind == "exe",
                    runner: tui_nonempty_owned(values, "runner"),
                    no_interpolate: false,
                    dependencies: tui_dependency_list(tui_value(values, "dependencies")),
                    dependencies_explicit: !tui_value(values, "dependencies").is_empty(),
                    requires_python: tui_nonempty_owned(values, "python"),
                },
            )?;
            tui_complete(service, "Entry added")
        }
        FormPurpose::Settings => {
            tui_submit_settings(service, store, state_dir, tui_selector(&selector)?, values)?;
            tui_complete(service, "Settings saved")
        }
        FormPurpose::Preferences => {
            let config = FileConfigStore::new(config_dir);
            config.set_many(values)?;
            tui_complete(service, "Preferences saved")
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
            tui_complete(service, "Prompt runners saved")
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
            tui_complete(service, "Presets saved")
        }
        FormPurpose::Rename => {
            rename(
                service,
                tui_selector(&selector)?,
                tui_required(values, "name")?,
            )?;
            tui_complete(service, "Entry renamed")
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
        tui_complete(service, &format!("Run finished with exit status {exit}"))
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
    let mut settings = effective_settings(store, &entry);
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
    settings.dependencies = tui_dependency_list(tui_value(values, "dependencies"));
    settings.requires_python = tui_value(values, "python").to_owned();
    if !settings.dependencies.is_empty()
        && !matches!(entry.meta.kind.as_str(), "python" | "js" | "ts")
    {
        return Err(CliError::Usage(Message::new(
            "package dependencies apply only to Python and JavaScript entries",
        )));
    }
    if !settings.requires_python.is_empty() && entry.meta.kind.as_str() != "python" {
        return Err(CliError::Usage(Message::new(
            "a Python constraint applies only to Python entries",
        )));
    }
    if entry.meta.kind.as_str() == "python" {
        for requirement in &settings.dependencies {
            validate_pep508_requirement(requirement)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
        if !settings.requires_python.is_empty() {
            validate_pep440_specifiers(&settings.requires_python)
                .map_err(|error| CliError::Usage(error.message()))?;
        }
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
    let original_source = source_path(store, &entry).and_then(|path| fs::read_to_string(path).ok());
    let source_interface_names = original_source
        .as_deref()
        .map_or_else(BTreeSet::new, |source| {
            managed_params(entry.meta.kind.as_str(), source)
                .into_iter()
                .chain(cli_params(entry.meta.kind.as_str(), source))
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
    let mut rewritten_source = original_source.clone();
    if entry.meta.kind.as_str() == "python" && entry.meta.mode == StorageMode::Copy {
        let source = original_source.clone().ok_or_else(|| {
            CliError::Usage(Message::new("the Python stored copy is not valid UTF-8"))
        })?;
        rewritten_source = Some(
            write_uv_metadata(&source, &settings.dependencies, &settings.requires_python)
                .map_err(|error| CliError::Usage(error.message()))?,
        );
        settings.dependencies.clear();
        settings.requires_python.clear();
    }
    let source_requested = tui_bool(tui_value(values, "source:resync"))?
        || !tui_value(values, "source:manage").is_empty()
        || !tui_value(values, "source:unmanage").is_empty()
        || !tui_value(values, "source:normalize").is_empty();
    if let Some(source) = rewritten_source.take() {
        let mut rewritten = prepare_source_management(
            entry.meta.kind.as_str(),
            entry.meta.mode,
            source,
            tui_bool(tui_value(values, "source:resync"))?,
            &tui_split_list(tui_value(values, "source:manage")),
            &tui_split_list(tui_value(values, "source:unmanage")),
            &tui_split_list(tui_value(values, "source:normalize")),
        )?;
        let mut managed = managed_params(entry.meta.kind.as_str(), &rewritten);
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
        rewritten = write_managed_params(entry.meta.kind.as_str(), &rewritten, &managed)
            .map_err(|error| CliError::Usage(error.message()))?;
        rewritten_source = Some(rewritten);
    } else if source_requested {
        return Err(CliError::Usage(Message::new(
            "the stored source is not valid UTF-8",
        )));
    }
    let source = rewritten_source
        .filter(|rewritten| original_source.as_ref() != Some(rewritten))
        .map(String::into_bytes);
    let claimed = service.claim_identity(&entry)?;
    let entry = service.update_entry(
        &claimed,
        UpdateEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            settings: settings.clone(),
            workdir: tui_value(values, "workdir").to_owned(),
            source,
            expected_source_hash: entry.meta.source_hash.clone(),
        },
    )?;
    let state = FormStateService::new(FileFormStateStore::new(state_dir));
    state.purge_secrets(&entry.slug, &declarations)?;
    Ok(())
}

fn tui_complete(service: &LibraryService<FileStore>, message: &str) -> Result<UiAction, CliError> {
    Ok(UiAction::Complete {
        scan: Some(service.list()?),
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

fn fallback_stored_name(source: &Path) -> String {
    source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("script")
        .to_owned()
}

fn stored_name(kind: &str, source: &Path) -> String {
    if matches!(kind, "js" | "ts")
        && let Some(extension) = source.extension().and_then(|value| value.to_str())
        && matches!(extension, "js" | "mjs" | "cjs" | "ts" | "mts" | "cts")
    {
        return format!("script.{extension}");
    }
    stored_filename(kind)
        .map(str::to_owned)
        .unwrap_or_else(|| fallback_stored_name(source))
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
    #[error("confirmation is required; pass --yes to remove the entry")]
    ConfirmationRequired,
    #[error("confirmation is required for {0}; pass --yes")]
    ConfirmationRequiredFor(&'static str),
    #[error("operation cancelled")]
    Aborted,
    #[error("could not {operation} {path}: {source}")]
    Source {
        operation: &'static str,
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("{path} is not valid UTF-8")]
    SourceEncoding { path: String },
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
            Self::ConfirmationRequired => {
                Message::new("confirmation is required; pass --yes to remove the entry")
            }
            Self::ConfirmationRequiredFor(operation) => {
                Message::new("confirmation is required for {}; pass --yes").with(operation)
            }
            Self::Aborted => Message::new("operation cancelled"),
            Self::Source {
                operation,
                path,
                source,
            } => Message::new("could not {} {}: {}")
                .with(operation)
                .with(path)
                .with(source),
            Self::SourceEncoding { path } => Message::new("{} is not valid UTF-8").with(path),
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
            Self::Repository(error) => error.exit_class().code() as i32,
            Self::Run(error) => error.exit_code(),
            Self::Dependencies(_) => ExitClass::Skit.code() as i32,
            Self::ConfirmationRequired | Self::ConfirmationRequiredFor(_) | Self::Usage(_) => {
                ExitClass::Usage.code() as i32
            }
            Self::Aborted => ExitClass::Aborted.code() as i32,
            Self::Json(_)
            | Self::Io(_)
            | Self::Tui(_)
            | Self::Config(_)
            | Self::State(_)
            | Self::Source { .. }
            | Self::SourceEncoding { .. }
            | Self::DataDirectoryUnavailable
            | Self::DirectoryUnavailable(_) => ExitClass::Skit.code() as i32,
        }
    }
}

const fn mode_name(mode: StorageMode) -> &'static str {
    match mode {
        StorageMode::Copy => "copy",
        StorageMode::Reference => "reference",
    }
}
