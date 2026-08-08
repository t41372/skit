use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{
    Entry, LaunchPlanError, PrepareRunError, PreparedRun, ProgramResolver, PythonInjectError,
    RunRequest, TempScript, TempScriptError, build_launch_plan, inject_python_managed,
    materialize_temp_script, prepare_run, read_python_params,
};

/// A launch-ready run plus any ephemeral source snapshot that must outlive the child.
#[derive(Debug)]
pub struct PreparedExecution {
    pub run: PreparedRun,
    injected: Option<TempScript>,
}

impl PreparedExecution {
    /// The materialized injected source, when this run needed one.
    #[must_use]
    pub fn injected_path(&self) -> Option<&Path> {
        self.injected.as_ref().map(TempScript::path)
    }
}

/// Failure while crossing from resolved form values to a launch-ready source snapshot.
#[derive(Debug)]
pub enum PrepareExecutionError {
    Prepare(PrepareRunError),
    SourceIo { path: PathBuf, source: io::Error },
    PythonInject(PythonInjectError),
    Temp(TempScriptError),
    Launch(LaunchPlanError),
    UnsupportedInjectionKind(String),
}

impl fmt::Display for PrepareExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepare(source) => source.fmt(formatter),
            Self::SourceIo { path, source } => {
                write!(
                    formatter,
                    "cannot read managed source {}: {source}",
                    path.display()
                )
            }
            Self::PythonInject(source) => source.fmt(formatter),
            Self::Temp(source) => source.fmt(formatter),
            Self::Launch(source) => source.fmt(formatter),
            Self::UnsupportedInjectionKind(kind) => {
                write!(
                    formatter,
                    "managed injection is not implemented for kind: {kind}"
                )
            }
        }
    }
}

impl StdError for PrepareExecutionError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Prepare(source) => Some(source),
            Self::SourceIo { source, .. } => Some(source),
            Self::PythonInject(source) => Some(source),
            Self::Temp(source) => Some(source),
            Self::Launch(source) => Some(source),
            Self::UnsupportedInjectionKind(_) => None,
        }
    }
}

impl From<PrepareRunError> for PrepareExecutionError {
    fn from(value: PrepareRunError) -> Self {
        Self::Prepare(value)
    }
}

impl From<PythonInjectError> for PrepareExecutionError {
    fn from(value: PythonInjectError) -> Self {
        Self::PythonInject(value)
    }
}

impl From<TempScriptError> for PrepareExecutionError {
    fn from(value: TempScriptError) -> Self {
        Self::Temp(value)
    }
}

impl From<LaunchPlanError> for PrepareExecutionError {
    fn from(value: LaunchPlanError) -> Self {
        Self::Launch(value)
    }
}

/// Resolve a run and materialize its exact pre-spawn source snapshot when required.
///
/// Managed Python values are injected in memory, written to one private OS-temp file,
/// and both actual/masked launch plans are rebuilt to consume that same path. The
/// returned guard owns cleanup; callers must keep `PreparedExecution` alive until the
/// child has exited. Stored/reference source bytes are never modified.
///
/// # Errors
///
/// Returns ordinary run-preparation failures plus source-read, injection, temp-file, or
/// final launch-snapshot failures.
pub fn prepare_execution(
    entry: &Entry,
    request: RunRequest<'_>,
    programs: &impl ProgramResolver,
) -> Result<PreparedExecution, PrepareExecutionError> {
    let mut run = prepare_run(entry, request, programs)?;
    if run.assembly.inject_values.is_empty() {
        return Ok(PreparedExecution {
            run,
            injected: None,
        });
    }
    if entry.meta.kind != "python" {
        return Err(PrepareExecutionError::UnsupportedInjectionKind(
            entry.meta.kind.clone(),
        ));
    }

    let source_path = entry.script_path();
    let source =
        fs::read_to_string(&source_path).map_err(|source| PrepareExecutionError::SourceIo {
            path: source_path.clone(),
            source,
        })?;
    let specs = read_python_params(&source);
    let injected_text = inject_python_managed(&source, &specs, &run.assembly.inject_values)?;
    let injected = materialize_temp_script(&injected_text, ".py")?;

    let mut options = request.launch_options.clone();
    options.script_override = Some(injected.path().to_owned());
    run.launch = build_launch_plan(entry, &run.assembly, &options, programs)?;
    let mut masked_assembly = run.assembly.clone();
    masked_assembly.args = run.assembly.masked_args.clone();
    masked_assembly.env_values = run.assembly.masked_env.clone();
    run.masked_launch = build_launch_plan(entry, &masked_assembly, &options, programs)?;

    Ok(PreparedExecution {
        run,
        injected: Some(injected),
    })
}
