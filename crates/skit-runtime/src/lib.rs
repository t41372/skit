//! Provide OS and process adapters for skit.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntrySettings, StorageMode};
use thiserror::Error;

/// Hold file paths that are resolved before launch planning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchPaths {
    /// Script or prompt source to launch.
    pub script: PathBuf,
    /// Entry directory in the skit data store.
    pub entry_dir: PathBuf,
    /// Directory from which the user invoked skit.
    pub invoke_cwd: PathBuf,
}

/// Define one prompt runner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRunner {
    /// Runner name from the configuration.
    pub name: String,
    /// Runner argv. One token must be `{{prompt}}`.
    pub argv: Vec<String>,
}

/// Inspect programs and paths without starting a process.
pub trait ProgramProbe: std::fmt::Debug {
    /// Find one program.
    fn find_program(&self, name: &str) -> Option<PathBuf>;
    /// Return true when `path` is a regular file.
    fn is_file(&self, path: &Path) -> bool;
    /// Return true when `path` is a directory.
    fn is_dir(&self, path: &Path) -> bool;
    /// Return true when `path` can be executed directly.
    fn is_executable(&self, path: &Path) -> bool;
}

/// Inspect the local machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProbe;

impl ProgramProbe for SystemProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        find_program(name)
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn is_executable(&self, path: &Path) -> bool {
        is_executable(path)
    }
}

/// Hold an immutable process plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchPlan {
    /// Executable path.
    pub program: PathBuf,
    /// Arguments after the executable.
    pub args: Vec<String>,
    /// Environment variables to add or replace.
    pub env: BTreeMap<String, String>,
    /// Child working directory.
    pub cwd: PathBuf,
    /// Safe command text for a dry run.
    pub display: String,
}

/// Report a launch refusal or process failure.
#[derive(Debug, Error)]
pub enum LaunchError {
    /// The entry kind has no runtime in this build.
    #[error("unknown entry kind: {kind}")]
    UnknownKind { kind: String },
    /// A required source file is gone.
    #[error("launch target does not exist: {path}")]
    TargetMissing { path: PathBuf },
    /// A direct executable cannot run.
    #[error("launch target is not executable: {path}")]
    TargetNotExecutable { path: PathBuf },
    /// A required runtime or command is not on PATH.
    #[error("required program was not found: {name}")]
    ProgramNotFound { name: String },
    /// A declared external command is not on PATH.
    #[error("required command was not found: {name}")]
    MissingNeed { name: String },
    /// The selected working directory does not exist.
    #[error("working directory does not exist: {path}")]
    WorkdirMissing { path: PathBuf },
    /// A custom work directory is not absolute.
    #[error("custom working directory must be absolute: {value}")]
    InvalidWorkdir { value: String },
    /// The command template needs a value that is not available.
    #[error("command template needs a value for {name}")]
    MissingTemplateValue { name: String },
    /// A placeholder is inside a shell quote context that skit does not rewrite.
    #[error("command template placeholder {name} is inside shell quotes")]
    UnsafeTemplatePlaceholder { name: String },
    /// A prompt needs a selected runner.
    #[error("prompt runner is required")]
    PromptRunnerRequired,
    /// A prompt body was not prepared.
    #[error("prompt body is required")]
    PromptBodyRequired,
    /// A prompt runner does not have one valid prompt token.
    #[error("prompt runner {name:?} must contain exactly one {{prompt}} token")]
    InvalidPromptRunner { name: String },
    /// The child process could not start or wait.
    #[error("could not {operation} child process: {source}")]
    Process {
        /// Operation that failed.
        operation: &'static str,
        /// Operating-system error.
        #[source]
        source: io::Error,
    },
}

impl LaunchError {
    /// Return the stable skit exit code for a pre-spawn error.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::TargetMissing { .. } => 127,
            Self::TargetNotExecutable { .. }
            | Self::ProgramNotFound { .. }
            | Self::MissingNeed { .. }
            | Self::PromptRunnerRequired
            | Self::InvalidPromptRunner { .. } => 126,
            Self::UnknownKind { .. }
            | Self::WorkdirMissing { .. }
            | Self::InvalidWorkdir { .. }
            | Self::MissingTemplateValue { .. }
            | Self::UnsafeTemplatePlaceholder { .. }
            | Self::PromptBodyRequired
            | Self::Process { .. } => 125,
        }
    }
}

/// Build one immutable process plan.
pub fn build_launch_plan<P: ProgramProbe>(
    entry: &Entry,
    paths: &LaunchPaths,
    assembly: &Assembly,
    prompt_body: Option<&str>,
    prompt_runner: Option<&PromptRunner>,
    probe: &P,
) -> Result<LaunchPlan, LaunchError> {
    let settings = EntrySettings::from_meta(&entry.meta);
    for need in &settings.needs {
        if probe.find_program(need).is_none() {
            return Err(LaunchError::MissingNeed { name: need.clone() });
        }
    }
    let cwd = resolve_workdir(entry, paths, probe)?;
    let kind = entry.meta.kind.as_str();

    let (program, args, display_args) = match kind {
        "python" => python_plan(entry, paths, assembly, &settings, probe)?,
        "shell" => interpreted_plan(paths, assembly, interpreter(&settings, "bash"), &[], probe)?,
        "fish" => interpreted_plan(paths, assembly, interpreter(&settings, "fish"), &[], probe)?,
        "powershell" => interpreted_plan(
            paths,
            assembly,
            interpreter(&settings, "pwsh"),
            &["-File"],
            probe,
        )?,
        "ruby" => interpreted_plan(paths, assembly, interpreter(&settings, "ruby"), &[], probe)?,
        "perl" => interpreted_plan(paths, assembly, interpreter(&settings, "perl"), &[], probe)?,
        "lua" => interpreted_plan(paths, assembly, interpreter(&settings, "lua"), &[], probe)?,
        "r" => interpreted_plan(
            paths,
            assembly,
            interpreter(&settings, "Rscript"),
            &[],
            probe,
        )?,
        "js" | "ts" => javascript_plan(paths, assembly, &settings, probe)?,
        "exe" => direct_plan(entry, assembly, probe)?,
        "command" => command_plan(assembly, &settings, probe)?,
        "prompt" => prompt_plan(prompt_body, prompt_runner, assembly, probe)?,
        _ => {
            return Err(LaunchError::UnknownKind {
                kind: kind.to_owned(),
            });
        }
    };

    let display = display_command(&program, &display_args, &assembly.masked_env);
    Ok(LaunchPlan {
        program,
        args,
        env: assembly.env_values.clone(),
        cwd,
        display,
    })
}

fn python_plan<P: ProgramProbe>(
    entry: &Entry,
    paths: &LaunchPaths,
    assembly: &Assembly,
    settings: &EntrySettings,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    require_file(&paths.script, probe)?;
    let uv = require_program("uv", probe)?;
    let mut prefix = vec!["run".to_owned(), "--no-project".to_owned()];
    if !settings.requires_python.is_empty() {
        prefix.push("--python".to_owned());
        prefix.push(settings.requires_python.clone());
    }
    if entry.meta.mode == StorageMode::Reference {
        for dependency in &settings.dependencies {
            prefix.push("--with".to_owned());
            prefix.push(dependency.clone());
        }
    }
    prefix.push("--script".to_owned());
    prefix.push(paths.script.display().to_string());
    let mut args = prefix.clone();
    args.extend(assembly.args.iter().cloned());
    let mut display = prefix;
    display.extend(assembly.masked_args.iter().cloned());
    Ok((uv, args, display))
}

fn interpreted_plan<P: ProgramProbe>(
    paths: &LaunchPaths,
    assembly: &Assembly,
    interpreter: &str,
    interpreter_args: &[&str],
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    require_file(&paths.script, probe)?;
    let program = require_program(interpreter, probe)?;
    let mut prefix = interpreter_args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    prefix.push(paths.script.display().to_string());
    let mut args = prefix.clone();
    args.extend(assembly.args.iter().cloned());
    let mut display = prefix;
    display.extend(assembly.masked_args.iter().cloned());
    Ok((program, args, display))
}

fn javascript_plan<P: ProgramProbe>(
    paths: &LaunchPaths,
    assembly: &Assembly,
    settings: &EntrySettings,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    require_file(&paths.script, probe)?;
    let runtime = if !settings.interpreter.is_empty() {
        settings.interpreter.as_str()
    } else {
        ["deno", "bun", "node"]
            .into_iter()
            .find(|name| probe.find_program(name).is_some())
            .ok_or_else(|| LaunchError::ProgramNotFound {
                name: "deno, bun, or node".to_owned(),
            })?
    };
    let program = require_program(runtime, probe)?;
    let mut prefix = if runtime == "deno" {
        vec!["run".to_owned(), "--allow-all".to_owned()]
    } else {
        Vec::new()
    };
    prefix.push(paths.script.display().to_string());
    let mut args = prefix.clone();
    args.extend(assembly.args.iter().cloned());
    let mut display = prefix;
    display.extend(assembly.masked_args.iter().cloned());
    Ok((program, args, display))
}

fn direct_plan<P: ProgramProbe>(
    entry: &Entry,
    assembly: &Assembly,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    let path = PathBuf::from(&entry.meta.source);
    if !probe.is_file(&path) {
        return Err(LaunchError::TargetMissing { path });
    }
    if !probe.is_executable(&path) {
        return Err(LaunchError::TargetNotExecutable { path });
    }
    Ok((path, assembly.args.clone(), assembly.masked_args.clone()))
}

fn command_plan<P: ProgramProbe>(
    assembly: &Assembly,
    settings: &EntrySettings,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    let command = render_command_template(&settings.template, &assembly.command_values)?;
    let masked = render_command_template(&settings.template, &assembly.masked_command_values)?;
    if cfg!(windows) {
        let shell = require_program("cmd.exe", probe)?;
        Ok((
            shell,
            vec!["/C".to_owned(), command],
            vec!["/C".to_owned(), masked],
        ))
    } else {
        let shell = require_program("sh", probe)?;
        Ok((
            shell,
            vec!["-c".to_owned(), command],
            vec!["-c".to_owned(), masked],
        ))
    }
}

fn prompt_plan<P: ProgramProbe>(
    prompt_body: Option<&str>,
    runner: Option<&PromptRunner>,
    assembly: &Assembly,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    let body = prompt_body.ok_or(LaunchError::PromptBodyRequired)?;
    let runner = runner.ok_or(LaunchError::PromptRunnerRequired)?;
    let prompt_count = runner
        .argv
        .iter()
        .filter(|value| value.as_str() == "{{prompt}}")
        .count();
    if prompt_count != 1
        || runner
            .argv
            .first()
            .is_none_or(|value| value == "{{prompt}}")
    {
        return Err(LaunchError::InvalidPromptRunner {
            name: runner.name.clone(),
        });
    }
    let program_name = &runner.argv[0];
    let program = require_program(program_name, probe)?;
    let args = runner.argv[1..]
        .iter()
        .map(|value| {
            if value == "{{prompt}}" {
                body.to_owned()
            } else {
                value.clone()
            }
        })
        .chain(assembly.args.iter().cloned())
        .collect::<Vec<_>>();
    let display = runner.argv[1..]
        .iter()
        .map(|value| {
            if value == "{{prompt}}" {
                "<prompt>".to_owned()
            } else {
                value.clone()
            }
        })
        .chain(assembly.masked_args.iter().cloned())
        .collect::<Vec<_>>();
    Ok((program, args, display))
}

/// Fill a command template with shell-quoted values.
///
/// A placeholder inside existing quotes is refused. This rule is safe and deterministic.
pub fn render_command_template(
    template: &str,
    values: &BTreeMap<String, String>,
) -> Result<String, LaunchError> {
    let chars = template.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;

    while index < chars.len() {
        let character = chars[index];
        if !cfg!(windows) && escaped {
            output.push(character);
            escaped = false;
            index += 1;
            continue;
        }
        if !cfg!(windows) && character == '\\' && quote != Some('\'') {
            output.push(character);
            escaped = true;
            index += 1;
            continue;
        }
        if character == '\'' && !cfg!(windows) && quote != Some('"') {
            quote = if quote == Some('\'') {
                None
            } else {
                Some('\'')
            };
            output.push(character);
            index += 1;
            continue;
        }
        if character == '"' && quote != Some('\'') {
            quote = if quote == Some('"') { None } else { Some('"') };
            output.push(character);
            index += 1;
            continue;
        }
        if character == '{' && chars.get(index + 1) == Some(&'{') {
            output.push('{');
            index += 2;
            continue;
        }
        if character == '}' && chars.get(index + 1) == Some(&'}') {
            output.push('}');
            index += 2;
            continue;
        }
        if character == '{'
            && let Some(end) = chars[index + 1..].iter().position(|value| *value == '}')
        {
            let end = index + 1 + end;
            let name = chars[index + 1..end].iter().collect::<String>();
            if valid_placeholder(&name) {
                if quote.is_some() {
                    return Err(LaunchError::UnsafeTemplatePlaceholder { name });
                }
                let value = values
                    .get(&name)
                    .ok_or_else(|| LaunchError::MissingTemplateValue { name: name.clone() })?;
                output.push_str(&quote_shell_arg(value));
                index = end + 1;
                continue;
            }
        }
        output.push(character);
        index += 1;
    }
    Ok(output)
}

fn valid_placeholder(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn interpreter<'a>(settings: &'a EntrySettings, default: &'a str) -> &'a str {
    if settings.interpreter.is_empty() {
        default
    } else {
        &settings.interpreter
    }
}

fn require_file<P: ProgramProbe>(path: &Path, probe: &P) -> Result<(), LaunchError> {
    if probe.is_file(path) {
        Ok(())
    } else {
        Err(LaunchError::TargetMissing {
            path: path.to_path_buf(),
        })
    }
}

fn require_program<P: ProgramProbe>(name: &str, probe: &P) -> Result<PathBuf, LaunchError> {
    probe
        .find_program(name)
        .ok_or_else(|| LaunchError::ProgramNotFound {
            name: name.to_owned(),
        })
}

fn resolve_workdir<P: ProgramProbe>(
    entry: &Entry,
    paths: &LaunchPaths,
    probe: &P,
) -> Result<PathBuf, LaunchError> {
    let cwd = match entry.meta.workdir.as_str() {
        "invoke" => paths.invoke_cwd.clone(),
        "store" => paths.entry_dir.clone(),
        "origin" => {
            let origin = if entry.meta.source.is_empty() {
                paths.invoke_cwd.clone()
            } else {
                Path::new(&entry.meta.source)
                    .parent()
                    .map_or_else(|| paths.invoke_cwd.clone(), Path::to_path_buf)
            };
            if entry.meta.mode == StorageMode::Copy && !probe.is_dir(&origin) {
                paths.invoke_cwd.clone()
            } else {
                origin
            }
        }
        custom => {
            let path = PathBuf::from(custom);
            if !path.is_absolute() {
                return Err(LaunchError::InvalidWorkdir {
                    value: custom.to_owned(),
                });
            }
            path
        }
    };
    if probe.is_dir(&cwd) {
        Ok(cwd)
    } else {
        Err(LaunchError::WorkdirMissing { path: cwd })
    }
}

/// Start a child and wait for its process status.
pub fn execute_launch(plan: &LaunchPlan) -> Result<i32, LaunchError> {
    let status = Command::new(&plan.program)
        .args(&plan.args)
        .envs(&plan.env)
        .current_dir(&plan.cwd)
        .status()
        .map_err(|source| LaunchError::Process {
            operation: "run",
            source,
        })?;
    Ok(status_code(status))
}

fn status_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    125
}

fn display_command(program: &Path, args: &[String], env: &BTreeMap<String, String>) -> String {
    let mut parts = env
        .iter()
        .map(|(name, value)| format!("{name}={}", quote_shell_arg(value)))
        .collect::<Vec<_>>();
    parts.push(quote_shell_arg(&program.display().to_string()));
    parts.extend(args.iter().map(|value| quote_shell_arg(value)));
    parts.join(" ")
}

fn quote_shell_arg(value: &str) -> String {
    if cfg!(windows) {
        quote_windows_arg(value)
    } else {
        quote_posix_arg(value)
    }
}

fn quote_posix_arg(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character))
    {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn quote_windows_arg(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_owned();
    }
    let mut output = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        if character == '"' {
            output.push_str(&"\\".repeat(slashes * 2 + 1));
            output.push('"');
        } else {
            output.push_str(&"\\".repeat(slashes));
            output.push(character);
        }
        slashes = 0;
    }
    output.push_str(&"\\".repeat(slashes * 2));
    output.push('"');
    output
}

fn find_program(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 || candidate.is_absolute() {
        return (candidate.is_file() && is_executable(candidate)).then(|| candidate.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        for filename in program_names(name) {
            let path = directory.join(filename);
            if path.is_file() && is_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

fn program_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        if Path::new(name).extension().is_some() {
            return vec![name.to_owned()];
        }
        let extensions = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        return extensions
            .split(';')
            .filter(|value| !value.is_empty())
            .map(|extension| format!("{name}{extension}"))
            .collect();
    }
    #[cfg(not(windows))]
    {
        vec![name.to_owned()]
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
