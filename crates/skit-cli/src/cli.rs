use std::{
    collections::BTreeMap,
    env,
    fs::{self, File, Metadata},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use clap::{Args, Parser, Subcommand};
use skit_application::{
    CreateEntry, EntryPayload, ExitClass, LibraryService, RepositoryError, SourcePermissions,
    form_state::{FormStateService, StateWriteError},
};
use skit_domain::{
    Entry, EntryKind, EntrySettings, EntrySummary, StorageMode,
    parameters::{ParamDecl, ParameterDelivery, ParameterType, coerce_default},
};
use skit_form::form_params;
use skit_language::{infer_kind, placeholder_params};
use skit_store::{ConfigError, FileConfigStore, FileFormStateStore, PromptRunner};
use skit_store::{FileStore, stored_filename};
use skit_ui::LibraryState;
use thiserror::Error;

use crate::run::{RunArgs, RunError};

#[cfg(test)]
mod tests;

/// Run the command-line entry point and return its process status.
#[must_use]
pub fn entry() -> i32 {
    let cli = Cli::parse();
    match execute(cli) {
        Ok(code) => code,
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
    /// Override the skit data directory.
    #[arg(long, global = true, value_name = "PATH")]
    data_dir: Option<PathBuf>,

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
        selector: String,
        /// Replacement description.
        description: String,
    },
    /// Rename one entry and derive its new slug.
    Rename {
        /// Entry slug or display name.
        selector: String,
        /// Replacement display name.
        name: String,
    },
    /// Remove one entry.
    Remove {
        /// Entry slug or display name.
        selector: String,
        /// Confirm the destructive operation.
        #[arg(long)]
        yes: bool,
    },
    /// Open an entry source in the configured editor.
    Edit {
        /// Entry slug or display name.
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
    selector: String,
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
        selector: String,
        /// Emit stable machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Delete one named preset.
    Delete {
        /// Entry slug or display name.
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

fn execute(cli: Cli) -> Result<i32, CliError> {
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
            reference,
            command_template,
            prompt,
            exe,
            runner,
            no_interpolate,
            dependencies,
            python,
            no_input: _,
        }) => {
            add(
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
            )?;
            Ok(0)
        }
        Some(Command::Run(args)) => crate::run::run(&service, &store, args).map_err(Into::into),
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
            deps(&service, args)?;
            Ok(0)
        }
        Some(Command::Doctor { json, rebuild }) => {
            doctor(&service, &store, json, rebuild)?;
            Ok(0)
        }
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
    let dependencies = dependencies
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

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

fn deps(service: &LibraryService<FileStore>, args: DepsArgs) -> Result<(), CliError> {
    let held = service.show(&args.selector)?;
    let mut settings = EntrySettings::from_meta(&held.meta);
    let kind = held.meta.kind.as_str();
    let package_change =
        !args.dependencies.is_empty() || args.clear || args.requires_python.is_some();
    if package_change && !matches!(kind, "python" | "js" | "ts") {
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
    if args.clear && !args.dependencies.is_empty() {
        return Err(CliError::Usage("use --dep or --clear, not both".to_owned()));
    }
    if args.clear_needs && !args.needs.is_empty() {
        return Err(CliError::Usage(
            "use --need or --clear-needs, not both".to_owned(),
        ));
    }
    let changed = !args.dependencies.is_empty()
        || args.clear
        || args.requires_python.is_some()
        || !args.needs.is_empty()
        || args.clear_needs;
    if args.clear {
        settings.dependencies.clear();
    } else if !args.dependencies.is_empty() {
        settings.dependencies = args.dependencies;
    }
    if let Some(version) = args.requires_python {
        settings.requires_python = version;
    }
    if args.clear_needs {
        settings.needs.clear();
    } else if !args.needs.is_empty() {
        settings.needs = args.needs;
    }
    if changed {
        let claimed = service.claim_identity(&held)?;
        service.update_settings(&claimed, &settings, &held.meta.workdir)?;
    }
    write_deps(&settings, args.json)
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
    let held = service.show(&args.selector)?;
    let mut settings = EntrySettings::from_meta(&held.meta);
    let source = source_path(store, &held)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
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
) -> Result<(), CliError> {
    let rebuilt_entries = rebuild.then(|| store.rebuild_registry()).transpose()?;
    let scan = service.list()?;
    let state_location = resolve_state_dir()?;
    let config_location = resolve_config_dir()?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "entries": scan.entries.len(),
                "diagnostics": scan.diagnostics,
                "rebuilt": rebuild,
                "rebuilt_entries": rebuilt_entries,
                "location": store.data_dir(),
                "state_location": state_location,
                "config_location": config_location,
            })
        );
    } else {
        println!("Entries: {}", scan.entries.len());
        println!("Data: {}", store.data_dir().display());
        println!("State: {}", state_location.display());
        println!("Config: {}", config_location.display());
        if rebuild {
            println!("Registry rebuilt.");
        }
        for diagnostic in scan.diagnostics {
            eprintln!("warning: {}", diagnostic.message);
        }
    }
    Ok(())
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
        kind => stored_filename(kind).is_some_and(|name| {
            !store
                .data_dir()
                .join("scripts")
                .join(entry.slug.as_str())
                .join(name)
                .is_file()
        }),
    }
}

fn tui(service: &LibraryService<FileStore>) -> Result<(), CliError> {
    let state = LibraryState::from_scan(service.list()?);
    skit_tui::run(state, || service.list())?;
    Ok(())
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
            Self::ConfirmationRequired | Self::ConfirmationRequiredFor(_) | Self::Usage(_) => {
                ExitClass::Usage.code() as i32
            }
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
