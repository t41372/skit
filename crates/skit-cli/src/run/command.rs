use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    io::{IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use clap::Args;
use clap_complete::ArgValueCandidates;
use skit_application::{
    LibraryService, RepositoryError, RepositoryOperation,
    delivery::{Assembly, injection_transparency_messages, transparency_messages},
    form_state::{FormStateService, StateWriteError, prefill},
    prompt_selection::PromptSelectionService,
    run_inputs::{RunInputError, assemble_run_inputs},
    tokens::TokenContext,
};
use skit_domain::{
    Entry, EntrySettings,
    parameters::{ParamDecl, ParameterDelivery},
};
use skit_form::{FormDrift, form_plan};
use skit_i18n::{Localize, Message};
use skit_language::{
    LanguageError, PromptEncodingError, decode_prompt, inject_values_for_interpreter,
    render_prompt_body,
};
use skit_runtime::{
    DependencyCommand, DependencyCommandOutput, DependencyCommandRunner, DependencyError,
    InterpreterPolicy, LaunchError, LaunchPaths, LaunchWarning, ProgramProbe, PromptRunner,
    ResolvedShellInterpreter, SystemDependencyCommandRunner, SystemInjectedCommandRunner,
    SystemJavaScriptSyntaxGateRunner, SystemProbe, UvBootstrapError, UvDownloadConsent,
    build_launch_plan_with_interpreter_policy, build_launch_preview,
    ensure_javascript_dependencies_for_module, ensure_managed_uv, execute_launch,
    javascript_dependency_install_announcement, javascript_module_type, managed_uv_path,
    resolve_javascript_runtime_program, retain_javascript_source_if_valid,
    retain_shell_source_if_valid, shell_self_location_warning, sweep_stale_injected_sources,
};
use skit_store::{
    ConfigError, FileConfigStore, FileFormStateStore, FileGlobExpander, FilePromptSelectionStore,
    FileStore, content_hash,
};
use thiserror::Error;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

use crate::cli::{entry_candidates, preset_candidates, runner_candidates};

#[derive(Clone, Copy, Debug)]
struct CliDependencyCommandRunner;

impl DependencyCommandRunner for CliDependencyCommandRunner {
    fn installation_started(&self, installer: &str) {
        eprintln!(
            "{}",
            javascript_dependency_install_announcement(installer)
                .localize(crate::cli::active_locale())
        );
    }

    fn run(&self, command: &DependencyCommand) -> io::Result<DependencyCommandOutput> {
        SystemDependencyCommandRunner.run(command)
    }
}

/// Options for `skit run`.
#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    /// Entry slug or display name.
    #[arg(add = ArgValueCandidates::new(entry_candidates))]
    pub(crate) selector: String,

    /// Set one field for this run.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    pub(crate) values: Vec<String>,

    /// Load one named preset.
    #[arg(
        long,
        short = 'p',
        add = ArgValueCandidates::new(preset_candidates)
    )]
    pub(crate) preset: Option<String>,

    /// Save accepted values as a named preset after the run.
    #[arg(long, value_name = "NAME")]
    pub(crate) save_preset: Option<String>,

    /// Select a prompt runner for this run.
    #[arg(long, add = ArgValueCandidates::new(runner_candidates))]
    pub(crate) runner: Option<String>,

    /// Whether the runner value came from a user selection rather than a form default.
    #[arg(skip)]
    pub(crate) runner_was_picked: bool,

    /// Print the masked launch command and do not start a child.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Do not open an interactive form.
    #[arg(long)]
    pub(crate) no_input: bool,

    /// Disable enhanced terminal presentation for this run.
    #[arg(long)]
    pub(crate) plain: bool,

    /// Bypass parameter handling and pass only the argument tail.
    #[arg(long)]
    pub(crate) raw: bool,

    /// Clear the remembered argument tail before this run.
    #[arg(long)]
    pub(crate) forget_args: bool,

    /// Arguments after `--`.
    #[arg(last = true, value_name = "ARGS")]
    pub(crate) extra_args: Vec<String>,
}

/// Run-command failure.
#[derive(Debug, Error)]
pub(crate) enum RunError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    State(#[from] StateWriteError),
    #[error(transparent)]
    Inputs(#[from] RunInputError),
    #[error(transparent)]
    Language(#[from] LanguageError),
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error(transparent)]
    Dependencies(#[from] DependencyError),
    #[error(transparent)]
    Uv(#[from] UvBootstrapError),
    #[error("Malformed --set (expected NAME=VALUE): {items}")]
    InvalidSet { items: String },
    #[error("Unknown parameter for --set: {names}. This entry's parameters: {valid}")]
    UnknownSet { names: String, valid: String },
    #[error("preset {name:?} does not exist")]
    PresetNotFound { name: String },
    #[error("{name} has no form fields, so there's nothing to save.")]
    PresetWithoutFields { name: String },
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("prompt body does not exist: {path}")]
    PromptBodyMissing { path: String },
    #[error(transparent)]
    Encoding(#[from] PromptEncodingError),
    #[error("could not write staged source {path}: {source}")]
    Stage {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("could not determine the platform state directory; set SKIT_STATE_DIR")]
    StateDirectoryUnavailable,
    #[error(
        "The runner {name} isn't configured (known: {known}). Manage runners with: skit runner list"
    )]
    RunnerNotFound { name: String, known: String },
    #[error(
        "No agents are configured. Add one with: skit runner add mycli -- mycli run {{{{prompt}}}}"
    )]
    NoRunnersConfigured,
    #[error(
        "No runner selected for {name}. Pass --runner NAME, or pin one with: skit params {name} --runner NAME"
    )]
    RunnerRequired { name: String },
    #[error("--runner only applies to prompt entries.")]
    RunnerUnsupported,
    #[error("--raw does not apply to {kind} entries because placeholders are part of the artifact")]
    RawUnsupported { kind: String },
    #[error("--raw runs the script as-is; --set, --preset, and --save-preset do not apply.")]
    RawConflict,
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("could not determine the platform configuration directory; set SKIT_CONFIG_DIR")]
    ConfigDirectoryUnavailable,
    #[error(
        "The script and its form definitions don't match anymore: {parameter}. Run `skit params {entry} --resync` to fix it."
    )]
    InjectionDrift { parameter: String, entry: String },
    #[error(
        "The script and its form definitions don't match anymore: {detail}. Run `skit params {entry} --resync` to fix it."
    )]
    InjectionSemanticDrift { detail: Message, entry: String },
    #[error("skit refused to run its own injected copy: {detail}")]
    InjectedCopy { detail: Message },
}

impl Localize for RunError {
    fn message(&self) -> Message {
        match self {
            Self::Repository(error) => error.message(),
            Self::State(error) => error.message(),
            Self::Inputs(error) => error.message(),
            Self::Language(error) => error.message(),
            Self::Launch(error) => error.message(),
            Self::Dependencies(error) => error.message(),
            Self::Uv(error) => error.message(),
            Self::Config(error) => error.message(),
            Self::InvalidSet { items } => {
                Message::new("Malformed --set (expected NAME=VALUE): {}").with(items)
            }
            Self::UnknownSet { names, valid } => {
                Message::new("Unknown parameter for --set: {}. This entry's parameters: {}")
                    .with(names)
                    .with(valid)
            }
            Self::PresetNotFound { name } => Message::new("preset {} does not exist").quoted(name),
            Self::PresetWithoutFields { name } => {
                Message::new("{} has no form fields, so there's nothing to save.").with(name)
            }
            Self::Read { path, source } => Message::new("could not read {}: {}")
                .with(path)
                .with(source),
            Self::PromptBodyMissing { path } => {
                Message::new("prompt body does not exist: {}").with(path)
            }
            Self::Encoding(error) => error.message(),
            Self::Stage { path, source } => Message::new("could not write staged source {}: {}")
                .with(path)
                .with(source),
            Self::StateDirectoryUnavailable => {
                Message::new("could not determine the platform state directory; set SKIT_STATE_DIR")
            }
            Self::RunnerNotFound { name, known } => Message::new(
                "The runner {} isn't configured (known: {}). Manage runners with: skit runner list",
            )
            .with(name)
            .with(known),
            Self::NoRunnersConfigured => Message::new(
                "No agents are configured. Add one with: skit runner add mycli -- mycli run {{prompt}}",
            ),
            Self::RunnerRequired { name } => Message::new(
                "No runner selected for {}. Pass --runner NAME, or pin one with: skit params {} --runner NAME",
            )
            .with(name)
            .with(name),
            Self::RunnerUnsupported => Message::new("--runner only applies to prompt entries."),
            Self::RawUnsupported { kind } => Message::new(
                "--raw does not apply to {} entries because placeholders are part of the artifact",
            )
            .with(kind),
            Self::RawConflict => Message::new(
                "--raw runs the script as-is; --set, --preset, and --save-preset do not apply.",
            ),
            Self::ConfigDirectoryUnavailable => Message::new(
                "could not determine the platform configuration directory; set SKIT_CONFIG_DIR",
            ),
            Self::InjectionDrift { parameter, entry } => Message::new(
                "The script and its form definitions don't match anymore: {}. Run `skit params {} --resync` to fix it.",
            )
            .with(parameter)
            .with(entry),
            Self::InjectionSemanticDrift { detail, entry } => Message::new(
                "The script and its form definitions don't match anymore: {}. Run `skit params {} --resync` to fix it.",
            )
            .nested(detail.clone())
            .with(entry),
            Self::InjectedCopy { detail } => {
                Message::new("skit refused to run its own injected copy: {}").nested(detail.clone())
            }
        }
    }
}

impl RunError {
    pub(crate) const fn exit_code(&self) -> i32 {
        match self {
            Self::Repository(error) => error.exit_class(RepositoryOperation::Launch).code() as i32,
            Self::InvalidSet { .. }
            | Self::UnknownSet { .. }
            | Self::PresetNotFound { .. }
            | Self::PresetWithoutFields { .. }
            | Self::RunnerUnsupported
            | Self::RawUnsupported { .. }
            | Self::RawConflict => 2,
            Self::Launch(error) => error.exit_code(),
            Self::PromptBodyMissing { .. } => 127,
            Self::Dependencies(DependencyError::InstallerNotFound { .. })
            | Self::Dependencies(DependencyError::InstallerStartFailed { .. })
            | Self::Dependencies(DependencyError::InstallFailed { .. })
            | Self::Dependencies(DependencyError::ClearFailed { .. })
            | Self::Dependencies(DependencyError::Io { .. })
            | Self::Dependencies(DependencyError::Rollback { .. }) => 126,
            // Version 0.4 wraps every uv bootstrap failure, refusal included, into a launch error
            // (`src/skit/langs/launch.py:57-63`), and a launch failure exits 125
            // (`src/skit/flows.py:868`).
            Self::Uv(_) => 125,
            Self::RunnerNotFound { .. }
            | Self::NoRunnersConfigured
            | Self::RunnerRequired { .. } => 126,
            Self::State(_)
            | Self::Inputs(_)
            | Self::Language(_)
            | Self::Read { .. }
            | Self::Encoding(_)
            | Self::Stage { .. }
            | Self::InjectionDrift { .. }
            | Self::InjectionSemanticDrift { .. }
            | Self::InjectedCopy { .. }
            | Self::StateDirectoryUnavailable
            | Self::Config(_)
            | Self::ConfigDirectoryUnavailable
            | Self::Dependencies(_) => 125,
        }
    }
}

pub(crate) fn run(
    service: &LibraryService<FileStore>,
    data_store: &FileStore,
    args: RunArgs,
) -> Result<i32, RunError> {
    let state_dir = resolve_state_dir()?;
    let config_dir = resolve_config_dir()?;
    run_with_roots(service, data_store, &state_dir, &config_dir, args)
}

pub(crate) fn run_with_roots(
    service: &LibraryService<FileStore>,
    data_store: &FileStore,
    state_dir: &Path,
    config_dir: &Path,
    args: RunArgs,
) -> Result<i32, RunError> {
    let _plain = args.plain;
    let _no_input = args.no_input;
    let held = service.show(&args.selector)?;
    if args.runner.is_some() && held.meta.kind.as_str() != "prompt" {
        return Err(RunError::RunnerUnsupported);
    }
    if args.raw && (!args.values.is_empty() || args.preset.is_some() || args.save_preset.is_some())
    {
        return Err(RunError::RawConflict);
    }
    if args.raw && matches!(held.meta.kind.as_str(), "command" | "prompt") {
        return Err(RunError::RawUnsupported {
            kind: held.meta.kind.as_str().to_owned(),
        });
    }
    let config = FileConfigStore::new(config_dir);
    let config_settings = config.settings()?;
    let mut entry = apply_runtime_defaults(held.clone(), &config_settings);
    let configured_bash = config_settings
        .get("shell.bash_path")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let interpreter_policy = InterpreterPolicy::for_current_host(configured_bash);
    let mut settings = EntrySettings::from_meta(&entry.meta);
    let state = FormStateService::new(FileFormStateStore::new(state_dir));
    let saved = state.load(&entry.slug);
    let base_environment = env::vars().collect::<BTreeMap<_, _>>();
    let mirror_environment = config.mirror_environment(&base_environment)?;

    let (source, expected_source_hash) = source_snapshot(data_store, &entry, &settings)?;
    let form = (!args.raw).then(|| form_plan(entry.meta.kind.as_str(), &source, &settings));
    let declarations = form
        .as_ref()
        .map_or_else(Vec::new, skit_form::FormPlan::declarations);
    if args.save_preset.is_some() && declarations.is_empty() {
        return Err(RunError::PresetWithoutFields {
            name: entry.meta.name.clone(),
        });
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
    let mut raw_values = if args.raw {
        BTreeMap::new()
    } else {
        prefill(&declarations, &saved.values, preset)
    };
    if let Some(parameter) = form
        .as_ref()
        .and_then(|plan| requested_drifted_parameter(&args.values, &plan.drift))
    {
        return Err(RunError::InjectionDrift {
            parameter,
            entry: entry.meta.name.clone(),
        });
    }
    apply_sets(&declarations, &args.values, &mut raw_values)?;

    let (extra_args, expand_extra, new_tail) = if args.raw {
        (args.extra_args.clone(), false, None)
    } else if !args.extra_args.is_empty() {
        (
            args.extra_args.clone(),
            false,
            Some(args.extra_args.clone()),
        )
    } else if args.forget_args {
        (Vec::new(), false, Some(Vec::new()))
    } else {
        (saved.extra_args.clone(), saved.extra_args_raw, None)
    };
    if !args.raw && args.extra_args.is_empty() && !args.forget_args && !saved.extra_args.is_empty()
    {
        eprintln!(
            "{}",
            skit_i18n::format_text(
                crate::cli::active_locale(),
                "Reusing your last arguments: {}",
                &[&extra_args.join(" ")],
            )
        );
    }

    let context = token_context();
    let glob = FileGlobExpander::new(&context.cwd);
    let explicit_arg_declarations = (!args.extra_args.is_empty()).then(|| {
        declarations
            .iter()
            .filter(|declaration| {
                declaration.delivery != ParameterDelivery::Flag
                    || !declaration.required
                    || raw_values
                        .get(&declaration.name)
                        .is_some_and(|value| !value.trim().is_empty())
            })
            .cloned()
            .collect::<Vec<_>>()
    });
    let assembly_declarations = explicit_arg_declarations
        .as_deref()
        .unwrap_or(&declarations);
    let assembly = if args.raw {
        skit_application::delivery::Assembly {
            args: extra_args.clone(),
            masked_args: extra_args.clone(),
            ..Default::default()
        }
    } else {
        assemble_run_inputs(
            assembly_declarations,
            &raw_values,
            &extra_args,
            expand_extra,
            &context,
            &glob,
        )?
    };

    let runner = resolve_runner(
        &config,
        state_dir,
        args.runner.as_deref(),
        &settings.runner,
        args.runner_was_picked,
    )?;
    if entry.meta.kind.as_str() == "prompt" && runner.is_none() {
        if config.runners()?.is_empty() {
            return Err(RunError::NoRunnersConfigured);
        }
        return Err(RunError::RunnerRequired {
            name: entry.meta.name.clone(),
        });
    }
    if !args.dry_run
        && matches!(entry.meta.kind.as_str(), "js" | "ts")
        && entry.meta.mode == skit_domain::StorageMode::Reference
        && !settings.dependencies.is_empty()
    {
        return Err(DependencyError::CopyStorageRequired.into());
    }

    let javascript_runtime = if !args.dry_run && matches!(entry.meta.kind.as_str(), "js" | "ts") {
        let runtime = resolve_javascript_runtime_program(&settings, &SystemProbe)?;
        pin_interpreter(&mut settings, &mut entry, &runtime.program);
        Some(runtime)
    } else {
        None
    };

    let needs_uv_bootstrap = entry.meta.kind.as_str() == "python"
        && settings.interpreter.is_empty()
        && SystemProbe.find_program("uv").is_none()
        && !managed_uv_path(data_store.data_dir()).is_file();
    if entry.meta.kind.as_str() == "python"
        && settings.interpreter.is_empty()
        && SystemProbe.find_program("uv").is_none()
        && !needs_uv_bootstrap
    {
        pin_interpreter(
            &mut settings,
            &mut entry,
            &managed_uv_path(data_store.data_dir()),
        );
    }
    let script = if entry.meta.kind.as_str() == "command" {
        PathBuf::new()
    } else {
        launch_payload_path(data_store, &entry)?
    };
    let prompt_body = (entry.meta.kind.as_str() == "prompt")
        .then(|| render_prompt_body(&source, &assembly.command_values, settings.interpolate));
    let prompt_display_body = (entry.meta.kind.as_str() == "prompt").then(|| {
        render_prompt_body(
            &source,
            &assembly.masked_command_values,
            settings.interpolate,
        )
    });
    let paths = LaunchPaths {
        script,
        entry_dir: data_store.entry_dir_path(&entry.slug),
        invoke_cwd: PathBuf::from(&context.cwd),
    };
    let preflight_plan = if args.dry_run {
        let _ = build_launch_preview(
            &entry,
            &paths,
            &assembly,
            prompt_body.as_deref(),
            prompt_display_body.as_deref(),
            runner.as_ref(),
            &SystemProbe,
        )?;
        None
    } else if !needs_uv_bootstrap {
        Some(build_launch_plan_with_interpreter_policy(
            &entry,
            &paths,
            &assembly,
            prompt_body.as_deref(),
            runner.as_ref(),
            &interpreter_policy,
            &SystemProbe,
        )?)
    } else {
        None
    };
    let shell_interpreter = if entry.meta.kind.as_str() == "shell" {
        preflight_plan.as_ref().map(|plan| {
            ResolvedShellInterpreter::new(
                if settings.interpreter.is_empty() {
                    "bash".to_owned()
                } else {
                    settings.interpreter.clone()
                },
                plan.program.clone(),
            )
        })
    } else {
        None
    };
    let shell_uses_self_location = entry.meta.kind.as_str() == "shell"
        && form.as_ref().is_some_and(|plan| plan.uses_self_location);

    let prepared = if args.dry_run {
        None
    } else {
        Some(data_store.prepare_launch(&held, expected_source_hash.as_deref())?)
    };

    if entry.meta.kind.as_str() == "python"
        && settings.interpreter.is_empty()
        && SystemProbe.find_program("uv").is_none()
        && !args.dry_run
    {
        let mirror = config.mirror()?;
        let mirror_base =
            (mirror.enabled && !mirror.uv_binary.is_empty()).then_some(mirror.uv_binary.as_str());
        bootstrap_private_uv(
            &mut settings,
            &mut entry,
            data_store.data_dir(),
            mirror_base,
            &TerminalUvConsent,
            ensure_managed_uv,
        )?;
    }

    let staged = if args.dry_run {
        None
    } else {
        stage_injected_source_with_shell_interpreter(
            data_store,
            &entry,
            &source,
            &declarations,
            &assembly,
            shell_interpreter
                .as_ref()
                .map(ResolvedShellInterpreter::name),
        )?
    };
    let staged = match (staged, entry.meta.kind.as_str()) {
        (Some(source), "js" | "ts") => {
            let path = source.path.clone();
            Some(
                retain_javascript_source_if_valid(
                    source,
                    javascript_runtime.as_ref(),
                    &path,
                    &SystemJavaScriptSyntaxGateRunner,
                )
                .map_err(|error| RunError::InjectedCopy {
                    detail: error.message(),
                })?,
            )
        }
        (Some(source), "shell") => {
            let path = source.path.clone();
            Some(
                retain_shell_source_if_valid(
                    source,
                    shell_interpreter.as_ref(),
                    &path,
                    &SystemInjectedCommandRunner,
                )
                .map_err(|error| RunError::InjectedCopy {
                    detail: error.message(),
                })?,
            )
        }
        (source, _) => source,
    };
    if staged.is_some()
        && entry.meta.kind.as_str() == "shell"
        && let Some(warning) = shell_self_location_warning(shell_uses_self_location)
    {
        eprintln!("{}", warning.localize(crate::cli::active_locale()));
    }
    if !args.dry_run {
        let entry_dir = data_store.entry_dir_path(&entry.slug);
        sweep_injected_launch_sources(data_store, &entry);
        sweep_stale_launch_snapshots(&entry_dir, !assembly.inject_values.is_empty());
        if matches!(entry.meta.kind.as_str(), "js" | "ts")
            && entry.meta.mode == skit_domain::StorageMode::Copy
        {
            let runtime = javascript_runtime
                .as_ref()
                .expect("a non-dry JavaScript launch has a resolved runtime");
            ensure_javascript_dependencies_for_module(
                &entry_dir,
                runtime.kind.name(),
                &settings.dependencies,
                javascript_module_type(&entry.meta.source),
                &mirror_environment,
                &SystemProbe,
                &CliDependencyCommandRunner,
            )?;
        }
    }
    let script = if let Some(staged) = staged.as_ref() {
        staged.path.clone()
    } else if entry.meta.kind.as_str() == "command" {
        PathBuf::new()
    } else if let Some(path) = prepared.as_ref().and_then(|launch| launch.payload_path()) {
        path.to_path_buf()
    } else {
        launch_payload_path(data_store, &entry)?
    };
    let prompt_body = if entry.meta.kind.as_str() == "prompt" {
        Some(render_prompt_body(
            &source,
            &assembly.command_values,
            settings.interpolate,
        ))
    } else {
        None
    };
    let paths = LaunchPaths {
        script,
        entry_dir: data_store.entry_dir_path(&entry.slug),
        invoke_cwd: PathBuf::from(&context.cwd),
    };
    let prompt_display_body = if entry.meta.kind.as_str() == "prompt" {
        Some(render_prompt_body(
            &source,
            &assembly.masked_command_values,
            settings.interpolate,
        ))
    } else {
        None
    };
    let mut plan = if args.dry_run {
        build_launch_preview(
            &entry,
            &paths,
            &assembly,
            prompt_body.as_deref(),
            prompt_display_body.as_deref(),
            runner.as_ref(),
            &SystemProbe,
        )?
    } else {
        build_launch_plan_with_interpreter_policy(
            &entry,
            &paths,
            &assembly,
            prompt_body.as_deref(),
            runner.as_ref(),
            &interpreter_policy,
            &SystemProbe,
        )?
    };
    for (key, value) in mirror_environment {
        plan.env.entry(key).or_insert(value);
    }
    for warning in &plan.warnings {
        match warning {
            LaunchWarning::PiPromptProtected => eprintln!(
                "{}",
                skit_i18n::format_text(
                    crate::cli::active_locale(),
                    "Warning: Pi would interpret the beginning of this prompt as a CLI option, file, or package command. skit prepended one newline and is continuing; the prompt delivered to Pi is one character longer than the rendered text.",
                    &[],
                )
            ),
            LaunchWarning::AmpOneShot => eprintln!(
                "{}",
                skit_i18n::format_text(
                    crate::cli::active_locale(),
                    "The built-in amp runner is one-shot: amp -x runs this prompt once and does not open an interactive session.",
                    &[],
                )
            ),
        }
    }

    if !args.dry_run && prompt_sends_secret(&entry, &declarations, &assembly) {
        eprintln!(
            "{}",
            skit_i18n::format_text(
                crate::cli::active_locale(),
                "Secret-marked values are never saved by skit, but this prompt sends them to the selected agent as plaintext; the agent may log or sync them.",
                &[],
            )
        );
    }

    if args.forget_args {
        state.save_last(&entry.slug, &declarations, None, Some(Vec::new()), false)?;
    }
    if args.dry_run {
        if let Some(name) = args.save_preset.as_deref() {
            state.save_preset(&entry.slug, name, &declarations, &raw_values)?;
        }
        for message in injection_transparency_messages(&assembly) {
            println!("{}", message.localize(crate::cli::active_locale()));
        }
        println!("{}", plan.display);
        return Ok(0);
    }

    for message in transparency_messages(&assembly, &plan.display) {
        println!("{}", message.localize(crate::cli::active_locale()));
    }
    let exit = execute_launch(&plan)?;
    let slug = &entry.slug;
    let at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    let recorded_values = (!args.raw).then_some(&raw_values);
    state.record_completed_run_with(
        slug,
        i64::from(exit),
        &at,
        recorded_values,
        new_tail,
        false,
        args.save_preset.as_deref(),
        || -> Result<Vec<ParamDecl>, RunError> {
            let current = service.show(slug.as_str())?;
            let settings = EntrySettings::from_meta(&current.meta);
            let (source, _) = source_snapshot(data_store, &current, &settings)?;
            Ok(form_plan(current.meta.kind.as_str(), &source, &settings).declarations())
        },
    )??;
    Ok(exit)
}

fn prompt_sends_secret(entry: &Entry, declarations: &[ParamDecl], assembly: &Assembly) -> bool {
    entry.meta.kind.as_str() == "prompt"
        && declarations.iter().any(|field| {
            field.secret
                && field.delivery == ParameterDelivery::Placeholder
                && assembly
                    .command_values
                    .get(&field.name)
                    .is_some_and(|value| !value.is_empty())
        })
}

fn requested_drifted_parameter(sets: &[String], drift: &[FormDrift]) -> Option<String> {
    if sets.iter().any(|item| {
        item.split_once('=')
            .is_none_or(|(name, _)| name.trim().is_empty())
    }) {
        return None;
    }
    let requested = sets
        .iter()
        .filter_map(|item| item.split_once('=').map(|(name, _)| name.trim()))
        .collect::<BTreeSet<_>>();
    drift.iter().find_map(|item| match item {
        FormDrift::Missing { declaration } if requested.contains(declaration.name.as_str()) => {
            Some(declaration.name.clone())
        }
        FormDrift::Missing { .. }
        | FormDrift::TypeChanged { .. }
        | FormDrift::Rebound { .. }
        | FormDrift::PromptMissing { .. } => None,
    })
}

pub(crate) fn apply_sets(
    declarations: &[skit_domain::parameters::ParamDecl],
    sets: &[String],
    values: &mut BTreeMap<String, String>,
) -> Result<(), RunError> {
    let names = declarations
        .iter()
        .map(|item| item.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut pairs = Vec::with_capacity(sets.len());
    let mut malformed = Vec::new();
    for item in sets {
        match item.split_once('=') {
            Some((name, value)) if !name.trim().is_empty() => {
                pairs.push((name.trim(), value));
            }
            _ => malformed.push(item.clone()),
        }
    }
    if !malformed.is_empty() {
        return Err(RunError::InvalidSet {
            items: malformed.join(", "),
        });
    }
    let unknown = pairs
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !names.contains(name))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        let valid = names.into_iter().collect::<Vec<_>>().join(", ");
        return Err(RunError::UnknownSet {
            names: unknown.join(", "),
            valid: if valid.is_empty() {
                "—".to_owned()
            } else {
                valid
            },
        });
    }
    for (name, value) in pairs {
        values.insert(name.to_owned(), value.to_owned());
    }
    Ok(())
}

fn apply_runtime_defaults(mut entry: Entry, config: &BTreeMap<String, String>) -> Entry {
    let mut settings = EntrySettings::from_meta(&entry.meta);
    if settings.interpreter.is_empty()
        && matches!(entry.meta.kind.as_str(), "js" | "ts")
        && let Some(value) = config.get("js.runner")
        && !value.is_empty()
    {
        settings.interpreter.clone_from(value);
        settings.write_to_meta(&mut entry.meta);
    }
    entry
}

pub(crate) fn source_text(
    store: &FileStore,
    entry: &Entry,
    settings: &EntrySettings,
) -> Result<String, RunError> {
    source_snapshot(store, entry, settings).map(|(text, _hash)| text)
}

fn source_snapshot(
    store: &FileStore,
    entry: &Entry,
    settings: &EntrySettings,
) -> Result<(String, Option<String>), RunError> {
    match entry.meta.kind.as_str() {
        "command" => Ok((settings.template.clone(), None)),
        "exe" => Ok((String::new(), None)),
        "prompt" => {
            let path = launch_payload_path(store, entry)?;
            let bytes = read_prompt_bytes(&path, fs::read(&path))?;
            let hash = content_hash(&bytes);
            let text = decode_prompt(&bytes, path.display().to_string())?.to_owned();
            Ok((text, Some(hash)))
        }
        _ => {
            let path = store.payload_path(entry)?;
            let bytes = read_bytes(&path)?;
            let hash = content_hash(&bytes);
            Ok((String::from_utf8(bytes).unwrap_or_default(), Some(hash)))
        }
    }
}

fn read_prompt_bytes(path: &Path, bytes: io::Result<Vec<u8>>) -> Result<Vec<u8>, RunError> {
    bytes.map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            RunError::PromptBodyMissing {
                path: path.display().to_string(),
            }
        } else {
            RunError::Read {
                path: path.display().to_string(),
                source,
            }
        }
    })
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, RunError> {
    fs::read(path).map_err(|source| RunError::Read {
        path: path.display().to_string(),
        source,
    })
}

fn launch_payload_path(store: &FileStore, entry: &Entry) -> Result<PathBuf, RunError> {
    store.payload_path(entry).map_err(|error| match &error {
        RepositoryError::InvalidMutation { reason }
            if entry.meta.kind.as_str() == "prompt"
                && reason.template() == "copy entry has no stored payload" =>
        {
            RunError::PromptBodyMissing {
                path: store
                    .entry_dir_path(&entry.slug)
                    .join("prompt.md")
                    .display()
                    .to_string(),
            }
        }
        _ => RunError::Repository(error),
    })
}

#[cfg(test)]
fn stage_injected_source(
    store: &FileStore,
    entry: &Entry,
    source: &str,
    declarations: &[skit_domain::parameters::ParamDecl],
    assembly: &skit_application::delivery::Assembly,
) -> Result<Option<StagedSource>, RunError> {
    stage_injected_source_with_shell_interpreter(store, entry, source, declarations, assembly, None)
}

fn stage_injected_source_with_shell_interpreter(
    store: &FileStore,
    entry: &Entry,
    source: &str,
    declarations: &[skit_domain::parameters::ParamDecl],
    assembly: &skit_application::delivery::Assembly,
    resolved_shell: Option<&str>,
) -> Result<Option<StagedSource>, RunError> {
    let entry_dir = store.entry_dir_path(&entry.slug);
    if assembly.inject_values.is_empty() {
        return Ok(None);
    }
    let kind = entry.meta.kind.as_str();
    let settings = EntrySettings::from_meta(&entry.meta);
    let configured_interpreter = settings.interpreter.as_str();
    let interpreter = resolved_shell
        .or_else(|| (!configured_interpreter.is_empty()).then_some(configured_interpreter));
    let rewritten = inject_values_for_interpreter(
        kind,
        source,
        declarations,
        &assembly.inject_values,
        interpreter,
    )
    .map_err(|error| map_injection_language_error(error, &entry.meta.name))?;
    let original = launch_payload_path(store, entry)?;
    let suffix = original
        .extension()
        .and_then(|value| value.to_str())
        .map_or(String::new(), |value| format!(".{value}"));
    let adjacent_to_modules = matches!(kind, "js" | "ts")
        && entry.meta.mode == skit_domain::StorageMode::Copy
        && !settings.dependencies.is_empty();
    let file = new_injected_file(&entry_dir, &suffix, adjacent_to_modules)?;
    finish_staged_source(file, rewritten.as_bytes(), write_and_sync_staged_source).map(Some)
}

fn sweep_injected_launch_sources(store: &FileStore, entry: &Entry) {
    sweep_stale_injected_sources(&store.entry_dir_path(&entry.slug));
}

fn new_injected_file(
    entry_dir: &Path,
    suffix: &str,
    adjacent_to_modules: bool,
) -> Result<tempfile::NamedTempFile, RunError> {
    new_injected_file_with_ops(
        entry_dir,
        suffix,
        adjacent_to_modules,
        |builder, directory| builder.tempfile_in(directory),
        |builder| builder.tempfile(),
    )
}

fn new_injected_file_with_ops<E, S>(
    entry_dir: &Path,
    suffix: &str,
    adjacent_to_modules: bool,
    mut create_in_entry: E,
    mut create_in_system_temp: S,
) -> Result<tempfile::NamedTempFile, RunError>
where
    E: FnMut(&mut tempfile::Builder<'_, '_>, &Path) -> io::Result<tempfile::NamedTempFile>,
    S: FnMut(&mut tempfile::Builder<'_, '_>) -> io::Result<tempfile::NamedTempFile>,
{
    let mut builder = tempfile::Builder::new();
    builder.prefix(".injected-").suffix(suffix);
    let file = if adjacent_to_modules {
        create_in_entry(&mut builder, entry_dir).or_else(|_| create_in_system_temp(&mut builder))
    } else {
        // The OS temp directory is the normal home for a source that can contain plaintext secret
        // values. Keep the oracle's entry-directory fallback for a broken TMPDIR. A later
        // successful run removes an aged fallback after an abnormal process exit.
        create_in_system_temp(&mut builder).or_else(|_| create_in_entry(&mut builder, entry_dir))
    };
    file.map_err(|source| RunError::Stage {
        path: entry_dir.display().to_string(),
        source,
    })
}

fn map_injection_language_error(error: LanguageError, entry: &str) -> RunError {
    match error {
        LanguageError::BindingNotFound { name } => RunError::InjectionDrift {
            parameter: name,
            entry: entry.to_owned(),
        },
        error @ LanguageError::InjectedSourceInvalid { .. } => RunError::InjectedCopy {
            detail: error.message(),
        },
        LanguageError::SourceChanged => RunError::InjectionSemanticDrift {
            detail: LanguageError::SourceChanged.message(),
            entry: entry.to_owned(),
        },
        error => RunError::Language(error),
    }
}

fn finish_staged_source(
    mut file: tempfile::NamedTempFile,
    bytes: &[u8],
    write_and_sync: impl FnOnce(&mut fs::File, &[u8]) -> io::Result<()>,
) -> Result<StagedSource, RunError> {
    let path = file.path().to_path_buf();
    write_and_sync(file.as_file_mut(), bytes).map_err(|source| RunError::Stage {
        path: path.display().to_string(),
        source,
    })?;
    Ok(StagedSource { path, _file: file })
}

fn write_and_sync_staged_source(file: &mut fs::File, bytes: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    {
        let current = std::thread::current().id();
        let must_fail = STAGE_WRITE_FAULT
            .lock()
            .expect("stage-write fault mutex must not be poisoned")
            .as_ref()
            .is_some_and(|fault| fault.owner == current);
        if must_fail {
            file.write_all(&bytes[..bytes.len().min(6)])?;
            return Err(io::Error::other("injected staged-source write failure"));
        }
    }
    file.write_all(bytes).and_then(|()| file.sync_all())
}

#[cfg(test)]
struct StageWriteFault {
    owner: std::thread::ThreadId,
}

#[cfg(test)]
static STAGE_WRITE_FAULT: std::sync::Mutex<Option<StageWriteFault>> = std::sync::Mutex::new(None);

#[cfg(test)]
struct StageWriteFaultGuard {
    owner: std::thread::ThreadId,
}

#[cfg(test)]
impl StageWriteFaultGuard {
    fn for_current_thread() -> Self {
        let owner = std::thread::current().id();
        let mut fault = STAGE_WRITE_FAULT
            .lock()
            .expect("stage-write fault mutex must not be poisoned");
        assert!(fault.is_none(), "only one stage-write fault can be active");
        *fault = Some(StageWriteFault { owner });
        Self { owner }
    }
}

#[cfg(test)]
impl Drop for StageWriteFaultGuard {
    fn drop(&mut self) {
        let mut fault = STAGE_WRITE_FAULT
            .lock()
            .expect("stage-write fault mutex must not be poisoned");
        assert!(
            fault
                .as_ref()
                .is_some_and(|fault| fault.owner == self.owner),
            "stage-write fault ownership changed"
        );
        *fault = None;
    }
}

/// Ask for consent, announce the first private uv download, then pin the installed path.
///
/// `install` is the installer port and `consent` is the question port. The composition root passes
/// the real bootstrap and the real terminal. Version 0.4 asks before it downloads
/// (`src/skit/uvman.py:251-256`), announces the download only after consent
/// (`src/skit/uvman.py:259-265`), and reports the installed path when it finishes
/// (`src/skit/uvman.py:284-287`).
fn bootstrap_private_uv<F>(
    settings: &mut EntrySettings,
    entry: &mut Entry,
    data_dir: &Path,
    mirror_base: Option<&str>,
    consent: &dyn UvDownloadConsent,
    install: F,
) -> Result<(), RunError>
where
    F: FnOnce(&Path, Option<&str>) -> Result<PathBuf, UvBootstrapError>,
{
    let locale = crate::cli::active_locale();
    let destination = skit_runtime::managed_uv_path(data_dir);
    let private_dir = destination.parent().unwrap_or(data_dir);
    if !consent.allow_download(skit_runtime::UV_VERSION, private_dir) {
        return Err(UvBootstrapError::Declined.into());
    }
    eprintln!(
        "{}",
        skit_i18n::format_text(
            locale,
            "First run — downloading uv {}…",
            &[&skit_runtime::UV_VERSION],
        )
    );
    let installed = install(data_dir, mirror_base)?;
    eprintln!(
        "{}",
        skit_i18n::format_text(locale, "uv installed at: {}", &[&installed.display()])
    );
    pin_interpreter(settings, entry, &installed);
    Ok(())
}

/// Ask the real terminal before skit downloads its private uv.
#[derive(Clone, Copy, Debug)]
struct TerminalUvConsent;

impl UvDownloadConsent for TerminalUvConsent {
    fn allow_download(&self, version: &str, destination: &Path) -> bool {
        // Version 0.4 gates purely on the two streams (`src/skit/uvman.py:72-73`). `--no-input` is
        // deliberately not part of the gate: the interactive launch form sets that flag itself
        // before it hands the run on, so keying on it would silence the question on the one path
        // that has a user in front of it.
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return true;
        }
        // The question goes to stderr because stdout belongs to the script, and it ends with one
        // space rather than a newline (`src/skit/uvman.py:74-82`).
        eprint!(
            "{} ",
            skit_i18n::format_text(
                crate::cli::active_locale(),
                "skit needs Astral's uv to run Python scripts, but it wasn't found on this system. Download uv {} into skit's private directory ({})? This won't touch your PATH or global environment. [Y/n]",
                &[&version, &destination.display()],
            )
        );
        let _ = io::stderr().flush();
        let mut answer = String::new();
        let read = io::stdin().read_line(&mut answer).unwrap_or(0);
        consent_from_answer((read > 0).then_some(answer.as_str()))
    }
}

/// Read one consent answer.
///
/// `None` is end of input, which counts as consent (`src/skit/uvman.py:85-86`). Only an explicit
/// no declines, whatever its spacing or case (`src/skit/uvman.py:88`).
fn consent_from_answer(answer: Option<&str>) -> bool {
    answer.is_none_or(|answer| !matches!(answer.trim().to_lowercase().as_str(), "n" | "no"))
}

/// Pin one resolved interpreter path in the in-memory settings and metadata.
fn pin_interpreter(settings: &mut EntrySettings, entry: &mut Entry, path: &Path) {
    settings.interpreter = path.display().to_string();
    settings.write_to_meta(&mut entry.meta);
}

fn sweep_stale_launch_snapshots(entry_dir: &Path, include_launch_snapshots: bool) {
    if !include_launch_snapshots {
        return;
    }
    // A directory skit cannot list holds nothing skit owns.
    let items = fs::read_dir(entry_dir).into_iter().flatten();
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(24 * 60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    for item in items.flatten() {
        let path = item.path();
        let is_staged = item
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(".run-"));
        let is_stale_file = item
            .metadata()
            .ok()
            .filter(|metadata| metadata.is_file())
            .and_then(|metadata| metadata.modified().ok())
            .is_some_and(|modified| modified <= cutoff);
        if is_staged && is_stale_file {
            let _ = fs::remove_file(path);
        }
    }
}

#[derive(Debug)]
struct StagedSource {
    path: PathBuf,
    _file: tempfile::NamedTempFile,
}

fn configured_runner(config: &FileConfigStore, name: &str) -> Result<PromptRunner, RunError> {
    let runners = config.runners()?;
    let known = runners
        .iter()
        .map(|runner| runner.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let runner = runners
        .into_iter()
        .find(|runner| runner.name == name)
        .ok_or_else(|| RunError::RunnerNotFound {
            name: name.to_owned(),
            known: if known.is_empty() {
                "—".to_owned()
            } else {
                known
            },
        })?;
    Ok(PromptRunner {
        name: runner.name,
        argv: runner.argv,
    })
}

fn resolve_runner(
    config: &FileConfigStore,
    state_dir: &Path,
    runner_override: Option<&str>,
    runner_pin: &str,
    runner_was_picked: bool,
) -> Result<Option<PromptRunner>, RunError> {
    let picked = runner_override.map(str::trim);
    let name = picked.or_else(|| (!runner_pin.is_empty()).then_some(runner_pin));
    let runner = name
        .map(|name| configured_runner(config, name))
        .transpose()?;
    if let Some(name) = picked.filter(|_| runner_was_picked) {
        PromptSelectionService::new(FilePromptSelectionStore::new(state_dir))
            .remember_runner(name)?;
    }
    Ok(runner)
}

pub(crate) fn token_context() -> TokenContext {
    let utc = OffsetDateTime::now_utc();
    let local = UtcOffset::current_local_offset().map_or(utc, |offset| utc.to_offset(offset));
    let time = local.time();
    TokenContext {
        cwd: env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .display()
            .to_string(),
        home: home_dir().map(|path| path.display().to_string()),
        env: env::vars().collect(),
        today: local.date().to_string(),
        now: format!(
            "{:02}-{:02}-{:02}",
            time.hour(),
            time.minute(),
            time.second()
        ),
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn resolve_state_dir() -> Result<PathBuf, RunError> {
    if let Some(path) = env::var_os("SKIT_STATE_DIR") {
        return Ok(PathBuf::from(path));
    }
    platform_state_dir().ok_or(RunError::StateDirectoryUnavailable)
}

fn resolve_config_dir() -> Result<PathBuf, RunError> {
    if let Some(path) = env::var_os("SKIT_CONFIG_DIR") {
        return Ok(PathBuf::from(path));
    }
    platform_config_dir().ok_or(RunError::ConfigDirectoryUnavailable)
}

#[cfg(target_os = "windows")]
fn platform_state_dir() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .map(PathBuf::from)
        .map(|path| path.join("skit"))
}

#[cfg(target_os = "windows")]
fn platform_config_dir() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("skit"))
}

#[cfg(target_os = "macos")]
fn platform_config_dir() -> Option<PathBuf> {
    platform_state_dir()
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
fn platform_config_dir() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn platform_state_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).map(|path| {
        path.join("Library")
            .join("Application Support")
            .join("skit")
    })
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

#[cfg(not(any(unix, target_os = "windows")))]
fn platform_state_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use skit_application::{
        CreateEntry, EntryMutationRepository as _, EntryPayload, RepositoryError,
        SourcePermissions, payload_stored_name, run_inputs::RunInputError, tokens::TokenError,
    };
    use skit_domain::{
        EntryKind, EntryMeta, Slug,
        parameters::{ParamDecl, ParameterBinding, ParameterDelivery},
    };
    use skit_runtime::{DependencyError, LaunchError};
    use tempfile::TempDir;

    fn entry(kind: &str, interpreter: &str) -> Entry {
        let mut meta = EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap());
        let settings = EntrySettings {
            interpreter: interpreter.to_owned(),
            ..EntrySettings::default()
        };
        settings.write_to_meta(&mut meta);
        Entry {
            slug: Slug::parse("demo").unwrap(),
            meta,
        }
    }

    #[test]
    fn runtime_defaults_apply_only_to_unpinned_javascript_entries() {
        let config = BTreeMap::from([
            ("shell.bash_path".to_owned(), "/opt/bash".to_owned()),
            ("js.runner".to_owned(), "bun".to_owned()),
        ]);
        let shell = apply_runtime_defaults(entry("shell", ""), &config);
        let javascript = apply_runtime_defaults(entry("js", ""), &config);
        let pinned = apply_runtime_defaults(entry("ts", "deno"), &config);

        assert!(EntrySettings::from_meta(&shell.meta).interpreter.is_empty());
        assert_eq!(
            EntrySettings::from_meta(&javascript.meta).interpreter,
            "bun"
        );
        assert_eq!(EntrySettings::from_meta(&pinned.meta).interpreter, "deno");
    }

    #[test]
    fn only_a_nonempty_secret_prompt_placeholder_crosses_the_agent_warning_boundary() {
        let prompt = entry("prompt", "");
        let command = entry("command", "");
        let mut field = ParamDecl::new("api_key");
        field.delivery = ParameterDelivery::Placeholder;
        field.secret = true;
        let sent = skit_application::delivery::Assembly {
            command_values: BTreeMap::from([("api_key".to_owned(), "hunter2".to_owned())]),
            ..Default::default()
        };

        assert!(prompt_sends_secret(
            &prompt,
            std::slice::from_ref(&field),
            &sent
        ));
        assert!(!prompt_sends_secret(
            &command,
            std::slice::from_ref(&field),
            &sent
        ));

        field.delivery = ParameterDelivery::Flag;
        assert!(!prompt_sends_secret(&prompt, &[field.clone()], &sent));
        field.delivery = ParameterDelivery::Placeholder;
        field.secret = false;
        assert!(!prompt_sends_secret(&prompt, &[field.clone()], &sent));
        field.secret = true;
        assert!(!prompt_sends_secret(
            &prompt,
            &[field],
            &skit_application::delivery::Assembly {
                command_values: BTreeMap::from([("api_key".to_owned(), String::new())]),
                ..Default::default()
            }
        ));
    }

    #[test]
    fn run_error_codes_and_set_parser_cover_each_contract_class() {
        let errors = [
            (
                RunError::Repository(RepositoryError::NotFound {
                    query: "missing".to_owned(),
                }),
                127,
            ),
            (
                RunError::Launch(LaunchError::ProgramNotFound {
                    name: "runtime".to_owned(),
                }),
                126,
            ),
            (
                RunError::PromptBodyMissing {
                    path: "/data/prompt.md".to_owned(),
                },
                127,
            ),
            (
                RunError::Inputs(RunInputError::ExtraToken(TokenError::MissingEnvironment {
                    name: "MISSING".to_owned(),
                    token: "{env:MISSING}".to_owned(),
                })),
                125,
            ),
            (
                RunError::InvalidSet {
                    items: "bad".to_owned(),
                },
                2,
            ),
            (
                RunError::UnknownSet {
                    names: "bad".to_owned(),
                    valid: "good".to_owned(),
                },
                2,
            ),
            (
                RunError::PresetNotFound {
                    name: "bad".to_owned(),
                },
                2,
            ),
            (
                RunError::RawUnsupported {
                    kind: "prompt".to_owned(),
                },
                2,
            ),
            (RunError::RawConflict, 2),
            (
                RunError::Dependencies(DependencyError::InstallerNotFound {
                    name: "npm".to_owned(),
                }),
                126,
            ),
            (
                RunError::Dependencies(DependencyError::InstallerStartFailed {
                    installer: "npm".to_owned(),
                    reason: "permission denied".to_owned(),
                }),
                126,
            ),
            (
                RunError::Dependencies(DependencyError::ClearFailed {
                    item: "node_modules".to_owned(),
                    reason: "locked".to_owned(),
                }),
                126,
            ),
            (
                RunError::Dependencies(DependencyError::CopyStorageRequired),
                125,
            ),
            (
                RunError::RunnerNotFound {
                    name: "agent".to_owned(),
                    known: "claude, codex".to_owned(),
                },
                126,
            ),
            (RunError::NoRunnersConfigured, 126),
            (
                RunError::RunnerRequired {
                    name: "Review".to_owned(),
                },
                126,
            ),
            (RunError::StateDirectoryUnavailable, 125),
            (RunError::ConfigDirectoryUnavailable, 125),
            (
                RunError::InjectionDrift {
                    parameter: "WIDTH".to_owned(),
                    entry: "demo".to_owned(),
                },
                125,
            ),
            (
                RunError::InjectedCopy {
                    detail: Message::new("invalid copy"),
                },
                125,
            ),
            (
                RunError::InjectionSemanticDrift {
                    detail: LanguageError::SourceChanged.message(),
                    entry: "demo".to_owned(),
                },
                125,
            ),
        ];
        for (error, expected) in errors {
            assert_eq!(error.exit_code(), expected, "error={error}");
        }

        let declarations = [ParamDecl::new("name")];
        let mut values = BTreeMap::new();
        assert!(apply_sets(&declarations, &["bad".to_owned()], &mut values).is_err());
        assert!(apply_sets(&declarations, &["=bad".to_owned()], &mut values).is_err());
        assert!(apply_sets(&declarations, &["other=x".to_owned()], &mut values).is_err());
        apply_sets(&declarations, &["name=value=tail".to_owned()], &mut values).unwrap();
        assert_eq!(values["name"], "value=tail");
        apply_sets(&declarations, &[" name = padded".to_owned()], &mut values).unwrap();
        assert_eq!(values["name"], " padded");

        let unchanged = values.clone();
        let malformed = apply_sets(
            &declarations,
            &["name=changed".to_owned(), "broken".to_owned()],
            &mut values,
        )
        .unwrap_err();
        assert!(matches!(
            malformed,
            RunError::InvalidSet { items } if items == "broken"
        ));
        assert_eq!(values, unchanged);

        assert!(matches!(
            map_injection_language_error(
                LanguageError::BindingNotFound {
                    name: "WIDTH".to_owned(),
                },
                "demo",
            ),
            RunError::InjectionDrift { parameter, entry }
                if parameter == "WIDTH" && entry == "demo"
        ));
        assert!(matches!(
            map_injection_language_error(
                LanguageError::InjectedSourceInvalid {
                    kind: skit_language::InjectedSourceKind::JavaScript,
                },
                "demo",
            ),
            RunError::InjectedCopy { .. }
        ));
        assert!(matches!(
            map_injection_language_error(LanguageError::SourceChanged, "demo"),
            RunError::InjectionSemanticDrift { entry, .. } if entry == "demo"
        ));
        assert!(matches!(
            map_injection_language_error(
                LanguageError::InvalidValue {
                    name: "WIDTH".to_owned(),
                    value: "bad".to_owned(),
                    parameter_type: skit_domain::parameters::ParameterType::Int,
                },
                "demo",
            ),
            RunError::Language(LanguageError::InvalidValue { .. })
        ));

        let unknown = apply_sets(
            &declarations,
            &["z=1".to_owned(), "a=2".to_owned(), "z=3".to_owned()],
            &mut values,
        )
        .unwrap_err();
        assert!(matches!(
            unknown,
            RunError::UnknownSet { names, valid } if names == "a, z" && valid == "name"
        ));
        assert_eq!(values, unchanged);
    }

    #[test]
    fn source_staging_and_prompt_rendering_keep_execution_files_private() {
        assert_eq!(
            render_prompt_body("Hello {{name}}", &BTreeMap::new(), false),
            "Hello {{name}}"
        );
        assert_eq!(
            render_prompt_body(
                "Hello {{name}} and {{other}}",
                &BTreeMap::from([
                    ("name".to_owned(), "Ada".to_owned()),
                    ("other".to_owned(), "Grace".to_owned()),
                ]),
                true,
            ),
            "Hello Ada and Grace"
        );

        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let directory = root.path().join("scripts/demo");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("script.sh"), "NAME=old\n").unwrap();
        let stale = directory.join(".injected-stale.sh");
        let live = directory.join(".injected-live.sh");
        let stale_launch_snapshot = directory.join(".run-stale.sh");
        fs::write(&stale, "stale secret").unwrap();
        fs::write(&live, "live secret").unwrap();
        fs::write(&stale_launch_snapshot, "stale launch bytes").unwrap();
        let stale_file = fs::File::options().write(true).open(&stale).unwrap();
        stale_file
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH)
                    .set_accessed(SystemTime::UNIX_EPOCH),
            )
            .unwrap();
        let stale_launch_file = fs::File::options()
            .write(true)
            .open(&stale_launch_snapshot)
            .unwrap();
        stale_launch_file
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH)
                    .set_accessed(SystemTime::UNIX_EPOCH),
            )
            .unwrap();
        let mut shell = entry("shell", "bash");
        let mut declaration = ParamDecl::new("NAME");
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        sweep_injected_launch_sources(&store, &shell);
        sweep_stale_launch_snapshots(&directory, true);
        let staged = stage_injected_source(
            &store,
            &shell,
            "NAME=old\n",
            std::slice::from_ref(&declaration),
            &skit_application::delivery::Assembly {
                inject_values: BTreeMap::from([("NAME".to_owned(), "new".to_owned())]),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(!stale.exists());
        assert!(!stale_launch_snapshot.exists());
        assert!(live.exists());
        assert!(!staged.path.starts_with(&directory));
        assert!(
            staged
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".injected-")
        );
        assert_eq!(fs::read_to_string(&staged.path).unwrap(), "NAME='new'\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&staged.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let staged_path = staged.path.clone();
        drop(staged);
        assert!(!staged_path.exists());

        let not_a_directory = root.path().join("not-a-directory");
        fs::write(&not_a_directory, "occupied").unwrap();
        let fallback = new_injected_file(&not_a_directory, ".js", true).unwrap();
        assert!(
            !fallback.path().starts_with(&not_a_directory),
            "an unavailable entry-directory target must fall back to the OS private temp directory"
        );
        drop(fallback);

        // A later run is the crash-recovery boundary even when it needs no injection itself.
        let recovered = directory.join(".injected-crashed.sh");
        fs::write(&recovered, "crashed secret").unwrap();
        let recovered_file = fs::File::options().write(true).open(&recovered).unwrap();
        recovered_file
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH)
                    .set_accessed(SystemTime::UNIX_EPOCH),
            )
            .unwrap();
        sweep_injected_launch_sources(&store, &shell);
        assert!(
            stage_injected_source(
                &store,
                &shell,
                "NAME=old\n",
                std::slice::from_ref(&declaration),
                &skit_application::delivery::Assembly::default(),
            )
            .unwrap()
            .is_none()
        );
        assert!(!recovered.exists());

        let mut javascript = entry("js", "node");
        fs::write(directory.join("script.js"), "const NAME = 'old';\n").unwrap();
        let staged = stage_injected_source(
            &store,
            &javascript,
            "const NAME = 'old';\n",
            std::slice::from_ref(&declaration),
            &skit_application::delivery::Assembly {
                inject_values: BTreeMap::from([("NAME".to_owned(), "new".to_owned())]),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(
            !staged.path.starts_with(&directory),
            "dependency-free JavaScript must not persist its injected values"
        );
        drop(staged);

        let mut settings = EntrySettings::from_meta(&javascript.meta);
        settings.dependencies = vec!["chalk".to_owned()];
        settings.write_to_meta(&mut javascript.meta);
        let staged = stage_injected_source(
            &store,
            &javascript,
            "const NAME = 'old';\n",
            &[declaration],
            &skit_application::delivery::Assembly {
                inject_values: BTreeMap::from([("NAME".to_owned(), "new".to_owned())]),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            staged.path.parent(),
            Some(directory.as_path()),
            "npm-backed JavaScript must remain adjacent to node_modules"
        );
        drop(staged);

        for fail_after_write in [false, true] {
            let file = new_injected_file(&directory, ".sh", true).unwrap();
            let failed_path = file.path().to_path_buf();
            let result = finish_staged_source(file, b"SECRET='plaintext'\n", |file, bytes| {
                if fail_after_write {
                    file.write_all(bytes)?;
                } else {
                    file.write_all(&bytes[..6])?;
                }
                Err(io::Error::other(if fail_after_write {
                    "simulated sync failure"
                } else {
                    "simulated write failure"
                }))
            });
            assert!(matches!(result, Err(RunError::Stage { .. })));
            assert!(!failed_path.exists());
        }
        assert_eq!(
            source_text(
                &store,
                &entry("command", ""),
                &EntrySettings {
                    template: "echo ok".to_owned(),
                    ..EntrySettings::default()
                }
            )
            .unwrap(),
            "echo ok"
        );
        let executable_path = directory.join("tool");
        fs::write(&executable_path, b"executable bytes").unwrap();
        let mut executable = entry("exe", "");
        executable.meta.mode = skit_domain::StorageMode::Reference;
        executable.meta.source = executable_path.display().to_string();
        assert_eq!(
            source_text(&store, &executable, &EntrySettings::default()).unwrap(),
            ""
        );

        fs::write(directory.join("prompt.md"), [0xff]).unwrap();
        shell.meta.kind = EntryKind::parse("prompt").unwrap();
        assert!(matches!(
            source_text(&store, &shell, &EntrySettings::default()),
            Err(RunError::Encoding(_))
        ));
        assert!(matches!(
            read_bytes(&directory.join("missing")),
            Err(RunError::Read { .. })
        ));
    }

    #[test]
    fn prompt_missing_drift_never_becomes_a_requested_source_binding() {
        assert_eq!(
            requested_drifted_parameter(
                &["name=value".to_owned()],
                &[FormDrift::PromptMissing {
                    names: vec!["name".to_owned()],
                }],
            ),
            None
        );
    }

    #[test]
    fn staged_source_creation_reports_the_entry_path_after_both_locations_fail() {
        let root = TempDir::new().unwrap();
        let events = RefCell::new(Vec::new());
        for adjacent in [true, false] {
            events.borrow_mut().clear();
            let error = new_injected_file_with_ops(
                root.path(),
                ".js",
                adjacent,
                |_, _| {
                    events.borrow_mut().push("entry");
                    Err(io::Error::other("entry temp failure"))
                },
                |_| {
                    events.borrow_mut().push("system");
                    Err(io::Error::other("system temp failure"))
                },
            )
            .unwrap_err();

            assert!(matches!(
                error,
                RunError::Stage { ref path, .. } if path == &root.path().display().to_string()
            ));
            assert_eq!(injected_stage_failure_path(&error), None);
            assert_eq!(
                events.borrow().as_slice(),
                if adjacent {
                    ["entry", "system"].as_slice()
                } else {
                    ["system", "entry"].as_slice()
                }
            );
        }
        assert!(root.path().read_dir().unwrap().next().is_none());
    }

    fn injected_stage_failure_path(error: &RunError) -> Option<PathBuf> {
        match error {
            RunError::Stage { path, source }
                if source.to_string() == "injected staged-source write failure" =>
            {
                Some(PathBuf::from(path))
            }
            _ => None,
        }
    }

    #[test]
    fn test_write_injected_cleanup_on_error() {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        fs::write(
            config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        let marker = data.path().join("child-launched");
        let marker_literal = serde_json::to_string(&marker.display().to_string()).unwrap();
        let source = format!(
            "from pathlib import Path\nPath({marker_literal}).write_text('launched')\nCITY = 'Taipei'\nprint(CITY)\n"
        );
        let mut city = ParamDecl::new("CITY");
        city.binding = ParameterBinding::Const;
        city.delivery = ParameterDelivery::Inject;
        let managed =
            skit_language::write_managed_params("python", &source, std::slice::from_ref(&city))
                .unwrap();
        let kind = EntryKind::parse("python").unwrap();
        let python = SystemProbe
            .find_program("python3")
            .or_else(|| SystemProbe.find_program("python"))
            .expect("the frozen Shim cleanup contract requires Python on this platform");
        let store = FileStore::new(data.path());
        let entry = store
            .create(CreateEntry {
                name: "Cleanup".to_owned(),
                kind: kind.clone(),
                mode: skit_domain::StorageMode::Copy,
                source: "cleanup.py".to_owned(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: managed.into_bytes(),
                    stored_name: Some(payload_stored_name(&kind, Path::new("cleanup.py"))),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    interpreter: python.display().to_string(),
                    ..EntrySettings::default()
                },
            })
            .unwrap();
        let entry_dir = store.entry_dir_path(&entry.slug);
        let source_path = store.payload_path(&entry).unwrap();
        let source_before = fs::read(&source_path).unwrap();
        let meta_before = fs::read(entry_dir.join("meta.toml")).unwrap();
        let config_before = fs::read(config.path().join("config.toml")).unwrap();
        assert!(fs::read_dir(state.path()).unwrap().next().is_none());

        let _fault = StageWriteFaultGuard::for_current_thread();
        let service = LibraryService::new(store.clone());
        let error = run_with_roots(
            &service,
            &store,
            state.path(),
            config.path(),
            RunArgs {
                selector: "cleanup".to_owned(),
                values: vec!["CITY=Kaohsiung".to_owned()],
                preset: None,
                save_preset: None,
                runner: None,
                runner_was_picked: false,
                dry_run: false,
                no_input: true,
                plain: true,
                raw: false,
                forget_args: false,
                extra_args: Vec::new(),
            },
        )
        .unwrap_err();
        let failed_path = injected_stage_failure_path(&error)
            .expect("the injected staged-source write fault must keep its path");
        assert_eq!(error.exit_code(), 125);
        assert!(
            !marker.exists(),
            "the Python child launched after a stage failure"
        );
        assert!(!failed_path.exists(), "the failed staged source survived");
        assert!(
            fs::read_dir(&entry_dir)
                .unwrap()
                .filter_map(Result::ok)
                .all(|item| !item.file_name().to_string_lossy().starts_with(".injected-"))
        );
        assert_eq!(fs::read(source_path).unwrap(), source_before);
        assert_eq!(fs::read(entry_dir.join("meta.toml")).unwrap(), meta_before);
        assert_eq!(
            fs::read(config.path().join("config.toml")).unwrap(),
            config_before
        );
        assert!(fs::read_dir(state.path()).unwrap().next().is_none());
    }

    #[test]
    fn test_build_sweeps_aged_injected_leftovers_but_not_fresh_ones() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let directory = root.path().join("scripts/demo");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("script.js"), "console.log('ok');\n").unwrap();
        let aged = directory.join(".injected-dead.js");
        let fresh = directory.join(".injected-live.js");
        fs::write(&aged, "old secret").unwrap();
        fs::write(&fresh, "live secret").unwrap();
        let old = SystemTime::now()
            .checked_sub(Duration::from_secs(2 * 60 * 60))
            .unwrap();
        fs::File::options()
            .write(true)
            .open(&aged)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();

        sweep_injected_launch_sources(&store, &entry("js", "node"));

        assert!(!aged.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn source_and_runner_adapters_report_missing_invalid_and_supported_payloads() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());

        let mut command = entry("command", "");
        EntrySettings {
            template: "echo ok".to_owned(),
            ..EntrySettings::default()
        }
        .write_to_meta(&mut command.meta);
        assert_eq!(
            source_text(&store, &command, &EntrySettings::from_meta(&command.meta)).unwrap(),
            "echo ok"
        );
        let executable_path = root.path().join("tool");
        fs::write(&executable_path, b"executable bytes").unwrap();
        let mut executable = entry("exe", "");
        executable.meta.mode = skit_domain::StorageMode::Reference;
        executable.meta.source = executable_path.display().to_string();
        assert_eq!(
            source_text(&store, &executable, &EntrySettings::default()).unwrap(),
            ""
        );

        let mut shell = entry("shell", "bash");
        shell.meta.mode = skit_domain::StorageMode::Reference;
        shell.meta.source = root.path().join("missing.sh").display().to_string();
        let error = source_text(&store, &shell, &EntrySettings::default()).unwrap_err();
        assert!(matches!(error, RunError::Read { .. }));

        let prompt = entry("prompt", "");
        let prompt_dir = store.entry_dir_path(&prompt.slug);
        assert!(matches!(
            launch_payload_path(&store, &prompt).unwrap_err(),
            RunError::Repository(_)
        ));
        fs::create_dir_all(&prompt_dir).unwrap();
        fs::write(prompt_dir.join("prompt.md"), [0xff]).unwrap();
        assert!(matches!(
            source_text(&store, &prompt, &EntrySettings::default()).unwrap_err(),
            RunError::Encoding(_)
        ));
        let missing_prompt = prompt_dir.join("missing.md");
        assert!(matches!(
            read_prompt_bytes(
                &missing_prompt,
                Err(io::Error::new(io::ErrorKind::NotFound, "test missing"))
            ),
            Err(RunError::PromptBodyMissing { path })
                if path == missing_prompt.display().to_string()
        ));
        assert!(matches!(
            read_prompt_bytes(
                &missing_prompt,
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "test permission failure"
                ))
            ),
            Err(RunError::Read { path, .. }) if path == missing_prompt.display().to_string()
        ));

        let generic = entry("shell", "bash");
        let generic_dir = store.entry_dir_path(&generic.slug);
        fs::write(generic_dir.join("script.sh"), [0xff]).unwrap();
        assert_eq!(
            source_text(&store, &generic, &EntrySettings::default()).unwrap(),
            ""
        );

        let config = root.path().join("config");
        let config_store = FileConfigStore::new(&config);
        config_store
            .set_runner(
                skit_store::PromptRunner {
                    name: "local".to_owned(),
                    argv: vec!["printf".to_owned(), "{{prompt}}".to_owned()],
                },
                true,
            )
            .unwrap();
        assert_eq!(
            configured_runner(&config_store, "local").unwrap().name,
            "local"
        );
        assert!(matches!(
            configured_runner(&config_store, "missing").unwrap_err(),
            RunError::RunnerNotFound { .. }
        ));

        let state_dir = root.path().join("state");
        let selection = PromptSelectionService::new(FilePromptSelectionStore::new(&state_dir));
        let defaulted = resolve_runner(&config_store, &state_dir, Some("local"), "", false)
            .unwrap()
            .unwrap();
        assert_eq!(defaulted.name, "local");
        assert_eq!(selection.last_runner(), "");

        let picked = resolve_runner(&config_store, &state_dir, Some(" local "), "", true)
            .unwrap()
            .unwrap();
        assert_eq!(picked.name, "local");
        assert_eq!(selection.last_runner(), "local");

        selection.remember_runner("prior").unwrap();
        assert!(matches!(
            resolve_runner(&config_store, &state_dir, Some(" missing "), "", true),
            Err(RunError::RunnerNotFound { .. })
        ));
        assert_eq!(selection.last_runner(), "prior");

        let pinned = resolve_runner(&config_store, &state_dir, None, "local", false)
            .unwrap()
            .unwrap();
        assert_eq!(pinned.name, "local");
        assert_eq!(selection.last_runner(), "prior");
    }

    #[test]
    fn platform_context_paths_are_explicit() {
        assert!(platform_state_dir().is_some());
        assert!(platform_config_dir().is_some());
        assert!(resolve_state_dir().is_ok());
        assert!(resolve_config_dir().is_ok());
        assert!(home_dir().is_some());
        let context = token_context();
        assert!(!context.cwd.is_empty());
        assert!(!context.today.is_empty());
        assert!(!context.now.is_empty());
    }
}

#[cfg(test)]
mod bootstrap_tests {
    use std::{
        cell::RefCell,
        path::{Path, PathBuf},
    };

    use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
    use skit_runtime::{AllowUvDownload, UvBootstrapError, UvDownloadConsent};

    use super::{RunError, bootstrap_private_uv, consent_from_answer};

    #[derive(Debug)]
    struct RecordedConsent {
        allow: bool,
        asked: RefCell<Option<(String, PathBuf)>>,
    }

    impl UvDownloadConsent for RecordedConsent {
        fn allow_download(&self, version: &str, destination: &Path) -> bool {
            *self.asked.borrow_mut() = Some((version.to_owned(), destination.to_path_buf()));
            self.allow
        }
    }

    fn python_entry() -> Entry {
        Entry {
            slug: Slug::parse("demo").unwrap(),
            meta: EntryMeta::minimal("Demo", EntryKind::parse("python").unwrap()),
        }
    }

    fn successful_test_uv_install(
        _data_dir: &Path,
        _mirror_base: Option<&str>,
    ) -> Result<PathBuf, UvBootstrapError> {
        Ok(PathBuf::from("/data/bin/uv"))
    }

    #[test]
    fn a_completed_bootstrap_pins_the_installed_uv_in_settings_and_metadata() {
        let mut entry = python_entry();
        let mut settings = EntrySettings::default();
        let installed = PathBuf::from("/data/bin/uv");

        bootstrap_private_uv(
            &mut settings,
            &mut entry,
            Path::new("/data"),
            Some("https://mirror.example/uv"),
            &AllowUvDownload,
            |data_dir, mirror_base| {
                assert_eq!(data_dir, Path::new("/data"));
                assert_eq!(mirror_base, Some("https://mirror.example/uv"));
                Ok(PathBuf::from("/data/bin/uv"))
            },
        )
        .unwrap();

        assert_eq!(settings.interpreter, installed.display().to_string());
        assert_eq!(
            EntrySettings::from_meta(&entry.meta).interpreter,
            installed.display().to_string()
        );
    }

    /// Version 0.4 asks before it downloads, and names the version and the private directory
    /// (`src/skit/uvman.py:251` and `src/skit/uvman.py:74-81`).
    #[test]
    fn consent_is_asked_with_the_version_and_the_private_directory() {
        let consent = RecordedConsent {
            allow: true,
            asked: RefCell::new(None),
        };
        bootstrap_private_uv(
            &mut EntrySettings::default(),
            &mut python_entry(),
            Path::new("/data"),
            None,
            &consent,
            successful_test_uv_install,
        )
        .unwrap();

        let (version, destination) = consent
            .asked
            .borrow_mut()
            .take()
            .expect("consent was not asked");
        assert_eq!(version, skit_runtime::UV_VERSION);
        assert_eq!(destination, Path::new("/data").join("bin"));
    }

    /// Version 0.4 treats end of input as consent and refuses only on an explicit no
    /// (`src/skit/uvman.py:85-88`).
    #[test]
    fn test_consent_interactive_answers() {
        assert!(consent_from_answer(None));
        for consenting in ["", "\n", " ", "y", "Y", "yes", "sure", "nope"] {
            assert!(consent_from_answer(Some(consenting)), "{consenting:?}");
        }
        for refusing in ["n", "N", "no", "NO", "  no  ", "No\n"] {
            assert!(!consent_from_answer(Some(refusing)), "{refusing:?}");
        }
    }

    /// A refusal downloads nothing, keeps the entry untouched, and carries the version 0.4
    /// self-install guidance (`src/skit/uvman.py:252-256`).
    #[test]
    fn a_refused_download_never_reaches_the_installer_and_leaves_no_pin() {
        let consent = RecordedConsent {
            allow: false,
            asked: RefCell::new(None),
        };
        let mut entry = python_entry();
        let mut settings = EntrySettings::default();

        let error = bootstrap_private_uv(
            &mut settings,
            &mut entry,
            Path::new("/data"),
            None,
            &consent,
            successful_test_uv_install,
        )
        .expect_err("a refusal must fail the run");

        assert!(matches!(error, RunError::Uv(UvBootstrapError::Declined),));
        assert!(settings.interpreter.is_empty());
        assert!(EntrySettings::from_meta(&entry.meta).interpreter.is_empty());
        assert_eq!(
            error.to_string(),
            "Download declined. Install uv yourself (https://docs.astral.sh/uv/getting-started/installation/) and skit will pick it up automatically."
        );
        // Version 0.4 turns every uv bootstrap failure into a launch failure
        // (`src/skit/langs/launch.py:57-63`), which exits 125 (`src/skit/flows.py:868`).
        assert_eq!(error.exit_code(), 125);
    }
}

#[cfg(test)]
mod localization_tests {
    use std::io;

    use skit_application::{
        RepositoryError, form_state::StateWriteError, run_inputs::RunInputError, tokens::TokenError,
    };
    use skit_i18n::{Locale, Localize as _, Message};
    use skit_language::LanguageError;
    use skit_runtime::{DependencyError, LaunchError, UvBootstrapError};
    use skit_store::ConfigError;

    use super::RunError;

    /// Check that English text does not drift and that each locale fills every hole.
    fn assert_localized(error: &RunError, values: &[&str]) {
        let message = error.message();
        assert_eq!(error.to_string(), message.localize(Locale::En));
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            let text = message.localize(locale);
            let template = message.template();
            assert!(!text.trim().is_empty(), "{template} is empty");
            assert!(!text.contains("{}"), "{template} kept an empty hole");
            for value in values {
                assert!(text.contains(value), "{text} lost the value {value}");
            }
        }
    }

    fn io_failure() -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, "permission denied")
    }

    #[test]
    fn every_run_error_localizes_and_keeps_its_values() {
        assert_localized(
            &RunError::Repository(RepositoryError::NotFound {
                query: "missing".to_owned(),
            }),
            &["missing"],
        );
        assert_localized(
            &RunError::State(StateWriteError::Encode {
                reason: "unsupported value".to_owned(),
            }),
            &["unsupported value"],
        );
        assert_localized(
            &RunError::Inputs(RunInputError::ExtraToken(TokenError::MissingEnvironment {
                name: "TAIL".to_owned(),
                token: "{env:TAIL}".to_owned(),
            })),
            &["TAIL"],
        );
        assert_localized(
            &RunError::Language(LanguageError::InvalidMetadata {
                reason: Message::new("tool is not a table"),
            }),
            &[],
        );
        assert_localized(
            &RunError::Launch(LaunchError::MissingNeed {
                name: "rsync".to_owned(),
            }),
            &["rsync"],
        );
        assert_localized(
            &RunError::Dependencies(DependencyError::InstallFailed {
                installer: "npm".to_owned(),
                exit_code: Some(23),
                detail: "package missing".to_owned(),
            }),
            &["npm", "package missing"],
        );
        assert_localized(
            &RunError::Dependencies(DependencyError::ClearFailed {
                item: "node_modules".to_owned(),
                reason: "locked".to_owned(),
            }),
            &["node_modules", "locked"],
        );
        assert_localized(
            &RunError::Uv(UvBootstrapError::Checksum {
                expected: "aaaa".to_owned(),
                actual: "bbbb".to_owned(),
            }),
            &["aaaa", "bbbb"],
        );
        assert_localized(
            &RunError::Config(ConfigError::Encode {
                reason: "unsupported value".to_owned(),
            }),
            &["unsupported value"],
        );
        assert_localized(
            &RunError::InvalidSet {
                items: "novalue".to_owned(),
            },
            &["novalue"],
        );
        assert_localized(
            &RunError::UnknownSet {
                names: "target".to_owned(),
                valid: "output".to_owned(),
            },
            &["target", "output"],
        );
        assert_localized(
            &RunError::PresetNotFound {
                name: "nightly".to_owned(),
            },
            &["nightly"],
        );
        assert_localized(
            &RunError::PresetWithoutFields {
                name: "No args".to_owned(),
            },
            &["No args"],
        );
        assert_localized(
            &RunError::Read {
                path: "/data/demo.py".to_owned(),
                source: io_failure(),
            },
            &["/data/demo.py", "permission denied"],
        );
        assert_localized(
            &RunError::PromptBodyMissing {
                path: "/data/prompt.md".to_owned(),
            },
            &["/data/prompt.md"],
        );
        assert_localized(
            &RunError::Encoding(
                skit_language::decode_prompt(&[0xff], "/data/demo.py")
                    .expect_err("fixture must be invalid"),
            ),
            &["/data/demo.py", "0"],
        );
        assert_localized(
            &RunError::Stage {
                path: "/tmp/staged.py".to_owned(),
                source: io_failure(),
            },
            &["/tmp/staged.py", "permission denied"],
        );
        assert_localized(&RunError::StateDirectoryUnavailable, &[]);
        assert_localized(
            &RunError::RunnerNotFound {
                name: "claude".to_owned(),
                known: "codex, amp".to_owned(),
            },
            &["claude", "codex, amp"],
        );
        assert_localized(&RunError::NoRunnersConfigured, &[]);
        assert_localized(
            &RunError::RunnerRequired {
                name: "Review".to_owned(),
            },
            &["Review"],
        );
        assert_localized(
            &RunError::RawUnsupported {
                kind: "prompt".to_owned(),
            },
            &["prompt"],
        );
        assert_localized(&RunError::RawConflict, &[]);
        assert_localized(&RunError::ConfigDirectoryUnavailable, &[]);
        assert_localized(
            &RunError::InjectionDrift {
                parameter: "WIDTH".to_owned(),
                entry: "demo".to_owned(),
            },
            &["WIDTH", "demo"],
        );
        assert_localized(
            &RunError::InjectedCopy {
                detail: Message::new("invalid copy"),
            },
            &["invalid copy"],
        );
        let semantic_drift = RunError::InjectionSemanticDrift {
            detail: LanguageError::SourceChanged.message(),
            entry: "demo".to_owned(),
        };
        assert_localized(&semantic_drift, &["demo"]);
        for (locale, expected) in [
            (
                Locale::En,
                "The script and its form definitions don't match anymore: source changed after semantic edit planning. Run `skit params demo --resync` to fix it.",
            ),
            (
                Locale::ZhCn,
                "脚本内容和表单定义对不上了：源文件在语义编辑规划后已更改。运行 `skit params demo --resync` 即可修复。",
            ),
            (
                Locale::ZhTw,
                "腳本內容和表單定義對不上了：來源在語義編輯規劃後已變更。執行 `skit params demo --resync` 即可修復。",
            ),
        ] {
            assert_eq!(semantic_drift.message().localize(locale), expected);
        }
    }
}
