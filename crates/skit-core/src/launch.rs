use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{Assembly, Entry, Platform, spec_for};

/// A side-effect-free executable lookup seam. Frontends can resolve PATH/config once
/// and tests can supply an in-memory map.
pub trait ProgramResolver {
    fn resolve(&self, name: &str) -> Option<PathBuf>;
}

impl<F> ProgramResolver for F
where
    F: Fn(&str) -> Option<PathBuf>,
{
    fn resolve(&self, name: &str) -> Option<PathBuf> {
        self(name)
    }
}

/// Frontend-owned context that affects launch policy but not entry metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOptions {
    pub platform: Platform,
    pub invoke_cwd: PathBuf,
    pub js_runner: Option<String>,
    pub windows_bash: Option<PathBuf>,
    pub script_override: Option<PathBuf>,
}

impl LaunchOptions {
    /// Create options for one invocation.
    #[must_use]
    pub fn new(platform: Platform, invoke_cwd: impl Into<PathBuf>) -> Self {
        Self {
            platform,
            invoke_cwd: invoke_cwd.into(),
            js_runner: None,
            windows_bash: None,
            script_override: None,
        }
    }
}

/// An immutable process launch snapshot. No source or config needs to be re-read after
/// this is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env_overlay: BTreeMap<String, String>,
}

/// Named launch refusal classes. The CLI later maps these to the existing 125/126/127
/// exit-code convention without re-catching language-specific errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlanError {
    UnsupportedKind(String),
    TargetMissing(PathBuf),
    NotRunnable(PathBuf),
    MissingInterpreter(String),
    MissingJavaScriptRuntime(Vec<String>),
    MissingNeeds(Vec<String>),
    WorkingDirectoryMissing(PathBuf),
}

impl fmt::Display for LaunchPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedKind(kind) => write!(formatter, "launch is not implemented for kind: {kind}"),
            Self::TargetMissing(path) => write!(formatter, "the launch target does not exist: {}", path.display()),
            Self::NotRunnable(path) => write!(formatter, "the executable is not runnable: {}", path.display()),
            Self::MissingInterpreter(name) => write!(formatter, "the interpreter is not installed or on PATH: {name}"),
            Self::MissingJavaScriptRuntime(candidates) => write!(
                formatter,
                "no JavaScript runtime found (looked for: {})",
                candidates.join(", ")
            ),
            Self::MissingNeeds(names) => write!(
                formatter,
                "missing required command(s): {}",
                names.join(", ")
            ),
            Self::WorkingDirectoryMissing(path) => write!(
                formatter,
                "the working directory does not exist: {}",
                path.display()
            ),
        }
    }
}

impl StdError for LaunchPlanError {}

/// Build one immutable launch plan for the currently parser-free runnable kinds.
///
/// Python, prompt, and command-template execution are intentionally refused here until
/// their injection/rendering and quoting contracts land in Rust as complete slices.
///
/// # Errors
///
/// Returns a named refusal if the target/runtime/declared needs/workdir are unavailable
/// or if this kind still requires a deeper launch implementation.
pub fn build_launch_plan(
    entry: &Entry,
    assembly: &Assembly,
    options: &LaunchOptions,
    programs: &impl ProgramResolver,
) -> Result<LaunchPlan, LaunchPlanError> {
    let argv = match entry.meta.kind.as_str() {
        "exe" => direct_argv(entry, &assembly.args, options.platform)?,
        "js" | "ts" => javascript_argv(entry, &assembly.args, options, programs)?,
        "python" | "prompt" | "command" => {
            return Err(LaunchPlanError::UnsupportedKind(entry.meta.kind.clone()));
        }
        kind => interpreted_argv(entry, kind, &assembly.args, options, programs)?,
    };

    let missing_needs = entry
        .meta
        .needs
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|name| programs.resolve(name).is_none())
        .cloned()
        .collect::<Vec<_>>();
    if !missing_needs.is_empty() {
        return Err(LaunchPlanError::MissingNeeds(missing_needs));
    }

    let cwd = resolve_workdir(entry, &options.invoke_cwd);
    if !cwd.is_dir() {
        return Err(LaunchPlanError::WorkingDirectoryMissing(cwd));
    }

    Ok(LaunchPlan {
        argv,
        cwd,
        env_overlay: assembly.env_values.clone(),
    })
}

fn direct_argv(
    entry: &Entry,
    extra: &[String],
    platform: Platform,
) -> Result<Vec<String>, LaunchPlanError> {
    let source = PathBuf::from(&entry.meta.source);
    if !source.exists() {
        return Err(LaunchPlanError::TargetMissing(source));
    }
    if !source.is_file() || (platform != Platform::Windows && !posix_executable(&source)) {
        return Err(LaunchPlanError::NotRunnable(source));
    }
    let mut argv = vec![entry.meta.source.clone()];
    argv.extend(extra.iter().cloned());
    Ok(argv)
}

fn interpreted_argv(
    entry: &Entry,
    kind: &str,
    extra: &[String],
    options: &LaunchOptions,
    programs: &impl ProgramResolver,
) -> Result<Vec<String>, LaunchPlanError> {
    let Some(spec) = spec_for(kind) else {
        return Err(LaunchPlanError::UnsupportedKind(kind.to_owned()));
    };
    if spec.default_interpreter.is_empty() {
        return Err(LaunchPlanError::UnsupportedKind(kind.to_owned()));
    }
    let script = options
        .script_override
        .clone()
        .unwrap_or_else(|| entry.script_path());
    if !script.exists() {
        return Err(LaunchPlanError::TargetMissing(script));
    }

    let name = if entry.meta.interpreter.is_empty() {
        spec.default_interpreter
    } else {
        &entry.meta.interpreter
    };
    let interpreter = resolve_interpreter(name, options, programs)?;
    let mut argv = vec![interpreter.to_string_lossy().into_owned()];
    if kind == "powershell" {
        argv.push("-File".to_owned());
    }
    argv.push(script.to_string_lossy().into_owned());
    argv.extend(extra.iter().cloned());
    Ok(argv)
}

fn resolve_interpreter(
    name: &str,
    options: &LaunchOptions,
    programs: &impl ProgramResolver,
) -> Result<PathBuf, LaunchPlanError> {
    if let Some(path) = programs.resolve(name) {
        return Ok(path);
    }
    if options.platform == Platform::Windows
        && matches!(name, "bash" | "sh" | "zsh")
        && let Some(path) = &options.windows_bash
        && path.exists()
    {
        return Ok(path.clone());
    }
    Err(LaunchPlanError::MissingInterpreter(name.to_owned()))
}

fn javascript_argv(
    entry: &Entry,
    extra: &[String],
    options: &LaunchOptions,
    programs: &impl ProgramResolver,
) -> Result<Vec<String>, LaunchPlanError> {
    const ORDER: &[&str] = &["deno", "bun", "node"];
    let script = options
        .script_override
        .clone()
        .unwrap_or_else(|| entry.script_path());
    if !script.exists() {
        return Err(LaunchPlanError::TargetMissing(script));
    }

    let override_name = if !entry.meta.interpreter.is_empty() {
        Some(entry.meta.interpreter.as_str())
    } else {
        options.js_runner.as_deref()
    };
    let candidates = override_name.map_or_else(
        || ORDER.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>(),
        |name| vec![name.to_owned()],
    );
    let Some((runner_name, runner_path)) = candidates
        .iter()
        .find_map(|name| programs.resolve(name).map(|path| (name.as_str(), path)))
    else {
        return Err(LaunchPlanError::MissingJavaScriptRuntime(candidates));
    };

    let mut argv = vec![runner_path.to_string_lossy().into_owned()];
    match runner_name {
        "deno" => argv.extend(["run".to_owned(), "--allow-all".to_owned()]),
        "bun" => argv.push("run".to_owned()),
        _ => {}
    }
    argv.push(script.to_string_lossy().into_owned());
    argv.extend(extra.iter().cloned());
    Ok(argv)
}

fn resolve_workdir(entry: &Entry, invoke_cwd: &Path) -> PathBuf {
    match entry.meta.workdir.as_str() {
        "invoke" => invoke_cwd.to_owned(),
        "store" => entry.dir.clone(),
        "origin" => {
            let origin = if entry.meta.source.is_empty() {
                invoke_cwd.to_owned()
            } else {
                Path::new(&entry.meta.source)
                    .parent()
                    .map_or_else(|| invoke_cwd.to_owned(), Path::to_owned)
            };
            if entry.meta.mode == "copy" && !origin.is_dir() {
                invoke_cwd.to_owned()
            } else {
                origin
            }
        }
        path => PathBuf::from(path),
    }
}

#[cfg(unix)]
fn posix_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn posix_executable(_path: &Path) -> bool {
    true
}
