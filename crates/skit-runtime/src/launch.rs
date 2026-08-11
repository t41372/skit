use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntrySettings, StorageMode};
use skit_i18n::{Localize, Message};
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
    /// Runner argv. One non-program token must contain `{{prompt}}` once.
    pub argv: Vec<String>,
}

/// Describe one non-fatal change to a launch plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchWarning {
    /// skit added one newline to keep a Pi prompt in message mode.
    PiPromptProtected,
    /// The built-in Amp runner executes one prompt and does not open a session.
    AmpOneShot,
}

/// Inspect programs and paths without starting a process.
pub trait ProgramProbe: std::fmt::Debug {
    /// Find one program.
    fn find_program(&self, name: &str) -> Option<PathBuf>;
    /// Return true when `path` is a regular file.
    fn is_file(&self, path: &Path) -> bool;
    /// Return true when `path` is a directory.
    fn is_dir(&self, path: &Path) -> bool;
    /// Return true when any filesystem object exists at `path`.
    fn exists(&self, path: &Path) -> bool {
        self.is_file(path) || self.is_dir(path)
    }
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

    fn exists(&self, path: &Path) -> bool {
        path.exists()
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
    /// Important non-fatal launch changes.
    pub warnings: Vec<LaunchWarning>,
}

#[derive(Debug)]
struct PromptProcessPlan {
    program: PathBuf,
    args: Vec<String>,
    display_args: Vec<String>,
    warning: Option<LaunchWarning>,
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
    /// One or more managed command placeholders do not have values.
    #[error("Missing parameter values: {name}")]
    MissingTemplateValue { name: String },
    /// A placeholder is in double quotes nested inside a backtick substitution.
    #[error(
        "Can't safely fill in a value inside double quotes nested in a `…` command substitution — the shell strips one layer of escaping there. Rewrite that part of the template with $(…) instead of backticks."
    )]
    UnsafeTemplatePlaceholder { name: String },
    /// A prompt needs a selected runner.
    #[error("prompt runner is required")]
    PromptRunnerRequired,
    /// A prompt body was not prepared.
    #[error("prompt body is required")]
    PromptBodyRequired,
    /// A prompt runner does not have one valid prompt token.
    #[error(
        "prompt runner {name:?} must contain exactly one {{{{prompt}}}} marker outside the program token"
    )]
    InvalidPromptRunner { name: String },
    /// A prompt argument contains a NUL character.
    #[error("the rendered prompt contains a NUL character")]
    PromptContainsNul,
    /// A prompt command line exceeds the safe platform limit.
    #[error(
        "the rendered prompt makes the command line {size} {unit}; the limit is {limit} {unit}"
    )]
    PromptArgvTooLong {
        /// Measured command-line size.
        size: usize,
        /// Safe platform limit.
        limit: usize,
        /// Unit used for the measurement.
        unit: &'static str,
    },
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

impl Localize for LaunchError {
    fn message(&self) -> Message {
        match self {
            Self::UnknownKind { kind } => Message::new("unknown entry kind: {}").with(kind),
            Self::TargetMissing { path } => {
                Message::new("launch target does not exist: {}").with(path.display())
            }
            Self::TargetNotExecutable { path } => {
                Message::new("launch target is not executable: {}").with(path.display())
            }
            Self::ProgramNotFound { name } => {
                Message::new("required program was not found: {}").with(name)
            }
            Self::MissingNeed { name } => {
                Message::new("required command was not found: {}").with(name)
            }
            Self::WorkdirMissing { path } => {
                Message::new("working directory does not exist: {}").with(path.display())
            }
            Self::InvalidWorkdir { value } => {
                Message::new("custom working directory must be absolute: {}").with(value)
            }
            Self::MissingTemplateValue { name } => {
                Message::new("Missing parameter values: {}").with(name)
            }
            Self::UnsafeTemplatePlaceholder { .. } => Message::new(
                "Can't safely fill in a value inside double quotes nested in a `…` command substitution — the shell strips one layer of escaping there. Rewrite that part of the template with $(…) instead of backticks.",
            ),
            Self::PromptRunnerRequired => Message::new("prompt runner is required"),
            Self::PromptBodyRequired => Message::new("prompt body is required"),
            Self::InvalidPromptRunner { name } => Message::new(
                "prompt runner {} must contain exactly one {{prompt}} marker outside the program token",
            )
            .quoted(name),
            Self::PromptContainsNul => Message::new("the rendered prompt contains a NUL character"),
            Self::PromptArgvTooLong { size, limit, unit } => Message::new(
                "the rendered prompt makes the command line {} {}; the limit is {} {}",
            )
            .with(size)
            .with(unit)
            .with(limit)
            .with(unit),
            Self::Process { operation, source } => Message::new("could not {} child process: {}")
                .nested(Message::term(operation))
                .with(source),
        }
    }
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
            | Self::PromptContainsNul
            | Self::PromptArgvTooLong { .. }
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
    build_launch_plan_inner(
        entry,
        paths,
        assembly,
        prompt_body,
        None,
        prompt_runner,
        probe,
    )
}

/// Build a complete launch preview without looking up programs on `PATH`.
///
/// The preview is total: for an entry kind that this skit version does not know, the
/// preview degrades to the stored command template instead of an error. The run path
/// (`build_launch_plan`) keeps its refusal.
pub fn build_launch_preview<P: ProgramProbe>(
    entry: &Entry,
    paths: &LaunchPaths,
    assembly: &Assembly,
    prompt_body: Option<&str>,
    prompt_display_body: Option<&str>,
    prompt_runner: Option<&PromptRunner>,
    probe: &P,
) -> Result<LaunchPlan, LaunchError> {
    if !is_known_launch_kind(entry.meta.kind.as_str()) {
        return Ok(unknown_kind_preview(entry, paths, assembly));
    }
    build_launch_plan_inner(
        entry,
        paths,
        assembly,
        prompt_body,
        prompt_display_body,
        prompt_runner,
        &PreviewProbe { local: probe },
    )
}

/// Report whether this skit version can assemble a launch for `kind`.
///
/// Keep this list synchronized with the kind `match` in `build_launch_plan_inner`.
fn is_known_launch_kind(kind: &str) -> bool {
    matches!(
        kind,
        "python"
            | "shell"
            | "fish"
            | "powershell"
            | "ruby"
            | "perl"
            | "lua"
            | "r"
            | "js"
            | "ts"
            | "exe"
            | "command"
            | "prompt"
    )
}

/// Build the degraded preview for a kind written by a newer skit.
///
/// The template is the only launch material the metadata itself carries, so it becomes
/// the display text. The template stays raw: it is descriptive text, not an argument.
fn unknown_kind_preview(entry: &Entry, paths: &LaunchPaths, assembly: &Assembly) -> LaunchPlan {
    let mut parts = assembly
        .masked_env
        .iter()
        .map(|(name, value)| format!("{name}={}", quote_shell_arg(value)))
        .collect::<Vec<_>>();
    parts.push(EntrySettings::from_meta(&entry.meta).template);
    LaunchPlan {
        program: PathBuf::new(),
        args: Vec::new(),
        env: assembly.env_values.clone(),
        cwd: paths.invoke_cwd.clone(),
        display: parts.join(" "),
        warnings: Vec::new(),
    }
}

#[derive(Debug)]
struct PreviewProbe<'a, P> {
    local: &'a P,
}

impl<P: ProgramProbe> ProgramProbe for PreviewProbe<'_, P> {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(name))
    }

    fn is_file(&self, path: &Path) -> bool {
        self.local.is_file(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.local.is_dir(path)
    }

    fn exists(&self, path: &Path) -> bool {
        self.local.exists(path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        self.local.is_executable(path)
    }
}

fn build_launch_plan_inner<P: ProgramProbe>(
    entry: &Entry,
    paths: &LaunchPaths,
    assembly: &Assembly,
    prompt_body: Option<&str>,
    prompt_display_body: Option<&str>,
    prompt_runner: Option<&PromptRunner>,
    probe: &P,
) -> Result<LaunchPlan, LaunchError> {
    let settings = EntrySettings::from_meta(&entry.meta);
    for need in &settings.needs {
        if probe.find_program(need).is_none() {
            return Err(LaunchError::MissingNeed { name: need.clone() });
        }
    }
    let cwd = resolve_launch_workdir(entry, paths, probe)?;
    let kind = entry.meta.kind.as_str();

    let mut warnings = Vec::new();
    let (program, args, display_args) = match kind {
        "python" => python_plan(paths, assembly, &settings, probe)?,
        "shell" => interpreted_plan(paths, assembly, interpreter(&settings, "bash"), &[], probe)?,
        "fish" => interpreted_plan(paths, assembly, interpreter(&settings, "fish"), &[], probe)?,
        "powershell" => powershell_plan(paths, assembly, &settings, probe)?,
        "ruby" => interpreted_plan(paths, assembly, interpreter(&settings, "ruby"), &[], probe)?,
        "perl" => interpreted_plan(paths, assembly, interpreter(&settings, "perl"), &[], probe)?,
        "lua" => interpreted_plan(paths, assembly, interpreter(&settings, "lua"), &[], probe)?,
        "r" => r_plan(paths, assembly, &settings, probe)?,
        "js" | "ts" => javascript_plan(paths, assembly, &settings, probe)?,
        "exe" => direct_plan(entry, assembly, probe)?,
        "command" => command_plan(assembly, &settings, probe)?,
        "prompt" => {
            let plan = prompt_plan(
                prompt_body,
                prompt_display_body,
                prompt_runner,
                assembly,
                probe,
            )?;
            warnings.extend(plan.warning);
            if prompt_runner.is_some_and(is_builtin_amp_runner) {
                warnings.push(LaunchWarning::AmpOneShot);
            }
            (plan.program, plan.args, plan.display_args)
        }
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
        warnings,
    })
}

fn is_builtin_amp_runner(runner: &PromptRunner) -> bool {
    runner.name == "amp"
        && runner.argv == ["amp".to_owned(), "-x".to_owned(), "{{prompt}}".to_owned()]
}

fn python_plan<P: ProgramProbe>(
    paths: &LaunchPaths,
    assembly: &Assembly,
    settings: &EntrySettings,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    require_file(&paths.script, probe)?;
    let uv = require_program(interpreter(settings, "uv"), probe)?;
    let mut prefix = vec!["run".to_owned(), "--no-project".to_owned()];
    if !settings.requires_python.is_empty() {
        prefix.push("--python".to_owned());
        prefix.push(settings.requires_python.clone());
    }
    for dependency in &settings.dependencies {
        prefix.push("--with".to_owned());
        prefix.push(dependency.clone());
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

fn powershell_plan<P: ProgramProbe>(
    paths: &LaunchPaths,
    assembly: &Assembly,
    settings: &EntrySettings,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    interpreted_plan(
        paths,
        assembly,
        interpreter(settings, "pwsh"),
        &["-File"],
        probe,
    )
}

fn r_plan<P: ProgramProbe>(
    paths: &LaunchPaths,
    assembly: &Assembly,
    settings: &EntrySettings,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    interpreted_plan(
        paths,
        assembly,
        interpreter(settings, "Rscript"),
        &[],
        probe,
    )
}

fn javascript_plan<P: ProgramProbe>(
    paths: &LaunchPaths,
    assembly: &Assembly,
    settings: &EntrySettings,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    require_file(&paths.script, probe)?;
    let runtime = resolve_javascript_runtime(settings, probe)?;
    let program = require_program(&runtime, probe)?;
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

/// Select the JavaScript runtime by entry pin and deterministic availability order.
pub fn resolve_javascript_runtime<P: ProgramProbe>(
    settings: &EntrySettings,
    probe: &P,
) -> Result<String, LaunchError> {
    if !settings.interpreter.is_empty() {
        require_program(&settings.interpreter, probe)?;
        return Ok(settings.interpreter.clone());
    }
    ["deno", "bun", "node"]
        .into_iter()
        .find(|name| probe.find_program(name).is_some())
        .map(str::to_owned)
        .ok_or_else(|| LaunchError::ProgramNotFound {
            name: "deno, bun, or node".to_owned(),
        })
}

fn direct_plan<P: ProgramProbe>(
    entry: &Entry,
    assembly: &Assembly,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    let path = PathBuf::from(&entry.meta.source);
    if !probe.exists(&path) {
        return Err(LaunchError::TargetMissing { path });
    }
    if !probe.is_file(&path) || !probe.is_executable(&path) {
        return Err(LaunchError::TargetNotExecutable { path });
    }
    Ok((path, assembly.args.clone(), assembly.masked_args.clone()))
}

fn command_plan<P: ProgramProbe>(
    assembly: &Assembly,
    settings: &EntrySettings,
    probe: &P,
) -> Result<(PathBuf, Vec<String>, Vec<String>), LaunchError> {
    let missing = settings
        .params
        .iter()
        .filter(|name| !assembly.command_values.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(LaunchError::MissingTemplateValue {
            name: missing.join(", "),
        });
    }
    let command = append_shell_args(
        render_command_template(&settings.template, &assembly.command_values)?,
        &assembly.args,
    );
    let masked = append_shell_args(
        render_command_template(&settings.template, &assembly.masked_command_values)?,
        &assembly.masked_args,
    );
    #[cfg(windows)]
    {
        let shell = require_program("cmd.exe", probe)?;
        return Ok((
            shell,
            vec!["/C".to_owned(), command],
            vec!["/C".to_owned(), masked],
        ));
    }
    #[cfg(not(windows))]
    {
        let shell = require_program("sh", probe)?;
        Ok((
            shell,
            vec!["-c".to_owned(), command],
            vec!["-c".to_owned(), masked],
        ))
    }
}

fn append_shell_args(mut command: String, args: &[String]) -> String {
    if !args.is_empty() {
        command.push(' ');
        command.push_str(
            &args
                .iter()
                .map(|value| quote_shell_arg(value))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    command
}

fn prompt_plan<P: ProgramProbe>(
    prompt_body: Option<&str>,
    prompt_display_body: Option<&str>,
    runner: Option<&PromptRunner>,
    assembly: &Assembly,
    probe: &P,
) -> Result<PromptProcessPlan, LaunchError> {
    let body = prompt_body.ok_or(LaunchError::PromptBodyRequired)?;
    let runner = runner.ok_or(LaunchError::PromptRunnerRequired)?;
    let marker = "{{prompt}}";
    let program_token = runner
        .argv
        .first()
        .ok_or_else(|| LaunchError::InvalidPromptRunner {
            name: runner.name.clone(),
        })?;
    let marker_count = runner
        .argv
        .iter()
        .skip(1)
        .map(|token| token.matches(marker).count())
        .sum::<usize>();
    if marker_count != 1 || program_token.contains(marker) {
        return Err(LaunchError::InvalidPromptRunner {
            name: runner.name.clone(),
        });
    }
    let (body, protected) = protect_pi_prompt(program_token, body);
    let filled = fill_prompt_argv(&runner.argv, &body, &assembly.args);
    validate_prompt_argv(&filled)?;
    let program = require_program(program_token, probe)?;
    let args = filled.into_iter().skip(1).collect::<Vec<_>>();
    let display_body = prompt_display_body.map_or_else(
        || "<rendered prompt omitted; use --dry-run to inspect it>".to_owned(),
        |body| {
            if protected {
                format!("\n{body}")
            } else {
                body.to_owned()
            }
        },
    );
    let display = fill_prompt_argv(&runner.argv, &display_body, &assembly.masked_args)
        .into_iter()
        .skip(1)
        .collect::<Vec<_>>();
    let warning = protected.then_some(LaunchWarning::PiPromptProtected);
    Ok(PromptProcessPlan {
        program,
        args,
        display_args: display,
        warning,
    })
}

fn fill_prompt_argv(template: &[String], body: &str, extra: &[String]) -> Vec<String> {
    let mut filled = template
        .iter()
        .map(|token| token.replace("{{prompt}}", body))
        .collect::<Vec<_>>();
    let insertion = template
        .iter()
        .position(|token| token == "--")
        .unwrap_or(filled.len());
    filled.splice(insertion..insertion, extra.iter().cloned());
    filled
}

const POSIX_PROMPT_ARGV_LIMIT: usize = 100_000;
const WINDOWS_PROMPT_ARGV_LIMIT: usize = 60_000;
const PI_PACKAGE_COMMANDS: [&str; 6] =
    ["config", "install", "list", "remove", "uninstall", "update"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PromptPlatform {
    Posix,
    Windows,
}

/// The build target's prompt platform. This is a compile-time choice.
const CURRENT_PROMPT_PLATFORM: PromptPlatform = if cfg!(windows) {
    PromptPlatform::Windows
} else {
    PromptPlatform::Posix
};

fn protect_pi_prompt(program: &str, body: &str) -> (String, bool) {
    let program = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    let is_pi = matches!(program.as_str(), "pi" | "pi.cmd" | "pi.exe" | "pi.ps1");
    let ambiguous = body.starts_with(['-', '@']) || PI_PACKAGE_COMMANDS.contains(&body);
    if is_pi && ambiguous {
        (format!("\n{body}"), true)
    } else {
        (body.to_owned(), false)
    }
}

fn validate_prompt_argv(argv: &[String]) -> Result<(), LaunchError> {
    if argv.iter().any(|token| token.contains('\0')) {
        return Err(LaunchError::PromptContainsNul);
    }
    let (size, limit, unit) = prompt_argv_size(argv, CURRENT_PROMPT_PLATFORM);
    if size > limit {
        Err(LaunchError::PromptArgvTooLong { size, limit, unit })
    } else {
        Ok(())
    }
}

fn prompt_argv_size(argv: &[String], platform: PromptPlatform) -> (usize, usize, &'static str) {
    match platform {
        PromptPlatform::Posix => (
            argv.iter().map(|token| token.len().saturating_add(1)).sum(),
            POSIX_PROMPT_ARGV_LIMIT,
            "bytes",
        ),
        PromptPlatform::Windows => {
            let command = argv
                .iter()
                .map(|token| quote_windows_arg(token))
                .collect::<Vec<_>>()
                .join(" ");
            (
                command
                    .encode_utf16()
                    .count()
                    .saturating_mul(2)
                    .saturating_add(2),
                WINDOWS_PROMPT_ARGV_LIMIT,
                "bytes",
            )
        }
    }
}

/// Fill a command template with shell-quoted values.
///
/// The template is user-authored shell text. skit replaces known `{name}` slots in one pass,
/// restores template-level double-brace escapes, and leaves unknown slots unchanged. Replacement
/// text is not scanned again, so braces in a value remain byte-exact.
pub fn render_command_template(
    template: &str,
    values: &BTreeMap<String, String>,
) -> Result<String, LaunchError> {
    #[cfg(windows)]
    {
        render_windows_command_template(template, values)
    }
    #[cfg(not(windows))]
    {
        render_posix_command_template(template, values)
    }
}

#[derive(Clone, Copy, Debug)]
enum TemplateToken<'a> {
    OpenBrace,
    CloseBrace,
    Placeholder(&'a str),
}

#[derive(Clone, Copy, Debug)]
struct TemplateTokenSpan<'a> {
    start: usize,
    end: usize,
    token: TemplateToken<'a>,
}

fn next_template_token(template: &str, mut index: usize) -> Option<TemplateTokenSpan<'_>> {
    let bytes = template.as_bytes();
    while index < bytes.len() {
        if bytes[index..].starts_with(b"{{") {
            return Some(TemplateTokenSpan {
                start: index,
                end: index.saturating_add(2),
                token: TemplateToken::OpenBrace,
            });
        }
        if bytes[index..].starts_with(b"}}") {
            return Some(TemplateTokenSpan {
                start: index,
                end: index.saturating_add(2),
                token: TemplateToken::CloseBrace,
            });
        }
        if bytes[index] == b'{' && (index == 0 || bytes[index - 1] != b'{') {
            let name_start = index.saturating_add(1);
            let mut end = name_start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end = end.saturating_add(1);
            }
            if end < bytes.len()
                && bytes[end] == b'}'
                && bytes.get(end.saturating_add(1)) != Some(&b'}')
                && let Some(name) = template.get(name_start..end)
                && valid_placeholder(name)
            {
                return Some(TemplateTokenSpan {
                    start: index,
                    end: end.saturating_add(1),
                    token: TemplateToken::Placeholder(name),
                });
            }
        }
        let character = template[index..]
            .chars()
            .next()
            .expect("index is on a UTF-8 boundary");
        index = index.saturating_add(character.len_utf8());
    }
    None
}

#[cfg(windows)]
fn render_windows_command_template(
    template: &str,
    values: &BTreeMap<String, String>,
) -> Result<String, LaunchError> {
    let mut output = String::new();
    let mut position = 0;
    while let Some(span) = next_template_token(template, position) {
        output.push_str(&template[position..span.start]);
        match span.token {
            TemplateToken::OpenBrace => output.push('{'),
            TemplateToken::CloseBrace => output.push('}'),
            TemplateToken::Placeholder(name) => {
                if let Some(value) = values.get(name) {
                    output.push_str(&quote_windows_arg(value));
                } else {
                    output.push_str(&template[span.start..span.end]);
                }
            }
        }
        position = span.end;
    }
    output.push_str(&template[position..]);
    Ok(output)
}

#[cfg(not(windows))]
#[derive(Clone, Debug, Default)]
struct PosixQuoteState {
    frames: Vec<char>,
    escape_pending: bool,
}

#[cfg(not(windows))]
impl PosixQuoteState {
    fn advance(&mut self, text: &str) {
        let chars = text.chars().collect::<Vec<_>>();
        let mut index = 0;
        if self.escape_pending {
            if chars.is_empty() {
                return;
            }
            self.escape_pending = false;
            index = 1;
        }
        while index < chars.len() {
            let character = chars[index];
            let top = self.frames.last().copied();
            if top == Some('\'') {
                if character == '\'' {
                    self.frames.pop();
                }
            } else if character == '\\' {
                if index.saturating_add(1) >= chars.len() {
                    self.escape_pending = true;
                    return;
                }
                index = index.saturating_add(1);
            } else if character == '$' && chars.get(index.saturating_add(1)) == Some(&'(') {
                self.frames.push('(');
                index = index.saturating_add(1);
            } else if matches!(character, '`' | '"') {
                if top == Some(character) {
                    self.frames.pop();
                } else {
                    self.frames.push(character);
                }
            } else if character == '\'' && top != Some('"') {
                self.frames.push(character);
            } else if character == ')' && top == Some('(') {
                self.frames.pop();
            }
            index = index.saturating_add(1);
        }
    }

    fn take_pending_escape(&mut self) -> bool {
        std::mem::take(&mut self.escape_pending)
    }

    fn quote_value(&self, name: &str, value: &str) -> Result<String, LaunchError> {
        match self.frames.last().copied() {
            Some('\'') => Ok(value.replace('\'', "'\\''")),
            Some('"') => {
                if self.frames.contains(&'`') {
                    return Err(LaunchError::UnsafeTemplatePlaceholder {
                        name: name.to_owned(),
                    });
                }
                let value = value.replace('\\', "\\\\").replace('"', "\\\"");
                Ok(value.replace('$', "\\$").replace('`', "\\`"))
            }
            _ => Ok(quote_posix_arg(value)),
        }
    }
}

#[cfg(not(windows))]
fn render_posix_command_template(
    template: &str,
    values: &BTreeMap<String, String>,
) -> Result<String, LaunchError> {
    let mut output = String::new();
    let mut state = PosixQuoteState::default();
    let mut position = 0;
    while let Some(span) = next_template_token(template, position) {
        let chunk = &template[position..span.start];
        output.push_str(chunk);
        state.advance(chunk);
        let pending_escape = state.take_pending_escape();
        match span.token {
            TemplateToken::OpenBrace => output.push('{'),
            TemplateToken::CloseBrace => output.push('}'),
            TemplateToken::Placeholder(name) => {
                if let Some(value) = values.get(name) {
                    if pending_escape {
                        output.push('\\');
                    }
                    output.push_str(&state.quote_value(name, value)?);
                } else {
                    output.push_str(&template[span.start..span.end]);
                }
            }
        }
        position = span.end;
    }
    output.push_str(&template[position..]);
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

/// Resolve the child working directory with the same rules used by launch planning.
///
/// Frontends use this projection for path completion. Keeping the resolver public prevents a
/// form adapter from approximating `origin`, copy fallback, or custom-path validation.
pub fn resolve_launch_workdir<P: ProgramProbe>(
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
        128 + status
            .signal()
            .expect("a Unix status without a code has a signal")
    }
    #[cfg(not(unix))]
    {
        125
    }
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

#[cfg(windows)]
fn quote_shell_arg(value: &str) -> String {
    quote_windows_arg(value)
}

#[cfg(not(windows))]
fn quote_shell_arg(value: &str) -> String {
    quote_posix_arg(value)
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

#[cfg(test)]
mod private_tests {
    use super::*;
    use skit_domain::{EntryKind, EntryMeta, Slug};
    use tempfile::TempDir;

    #[derive(Debug, Default)]
    struct Probe {
        programs: BTreeMap<String, PathBuf>,
        files: Vec<PathBuf>,
        directories: Vec<PathBuf>,
        executables: Vec<PathBuf>,
    }

    impl ProgramProbe for Probe {
        fn find_program(&self, name: &str) -> Option<PathBuf> {
            self.programs.get(name).cloned()
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.iter().any(|candidate| candidate == path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.directories.iter().any(|candidate| candidate == path)
        }

        fn is_executable(&self, path: &Path) -> bool {
            self.executables.iter().any(|candidate| candidate == path)
        }
    }

    fn entry(kind: &str) -> Entry {
        Entry {
            slug: Slug::parse("test").unwrap(),
            meta: EntryMeta::minimal("Test", EntryKind::parse(kind).unwrap()),
        }
    }

    fn paths() -> LaunchPaths {
        LaunchPaths {
            script: PathBuf::from("/data/test/script"),
            entry_dir: PathBuf::from("/data/test"),
            invoke_cwd: PathBuf::from("/invoke"),
        }
    }

    #[test]
    fn command_and_workdir_plans_cover_literal_and_refusal_paths() {
        let mut command = entry("command");
        command.meta.workdir = "/custom".to_owned();
        EntrySettings {
            template: "tool {{literal}} {value}".to_owned(),
            ..EntrySettings::default()
        }
        .write_to_meta(&mut command.meta);
        let probe = Probe {
            programs: BTreeMap::from([("sh".to_owned(), PathBuf::from("/bin/sh"))]),
            directories: vec![PathBuf::from("/custom")],
            ..Probe::default()
        };
        let assembly = Assembly {
            command_values: BTreeMap::from([("value".to_owned(), "a b".to_owned())]),
            masked_command_values: BTreeMap::from([("value".to_owned(), "***".to_owned())]),
            masked_env: BTreeMap::from([("TOKEN".to_owned(), "***".to_owned())]),
            ..Assembly::default()
        };
        let plan = build_launch_plan(&command, &paths(), &assembly, None, None, &probe).unwrap();
        assert_eq!(plan.program, PathBuf::from("/bin/sh"));
        assert!(plan.display.contains("TOKEN=\"***\"") || plan.display.contains("TOKEN='***'"));
        assert!(plan.args[1].contains("{literal}"));

        command.meta.workdir = "relative".to_owned();
        assert!(matches!(
            build_launch_plan(&command, &paths(), &assembly, None, None, &probe),
            Err(LaunchError::InvalidWorkdir { .. })
        ));
        command.meta.workdir = "/missing".to_owned();
        assert!(matches!(
            build_launch_plan(&command, &paths(), &assembly, None, None, &probe),
            Err(LaunchError::WorkdirMissing { .. })
        ));
    }

    #[test]
    fn target_prompt_and_template_refusals_have_stable_exit_codes() {
        assert_eq!(
            LaunchError::TargetMissing {
                path: PathBuf::new()
            }
            .exit_code(),
            127
        );
        assert_eq!(
            LaunchError::TargetNotExecutable {
                path: PathBuf::new()
            }
            .exit_code(),
            126
        );
        assert_eq!(LaunchError::PromptRunnerRequired.exit_code(), 126);
        assert_eq!(LaunchError::PromptBodyRequired.exit_code(), 125);
        assert_eq!(
            LaunchError::Process {
                operation: "test",
                source: io::Error::other("failed"),
            }
            .exit_code(),
            125
        );

        let mut executable = entry("exe");
        executable.meta.source = "/bin/tool".to_owned();
        executable.meta.workdir = "store".to_owned();
        let mut probe = Probe {
            directories: vec![PathBuf::from("/data/test")],
            ..Probe::default()
        };
        assert!(matches!(
            build_launch_plan(
                &executable,
                &paths(),
                &Assembly::default(),
                None,
                None,
                &probe,
            ),
            Err(LaunchError::TargetMissing { .. })
        ));
        probe.files.push(PathBuf::from("/bin/tool"));
        assert!(matches!(
            build_launch_plan(
                &executable,
                &paths(),
                &Assembly::default(),
                None,
                None,
                &probe,
            ),
            Err(LaunchError::TargetNotExecutable { .. })
        ));

        let mut prompt = entry("prompt");
        prompt.meta.workdir = "store".to_owned();
        assert!(matches!(
            build_launch_plan(&prompt, &paths(), &Assembly::default(), None, None, &probe,),
            Err(LaunchError::PromptBodyRequired)
        ));
        assert!(matches!(
            build_launch_plan(
                &prompt,
                &paths(),
                &Assembly::default(),
                Some("body"),
                None,
                &probe,
            ),
            Err(LaunchError::PromptRunnerRequired)
        ));
        assert_eq!(
            render_command_template("tool {missing}", &BTreeMap::new()).unwrap(),
            "tool {missing}"
        );
        assert_eq!(
            render_command_template(r#"tool \\"x" {{ok}} }}"#, &BTreeMap::new()).unwrap(),
            r#"tool \\"x" {ok} }"#
        );
    }

    #[test]
    fn system_probe_and_process_execution_use_real_operating_system_status() {
        let root = TempDir::new().unwrap();
        let executable = root.path().join("tool");
        fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut permissions = fs::metadata(&executable).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&executable, permissions).unwrap();
        }
        let probe = SystemProbe;
        assert!(probe.is_file(&executable));
        assert!(probe.is_dir(root.path()));
        assert!(probe.is_executable(&executable));
        assert_eq!(
            probe.find_program(executable.to_str().unwrap()),
            Some(executable)
        );
        assert!(probe.find_program("sh").is_some());
        assert!(
            probe
                .find_program("skit-program-that-does-not-exist")
                .is_none()
        );
        assert!(!probe.is_executable(root.path()));

        let plan = LaunchPlan {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), "exit 7".to_owned()],
            env: BTreeMap::new(),
            cwd: root.path().to_owned(),
            display: String::new(),
            warnings: Vec::new(),
        };
        assert_eq!(execute_launch(&plan).unwrap(), 7);
        let missing = LaunchPlan {
            program: root.path().join("missing"),
            ..plan
        };
        assert!(matches!(
            execute_launch(&missing),
            Err(LaunchError::Process { .. })
        ));

        #[cfg(unix)]
        {
            let status = Command::new("/bin/sh")
                .args(["-c", "kill -TERM $$"])
                .status()
                .unwrap();
            assert_eq!(status_code(status), 143);
        }
    }

    #[test]
    fn windows_quoting_is_deterministic_on_every_build_host() {
        assert_eq!(quote_windows_arg("plain"), "plain");
        assert_eq!(quote_windows_arg(""), "\"\"");
        assert_eq!(quote_windows_arg("a b"), "\"a b\"");
        assert_eq!(quote_windows_arg("a\\\"b\\"), "\"a\\\\\\\"b\\\\\"");
    }

    #[test]
    fn windows_prompt_size_uses_the_quoted_utf16_command_line() {
        let argv = ["pi.exe".to_owned(), "x".repeat(60_000)];
        let (size, limit, unit) = prompt_argv_size(&argv, PromptPlatform::Windows);
        assert!(size > limit);
        assert_eq!(limit, 60_000);
        assert_eq!(unit, "bytes");
    }
}
