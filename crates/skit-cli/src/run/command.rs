use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    fs::OpenOptions,
    io,
    io::{IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use clap::Args;
use clap_complete::ArgValueCandidates;
use skit_application::{
    LibraryService, RepositoryError, RepositoryOperation,
    delivery::{Assembly, transparency_messages},
    form_state::{FormStateService, StateWriteError, prefill},
    prompt_selection::PromptSelectionService,
    run_inputs::{RunInputError, assemble_run_inputs},
    tokens::TokenContext,
};
use skit_domain::{
    Entry, EntryId, EntrySettings,
    parameters::{ParamDecl, ParameterDelivery},
};
use skit_form::form_params;
use skit_i18n::{Localize, Message};
use skit_language::{LanguageError, inject_values_for_interpreter, render_prompt_body};
use skit_runtime::{
    DependencyError, LaunchError, LaunchPaths, LaunchWarning, ProgramProbe, PromptRunner,
    SystemDependencyCommandRunner, SystemProbe, UvBootstrapError, UvDownloadConsent,
    build_launch_plan, build_launch_preview, ensure_javascript_dependencies_for_module,
    ensure_managed_uv, execute_launch, javascript_module_type, managed_uv_path,
    resolve_javascript_runtime,
};
use skit_store::{
    ConfigError, FileConfigStore, FileFormStateStore, FileGlobExpander, FilePromptSelectionStore,
    FileStore, content_hash,
};
use thiserror::Error;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

use crate::cli::{entry_candidates, preset_candidates, runner_candidates};

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
    #[error("--set needs NAME=VALUE; got {value:?}")]
    InvalidSet { value: String },
    #[error("unknown parameter in --set: {name}")]
    UnknownSet { name: String },
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
    #[error("{path} is not valid UTF-8")]
    Encoding { path: String },
    #[error("could not write staged source {path}: {source}")]
    Stage {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("could not determine the platform state directory; set SKIT_STATE_DIR")]
    StateDirectoryUnavailable,
    #[error("prompt runner {name:?} is not configured")]
    RunnerNotFound { name: String },
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
            Self::InvalidSet { value } => {
                Message::new("--set needs NAME=VALUE; got {}").quoted(value)
            }
            Self::UnknownSet { name } => Message::new("unknown parameter in --set: {}").with(name),
            Self::PresetNotFound { name } => Message::new("preset {} does not exist").quoted(name),
            Self::PresetWithoutFields { name } => {
                Message::new("{} has no form fields, so there's nothing to save.").with(name)
            }
            Self::Read { path, source } => Message::new("could not read {}: {}")
                .with(path)
                .with(source),
            Self::Encoding { path } => Message::new("{} is not valid UTF-8").with(path),
            Self::Stage { path, source } => Message::new("could not write staged source {}: {}")
                .with(path)
                .with(source),
            Self::StateDirectoryUnavailable => {
                Message::new("could not determine the platform state directory; set SKIT_STATE_DIR")
            }
            Self::RunnerNotFound { name } => {
                Message::new("prompt runner {} is not configured").quoted(name)
            }
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
            Self::Dependencies(DependencyError::InstallerNotFound { .. })
            | Self::Dependencies(DependencyError::InstallFailed { .. })
            | Self::Dependencies(DependencyError::Io { .. })
            | Self::Dependencies(DependencyError::Rollback { .. }) => 126,
            // Version 0.4 wraps every uv bootstrap failure, refusal included, into a launch error
            // (`src/skit/langs/launch.py:57-63`), and a launch failure exits 125
            // (`src/skit/flows.py:868`).
            Self::Uv(_) => 125,
            Self::RunnerNotFound { .. } => 126,
            Self::State(_)
            | Self::Inputs(_)
            | Self::Language(_)
            | Self::Read { .. }
            | Self::Encoding { .. }
            | Self::Stage { .. }
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
    let mut entry = apply_runtime_defaults(held.clone(), &config.settings()?);
    let mut settings = EntrySettings::from_meta(&entry.meta);
    let state = FormStateService::new(FileFormStateStore::new(state_dir));
    let saved = state.load(&entry.slug);
    let base_environment = env::vars().collect::<BTreeMap<_, _>>();
    let mirror_environment = config.mirror_environment(&base_environment)?;

    let (source, expected_source_hash) = source_snapshot(data_store, &entry, &settings)?;
    let declarations = if args.raw {
        Vec::new()
    } else {
        form_params(entry.meta.kind.as_str(), &source, &settings)
    };
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
    let assembly = if args.raw {
        skit_application::delivery::Assembly {
            args: extra_args.clone(),
            masked_args: extra_args.clone(),
            ..Default::default()
        }
    } else {
        assemble_run_inputs(
            &declarations,
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
    if !args.dry_run
        && matches!(entry.meta.kind.as_str(), "js" | "ts")
        && entry.meta.mode == skit_domain::StorageMode::Reference
        && !settings.dependencies.is_empty()
    {
        return Err(DependencyError::CopyStorageRequired.into());
    }

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
        data_store.payload_path(&entry)?
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
    if args.dry_run {
        let _ = build_launch_preview(
            &entry,
            &paths,
            &assembly,
            prompt_body.as_deref(),
            prompt_display_body.as_deref(),
            runner.as_ref(),
            &SystemProbe,
        )?;
    } else if !needs_uv_bootstrap {
        let _ = build_launch_plan(
            &entry,
            &paths,
            &assembly,
            prompt_body.as_deref(),
            runner.as_ref(),
            &SystemProbe,
        )?;
    }

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

    if !args.dry_run
        && matches!(entry.meta.kind.as_str(), "js" | "ts")
        && entry.meta.mode == skit_domain::StorageMode::Copy
    {
        let runtime = resolve_javascript_runtime(&settings, &SystemProbe)?;
        let entry_dir = data_store.entry_dir_path(&entry.slug);
        ensure_javascript_dependencies_for_module(
            &entry_dir,
            &runtime,
            &settings.dependencies,
            javascript_module_type(&entry.meta.source),
            &mirror_environment,
            &SystemProbe,
            &SystemDependencyCommandRunner,
        )?;
    }

    let staged = if args.dry_run {
        None
    } else {
        stage_injected_source(data_store, &entry, &source, &declarations, &assembly)?
    };
    let script = if let Some(staged) = staged.as_ref() {
        staged.path.clone()
    } else if entry.meta.kind.as_str() == "command" {
        PathBuf::new()
    } else if let Some(path) = prepared.as_ref().and_then(|launch| launch.payload_path()) {
        path.to_path_buf()
    } else {
        data_store.payload_path(&entry)?
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
        build_launch_plan(
            &entry,
            &paths,
            &assembly,
            prompt_body.as_deref(),
            runner.as_ref(),
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
        println!("{}", plan.display);
        return Ok(0);
    }

    for message in transparency_messages(&assembly, &plan.display) {
        println!("{}", message.localize(crate::cli::active_locale()));
    }
    let exit = execute_launch(&plan)?;
    let slug = &entry.slug;
    let fields = &declarations;
    if !args.raw {
        state.purge_secrets(&entry.slug, &declarations)?;
        state.save_last(slug, fields, Some(&raw_values), new_tail, false)?;
        if let Some(name) = args.save_preset.as_deref() {
            state.save_preset(&entry.slug, name, &declarations, &raw_values)?;
        }
    }
    let at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    let recorded_values = (!args.raw).then_some(&raw_values);
    state.record_run(slug, i64::from(exit), &at, fields, recorded_values)?;
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

fn apply_sets(
    declarations: &[skit_domain::parameters::ParamDecl],
    sets: &[String],
    values: &mut BTreeMap<String, String>,
) -> Result<(), RunError> {
    let names = declarations
        .iter()
        .map(|item| item.name.as_str())
        .collect::<BTreeSet<_>>();
    for item in sets {
        let Some((name, value)) = item.split_once('=') else {
            return Err(RunError::InvalidSet {
                value: item.clone(),
            });
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(RunError::InvalidSet {
                value: item.clone(),
            });
        }
        if !names.contains(name) {
            return Err(RunError::UnknownSet {
                name: name.to_owned(),
            });
        }
        values.insert(name.to_owned(), value.to_owned());
    }
    Ok(())
}

fn apply_runtime_defaults(mut entry: Entry, config: &BTreeMap<String, String>) -> Entry {
    let mut settings = EntrySettings::from_meta(&entry.meta);
    if settings.interpreter.is_empty() {
        let key = match entry.meta.kind.as_str() {
            "shell" => Some("shell.bash_path"),
            "js" | "ts" => Some("js.runner"),
            _ => None,
        };
        if let Some(value) = key.and_then(|key| config.get(key))
            && !value.is_empty()
        {
            settings.interpreter.clone_from(value);
            settings.write_to_meta(&mut entry.meta);
        }
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
            let path = store.payload_path(entry)?;
            let bytes = read_bytes(&path)?;
            let hash = content_hash(&bytes);
            let text = String::from_utf8(bytes).map_err(|_| RunError::Encoding {
                path: path.display().to_string(),
            })?;
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

fn read_bytes(path: &Path) -> Result<Vec<u8>, RunError> {
    fs::read(path).map_err(|source| RunError::Read {
        path: path.display().to_string(),
        source,
    })
}

fn stage_injected_source(
    store: &FileStore,
    entry: &Entry,
    source: &str,
    declarations: &[skit_domain::parameters::ParamDecl],
    assembly: &skit_application::delivery::Assembly,
) -> Result<Option<StagedSource>, RunError> {
    if assembly.inject_values.is_empty() {
        return Ok(None);
    }
    let kind = entry.meta.kind.as_str();
    let interpreter = EntrySettings::from_meta(&entry.meta).interpreter;
    let rewritten = inject_values_for_interpreter(
        kind,
        source,
        declarations,
        &assembly.inject_values,
        (!interpreter.is_empty()).then_some(interpreter.as_str()),
    )?;
    let entry_dir = store.entry_dir_path(&entry.slug);
    sweep_staged_sources(&entry_dir);
    let original = store.payload_path(entry)?;
    let suffix = original
        .extension()
        .and_then(|value| value.to_str())
        .map_or(String::new(), |value| format!(".{value}"));
    let path = entry_dir.join(format!(".run-{}{}", EntryId::generate().as_str(), suffix));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let stage_error = |source| RunError::Stage {
        path: path.display().to_string(),
        source,
    };
    let mut file = options.open(&path).map_err(stage_error)?;
    file.write_all(rewritten.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(stage_error)?;
    Ok(Some(StagedSource { path }))
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

fn sweep_staged_sources(entry_dir: &Path) {
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
}

impl Drop for StagedSource {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn configured_runner(config: &FileConfigStore, name: &str) -> Result<PromptRunner, RunError> {
    let runner = config
        .runners()?
        .into_iter()
        .find(|runner| runner.name == name)
        .ok_or_else(|| RunError::RunnerNotFound {
            name: name.to_owned(),
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
    use super::*;
    use skit_application::{RepositoryError, run_inputs::RunInputError, tokens::TokenError};
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
    fn runtime_defaults_apply_only_to_unpinned_shell_and_javascript_entries() {
        let config = BTreeMap::from([
            ("shell.bash_path".to_owned(), "/opt/bash".to_owned()),
            ("js.runner".to_owned(), "bun".to_owned()),
        ]);
        let shell = apply_runtime_defaults(entry("shell", ""), &config);
        let javascript = apply_runtime_defaults(entry("js", ""), &config);
        let pinned = apply_runtime_defaults(entry("ts", "deno"), &config);

        assert_eq!(
            EntrySettings::from_meta(&shell.meta).interpreter,
            "/opt/bash"
        );
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
                RunError::Inputs(RunInputError::ExtraToken(TokenError::MissingEnvironment {
                    name: "MISSING".to_owned(),
                    token: "{env:MISSING}".to_owned(),
                })),
                125,
            ),
            (
                RunError::InvalidSet {
                    value: "bad".to_owned(),
                },
                2,
            ),
            (
                RunError::UnknownSet {
                    name: "bad".to_owned(),
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
                RunError::Dependencies(DependencyError::CopyStorageRequired),
                125,
            ),
            (
                RunError::RunnerNotFound {
                    name: "agent".to_owned(),
                },
                126,
            ),
            (RunError::StateDirectoryUnavailable, 125),
            (RunError::ConfigDirectoryUnavailable, 125),
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
        let stale = directory.join(".run-stale.sh");
        let live = directory.join(".run-live.sh");
        fs::write(&stale, "stale secret").unwrap();
        fs::write(&live, "live secret").unwrap();
        let stale_file = fs::File::options().write(true).open(&stale).unwrap();
        stale_file
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
        assert!(live.exists());
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

        assert!(
            stage_injected_source(
                &store,
                &shell,
                "NAME=old\n",
                &[declaration],
                &skit_application::delivery::Assembly::default(),
            )
            .unwrap()
            .is_none()
        );
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
            Err(RunError::Encoding { .. })
        ));
        assert!(matches!(
            read_bytes(&directory.join("missing")),
            Err(RunError::Read { .. })
        ));
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
        fs::create_dir_all(&prompt_dir).unwrap();
        fs::write(prompt_dir.join("prompt.md"), [0xff]).unwrap();
        assert!(matches!(
            source_text(&store, &prompt, &EntrySettings::default()).unwrap_err(),
            RunError::Encoding { .. }
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
    fn platform_context_and_staging_failure_paths_are_explicit() {
        assert!(platform_state_dir().is_some());
        assert!(platform_config_dir().is_some());
        assert!(resolve_state_dir().is_ok());
        assert!(resolve_config_dir().is_ok());
        assert!(home_dir().is_some());
        let context = token_context();
        assert!(!context.cwd.is_empty());
        assert!(!context.today.is_empty());
        assert!(!context.now.is_empty());

        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let shell = entry("shell", "bash");
        let entry_dir = store.entry_dir_path(&shell.slug);
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("script.sh"), "NAME=old\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&entry_dir, fs::Permissions::from_mode(0o555)).unwrap();
        }
        let mut declaration = ParamDecl::new("NAME");
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        let mut assembly = skit_application::delivery::Assembly::default();
        assembly
            .inject_values
            .insert("NAME".to_owned(), "updated".to_owned());
        let error = stage_injected_source(&store, &shell, "NAME=old\n", &[declaration], &assembly)
            .unwrap_err();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&entry_dir, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(matches!(error, RunError::Stage { .. }), "{error:?}");
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
            |_, _| Ok(PathBuf::from("/data/bin/uv")),
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
    fn only_an_explicit_no_declines_and_end_of_input_consents() {
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
            |_, _| panic!("a refused download must not reach the installer"),
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
                program: "npm".to_owned(),
            }),
            &["npm"],
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
                value: "novalue".to_owned(),
            },
            &["novalue"],
        );
        assert_localized(
            &RunError::UnknownSet {
                name: "target".to_owned(),
            },
            &["target"],
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
            &RunError::Encoding {
                path: "/data/demo.py".to_owned(),
            },
            &["/data/demo.py"],
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
            },
            &["claude"],
        );
        assert_localized(
            &RunError::RawUnsupported {
                kind: "prompt".to_owned(),
            },
            &["prompt"],
        );
        assert_localized(&RunError::RawConflict, &[]);
        assert_localized(&RunError::ConfigDirectoryUnavailable, &[]);
    }
}
