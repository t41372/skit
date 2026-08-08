use std::{
    collections::BTreeMap,
    env,
    fs::{self, File, Metadata},
    io::{self, IsTerminal as _, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use clap::{Args, CommandFactory as _, Parser, Subcommand};
use clap_complete::{ArgValueCandidates, CompleteEnv, CompletionCandidate, Shell, generate};
use skit_application::{
    CreateEntry, EntryPayload, ExitClass, LibraryService, RepositoryError, SourcePermissions,
    form_state::{FormStateService, StateWriteError, prefill},
};
use skit_domain::{
    Entry, EntryKind, EntrySettings, EntrySummary, StorageMode,
    parameters::{ParamDecl, ParameterDelivery, ParameterType, coerce_default},
};
use skit_form::form_params;
use skit_i18n::{Locale, detect_locale, render as localize};
use skit_language::{
    detect_candidates, external_dependencies, infer_kind, managed_params, normalize_shell_default,
    placeholder_params, read_uv_metadata, write_managed_params, write_uv_metadata,
};
use skit_runtime::{
    DependencyError, ProgramProbe, SystemProbe, clear_javascript_dependencies, managed_uv_path,
    resolve_javascript_runtime,
};
use skit_store::{ConfigError, FileConfigStore, FileFormStateStore, PromptRunner};
use skit_store::{FileStore, stored_filename, stored_filenames};
use skit_ui::{
    Action as UiAction, Effect as UiEffect, FormField, FormPurpose, FormView, HostRequest,
    LibraryState, ReportItem, ReportView, Screen,
};
use thiserror::Error;

use crate::run::{RunArgs, RunError};

#[cfg(test)]
mod tests;

/// Run the command-line entry point and return its process status.
#[must_use]
pub fn entry() -> i32 {
    match CompleteEnv::with_factory(Cli::command)
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
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
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
            eprintln!("{}", localize(locale, &error.to_string()));
            error.exit_code()
        }
    }
}

fn active_locale() -> Locale {
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
        #[arg(long = "cmd", conflicts_with = "source")]
        command_template: Option<String>,
        /// Treat the source as a prompt entry.
        #[arg(long)]
        prompt: bool,
        /// Force executable kind inference.
        #[arg(long)]
        exe: bool,
        /// Pin a prompt runner.
        #[arg(long)]
        runner: Option<String>,
        /// Disable prompt placeholder insertion.
        #[arg(long)]
        no_interpolate: bool,
        /// Add one package dependency. Repeat for more than one value.
        #[arg(long = "dep")]
        dependencies: Vec<String>,
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
        #[arg(long)]
        yes: bool,
    },
    /// Open an entry source in the configured editor.
    Edit {
        /// Entry slug or display name.
        #[arg(add = ArgValueCandidates::new(entry_candidates))]
        selector: String,
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
    /// Set a flag as NAME=--FLAG. An empty flag makes the field positional.
    #[arg(long = "flag")]
    flags: Vec<String>,
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
    #[arg(long)]
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
        name: String,
        /// Confirm removal.
        #[arg(long)]
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
        name: String,
        /// Confirm deletion.
        #[arg(long)]
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

fn execute(cli: Cli) -> Result<i32, CliError> {
    if cli.show_completion {
        write_completion(detect_shell()?, &mut io::stdout());
        return Ok(0);
    }
    if cli.install_completion {
        let shell = detect_shell()?;
        let path = completion_path(shell)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&path)?;
        write_completion(shell, &mut output);
        println!("Installed completion: {}", path.display());
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
                    dependencies,
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
        Some(Command::Remove { selector, yes }) => {
            remove(&service, &selector, yes)?;
            Ok(0)
        }
        Some(Command::Edit { selector }) => {
            edit(&service, &store, &selector)?;
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
        return Err(CliError::Usage(
            "--edit needs an editor; use standard input as `skit add - --name NAME`".to_owned(),
        ));
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
                return Err(CliError::Usage(
                    "a prompt body is required; pipe it to `skit add - --prompt --name NAME`"
                        .to_owned(),
                ));
            }
            return add_draft(service, options, true);
        }
        if no_input || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(CliError::Usage(
                "add needs a source path, standard input as `-`, --edit, --prompt, or --cmd"
                    .to_owned(),
            ));
        }
        let Screen::Form(form) = tui_add_form() else {
            unreachable!("the add screen is a form")
        };
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
                dependencies: tui_split_list(tui_value(&values, "dependencies")),
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
        return Err(CliError::Usage(format!(
            "the draft is empty and was kept at {}",
            draft.display()
        )));
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
        eprintln!("Your draft was kept at {}", draft.display());
    }
    result
}

fn validate_prompt_runner(name: Option<&str>) -> Result<(), CliError> {
    let Some(name) = name else {
        return Ok(());
    };
    let exists = FileConfigStore::new(resolve_config_dir()?)
        .runners()?
        .iter()
        .any(|runner| runner.name == name);
    if exists {
        Ok(())
    } else {
        Err(CliError::Usage(format!(
            "prompt runner {name:?} is not configured"
        )))
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
        _ => Err(CliError::Usage(
            "could not detect the shell; set SHELL before completion setup".to_owned(),
        )),
    }
}

fn completion_path(shell: Shell) -> Result<PathBuf, CliError> {
    let home = user_home().ok_or_else(|| {
        CliError::Usage("could not determine the home directory for completion setup".to_owned())
    })?;
    let path = match shell {
        Shell::Bash => env::var_os("XDG_DATA_HOME").map_or_else(
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
        ),
        Shell::Fish => env::var_os("XDG_CONFIG_HOME")
            .map_or_else(|| home.join(".config"), PathBuf::from)
            .join("fish")
            .join("completions")
            .join("skit.fish"),
        Shell::Zsh => env::var_os("XDG_DATA_HOME")
            .map_or_else(|| home.join(".local").join("share"), PathBuf::from)
            .join("zsh")
            .join("site-functions")
            .join("_skit"),
        Shell::Elvish => env::var_os("XDG_CONFIG_HOME")
            .map_or_else(|| home.join(".config"), PathBuf::from)
            .join("elvish")
            .join("lib")
            .join("skit.elv"),
        Shell::PowerShell => home
            .join("Documents")
            .join("PowerShell")
            .join("Completions")
            .join("_skit.ps1"),
        _ => {
            return Err(CliError::Usage(
                "the detected shell does not have an installation path".to_owned(),
            ));
        }
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

    let form = interactive_run_form(service, store, &args)?;
    let use_plain = args.plain
        || FileConfigStore::new(resolve_config_dir()?)
            .get("form")?
            .eq_ignore_ascii_case("plain");
    let values = if use_plain {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        let stdout = io::stdout();
        let mut output = stdout.lock();
        collect_plain_form(&form, &mut input, &mut output, |_| {
            rpassword::read_password()
        })
        .map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                CliError::Aborted
            } else {
                CliError::Io(error)
            }
        })?
    } else {
        skit_tui::collect_form(form, active_locale())?.ok_or(CliError::Aborted)?
    };
    apply_interactive_run_values(&mut args, &values)?;
    args.no_input = true;
    crate::run::run(service, store, args).map_err(Into::into)
}

fn interactive_run_form(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    args: &RunArgs,
) -> Result<FormView, CliError> {
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
    let Screen::Form(mut form) = tui_run_form(
        &entry,
        &declarations,
        &initial,
        &runners,
        &saved.presets.keys().cloned().collect::<Vec<_>>(),
    ) else {
        unreachable!("the run form builder always returns a form")
    };
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
        &shlex::try_join(args.extra_args.iter().map(String::as_str)).unwrap_or_default(),
    );
    set_form_value(&mut form, "_skit_dry_run", &args.dry_run.to_string());
    Ok(form)
}

fn set_form_value(form: &mut FormView, key: &str, value: &str) {
    if let Some(field) = form.fields.iter_mut().find(|field| field.key == key) {
        field.value = value.to_owned();
    }
}

fn apply_interactive_run_values(
    args: &mut RunArgs,
    values: &BTreeMap<String, String>,
) -> Result<(), CliError> {
    args.values = values
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("value:")
                .filter(|_| !value.is_empty())
                .map(|name| format!("{name}={value}"))
        })
        .collect();
    args.preset = tui_nonempty_owned(values, "_skit_preset");
    args.save_preset = tui_nonempty_owned(values, "_skit_save_preset");
    args.runner = tui_nonempty_owned(values, "_skit_runner");
    args.dry_run = tui_bool(tui_value(values, "_skit_dry_run"));
    args.extra_args = shlex::split(tui_value(values, "_skit_args"))
        .ok_or_else(|| CliError::Usage("extra arguments have invalid quoting".to_owned()))?;
    Ok(())
}

fn collect_plain_form<R, W, F>(
    form: &FormView,
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
        if field.value.is_empty() || field.secret {
            write!(output, "{}: ", field.label)?;
        } else {
            write!(output, "{} [{}]: ", field.label, field.value)?;
        }
        output.flush()?;
        let value = if field.secret {
            read_secret(&field.label)?
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
        let settings = EntrySettings::from_meta(&entry.meta);
        let declarations = entry_parameters(store, &entry);
        let state =
            FormStateService::new(FileFormStateStore::new(resolve_state_dir()?)).load(&entry.slug);
        let parameter_source = parameter_source(entry.meta.kind.as_str(), &declarations);
        let fields = declarations
            .iter()
            .map(|item| field_json(item, parameter_source))
            .collect::<Vec<_>>();
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
            "degraded_reason": serde_json::Value::Null,
            "drift": false,
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
        writeln!(output, "{} ({})", entry.meta.name, entry.slug)?;
        writeln!(output, "kind: {}", entry.meta.kind)?;
        writeln!(output, "mode: {}", mode_name(entry.meta.mode))?;
        if !entry.meta.description.is_empty() {
            writeln!(output, "{}", entry.meta.description)?;
        }
    }
    Ok(())
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn parameter_source(kind: &str, declarations: &[ParamDecl]) -> &'static str {
    if matches!(kind, "command" | "prompt") {
        "command"
    } else if declarations.is_empty() {
        "none"
    } else if declarations
        .iter()
        .any(|item| item.binding != skit_domain::parameters::ParameterBinding::None)
    {
        "inject"
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

fn field_json(item: &ParamDecl, source: &str) -> serde_json::Value {
    serde_json::json!({
        "key": item.name,
        "label": if item.prompt.is_empty() { &item.name } else { &item.prompt },
        "type": item.parameter_type.as_str(),
        "source": source,
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
    requires_python: Option<String>,
}

fn add(service: &LibraryService<FileStore>, options: AddOptions) -> Result<(), CliError> {
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
        requires_python,
    } = options;
    let mut dependencies = dependencies
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if prompt {
        validate_prompt_runner(runner.as_deref())?;
    }

    if let Some(template) = command_template {
        if !dependencies.is_empty() || requires_python.is_some() {
            return Err(CliError::Usage(
                "command entries do not take package dependencies".to_owned(),
            ));
        }
        let kind = EntryKind::parse("command".to_owned()).expect("command kind is valid");
        let entry = service.add(CreateEntry {
            name: name.unwrap_or_else(|| "Command".to_owned()),
            kind,
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description,
            payload: None,
        })?;
        let parameters = placeholder_params("command", &template);
        let settings = EntrySettings {
            params: parameters.iter().map(|item| item.name.clone()).collect(),
            parameters,
            template,
            ..EntrySettings::default()
        };
        let claimed = service.claim_identity(&entry)?;
        let entry = service.update_settings(&claimed, &settings, "invoke")?;
        println!("Added: {} ({})", entry.meta.name, entry.slug);
        return Ok(());
    }

    let input = source
        .as_deref()
        .ok_or_else(|| CliError::Usage("add needs a source path or --cmd COMMAND".to_owned()))?;
    if reference && input == Path::new("-") {
        return Err(CliError::Usage(
            "standard input cannot be a referenced entry".to_owned(),
        ));
    }
    let (source, source_record, bytes, permissions) = if input == Path::new("-") {
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
    let source_text = String::from_utf8_lossy(&bytes).into_owned();
    let shebang = source_text
        .lines()
        .next()
        .filter(|line| line.starts_with("#!"));
    let inferred = if prompt {
        Some("prompt")
    } else {
        infer_kind(&source, shebang, executable)
    };
    let kind = kind.as_deref().or(inferred).ok_or_else(|| {
        CliError::Usage("could not infer the entry kind; pass --kind KIND".to_owned())
    })?;
    let kind =
        EntryKind::parse(kind.to_owned()).map_err(|error| RepositoryError::InvalidMutation {
            reason: error.to_string(),
        })?;
    let kind_name = kind.as_str().to_owned();
    if dependencies.is_empty() {
        dependencies = external_dependencies(&kind_name, &source_text);
    }
    let supports_dependencies = matches!(kind_name.as_str(), "python" | "js" | "ts");
    if !dependencies.is_empty() && !supports_dependencies {
        return Err(CliError::Usage(format!(
            "{kind_name} entries do not take package dependencies"
        )));
    }
    if requires_python.is_some() && kind_name != "python" {
        return Err(CliError::Usage(format!(
            "a Python constraint does not apply to {kind_name} entries"
        )));
    }
    if reference && matches!(kind_name.as_str(), "js" | "ts") && !dependencies.is_empty() {
        return Err(CliError::Usage(
            "reference entries do not take managed dependencies".to_owned(),
        ));
    }
    if runner.is_some() && kind_name != "prompt" {
        return Err(CliError::Usage(
            "--runner only applies to prompt entries".to_owned(),
        ));
    }
    if kind_name == "prompt" {
        validate_prompt_runner(runner.as_deref())?;
    }
    let stored_name = stored_name(&kind_name, &source);
    let mode = if reference {
        StorageMode::Reference
    } else {
        StorageMode::Copy
    };
    let entry = service.add(CreateEntry {
        name,
        kind,
        mode,
        source: source_record,
        workdir: if reference { "origin" } else { "invoke" }.to_owned(),
        description,
        payload: Some(EntryPayload {
            bytes,
            stored_name: Some(stored_name),
            permissions,
        }),
    })?;
    let mut settings = EntrySettings {
        dependencies,
        requires_python: requires_python.unwrap_or_default(),
        runner: runner.unwrap_or_default(),
        interpolate: !no_interpolate,
        ..EntrySettings::default()
    };
    if kind_name == "prompt" && settings.interpolate {
        settings.parameters = placeholder_params("prompt", &source_text);
        settings.params = settings
            .parameters
            .iter()
            .map(|item| item.name.clone())
            .collect();
    }
    let has_settings = kind_name == "prompt"
        || !settings.dependencies.is_empty()
        || !settings.requires_python.is_empty();
    let entry = if has_settings {
        let claimed = service.claim_identity(&entry)?;
        service.update_settings(&claimed, &settings, &entry.meta.workdir)?
    } else {
        entry
    };
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

fn edit(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    selector: &str,
) -> Result<(), CliError> {
    let held = service.show(selector)?;
    let target = source_path(store, &held).ok_or_else(|| {
        CliError::Usage(format!(
            "entry {} does not have an editable source",
            held.slug
        ))
    })?;
    let editor = FileConfigStore::new(resolve_config_dir()?)
        .get("editor")
        .unwrap_or_default();
    let editor = if editor.trim().is_empty() {
        env::var("VISUAL")
            .or_else(|_| env::var("EDITOR"))
            .map_err(|_| CliError::Usage("configure an editor before you use edit".to_owned()))?
    } else {
        editor
    };
    let mut argv = shlex::split(&editor)
        .ok_or_else(|| CliError::Usage("the editor command has invalid quoting".to_owned()))?;
    if argv.is_empty() {
        return Err(CliError::Usage("the editor command is empty".to_owned()));
    }

    if held.meta.mode == StorageMode::Reference {
        let status = ProcessCommand::new(&argv[0])
            .args(&argv[1..])
            .arg(&target)
            .status()
            .map_err(|error| source_error("start editor for", &target, error))?;
        if !status.success() {
            return Err(CliError::Usage(format!(
                "the editor exited with status {}",
                status.code().unwrap_or(1)
            )));
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
        return Err(CliError::Usage(format!(
            "the editor exited with status {}",
            status.code().unwrap_or(1)
        )));
    }
    let edited = fs::read(&staged).map_err(|error| source_error("read", &staged, error))?;
    if edited != original {
        let claimed = service.claim_identity(&held)?;
        service.commit_copy_edit(&claimed, &edited, &held.meta.source_hash)?;
        println!("Edited: {} ({})", held.meta.name, held.slug);
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
            .map_err(|_| CliError::Usage("configure an editor before you use --edit".to_owned()))?
    } else {
        configured
    };
    let mut argv = shlex::split(&editor)
        .ok_or_else(|| CliError::Usage("the editor command has invalid quoting".to_owned()))?;
    if argv.is_empty() {
        return Err(CliError::Usage("the editor command is empty".to_owned()));
    }
    let status = ProcessCommand::new(argv.remove(0))
        .args(argv)
        .arg(target)
        .status()
        .map_err(|error| source_error("start editor for", target, error))?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError::Usage(format!(
            "the editor exited with status {}",
            status.code().unwrap_or(1)
        )))
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
        return Err(CliError::Usage(format!(
            "{} does not take package dependencies; only --need applies",
            held.meta.name
        )));
    }
    if args.requires_python.is_some() && kind != "python" {
        return Err(CliError::Usage(format!(
            "a Python constraint does not apply to {kind} entries"
        )));
    }
    if package_change
        && matches!(kind.as_str(), "js" | "ts")
        && held.meta.mode == StorageMode::Reference
        && !args.clear
    {
        return Err(CliError::Usage(
            "managed dependencies require copy storage".to_owned(),
        ));
    }
    if args.clear && !args.dependencies.is_empty() {
        return Err(CliError::Usage("use --dep or --clear, not both".to_owned()));
    }
    if args.clear_needs && !args.needs.is_empty() {
        return Err(CliError::Usage(
            "use --need or --clear-needs, not both".to_owned(),
        ));
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
    if package_change && python_copy {
        let source = source.ok_or_else(|| {
            CliError::Usage("the Python stored copy is not readable UTF-8".to_owned())
        })?;
        let rewritten = write_uv_metadata(&source, &effective_dependencies, &effective_python)
            .map_err(|error| CliError::Usage(error.to_string()))?;
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
        settings.needs = args.needs;
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
        println!("dependencies: {}", settings.dependencies.join(", "));
        println!("requires_python: {}", settings.requires_python);
        println!("needs: {}", settings.needs.join(", "));
    }
    Ok(())
}

fn params(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    args: ParamsArgs,
) -> Result<(), CliError> {
    let mut held = service.show(&args.selector)?;
    let mut source = source_path(store, &held)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let has_source_operation = args.resync
        || !args.manage.is_empty()
        || !args.unmanage.is_empty()
        || !args.normalize.is_empty();
    let has_other_operation = !args.add.is_empty()
        || !args.remove.is_empty()
        || !args.parameter_types.is_empty()
        || !args.defaults.is_empty()
        || !args.choices.is_empty()
        || !args.delivery.is_empty()
        || !args.flags.is_empty()
        || !args.help_text.is_empty()
        || !args.prompts.is_empty()
        || !args.env_sources.is_empty()
        || !args.required.is_empty()
        || !args.optional.is_empty()
        || !args.secret.is_empty()
        || !args.no_secret.is_empty()
        || args.workdir.is_some()
        || args.template.is_some()
        || args.interpreter.is_some()
        || args.runner.is_some()
        || args.interpolate
        || args.no_interpolate;
    if has_source_operation && has_other_operation {
        return Err(CliError::Usage(
            "source management must be a separate params operation".to_owned(),
        ));
    }
    if has_source_operation {
        if held.meta.mode == StorageMode::Reference {
            return Err(CliError::Usage(
                "source management applies only to a stored copy".to_owned(),
            ));
        }
        let kind = held.meta.kind.as_str();
        let mut managed = managed_params(kind, &source);
        let candidates = detect_candidates(kind, &source);
        if args.resync {
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
        for name in &args.manage {
            if managed.iter().any(|item| item.name == *name) {
                continue;
            }
            let candidate = candidates
                .iter()
                .find(|item| item.name == *name)
                .cloned()
                .ok_or_else(|| CliError::Usage(format!("unknown source parameter: {name}")))?;
            managed.push(candidate);
        }
        if !args.unmanage.is_empty() {
            managed.retain(|item| !args.unmanage.contains(&item.name));
        }
        for name in &args.normalize {
            if kind != "shell" {
                return Err(CliError::Usage(
                    "--normalize applies only to shell entries".to_owned(),
                ));
            }
            source = normalize_shell_default(&source, name)
                .map_err(|error| CliError::Usage(error.to_string()))?;
            let normalized = detect_candidates(kind, &source)
                .into_iter()
                .find(|item| item.name == *name)
                .ok_or_else(|| CliError::Usage(format!("could not normalize {name}")))?;
            if let Some(item) = managed.iter_mut().find(|item| item.name == *name) {
                *item = normalized;
            } else {
                managed.push(normalized);
            }
        }
        let rewritten = write_managed_params(kind, &source, &managed)
            .map_err(|error| CliError::Usage(error.to_string()))?;
        if rewritten != source {
            let claimed = service.claim_identity(&held)?;
            held =
                service.commit_copy_edit(&claimed, rewritten.as_bytes(), &held.meta.source_hash)?;
            source = rewritten;
        }
    }
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
            return Err(CliError::Usage(format!("parameter already exists: {name}")));
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
                .map_err(|error| CliError::Usage(error.to_string()))?,
        );
        changed = true;
    }
    for spec in args.delivery {
        let (name, value) = assignment(&spec, "delivery")?;
        parameter_mut(&mut declarations, name)?.delivery = parse_delivery(value)?;
        changed = true;
    }
    for spec in args.flags {
        let (name, value) = assignment(&spec, "flag")?;
        parameter_mut(&mut declarations, name)?.flag = value.to_owned();
        changed = true;
    }
    for spec in args.help_text {
        let (name, value) = assignment(&spec, "help text")?;
        parameter_mut(&mut declarations, name)?.help = value.to_owned();
        changed = true;
    }
    for spec in args.prompts {
        let (name, value) = assignment(&spec, "prompt")?;
        parameter_mut(&mut declarations, name)?.prompt = value.to_owned();
        changed = true;
    }
    for spec in args.env_sources {
        let (name, value) = assignment(&spec, "environment source")?;
        parameter_mut(&mut declarations, name)?.env_source = value.to_owned();
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
        changed = true;
    }
    if let Some(value) = args.interpreter {
        settings.interpreter = value;
        changed = true;
    }
    if let Some(value) = args.runner {
        settings.runner = value;
        changed = true;
    }
    if args.interpolate || args.no_interpolate {
        settings.interpolate = args.interpolate;
        changed = true;
    }
    if changed {
        settings.parameters = declarations.clone();
        if matches!(held.meta.kind.as_str(), "command" | "prompt") {
            settings.params = declarations
                .iter()
                .filter(|item| item.delivery == ParameterDelivery::Placeholder)
                .map(|item| item.name.clone())
                .collect();
        }
        let claimed = service.claim_identity(&held)?;
        service.update_settings(&claimed, &settings, &workdir)?;
        if !args.secret.is_empty() {
            let state = FormStateService::new(FileFormStateStore::new(resolve_state_dir()?));
            state.purge_secrets(&held.slug, &declarations)?;
        }
    }
    write_params(&declarations, args.json)
}

fn write_params(declarations: &[ParamDecl], json: bool) -> Result<(), CliError> {
    if json {
        let rows = declarations
            .iter()
            .map(|item| serde_json::Value::Object(item.to_meta_map().into_iter().collect()))
            .collect::<Vec<_>>();
        println!("{}", serde_json::json!({"parameters": rows}));
    } else {
        for item in declarations {
            println!(
                "{}\t{}\t{}{}",
                item.name,
                item.parameter_type.as_str(),
                item.delivery.as_str(),
                if item.secret { "\tsecret" } else { "" }
            );
        }
    }
    Ok(())
}

fn assignment<'a>(value: &'a str, field: &str) -> Result<(&'a str, &'a str), CliError> {
    value
        .split_once('=')
        .filter(|(name, _)| !name.is_empty())
        .ok_or_else(|| CliError::Usage(format!("{field} needs NAME=VALUE")))
}

fn parameter_mut<'a>(
    declarations: &'a mut [ParamDecl],
    name: &str,
) -> Result<&'a mut ParamDecl, CliError> {
    declarations
        .iter_mut()
        .find(|item| item.name == name)
        .ok_or_else(|| CliError::Usage(format!("unknown parameter: {name}")))
}

fn parse_parameter_type(value: &str) -> Result<ParameterType, CliError> {
    match value {
        "str" => Ok(ParameterType::Str),
        "int" => Ok(ParameterType::Int),
        "float" => Ok(ParameterType::Float),
        "bool" => Ok(ParameterType::Bool),
        "choice" => Ok(ParameterType::Choice),
        "path" => Ok(ParameterType::Path),
        _ => Err(CliError::Usage(format!("unknown parameter type: {value}"))),
    }
}

fn parse_delivery(value: &str) -> Result<ParameterDelivery, CliError> {
    match value {
        "inject" => Ok(ParameterDelivery::Inject),
        "env" => Ok(ParameterDelivery::Env),
        "flag" => Ok(ParameterDelivery::Flag),
        "placeholder" => Ok(ParameterDelivery::Placeholder),
        _ => Err(CliError::Usage(format!(
            "unknown parameter delivery: {value}"
        ))),
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
    match (key, value) {
        (Some(key), Some(value)) => {
            store.set(key, value)?;
            if json {
                println!("{}", serde_json::json!({"key": key, "value": value}));
            } else {
                println!("Set: {key}={value}");
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
        (None, Some(_)) => unreachable!("clap cannot parse a value without a key"),
    }
    Ok(())
}

fn runner(command: RunnerCommand) -> Result<(), CliError> {
    let store = FileConfigStore::new(resolve_config_dir()?);
    match command {
        RunnerCommand::List { json, all: _ } => {
            let runners = store.runners()?;
            if json {
                let rows = runners
                    .into_iter()
                    .map(|runner| (runner.name, runner.argv))
                    .collect::<BTreeMap<_, _>>();
                println!("{}", serde_json::json!({"runners": rows}));
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
            println!("Added runner: {name}");
        }
        RunnerCommand::Remove {
            name,
            yes,
            no_input: _,
        } => {
            if !yes {
                return Err(CliError::ConfirmationRequiredFor("runner removal"));
            }
            if !store.remove_runner(&name)? {
                return Err(CliError::Usage(format!("unknown prompt runner: {name}")));
            }
            println!("Removed runner: {name}");
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
            let current = state.load(&entry.slug);
            let values = if from_last {
                &current.last_run.values
            } else {
                &current.values
            };
            state.save_preset(&entry.slug, &name, &declarations, values)?;
            println!("Saved preset: {name}");
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
            no_input: _,
        } => {
            if !yes {
                return Err(CliError::ConfirmationRequiredFor("preset deletion"));
            }
            let entry = service.show(&selector)?;
            if !state.delete_preset(&entry.slug, &name)? {
                return Err(CliError::Usage(format!("unknown preset: {name}")));
            }
            println!("Deleted preset: {name}");
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
            Some(path) => println!("OK uv: {}", path.display()),
            None => println!("ERROR uv: not found"),
        }
        println!("Entries: {}", scan.entries.len());
        println!("Library: {} ({} bytes)", scripts.display(), size);
        println!("State: {}", state_location.display());
        println!("Config: {}", config_location.display());
        if let Some(count) = rebuilt_entries {
            println!("Registry rebuilt: {count}");
        }
        for name in missing {
            println!("WARN {name}: the launch target is gone from disk");
        }
        for name in drift {
            println!(
                "WARN {name}: form definitions are out of sync; run: skit params {name} --resync"
            );
        }
        for (name, tools) in needs_missing {
            println!(
                "WARN {name}: missing external commands: {}",
                tools.join(", ")
            );
        }
        for (name, reason) in launch_blocked {
            println!("WARN {name}: a run would refuse to start: {reason}");
        }
        if !bad_runners.is_empty() {
            println!("WARN malformed prompt runners: {}", bad_runners.join(", "));
        }
        for problem in rebuild_problems {
            println!("WARN {problem}");
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
                agent_root(target.as_deref().unwrap_or("agents"), project)?
                    .join("skills")
                    .join("skit")
                    .join("SKILL.md")
            };
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| source_error("create", parent, error))?;
            }
            fs::write(&path, include_bytes!("../../../skills/skit/SKILL.md"))
                .map_err(|error| source_error("write", &path, error))?;
            println!("Installed Agent Skill: {}", path.display());
        }
    }
    Ok(())
}

fn agent_root(target: &str, project: bool) -> Result<PathBuf, CliError> {
    if project {
        let current = env::current_dir().map_err(CliError::Io)?;
        return match target {
            "claude" => Ok(current.join(".claude")),
            "codex" => Ok(current.join(".codex")),
            "agents" => Ok(current.join(".agents")),
            _ => Err(CliError::Usage(format!(
                "unknown agent convention: {target}"
            ))),
        };
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::Usage("could not determine the user directory".to_owned()))?;
    match target {
        "claude" => Ok(home.join(".claude")),
        "codex" => Ok(home.join(".codex")),
        "agents" => Ok(home.join(".agents")),
        _ => Err(CliError::Usage(format!(
            "unknown agent convention: {target}"
        ))),
    }
}

fn entry_parameters(store: &FileStore, entry: &Entry) -> Vec<ParamDecl> {
    let settings = EntrySettings::from_meta(&entry.meta);
    let source = source_path(store, entry)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    form_params(entry.meta.kind.as_str(), &source, &settings)
}

fn source_path(store: &FileStore, entry: &Entry) -> Option<PathBuf> {
    if entry.meta.mode == StorageMode::Reference {
        return (!entry.meta.source.is_empty()).then(|| PathBuf::from(&entry.meta.source));
    }
    let directory = store.data_dir().join("scripts").join(entry.slug.as_str());
    if let Some(name) = stored_filename(entry.meta.kind.as_str()) {
        let path = directory.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    let original_name = Path::new(&entry.meta.source).file_name()?;
    let path = directory.join(original_name);
    path.is_file().then_some(path)
}

fn entry_missing(store: &FileStore, entry: &Entry) -> bool {
    match entry.meta.kind.as_str() {
        "command" => false,
        "exe" => !Path::new(&entry.meta.source).is_file(),
        _ => source_path(store, entry).is_none_or(|path| !path.is_file()),
    }
}

fn summary_missing(store: &FileStore, entry: &EntrySummary) -> bool {
    if let Some(target) = &entry.target {
        return !Path::new(target).is_file();
    }
    match entry.kind.as_str() {
        "command" => false,
        "exe" => true,
        kind => {
            let directory = store.data_dir().join("scripts").join(entry.slug.as_str());
            let names = stored_filenames(kind);
            !names.is_empty() && !names.iter().any(|name| directory.join(name).is_file())
        }
    }
}

fn tui(service: &LibraryService<FileStore>) -> Result<(), CliError> {
    let state = LibraryState::from_scan(service.list()?);
    let store = service.repository();
    skit_tui::run(
        state,
        |effect| tui_effect(service, store, effect),
        active_locale(),
    )?;
    Ok(())
}

fn tui_effect(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    effect: UiEffect,
) -> Result<UiAction, CliError> {
    match effect {
        UiEffect::None | UiEffect::Quit => Ok(UiAction::ClearStatus),
        UiEffect::Reload => Ok(UiAction::Replace(service.list()?)),
        UiEffect::Open { request, selector } => Ok(UiAction::Present(tui_open(
            service, store, request, selector,
        )?)),
        UiEffect::Edit { selector } => {
            edit(service, store, &selector)?;
            Ok(tui_complete(service, "Source saved")?)
        }
        UiEffect::Remove { selector } => {
            remove(service, &selector, true)?;
            Ok(tui_complete(service, "Entry removed")?)
        }
        UiEffect::Submit {
            purpose,
            selector,
            values,
        } => tui_submit(service, store, purpose, selector, &values),
    }
}

fn tui_open(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    request: HostRequest,
    selector: Option<String>,
) -> Result<Screen, CliError> {
    match request {
        HostRequest::Run => {
            let entry = service.show(tui_selector(&selector)?)?;
            let declarations = entry_parameters(store, &entry);
            let saved = FormStateService::new(FileFormStateStore::new(resolve_state_dir()?))
                .load(&entry.slug);
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
            Ok(tui_settings_form(&entry))
        }
        HostRequest::Preferences => tui_preferences_form(),
        HostRequest::Health => tui_health_report(service, store),
        HostRequest::Runners => tui_runners_form(),
        HostRequest::Presets => {
            let entry = service.show(tui_selector(&selector)?)?;
            let saved = FormStateService::new(FileFormStateStore::new(resolve_state_dir()?))
                .load(&entry.slug);
            Ok(tui_presets_form(
                &entry,
                &saved.presets.keys().cloned().collect::<Vec<_>>(),
            ))
        }
        HostRequest::Rename => {
            let entry = service.show(tui_selector(&selector)?)?;
            Ok(Screen::Form(FormView {
                purpose: FormPurpose::Rename,
                title: format!("Rename {}", entry.meta.name),
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
        .ok_or_else(|| CliError::Usage("select an entry first".to_owned()))
}

fn tui_run_form(
    entry: &Entry,
    declarations: &[ParamDecl],
    saved: &BTreeMap<String, String>,
    runners: &[String],
    presets: &[String],
) -> Screen {
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
                FormField::secret(format!("value:{}", parameter.name), label, value)
            } else {
                FormField::text(format!("value:{}", parameter.name), label, value)
            }
        })
        .collect::<Vec<_>>();
    fields.extend([
        FormField::text("_skit_preset", tui_options_label("Preset", presets), ""),
        FormField::text("_skit_save_preset", "Save as preset", ""),
        FormField::text(
            "_skit_runner",
            tui_options_label("Prompt runner", runners),
            runners.first().cloned().unwrap_or_default(),
        ),
        FormField::text("_skit_args", "Extra arguments", ""),
        FormField::text("_skit_dry_run", "Dry run (true or false)", "false"),
    ]);
    Screen::Form(FormView {
        purpose: FormPurpose::Run,
        title: format!("Run {}", entry.meta.name),
        selector: Some(entry.slug.as_str().to_owned()),
        fields,
        focused: 0,
        submit_label: "Run".to_owned(),
    })
}

fn tui_add_form() -> Screen {
    Screen::Form(FormView {
        purpose: FormPurpose::Add,
        title: "Add an entry".to_owned(),
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
    })
}

fn tui_settings_form(entry: &Entry) -> Screen {
    let settings = EntrySettings::from_meta(&entry.meta);
    Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: format!("Settings for {}", entry.meta.name),
        selector: Some(entry.slug.as_str().to_owned()),
        fields: vec![
            FormField::text("name", "Name", &entry.meta.name),
            FormField::multiline("description", "Description", &entry.meta.description),
            FormField::text("workdir", "Working directory", &entry.meta.workdir),
            FormField::text("interpreter", "Interpreter", settings.interpreter),
            FormField::text("runner", "Prompt runner", settings.runner),
            FormField::text(
                "dependencies",
                "Package dependencies",
                settings.dependencies.join(", "),
            ),
            FormField::text("python", "Python constraint", settings.requires_python),
            FormField::text("needs", "Required commands", settings.needs.join(", ")),
            FormField::multiline("template", "Command template", settings.template),
            FormField::text(
                "interpolate",
                "Prompt interpolation (true or false)",
                settings.interpolate.to_string(),
            ),
        ],
        focused: 0,
        submit_label: "Save".to_owned(),
    })
}

fn tui_preferences_form() -> Result<Screen, CliError> {
    let settings = FileConfigStore::new(resolve_config_dir()?).settings()?;
    Ok(Screen::Form(FormView {
        purpose: FormPurpose::Preferences,
        title: "Preferences".to_owned(),
        selector: None,
        fields: settings
            .into_iter()
            .map(|(key, value)| FormField::text(key.clone(), key, value))
            .collect(),
        focused: 0,
        submit_label: "Save".to_owned(),
    }))
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
            detail: scan.entries.len().to_string(),
        },
        ReportItem {
            status: if missing == 0 { "ok" } else { "error" }.to_owned(),
            label: "Missing targets".to_owned(),
            detail: missing.to_string(),
        },
        ReportItem {
            status: "ok".to_owned(),
            label: "Data directory".to_owned(),
            detail: store.data_dir().display().to_string(),
        },
    ];
    items.extend(scan.diagnostics.into_iter().map(|diagnostic| ReportItem {
        status: "error".to_owned(),
        label: diagnostic.slug.unwrap_or_else(|| "Library".to_owned()),
        detail: diagnostic.message,
    }));
    Ok(Screen::Report(ReportView {
        title: "Health".to_owned(),
        items,
    }))
}

fn tui_runners_form() -> Result<Screen, CliError> {
    let runners = FileConfigStore::new(resolve_config_dir()?)
        .runners()?
        .into_iter()
        .map(|runner| format!("{}={}", runner.name, runner.argv.join(" ")))
        .collect::<Vec<_>>();
    Ok(Screen::Form(FormView {
        purpose: FormPurpose::Runners,
        title: format!("Prompt runners: {}", runners.join("; ")),
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
        title: format!("Presets for {}: {}", entry.meta.name, presets.join(", ")),
        selector: Some(entry.slug.as_str().to_owned()),
        fields: vec![
            FormField::text("name", "Preset name", ""),
            FormField::text("action", "Action (save or delete)", "save"),
        ],
        focused: 0,
        submit_label: "Apply".to_owned(),
    })
}

fn tui_options_label(label: &str, options: &[String]) -> String {
    if options.is_empty() {
        label.to_owned()
    } else {
        format!("{label} ({})", options.join(", "))
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

fn tui_submit(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    purpose: FormPurpose,
    selector: Option<String>,
    values: &BTreeMap<String, String>,
) -> Result<UiAction, CliError> {
    match purpose {
        FormPurpose::Run => tui_submit_run(service, store, tui_selector(&selector)?, values),
        FormPurpose::Add => {
            let source = tui_value(values, "source");
            let template = tui_value(values, "template");
            let kind = tui_value(values, "kind");
            add(
                service,
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
                    dependencies: tui_split_list(tui_value(values, "dependencies")),
                    requires_python: tui_nonempty_owned(values, "python"),
                },
            )?;
            Ok(tui_complete(service, "Entry added")?)
        }
        FormPurpose::Settings => {
            tui_submit_settings(service, tui_selector(&selector)?, values)?;
            Ok(tui_complete(service, "Settings saved")?)
        }
        FormPurpose::Preferences => {
            let config = FileConfigStore::new(resolve_config_dir()?);
            for (key, value) in values {
                config.set(key, value)?;
            }
            Ok(tui_complete(service, "Preferences saved")?)
        }
        FormPurpose::Runners => {
            let config = FileConfigStore::new(resolve_config_dir()?);
            let name = tui_required(values, "name")?;
            if tui_bool(tui_value(values, "remove")) {
                if !config.remove_runner(name)? {
                    return Err(CliError::Usage(format!("unknown prompt runner: {name}")));
                }
            } else {
                let argv = shlex::split(tui_required(values, "argv")?).ok_or_else(|| {
                    CliError::Usage("the runner arguments have invalid quoting".to_owned())
                })?;
                config.set_runner(
                    PromptRunner {
                        name: name.to_owned(),
                        argv,
                    },
                    true,
                )?;
            }
            Ok(tui_complete(service, "Prompt runners saved")?)
        }
        FormPurpose::Presets => {
            let selector = tui_selector(&selector)?;
            let entry = service.show(selector)?;
            let declarations = entry_parameters(store, &entry);
            let state = FormStateService::new(FileFormStateStore::new(resolve_state_dir()?));
            let name = tui_required(values, "name")?;
            if tui_value(values, "action").eq_ignore_ascii_case("delete") {
                if !state.delete_preset(&entry.slug, name)? {
                    return Err(CliError::Usage(format!("unknown preset: {name}")));
                }
            } else {
                let current = state.load(&entry.slug);
                state.save_preset(&entry.slug, name, &declarations, &current.values)?;
            }
            Ok(tui_complete(service, "Presets saved")?)
        }
        FormPurpose::Rename => {
            rename(
                service,
                tui_selector(&selector)?,
                tui_required(values, "name")?,
            )?;
            Ok(tui_complete(service, "Entry renamed")?)
        }
    }
}

fn tui_submit_run(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    selector: &str,
    values: &BTreeMap<String, String>,
) -> Result<UiAction, CliError> {
    let run_values = values
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("value:")
                .filter(|_| !value.is_empty())
                .map(|name| format!("{name}={value}"))
        })
        .collect();
    let extra_args = shlex::split(tui_value(values, "_skit_args"))
        .ok_or_else(|| CliError::Usage("extra arguments have invalid quoting".to_owned()))?;
    let exit = crate::run::run(
        service,
        store,
        RunArgs {
            selector: selector.to_owned(),
            values: run_values,
            preset: tui_nonempty_owned(values, "_skit_preset"),
            save_preset: tui_nonempty_owned(values, "_skit_save_preset"),
            runner: tui_nonempty_owned(values, "_skit_runner"),
            dry_run: tui_bool(tui_value(values, "_skit_dry_run")),
            no_input: true,
            plain: true,
            raw: false,
            forget_args: false,
            extra_args,
        },
    )?;
    if FileConfigStore::new(resolve_config_dir()?).get("after_run")? == "exit" {
        Ok(UiAction::Quit)
    } else {
        tui_complete(service, &format!("Run finished with exit status {exit}"))
    }
}

fn tui_submit_settings(
    service: &LibraryService<FileStore>,
    selector: &str,
    values: &BTreeMap<String, String>,
) -> Result<(), CliError> {
    let mut entry = service.show(selector)?;
    let name = tui_required(values, "name")?;
    if name != entry.meta.name {
        let claimed = service.claim_identity(&entry)?;
        entry = service.rename(&claimed, name)?;
    }
    let description = tui_value(values, "description");
    if description != entry.meta.description {
        let claimed = service.claim_identity(&entry)?;
        entry = service.describe(&claimed, description)?;
    }
    let mut settings = EntrySettings::from_meta(&entry.meta);
    settings.interpreter = tui_value(values, "interpreter").to_owned();
    settings.runner = tui_value(values, "runner").to_owned();
    settings.dependencies = tui_split_list(tui_value(values, "dependencies"));
    settings.requires_python = tui_value(values, "python").to_owned();
    settings.needs = tui_split_list(tui_value(values, "needs"));
    settings.template = tui_value(values, "template").to_owned();
    settings.interpolate = tui_bool(tui_value(values, "interpolate"));
    let claimed = service.claim_identity(&entry)?;
    service.update_settings(&claimed, &settings, tui_value(values, "workdir"))?;
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
        Err(CliError::Usage(format!("{key} is required")))
    } else {
        Ok(value)
    }
}

fn tui_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "1" | "on"
    )
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
    Usage(String),
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
    #[error("could not determine the platform data directory; pass --data-dir or SKIT_DATA_DIR")]
    DataDirectoryUnavailable,
    #[error("could not determine the platform {0} directory; set the matching SKIT_*_DIR variable")]
    DirectoryUnavailable(&'static str),
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
