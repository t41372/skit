use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Path, PathBuf},
};

use clap::Args;
use skit_application::{
    LibraryService, RepositoryError,
    form_state::{FormStateService, StateWriteError, prefill},
    run_inputs::{RunInputError, assemble_run_inputs},
    tokens::TokenContext,
};
use skit_domain::{Entry, EntryId, EntrySettings};
use skit_form::form_params;
use skit_language::{LanguageError, inject_values};
use skit_runtime::{
    DependencyError, LaunchError, LaunchPaths, PromptRunner, SystemDependencyCommandRunner,
    SystemProbe, build_launch_plan, ensure_javascript_dependencies, execute_launch,
    resolve_javascript_runtime,
};
use skit_store::{ConfigError, FileConfigStore, FileFormStateStore, FileGlobExpander, FileStore};
use thiserror::Error;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

/// Options for `skit run`.
#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    /// Entry slug or display name.
    pub(crate) selector: String,

    /// Set one field for this run.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    pub(crate) values: Vec<String>,

    /// Load one named preset.
    #[arg(long, short = 'p')]
    pub(crate) preset: Option<String>,

    /// Save accepted values as a named preset after the run.
    #[arg(long, value_name = "NAME")]
    pub(crate) save_preset: Option<String>,

    /// Select a prompt runner for this run.
    #[arg(long)]
    pub(crate) runner: Option<String>,

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
    #[error("--set needs NAME=VALUE; got {value:?}")]
    InvalidSet { value: String },
    #[error("unknown parameter in --set: {name}")]
    UnknownSet { name: String },
    #[error("preset {name:?} does not exist")]
    PresetNotFound { name: String },
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
    #[error("--raw does not apply to {kind} entries because placeholders are part of the artifact")]
    RawUnsupported { kind: String },
    #[error("--raw cannot be combined with --set, --preset, or --save-preset")]
    RawConflict,
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("could not determine the platform configuration directory; set SKIT_CONFIG_DIR")]
    ConfigDirectoryUnavailable,
}

impl RunError {
    pub(crate) const fn exit_code(&self) -> i32 {
        match self {
            Self::Repository(error) => error.exit_class().code() as i32,
            Self::Inputs(_)
            | Self::InvalidSet { .. }
            | Self::UnknownSet { .. }
            | Self::PresetNotFound { .. }
            | Self::RawUnsupported { .. }
            | Self::RawConflict => 2,
            Self::Launch(error) => error.exit_code(),
            Self::Dependencies(DependencyError::InstallerNotFound { .. }) => 126,
            Self::RunnerNotFound { .. } => 126,
            Self::State(_)
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
    let _plain = args.plain;
    let _no_input = args.no_input;
    let held = service.show(&args.selector)?;
    let settings = EntrySettings::from_meta(&held.meta);
    if args.raw && (!args.values.is_empty() || args.preset.is_some() || args.save_preset.is_some())
    {
        return Err(RunError::RawConflict);
    }
    if args.raw && matches!(held.meta.kind.as_str(), "command" | "prompt") {
        return Err(RunError::RawUnsupported {
            kind: held.meta.kind.as_str().to_owned(),
        });
    }
    let entry = service.claim_identity(&held)?;
    let state_root = resolve_state_dir()?;
    let state = FormStateService::new(FileFormStateStore::new(&state_root));
    let saved = state.load(&entry.slug);

    let source = source_text(data_store, &entry, &settings)?;
    if matches!(entry.meta.kind.as_str(), "js" | "ts") {
        if entry.meta.mode == skit_domain::StorageMode::Reference
            && !settings.dependencies.is_empty()
        {
            return Err(DependencyError::CopyStorageRequired.into());
        }
        if entry.meta.mode == skit_domain::StorageMode::Copy {
            let runtime = resolve_javascript_runtime(&settings, &SystemProbe)?;
            ensure_javascript_dependencies(
                &data_store.entry_dir_path(&entry.slug),
                &runtime,
                &settings.dependencies,
                &SystemProbe,
                &SystemDependencyCommandRunner,
            )?;
        }
    }
    let declarations = if args.raw {
        Vec::new()
    } else {
        form_params(entry.meta.kind.as_str(), &source, &settings)
    };
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

    let (extra_args, expand_extra, new_tail) = if args.forget_args {
        (Vec::new(), false, Some(Vec::new()))
    } else if !args.extra_args.is_empty() {
        (
            args.extra_args.clone(),
            false,
            Some(args.extra_args.clone()),
        )
    } else {
        (saved.extra_args.clone(), saved.extra_args_raw, None)
    };

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

    let staged = stage_injected_source(data_store, &entry, &source, &declarations, &assembly)?;
    let script = if let Some(staged) = staged.as_ref() {
        staged.path.clone()
    } else if entry.meta.kind.as_str() == "command" {
        PathBuf::new()
    } else {
        data_store.payload_path(&entry)?
    };
    let prompt_body = if entry.meta.kind.as_str() == "prompt" {
        Some(render_prompt(
            &source,
            &assembly.command_values,
            settings.interpolate,
        ))
    } else {
        None
    };
    let runner_name = args
        .runner
        .as_deref()
        .or_else(|| (!settings.runner.is_empty()).then_some(settings.runner.as_str()));
    let runner = runner_name.map(configured_runner).transpose()?;
    let plan = build_launch_plan(
        &entry,
        &LaunchPaths {
            script,
            entry_dir: data_store.entry_dir_path(&entry.slug),
            invoke_cwd: PathBuf::from(&context.cwd),
        },
        &assembly,
        prompt_body.as_deref(),
        runner.as_ref(),
        &SystemProbe,
    )?;

    if args.dry_run {
        println!("{}", plan.display);
        return Ok(0);
    }

    let exit = execute_launch(&plan)?;
    state.purge_secrets(&entry.slug, &declarations)?;
    state.save_last(
        &entry.slug,
        &declarations,
        Some(&raw_values),
        new_tail,
        false,
    )?;
    if let Some(name) = args.save_preset.as_deref() {
        state.save_preset(&entry.slug, name, &declarations, &raw_values)?;
    }
    let at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    state.record_run(
        &entry.slug,
        i64::from(exit),
        &at,
        &declarations,
        (!args.raw).then_some(&raw_values),
    )?;
    Ok(exit)
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

fn source_text(
    store: &FileStore,
    entry: &Entry,
    settings: &EntrySettings,
) -> Result<String, RunError> {
    match entry.meta.kind.as_str() {
        "command" => Ok(settings.template.clone()),
        "exe" => Ok(String::new()),
        "prompt" => read_utf8(&store.payload_path(entry)?),
        _ => {
            let path = store.payload_path(entry)?;
            match fs::read(&path) {
                Ok(bytes) => Ok(String::from_utf8(bytes).unwrap_or_default()),
                Err(source) => Err(RunError::Read {
                    path: path.display().to_string(),
                    source,
                }),
            }
        }
    }
}

fn read_utf8(path: &Path) -> Result<String, RunError> {
    let bytes = fs::read(path).map_err(|source| RunError::Read {
        path: path.display().to_string(),
        source,
    })?;
    String::from_utf8(bytes).map_err(|_| RunError::Encoding {
        path: path.display().to_string(),
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
    let rewritten = inject_values(
        entry.meta.kind.as_str(),
        source,
        declarations,
        &assembly.inject_values,
    )?;
    let original = store.payload_path(entry)?;
    let suffix = original
        .extension()
        .and_then(|value| value.to_str())
        .map_or(String::new(), |value| format!(".{value}"));
    let path = store.entry_dir_path(&entry.slug).join(format!(
        ".run-{}{}",
        EntryId::generate().as_str(),
        suffix
    ));
    fs::write(&path, rewritten.as_bytes()).map_err(|source| RunError::Stage {
        path: path.display().to_string(),
        source,
    })?;
    Ok(Some(StagedSource { path }))
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

fn render_prompt(text: &str, values: &BTreeMap<String, String>, interpolate: bool) -> String {
    if !interpolate {
        return text.to_owned();
    }
    let mut output = text.to_owned();
    for (name, value) in values {
        output = output.replace(&format!("{{{{{name}}}}}"), value);
    }
    output
}

fn configured_runner(name: &str) -> Result<PromptRunner, RunError> {
    let config = FileConfigStore::new(resolve_config_dir()?);
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

fn token_context() -> TokenContext {
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
