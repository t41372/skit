//! Materialize private JavaScript dependencies beside a stored entry.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use sha2::{Digest as _, Sha256};
use skit_i18n::{Localize, Message};
use thiserror::Error;

use crate::ProgramProbe;

const STAMP_NAME: &str = ".skit-deps";
const MARKER_NAME: &str = ".skit-deps-ok";
const BACKUP_NAME: &str = ".skit-deps.backup";
const BACKUP_INDEX: &str = ".items";
const STAGE_PREFIX: &str = ".skit-deps.tmp-";
const STALE_INJECTED_AGE: Duration = Duration::from_secs(60 * 60);
const OWNED_FILES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "bun.lock",
    "bun.lockb",
    "deno.lock",
    STAMP_NAME,
];

/// Preserve an explicit JavaScript module flavor from the original source name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JavaScriptModuleType {
    /// Use ECMAScript module semantics.
    Module,
    /// Use CommonJS module semantics.
    CommonJs,
}

impl JavaScriptModuleType {
    const fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::CommonJs => "commonjs",
        }
    }
}

/// Return the explicit module type encoded by a source filename.
#[must_use]
pub fn javascript_module_type(source: &str) -> Option<JavaScriptModuleType> {
    let source = source.to_ascii_lowercase();
    if source.ends_with(".mjs") || source.ends_with(".mts") {
        Some(JavaScriptModuleType::Module)
    } else if source.ends_with(".cjs") || source.ends_with(".cts") {
        Some(JavaScriptModuleType::CommonJs)
    } else {
        None
    }
}

/// One package-manager process request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyCommand {
    /// Resolved package-manager executable.
    pub program: PathBuf,
    /// Arguments after the executable.
    pub args: Vec<String>,
    /// Private entry directory.
    pub cwd: PathBuf,
    /// Child-only environment values.
    pub environment: BTreeMap<String, String>,
}

/// Start one package-manager command.
pub trait DependencyCommandRunner: std::fmt::Debug {
    /// Report that one installer process is about to start.
    fn installation_started(&self, _installer: &str) {}

    /// Return the child's status and captured diagnostic stream.
    fn run(&self, command: &DependencyCommand) -> io::Result<DependencyCommandOutput>;
}

/// Captured result of one package-manager process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyCommandOutput {
    /// Whether the process exited successfully.
    pub success: bool,
    /// Numeric exit code, or `None` when a signal or platform event ended the process.
    pub exit_code: Option<i32>,
    /// Exact stderr bytes. Decoding belongs to the failure presenter.
    pub stderr: Vec<u8>,
}

/// Start dependency commands on the local machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemDependencyCommandRunner;

impl DependencyCommandRunner for SystemDependencyCommandRunner {
    fn run(&self, command: &DependencyCommand) -> io::Result<DependencyCommandOutput> {
        Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.cwd)
            .envs(&command.environment)
            .output()
            .map(|output| DependencyCommandOutput {
                success: output.status.success(),
                exit_code: output.status.code(),
                stderr: output.stderr,
            })
    }
}

/// One reversible cleanup of a private JavaScript dependency environment.
///
/// The guard holds the persistent per-entry dependency lock. Call [`Self::finalize`] after every
/// dependent commit succeeds, or call [`Self::rollback`] when one fails. Dropping an unresolved
/// guard attempts disaster recovery, but normal callers must handle the typed result explicitly.
#[derive(Debug)]
#[must_use = "finalize or roll back the prepared JavaScript dependency cleanup"]
pub struct PreparedJavaScriptDependencyCleanup {
    entry_dir: PathBuf,
    backup_started: bool,
    resolved: bool,
    _lock: Option<DependencyLock>,
}

impl PreparedJavaScriptDependencyCleanup {
    /// Commit the cleanup and remove the quarantined old environment.
    pub fn finalize(&mut self) -> Result<(), DependencyError> {
        let result = if self.backup_started {
            finish_dependency_backup(&self.entry_dir)
        } else {
            Ok(())
        };
        if result.is_ok() || !path_exists(&self.entry_dir.join(BACKUP_NAME)) {
            self.resolved = true;
            self._lock.take();
        }
        result
    }

    /// Restore the quarantined old environment while the dependency lock remains held.
    pub fn rollback(&mut self) -> Result<(), DependencyError> {
        if self.resolved {
            return Ok(());
        }
        let result = if self.backup_started {
            recover_dependency_backup(&self.entry_dir)
        } else {
            Ok(())
        };
        if result.is_ok() {
            self.resolved = true;
            self._lock.take();
        }
        result
    }
}

impl Drop for PreparedJavaScriptDependencyCleanup {
    fn drop(&mut self) {
        if !self.resolved && self.backup_started {
            let _ = recover_dependency_backup(&self.entry_dir);
        }
    }
}

/// Report a private JavaScript dependency failure.
#[derive(Debug, Error)]
pub enum DependencyError {
    /// Managed dependencies cannot change a referenced source directory.
    #[error("managed JavaScript dependencies require copy storage")]
    CopyStorageRequired,
    /// A package specification cannot become a package.json key.
    #[error("invalid JavaScript package specification: {value}")]
    InvalidPackage { value: String },
    /// The selected runtime cannot manage npm dependencies.
    #[error("runtime {runtime:?} cannot manage JavaScript dependencies")]
    UnsupportedRuntime { runtime: String },
    /// The selected runtime's package manager is not available.
    #[error("{name} is needed to install this script's dependencies, but it isn't on your PATH.")]
    InstallerNotFound { name: String },
    /// A private support file could not be updated.
    #[error("could not {operation} JavaScript dependencies at {path}: {reason}")]
    Io {
        /// File operation name.
        operation: &'static str,
        /// Affected path.
        path: String,
        /// Operating-system detail.
        reason: String,
    },
    /// An old dependency artifact could not be removed.
    #[error("Couldn't clear the old dependency environment: {item}: {reason}")]
    ClearFailed {
        /// Entry-relative artifact name.
        item: String,
        /// Operating-system detail.
        reason: String,
    },
    /// A dependency update and its recovery both failed.
    #[error("rollback at {path} failed after {primary}: {rollback}")]
    Rollback {
        /// Affected entry directory.
        path: String,
        /// Failure that started recovery.
        primary: Box<Self>,
        /// Failure from the recovery attempt.
        rollback: Box<Self>,
    },
    /// The package manager could not start.
    #[error("Couldn't run {installer}: {reason}")]
    InstallerStartFailed {
        /// Package-manager name implied by the selected runtime.
        installer: String,
        /// Operating-system detail.
        reason: String,
    },
    /// The package manager returned a failure status.
    #[error("Installing dependencies failed ({installer}): {detail}")]
    InstallFailed {
        /// Package-manager name implied by the selected runtime.
        installer: String,
        /// Numeric status when the platform supplied one.
        exit_code: Option<i32>,
        /// Most useful trusted line from the package manager's stderr.
        detail: String,
    },
}

impl Localize for DependencyError {
    fn message(&self) -> Message {
        match self {
            Self::CopyStorageRequired => {
                Message::new("managed JavaScript dependencies require copy storage")
            }
            Self::InvalidPackage { value } => {
                Message::new("invalid JavaScript package specification: {}").with(value)
            }
            Self::UnsupportedRuntime { runtime } => {
                Message::new("runtime {} cannot manage JavaScript dependencies").quoted(runtime)
            }
            Self::InstallerNotFound { name } => Message::new(
                "{} is needed to install this script's dependencies, but it isn't on your PATH.",
            )
            .with(name),
            Self::Io {
                operation,
                path,
                reason,
            } => Message::new("could not {} JavaScript dependencies at {}: {}")
                .nested(Message::term(operation))
                .with(path)
                .with(reason),
            Self::ClearFailed { item, reason } => {
                Message::new("Couldn't clear the old dependency environment: {}")
                    .with(format!("{item}: {reason}"))
            }
            Self::Rollback {
                path,
                primary,
                rollback,
            } => Message::new("rollback at {} failed after {}: {}")
                .with(path)
                .nested(primary.message())
                .nested(rollback.message()),
            Self::InstallerStartFailed { installer, reason } => Message::new("Couldn't run {}: {}")
                .with(installer)
                .with(reason),
            Self::InstallFailed {
                installer, detail, ..
            } => Message::new("Installing dependencies failed ({}): {}")
                .with(installer)
                .with(detail),
        }
    }
}

/// Present the one receipt emitted immediately before a package manager starts.
#[must_use]
pub fn javascript_dependency_install_announcement(installer: &str) -> Message {
    Message::new("Installing dependencies ({})…").with(installer)
}

/// Build the deterministic private package.json document.
pub fn javascript_dependency_manifest(dependencies: &[String]) -> Result<String, DependencyError> {
    javascript_dependency_manifest_for_module(dependencies, None)
}

/// Build the deterministic private package.json with an explicit module type.
pub fn javascript_dependency_manifest_for_module(
    dependencies: &[String],
    module_type: Option<JavaScriptModuleType>,
) -> Result<String, DependencyError> {
    let mut rows: Vec<(String, String)> = Vec::new();
    for dependency in dependencies {
        if dependency.trim().is_empty() {
            continue;
        }
        let (name, version) = split_javascript_requirement(dependency.trim());
        if let Some((_, old_version)) = rows.iter_mut().find(|(old_name, _)| old_name == &name) {
            *old_version = version;
        } else {
            rows.push((name, version));
        }
    }
    let mut output = String::from("{\n  \"private\": true,\n");
    if let Some(module_type) = module_type {
        output.push_str(&format!(
            "  \"type\": {},\n",
            serde_json::to_string(module_type.as_manifest_value())
                .expect("a module type is valid JSON")
        ));
    }
    if rows.is_empty() {
        output.push_str("  \"dependencies\": {}\n}\n");
        return Ok(output);
    }
    output.push_str("  \"dependencies\": {\n");
    for (index, (name, version)) in rows.iter().enumerate() {
        let comma = if index + 1 == rows.len() { "" } else { "," };
        output.push_str(&format!(
            "    {}: {}{comma}\n",
            serde_json::to_string(name).expect("a package name is valid JSON"),
            serde_json::to_string(version).expect("a package version is valid JSON"),
        ));
    }
    output.push_str("  }\n}\n");
    Ok(output)
}

/// Split a comma-separated JavaScript package requirement list.
#[must_use]
pub fn split_javascript_requirements(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|requirement| !requirement.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Split one JavaScript package requirement into its package name and version range.
#[must_use]
pub fn split_javascript_requirement(requirement: &str) -> (String, String) {
    let (name, version) =
        requirement
            .rfind('@')
            .filter(|index| *index > 0)
            .map_or((requirement, "*"), |index| {
                let name = &requirement[..index];
                if name.ends_with('/') {
                    (requirement, "*")
                } else {
                    let version = &requirement[index + 1..];
                    (name, if version.is_empty() { "*" } else { version })
                }
            });
    (name.to_owned(), version.to_owned())
}

fn javascript_module_manifest(module_type: JavaScriptModuleType) -> String {
    format!(
        "{{\n  \"private\": true,\n  \"type\": {}\n}}\n",
        serde_json::to_string(module_type.as_manifest_value())
            .expect("a module type is valid JSON")
    )
}

/// Make the entry's private dependency tree match its declared packages.
pub fn ensure_javascript_dependencies<P, R>(
    entry_dir: &Path,
    runtime: &str,
    dependencies: &[String],
    probe: &P,
    runner: &R,
) -> Result<(), DependencyError>
where
    P: ProgramProbe,
    R: DependencyCommandRunner,
{
    ensure_javascript_dependencies_with_environment(
        entry_dir,
        runtime,
        dependencies,
        &BTreeMap::new(),
        probe,
        runner,
    )
}

/// Make the private dependency tree with explicit child-only environment values.
pub fn ensure_javascript_dependencies_with_environment<P, R>(
    entry_dir: &Path,
    runtime: &str,
    dependencies: &[String],
    environment: &BTreeMap<String, String>,
    probe: &P,
    runner: &R,
) -> Result<(), DependencyError>
where
    P: ProgramProbe,
    R: DependencyCommandRunner,
{
    ensure_javascript_dependencies_for_module(
        entry_dir,
        runtime,
        dependencies,
        None,
        environment,
        probe,
        runner,
    )
}

/// Report whether the private dependency tree is stale without locking or changing it.
pub fn javascript_dependencies_need_install(
    entry_dir: &Path,
    runtime: &str,
    dependencies: &[String],
) -> Result<bool, DependencyError> {
    javascript_dependencies_need_install_for_module(entry_dir, runtime, dependencies, None)
}

/// Report dependency staleness while preserving an explicit source module type.
pub fn javascript_dependencies_need_install_for_module(
    entry_dir: &Path,
    runtime: &str,
    dependencies: &[String],
    module_type: Option<JavaScriptModuleType>,
) -> Result<bool, DependencyError> {
    let state = resolve_dependency_state(runtime, dependencies, module_type)?;
    let marker_path = entry_dir.join("node_modules").join(MARKER_NAME);
    Ok(fs::read(marker_path).ok().as_deref() != Some(state.stamp.as_bytes()))
}

/// Check only the local package-manager requirement for a pending dependency install.
pub fn preflight_javascript_dependencies<P: ProgramProbe>(
    entry_dir: &Path,
    runtime: &str,
    dependencies: &[String],
    probe: &P,
) -> Result<(), DependencyError> {
    preflight_javascript_dependencies_for_module(entry_dir, runtime, dependencies, None, probe)
}

/// Check a pending install while preserving an explicit source module type.
pub fn preflight_javascript_dependencies_for_module<P: ProgramProbe>(
    entry_dir: &Path,
    runtime: &str,
    dependencies: &[String],
    module_type: Option<JavaScriptModuleType>,
    probe: &P,
) -> Result<(), DependencyError> {
    if dependencies.is_empty() {
        return Ok(());
    }
    let state = resolve_dependency_state(runtime, dependencies, module_type)?;
    let marker_path = entry_dir.join("node_modules").join(MARKER_NAME);
    if fs::read(marker_path).ok().as_deref() == Some(state.stamp.as_bytes()) {
        return Ok(());
    }
    resolve_javascript_dependency_installer(runtime, probe).map(|_| ())
}

/// Make a private dependency tree and preserve an explicit source module type.
pub fn ensure_javascript_dependencies_for_module<P, R>(
    entry_dir: &Path,
    runtime: &str,
    dependencies: &[String],
    module_type: Option<JavaScriptModuleType>,
    environment: &BTreeMap<String, String>,
    probe: &P,
    runner: &R,
) -> Result<(), DependencyError>
where
    P: ProgramProbe,
    R: DependencyCommandRunner,
{
    let _lock = dependency_lock(entry_dir)?;
    require_entry_directory(entry_dir)?;
    recover_dependency_backup(entry_dir)?;
    remove_staging_leftovers(entry_dir)?;
    if dependencies.is_empty() {
        return module_type.map_or_else(
            || clear_javascript_dependencies_unlocked(entry_dir),
            |module_type| ensure_module_manifest_unlocked(entry_dir, module_type),
        );
    }
    let state = resolve_dependency_state(runtime, dependencies, module_type)?;
    let marker_path = entry_dir.join("node_modules").join(MARKER_NAME);
    if fs::read(&marker_path).ok().as_deref() == Some(state.stamp.as_bytes()) {
        return Ok(());
    }

    // Resolve the installer before the transaction moves a complete environment aside.
    let command = dependency_command(entry_dir, runtime, environment, probe)?;
    begin_dependency_backup(entry_dir)?;
    let install = (|| {
        atomic_write(&entry_dir.join("package.json"), state.manifest.as_bytes())?;
        runner.installation_started(state.installer);
        let output =
            runner
                .run(&command)
                .map_err(|error| DependencyError::InstallerStartFailed {
                    installer: state.installer.to_owned(),
                    reason: error.to_string(),
                })?;
        if !output.success {
            return Err(DependencyError::InstallFailed {
                installer: state.installer.to_owned(),
                exit_code: output.exit_code,
                detail: javascript_dependency_failure_detail(&output.stderr),
            });
        }
        ensure_real_node_modules(entry_dir)?;
        atomic_write(&marker_path, state.stamp.as_bytes())
    })();
    match install {
        Ok(()) => finish_dependency_backup(entry_dir),
        Err(primary) => {
            let rollback = recover_dependency_backup(entry_dir);
            Err(combine_rollback_error(primary, rollback, entry_dir))
        }
    }
}

#[derive(Debug)]
struct DependencyState {
    installer: &'static str,
    manifest: String,
    stamp: String,
}

fn resolve_dependency_state(
    runtime: &str,
    dependencies: &[String],
    module_type: Option<JavaScriptModuleType>,
) -> Result<DependencyState, DependencyError> {
    let manifest = javascript_dependency_manifest_for_module(dependencies, module_type)?;
    let (installer, _) = installer_for_runtime(runtime);
    let stamp = dependency_stamp(installer, &manifest);
    Ok(DependencyState {
        installer,
        manifest,
        stamp,
    })
}

fn ensure_module_manifest_unlocked(
    entry_dir: &Path,
    module_type: JavaScriptModuleType,
) -> Result<(), DependencyError> {
    let manifest = javascript_module_manifest(module_type);
    let target = entry_dir.join("package.json");
    if read_optional(&target)?.as_deref() == Some(manifest.as_bytes()) {
        return Ok(());
    }
    let staged = TemporaryDependencyDirectory::new(entry_dir)?;
    atomic_write(&staged.path.join("package.json"), manifest.as_bytes())?;
    commit_dependency_stage(entry_dir, &staged.path)
}

/// Remove JavaScript dependency artifacts from one private entry directory.
pub fn clear_javascript_dependencies(entry_dir: &Path) -> Result<(), DependencyError> {
    let mut cleanup = prepare_javascript_dependency_cleanup(entry_dir)?;
    cleanup.finalize()
}

/// Quarantine every owned JavaScript dependency artifact under one persistent lock.
///
/// The entry directory is already in the requested cleared shape when this returns. The old tree
/// remains recoverable until the caller finalizes the guard.
pub fn prepare_javascript_dependency_cleanup(
    entry_dir: &Path,
) -> Result<PreparedJavaScriptDependencyCleanup, DependencyError> {
    let lock = dependency_lock(entry_dir)?;
    require_entry_directory(entry_dir)?;
    recover_dependency_backup(entry_dir)?;
    remove_staging_leftovers(entry_dir)?;
    sweep_stale_injected_sources(entry_dir);
    validate_dependency_item_shapes(entry_dir)?;
    let backup_started = dependency_items().any(|name| path_exists(&entry_dir.join(name)));
    if backup_started {
        begin_dependency_backup(entry_dir)?;
    }
    Ok(PreparedJavaScriptDependencyCleanup {
        entry_dir: entry_dir.to_owned(),
        backup_started,
        resolved: false,
        _lock: Some(lock),
    })
}

fn clear_javascript_dependencies_unlocked(entry_dir: &Path) -> Result<(), DependencyError> {
    clear_javascript_dependencies_unlocked_with(
        entry_dir,
        &mut system_remove_file,
        &mut system_remove_dir_all,
    )
}

fn clear_javascript_dependencies_unlocked_with<F, D>(
    entry_dir: &Path,
    remove_file: &mut F,
    remove_dir_all: &mut D,
) -> Result<(), DependencyError>
where
    F: FnMut(&Path) -> io::Result<()>,
    D: FnMut(&Path) -> io::Result<()>,
{
    sweep_stale_injected_sources(entry_dir);
    validate_dependency_item_shapes(entry_dir)?;
    if dependency_items().any(|name| path_exists(&entry_dir.join(name))) {
        let staged = TemporaryDependencyDirectory::new(entry_dir)?;
        commit_dependency_stage_with_remover(
            entry_dir,
            &staged.path,
            |entry_dir| Ok(unused_temporary_path(entry_dir)),
            remove_file,
            remove_dir_all,
        )?;
    }
    Ok(())
}

/// Remove secret-bearing injected source copies that are too old to belong to a live launch.
///
/// This hygiene operation is best-effort. It never blocks a launch or dependency cleanup.
pub fn sweep_stale_injected_sources(entry_dir: &Path) {
    sweep_stale_injected_at(entry_dir, SystemTime::now());
}

fn sweep_stale_injected_at(entry_dir: &Path, now: SystemTime) {
    sweep_stale_injected_before(entry_dir, now.checked_sub(STALE_INJECTED_AGE));
}

fn sweep_stale_injected_before(entry_dir: &Path, cutoff: Option<SystemTime>) {
    sweep_stale_injected_before_with(entry_dir, cutoff, &mut |path| fs::remove_file(path));
}

fn sweep_stale_injected_before_with<F>(
    entry_dir: &Path,
    cutoff: Option<SystemTime>,
    remove_file: &mut F,
) where
    F: FnMut(&Path) -> io::Result<()>,
{
    let Some(cutoff) = cutoff else {
        return;
    };
    let Ok(items) = fs::read_dir(entry_dir) else {
        return;
    };
    for item in items.flatten() {
        let path = item.path();
        let is_injected = item
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(".injected-"));
        let is_stale = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified < cutoff);
        if is_injected && is_stale {
            let _ = remove_file(&path);
        }
    }
}

fn validate_dependency_item_shapes(entry_dir: &Path) -> Result<(), DependencyError> {
    for name in OWNED_FILES {
        let path = entry_dir.join(name);
        let Some(metadata) = optional_symlink_metadata(&path)? else {
            continue;
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            return Err(DependencyError::Io {
                operation: "inspect",
                path: path.display().to_string(),
                reason: "an owned dependency file is a directory".to_owned(),
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
struct DependencyLock {
    _file: File,
}

fn dependency_lock(entry_dir: &Path) -> Result<DependencyLock, DependencyError> {
    let parent = entry_dir.parent().ok_or_else(|| DependencyError::Io {
        operation: "lock",
        path: entry_dir.display().to_string(),
        reason: "entry directory has no parent".to_owned(),
    })?;
    let lock_root = if parent.file_name().is_some_and(|name| name == "scripts") {
        parent.parent().unwrap_or(parent)
    } else {
        parent
    };
    let locks = lock_root.join(".locks");
    fs::create_dir_all(&locks).map_err(|error| io_error("create", &locks, error))?;
    let name = entry_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("entry");
    let path = locks.join(format!("{name}.skit-deps.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| io_error("open lock for", &path, error))?;
    file.lock()
        .map_err(|error| io_error("lock", &path, error))?;
    Ok(DependencyLock { _file: file })
}

fn dependency_command<P: ProgramProbe>(
    entry_dir: &Path,
    runtime: &str,
    environment: &BTreeMap<String, String>,
    probe: &P,
) -> Result<DependencyCommand, DependencyError> {
    let (_, args) = installer_for_runtime(runtime);
    let program = resolve_javascript_dependency_installer(runtime, probe)?;
    Ok(DependencyCommand {
        program,
        args: args.iter().map(|value| (*value).to_owned()).collect(),
        cwd: entry_dir.to_owned(),
        environment: environment.clone(),
    })
}

/// Resolve the package manager implied by a JavaScript runtime.
pub fn resolve_javascript_dependency_installer<P: ProgramProbe>(
    runtime: &str,
    probe: &P,
) -> Result<PathBuf, DependencyError> {
    let (installer, _) = installer_for_runtime(runtime);
    probe
        .find_program(installer)
        .ok_or_else(|| DependencyError::InstallerNotFound {
            name: installer.to_owned(),
        })
}

fn installer_for_runtime(runtime: &str) -> (&'static str, &'static [&'static str]) {
    match runtime {
        "bun" => ("bun", &["install", "--ignore-scripts"]),
        "deno" => ("deno", &["install"]),
        _ => (
            "npm",
            &["install", "--no-audit", "--no-fund", "--ignore-scripts"],
        ),
    }
}

const INSTALLER_NOISE: &[&str] = &[
    "A complete log of this run",
    "Note that you can also install",
    "tarball, folder, http url",
    "For a full report see",
    "If you are behind a proxy",
];

/// Select one stable user-facing cause from captured package-manager stderr.
#[must_use]
pub fn javascript_dependency_failure_detail(stderr: &[u8]) -> String {
    let text = strip_ansi(&String::from_utf8_lossy(stderr));
    let informative = text
        .lines()
        .filter(|raw| !npm_line_is_noise(raw))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !INSTALLER_NOISE.iter().any(|marker| line.contains(marker)))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    informative
        .iter()
        .rev()
        .find(|line| is_cause_line(line))
        .or_else(|| informative.last())
        .cloned()
        .unwrap_or_else(|| "?".to_owned())
}

fn npm_line_is_noise(line: &str) -> bool {
    let Some(mut remainder) = ["npm error", "npm warn", "npm ERR!"]
        .into_iter()
        .find_map(|prefix| line.strip_prefix(prefix))
    else {
        return false;
    };
    if let Some(after_space) = remainder.strip_prefix(' ') {
        remainder = after_space;
    }
    remainder = remainder.trim_end();
    if !remainder.is_empty() && remainder.bytes().all(|byte| byte.is_ascii_digit()) {
        return true;
    }
    if let Some((token, rest)) = remainder.split_once(' ')
        && !token.is_empty()
        && token.bytes().all(|byte| byte.is_ascii_digit())
    {
        remainder = rest;
    }
    remainder.is_empty()
        || remainder.starts_with([' ', '/', '{', '}'])
        || remainder.starts_with("at ")
        || is_windows_path(remainder)
}

fn is_windows_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
}

fn is_cause_line(line: &str) -> bool {
    let folded = line.to_ascii_lowercase();
    [
        "not found",
        "does not exist",
        "could not be found",
        "failed",
        "unable to",
        "refused",
        "denied",
        "conflict",
    ]
    .iter()
    .any(|marker| folded.contains(marker))
}

fn strip_ansi(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for escaped in chars.by_ref() {
                if escaped.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}

fn dependency_stamp(installer: &str, manifest: &str) -> String {
    let digest = Sha256::digest(format!("{installer}\n{manifest}").as_bytes());
    let mut stamp = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut stamp, "{byte:02x}").expect("writing to a string cannot fail");
    }
    stamp
}

static TEMPORARY_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct TemporaryDependencyDirectory {
    path: PathBuf,
}

impl TemporaryDependencyDirectory {
    fn new(entry_dir: &Path) -> Result<Self, DependencyError> {
        // The name holds the process id and a private counter, and staging leftovers are
        // already removed, so one candidate is enough. A collision is a real refusal.
        let path = unused_temporary_path(entry_dir);
        fs::create_dir(&path).map_err(|error| io_error("create", &path, error))?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryDependencyDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn begin_dependency_backup(entry_dir: &Path) -> Result<(), DependencyError> {
    begin_dependency_backup_with(entry_dir, |source, target| fs::rename(source, target))
}

fn begin_dependency_backup_with<F>(entry_dir: &Path, mut rename: F) -> Result<(), DependencyError>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    let backup = entry_dir.join(BACKUP_NAME);
    fs::create_dir(&backup).map_err(|error| io_error("create backup", &backup, error))?;
    let old_names = dependency_items()
        .filter(|name| path_exists(&entry_dir.join(name)))
        .collect::<Vec<_>>();
    let index = if old_names.is_empty() {
        String::new()
    } else {
        format!("{}\n", old_names.join("\n"))
    };
    atomic_write(&backup.join(BACKUP_INDEX), index.as_bytes())?;
    for name in dependency_items() {
        let current = entry_dir.join(name);
        if path_exists(&current)
            && let Err(error) = rename(&current, &backup.join(name))
        {
            let primary = io_error("backup", &current, error);
            let rollback = recover_dependency_backup(entry_dir);
            return Err(combine_rollback_error(primary, rollback, entry_dir));
        }
    }
    let _ = sync_directory(&backup);
    let _ = sync_directory(entry_dir);
    Ok(())
}

fn finish_dependency_backup(entry_dir: &Path) -> Result<(), DependencyError> {
    finish_dependency_backup_with(entry_dir, |source, target| fs::rename(source, target))
}

fn finish_dependency_backup_with<F>(entry_dir: &Path, rename: F) -> Result<(), DependencyError>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let backup = entry_dir.join(BACKUP_NAME);
    let cleanup = unused_temporary_path(entry_dir);
    if let Err(error) = rename(&backup, &cleanup) {
        let primary = io_error("commit dependency backup", &backup, error);
        let rollback = recover_dependency_backup(entry_dir);
        return Err(combine_rollback_error(primary, rollback, entry_dir));
    }
    let _ = sync_directory(entry_dir);
    finish_dependency_cleanup(
        entry_dir,
        &cleanup,
        &mut system_remove_file,
        &mut system_remove_dir_all,
    )
}

fn ensure_real_node_modules(entry_dir: &Path) -> Result<(), DependencyError> {
    let node_modules = entry_dir.join("node_modules");
    match optional_symlink_metadata(&node_modules)? {
        Some(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Some(_) => Err(DependencyError::Io {
            operation: "inspect",
            path: node_modules.display().to_string(),
            reason: "node_modules is not a directory".to_owned(),
        }),
        None => {
            fs::create_dir(&node_modules).map_err(|error| io_error("create", &node_modules, error))
        }
    }
}

fn commit_dependency_stage(entry_dir: &Path, stage: &Path) -> Result<(), DependencyError> {
    commit_dependency_stage_with_remover(
        entry_dir,
        stage,
        |entry_dir| Ok(unused_temporary_path(entry_dir)),
        &mut system_remove_file,
        &mut system_remove_dir_all,
    )
}

#[cfg(test)]
fn commit_dependency_stage_with<F>(
    entry_dir: &Path,
    stage: &Path,
    cleanup_path: F,
) -> Result<(), DependencyError>
where
    F: FnOnce(&Path) -> Result<PathBuf, DependencyError>,
{
    commit_dependency_stage_with_remover(
        entry_dir,
        stage,
        cleanup_path,
        &mut system_remove_file,
        &mut system_remove_dir_all,
    )
}

fn commit_dependency_stage_with_remover<F, R, D>(
    entry_dir: &Path,
    stage: &Path,
    cleanup_path: F,
    remove_file: &mut R,
    remove_dir_all: &mut D,
) -> Result<(), DependencyError>
where
    F: FnOnce(&Path) -> Result<PathBuf, DependencyError>,
    R: FnMut(&Path) -> io::Result<()>,
    D: FnMut(&Path) -> io::Result<()>,
{
    let backup = entry_dir.join(BACKUP_NAME);
    fs::create_dir(&backup).map_err(|error| io_error("create backup", &backup, error))?;
    let old_names = dependency_items()
        .filter(|name| path_exists(&entry_dir.join(name)))
        .collect::<Vec<_>>();
    let index = if old_names.is_empty() {
        String::new()
    } else {
        format!("{}\n", old_names.join("\n"))
    };
    atomic_write(&backup.join(BACKUP_INDEX), index.as_bytes())?;
    for name in dependency_items() {
        let current = entry_dir.join(name);
        if path_exists(&current)
            && let Err(error) = fs::rename(&current, backup.join(name))
        {
            let primary = io_error("backup", &current, error);
            let rollback = recover_dependency_backup(entry_dir);
            return Err(combine_rollback_error(primary, rollback, entry_dir));
        }
    }
    let _ = sync_directory(&backup);
    let _ = sync_directory(entry_dir);

    let mut new_names = Vec::new();
    for name in dependency_items() {
        let source = stage.join(name);
        if path_exists(&source) {
            if let Err(error) = fs::rename(&source, entry_dir.join(name)) {
                let primary = io_error("commit", &source, error);
                let rollback = rollback_dependency_stage(entry_dir, &new_names);
                return Err(combine_rollback_error(primary, rollback, entry_dir));
            }
            new_names.push(name);
        }
    }
    let _ = sync_directory(entry_dir);
    let cleanup = match cleanup_path(entry_dir) {
        Ok(cleanup) => cleanup,
        Err(primary) => {
            let rollback = rollback_dependency_stage(entry_dir, &new_names);
            return Err(combine_rollback_error(primary, rollback, entry_dir));
        }
    };
    if let Err(error) = fs::rename(&backup, &cleanup) {
        let primary = io_error("commit dependency backup", &backup, error);
        let rollback = rollback_dependency_stage(entry_dir, &new_names);
        return Err(combine_rollback_error(primary, rollback, entry_dir));
    }
    let _ = sync_directory(entry_dir);
    finish_dependency_cleanup(entry_dir, &cleanup, remove_file, remove_dir_all)
}

fn finish_dependency_cleanup<F, D>(
    entry_dir: &Path,
    cleanup: &Path,
    remove_file: &mut F,
    remove_dir_all: &mut D,
) -> Result<(), DependencyError>
where
    F: FnMut(&Path) -> io::Result<()>,
    D: FnMut(&Path) -> io::Result<()>,
{
    let removal = remove_dependency_cleanup(cleanup, remove_file, remove_dir_all);
    match removal {
        Ok(()) => {
            let _ = sync_directory(entry_dir);
            Ok(())
        }
        Err(failure) if !failure.removed_any => {
            let rollback = recover_dependency_cleanup(entry_dir, cleanup);
            Err(combine_rollback_error(failure.error, rollback, entry_dir))
        }
        Err(failure) => {
            // A deletion cannot be rolled back after an earlier artifact is gone. Keep the
            // remaining backup quarantined. The next dependency operation removes that staging
            // directory before it trusts a freshness marker or changes metadata.
            let _ = sync_directory(entry_dir);
            Err(failure.error)
        }
    }
}

#[derive(Debug)]
struct DependencyCleanupFailure {
    error: DependencyError,
    removed_any: bool,
}

fn remove_dependency_cleanup<F, D>(
    cleanup: &Path,
    remove_file: &mut F,
    remove_dir_all: &mut D,
) -> Result<(), DependencyCleanupFailure>
where
    F: FnMut(&Path) -> io::Result<()>,
    D: FnMut(&Path) -> io::Result<()>,
{
    let mut removed_any = false;
    for name in dependency_items() {
        match remove_cleanup_item(&cleanup.join(name), name, remove_file, remove_dir_all) {
            Ok(removed) => removed_any |= removed,
            Err(error) => return Err(DependencyCleanupFailure { error, removed_any }),
        }
    }
    match remove_cleanup_item(
        &cleanup.join(BACKUP_INDEX),
        BACKUP_INDEX,
        remove_file,
        remove_dir_all,
    ) {
        Ok(removed) => removed_any |= removed,
        Err(error) => return Err(DependencyCleanupFailure { error, removed_any }),
    }
    match fs::remove_dir(cleanup) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(DependencyCleanupFailure {
            error: clear_error(BACKUP_NAME, error),
            removed_any,
        }),
    }
}

fn remove_cleanup_item<F, D>(
    path: &Path,
    name: &str,
    remove_file: &mut F,
    remove_dir_all: &mut D,
) -> Result<bool, DependencyError>
where
    F: FnMut(&Path) -> io::Result<()>,
    D: FnMut(&Path) -> io::Result<()>,
{
    let existed = fs::symlink_metadata(path).is_ok();
    remove_path_with(path, remove_file, remove_dir_all)
        .map(|()| existed)
        .map_err(|error| clear_error(name, error))
}

fn clear_error(item: &str, error: io::Error) -> DependencyError {
    DependencyError::ClearFailed {
        item: item.to_owned(),
        reason: error.to_string(),
    }
}

fn recover_dependency_cleanup(entry_dir: &Path, cleanup: &Path) -> Result<(), DependencyError> {
    let backup = entry_dir.join(BACKUP_NAME);
    fs::rename(cleanup, &backup)
        .map_err(|error| io_error("restore dependency backup", &backup, error))?;
    recover_dependency_backup(entry_dir)
}

fn dependency_items() -> impl Iterator<Item = &'static str> {
    OWNED_FILES[..OWNED_FILES.len() - 1]
        .iter()
        .copied()
        .chain(["node_modules", STAMP_NAME])
}

fn require_entry_directory(entry_dir: &Path) -> Result<(), DependencyError> {
    let metadata =
        fs::symlink_metadata(entry_dir).map_err(|error| io_error("inspect", entry_dir, error))?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(DependencyError::Io {
            operation: "inspect",
            path: entry_dir.display().to_string(),
            reason: "the entry path is not a directory".to_owned(),
        })
    }
}

fn remove_staging_leftovers(entry_dir: &Path) -> Result<(), DependencyError> {
    let reader = fs::read_dir(entry_dir).map_err(|error| io_error("scan", entry_dir, error))?;
    for item in reader {
        let item = item.map_err(|error| io_error("scan", entry_dir, error))?;
        if item
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(STAGE_PREFIX))
        {
            remove_path(&item.path())?;
        }
    }
    Ok(())
}

fn recover_dependency_backup(entry_dir: &Path) -> Result<(), DependencyError> {
    let backup = entry_dir.join(BACKUP_NAME);
    if !path_exists(&backup) {
        return Ok(());
    }
    let mut stored_names = Vec::new();
    for item in fs::read_dir(&backup).map_err(|error| io_error("scan backup", &backup, error))? {
        let item = item.map_err(|error| io_error("scan backup", &backup, error))?;
        let name = backup_item_name(&item.file_name(), &item.path())?;
        if name.starts_with(&format!("{BACKUP_INDEX}.tmp-")) {
            remove_path(&item.path())?;
            continue;
        }
        if name != BACKUP_INDEX && !dependency_items().any(|allowed| allowed == name) {
            return Err(DependencyError::Io {
                operation: "recover backup",
                path: item.path().display().to_string(),
                reason: "the backup contains an unknown item".to_owned(),
            });
        }
        if name != BACKUP_INDEX {
            stored_names.push(name);
        }
    }
    let index_path = backup.join(BACKUP_INDEX);
    let old_names = if path_exists(&index_path) {
        let bytes = fs::read(&index_path).map_err(|error| io_error("read", &index_path, error))?;
        let text = std::str::from_utf8(&bytes).map_err(|error| DependencyError::Io {
            operation: "read",
            path: index_path.display().to_string(),
            reason: error.to_string(),
        })?;
        let names = text.lines().map(str::to_owned).collect::<Vec<_>>();
        if names
            .iter()
            .any(|name| !dependency_items().any(|allowed| allowed == name))
        {
            return Err(DependencyError::Io {
                operation: "recover backup",
                path: index_path.display().to_string(),
                reason: "the backup index contains an unknown item".to_owned(),
            });
        }
        names
    } else if stored_names.is_empty() {
        fs::remove_dir(&backup).map_err(|error| io_error("remove backup", &backup, error))?;
        let _ = sync_directory(entry_dir);
        return Ok(());
    } else {
        stored_names
    };

    for name in dependency_items() {
        if !old_names.iter().any(|old| old == name) {
            remove_path(&entry_dir.join(name))?;
        }
    }
    for name in &old_names {
        let source = backup.join(name);
        let target = entry_dir.join(name);
        if path_exists(&source) {
            remove_path(&target)?;
            fs::rename(&source, &target)
                .map_err(|error| io_error("recover backup", &target, error))?;
        } else if !path_exists(&target) {
            return Err(DependencyError::Io {
                operation: "recover backup",
                path: target.display().to_string(),
                reason: "a backup item is missing".to_owned(),
            });
        }
    }
    remove_path(&index_path)?;
    fs::remove_dir(&backup).map_err(|error| io_error("remove backup", &backup, error))?;
    let _ = sync_directory(entry_dir);
    Ok(())
}

fn backup_item_name(name: &std::ffi::OsStr, path: &Path) -> Result<String, DependencyError> {
    name.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DependencyError::Io {
            operation: "recover backup",
            path: path.display().to_string(),
            reason: "the backup item name is not valid UTF-8".to_owned(),
        })
}

fn remove_dependency_items(entry_dir: &Path, names: &[&str]) -> Result<(), DependencyError> {
    for name in names {
        remove_path(&entry_dir.join(name))?;
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), DependencyError> {
    let Some(metadata) = optional_symlink_metadata(path)? else {
        return Ok(());
    };
    remove_existing_path_with(
        path,
        &metadata,
        &mut system_remove_file,
        &mut system_remove_dir_all,
    )
    .map_err(|error| io_error("remove", path, error))
}

fn system_remove_file(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

fn system_remove_dir_all(path: &Path) -> io::Result<()> {
    fs::remove_dir_all(path)
}

fn remove_path_with<F, D>(
    path: &Path,
    remove_file: &mut F,
    remove_dir_all: &mut D,
) -> io::Result<()>
where
    F: FnMut(&Path) -> io::Result<()>,
    D: FnMut(&Path) -> io::Result<()>,
{
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    remove_existing_path_with(path, &metadata, remove_file, remove_dir_all)
}

fn remove_existing_path_with<F, D>(
    path: &Path,
    metadata: &fs::Metadata,
    remove_file: &mut F,
    remove_dir_all: &mut D,
) -> io::Result<()>
where
    F: FnMut(&Path) -> io::Result<()>,
    D: FnMut(&Path) -> io::Result<()>,
{
    let removal = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        remove_dir_all(path)
    } else {
        remove_file(path)
    };
    match removal {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Read one path's own metadata. `None` means the path does not exist.
fn optional_symlink_metadata(path: &Path) -> Result<Option<fs::Metadata>, DependencyError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("inspect", path, error)),
    }
}

/// Make one private staging name for this process.
fn unused_temporary_path(entry_dir: &Path) -> PathBuf {
    let id = TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
    entry_dir.join(format!("{STAGE_PREFIX}{}-{id}", std::process::id()))
}

fn combine_rollback_error(
    primary: DependencyError,
    rollback: Result<(), DependencyError>,
    path: &Path,
) -> DependencyError {
    match rollback {
        Ok(()) => primary,
        Err(rollback) => DependencyError::Rollback {
            path: path.display().to_string(),
            primary: Box::new(primary),
            rollback: Box::new(rollback),
        },
    }
}

fn rollback_dependency_stage(entry_dir: &Path, new_names: &[&str]) -> Result<(), DependencyError> {
    run_dependency_rollback(
        entry_dir,
        || remove_dependency_items(entry_dir, new_names),
        || recover_dependency_backup(entry_dir),
    )
}

fn run_dependency_rollback(
    path: &Path,
    remove_new: impl FnOnce() -> Result<(), DependencyError>,
    recover_old: impl FnOnce() -> Result<(), DependencyError>,
) -> Result<(), DependencyError> {
    let removal = remove_new();
    let recovery = recover_old();
    match (removal, recovery) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(primary), Err(rollback)) => Err(combine_rollback_error(primary, Err(rollback), path)),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, DependencyError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("read", path, error)),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), DependencyError> {
    atomic_write_with(path, bytes, File::sync_all, atomicwrites::replace_atomic)
}

fn atomic_write_with(
    path: &Path,
    bytes: &[u8],
    sync_file: impl FnOnce(&File) -> io::Result<()>,
    replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), DependencyError> {
    let parent = path.parent().ok_or_else(|| {
        io_error(
            "write",
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "write path has no parent"),
        )
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("dependency");
    let id = TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{name}.{}-{id}.tmp", std::process::id()));
    let outcome = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| io_error("write", &temporary, error))?;
        file.write_all(bytes)
            .map_err(|error| io_error("write", &temporary, error))?;
        sync_file(&file).map_err(|error| io_error("sync", &temporary, error))?;
        drop(file);
        replace(&temporary, path).map_err(|error| io_error("replace", path, error))
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
        return outcome;
    }
    let _ = sync_directory(parent);
    Ok(())
}

fn io_error(operation: &'static str, path: &Path, error: io::Error) -> DependencyError {
    DependencyError::Io {
        operation,
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod transaction_tests {
    use std::cell::Cell;

    use tempfile::TempDir;

    use super::*;

    fn dependency_temporary_paths(entry_dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(entry_dir)
            .unwrap()
            .map(|item| item.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(STAGE_PREFIX))
            })
            .collect()
    }

    #[test]
    fn prepared_cleanup_rollback_and_drop_recover_real_bytes() {
        let empty = TempDir::new().unwrap();
        let mut cleanup = prepare_javascript_dependency_cleanup(empty.path()).unwrap();
        cleanup.rollback().unwrap();
        cleanup.rollback().unwrap();

        let rolled_back = TempDir::new().unwrap();
        let package = rolled_back.path().join("package.json");
        fs::write(&package, b"rollback manifest\n").unwrap();
        let mut cleanup = prepare_javascript_dependency_cleanup(rolled_back.path()).unwrap();
        assert!(!package.exists());
        cleanup.rollback().unwrap();
        assert_eq!(fs::read(package).unwrap(), b"rollback manifest\n");

        let populated = TempDir::new().unwrap();
        let package = populated.path().join("package.json");
        fs::write(&package, b"authoritative manifest\n").unwrap();
        let cleanup = prepare_javascript_dependency_cleanup(populated.path()).unwrap();
        assert!(!package.exists());
        drop(cleanup);
        assert_eq!(fs::read(package).unwrap(), b"authoritative manifest\n");
        assert!(!populated.path().join(BACKUP_NAME).exists());
    }

    #[test]
    fn cleanup_index_remover_failure_keeps_the_index_and_typed_error() {
        let root = TempDir::new().unwrap();
        let cleanup = root.path().join("cleanup");
        fs::create_dir(&cleanup).unwrap();
        let index = cleanup.join(BACKUP_INDEX);
        fs::write(&index, b"package.json\n").unwrap();

        let failure = remove_dependency_cleanup(
            &cleanup,
            &mut |path| {
                assert_eq!(path, index);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "index held",
                ))
            },
            &mut system_remove_dir_all,
        )
        .unwrap_err();

        assert!(!failure.removed_any);
        assert!(matches!(
            failure.error,
            DependencyError::ClearFailed { ref item, ref reason }
                if item == BACKUP_INDEX && reason == "index held"
        ));
        assert_eq!(fs::read(index).unwrap(), b"package.json\n");
    }

    #[test]
    fn existing_path_removal_treats_not_found_as_success_and_keeps_other_errors_typed() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("artifact");
        fs::write(&path, b"authoritative\n").unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();

        remove_existing_path_with(
            &path,
            &metadata,
            &mut |_| Err(io::Error::new(io::ErrorKind::NotFound, "vanished")),
            &mut system_remove_dir_all,
        )
        .unwrap();
        let error = remove_existing_path_with(
            &path,
            &metadata,
            &mut |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "artifact locked",
                ))
            },
            &mut system_remove_dir_all,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read(path).unwrap(), b"authoritative\n");
    }

    #[test]
    fn parentless_atomic_write_is_typed_and_creates_no_artifact() {
        let error = atomic_write_with(
            Path::new(""),
            b"must not be written",
            File::sync_all,
            atomicwrites::replace_atomic,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            DependencyError::Io {
                operation: "write",
                ref reason,
                ..
            } if reason == "write path has no parent"
        ));
    }

    #[test]
    fn test_clean_failure_is_loud_not_silent() {
        let root = TempDir::new().unwrap();
        let package = root.path().join("package.json");
        fs::write(&package, b"authoritative manifest\n").unwrap();
        fs::write(root.path().join("meta.toml"), b"name = \"Demo\"\n").unwrap();
        let failing_root = root.path().to_owned();
        let mut remove_file = |path: &Path| {
            if path.starts_with(&failing_root)
                && path.file_name().is_some_and(|name| name == "package.json")
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "held by another process",
                ))
            } else {
                fs::remove_file(path)
            }
        };
        let successful = TempDir::new().unwrap();
        fs::write(successful.path().join("package.json"), b"disposable\n").unwrap();
        clear_javascript_dependencies_unlocked_with(
            successful.path(),
            &mut remove_file,
            &mut system_remove_dir_all,
        )
        .unwrap();

        let error = clear_javascript_dependencies_unlocked_with(
            root.path(),
            &mut remove_file,
            &mut system_remove_dir_all,
        )
        .unwrap_err();

        assert!(error.to_string().contains("package.json"), "{error}");
        assert_eq!(fs::read(package).unwrap(), b"authoritative manifest\n");
        assert_eq!(
            fs::read(root.path().join("meta.toml")).unwrap(),
            b"name = \"Demo\"\n"
        );
        assert!(dependency_temporary_paths(root.path()).is_empty());
    }

    #[test]
    fn test_clean_rmtree_failure_is_loud() {
        let root = TempDir::new().unwrap();
        let module = root.path().join("node_modules/chalk/index.js");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, b"module.exports = 1;\n").unwrap();
        let failing_root = root.path().to_owned();
        let mut remove_dir_all = |path: &Path| {
            if path.starts_with(&failing_root)
                && path.file_name().is_some_and(|name| name == "node_modules")
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "tree is locked",
                ))
            } else {
                fs::remove_dir_all(path)
            }
        };
        let successful = TempDir::new().unwrap();
        fs::create_dir(successful.path().join("node_modules")).unwrap();
        clear_javascript_dependencies_unlocked_with(
            successful.path(),
            &mut system_remove_file,
            &mut remove_dir_all,
        )
        .unwrap();

        let error = clear_javascript_dependencies_unlocked_with(
            root.path(),
            &mut system_remove_file,
            &mut remove_dir_all,
        )
        .unwrap_err();

        assert!(error.to_string().contains("node_modules"), "{error}");
        assert_eq!(fs::read(module).unwrap(), b"module.exports = 1;\n");
        assert!(dependency_temporary_paths(root.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_clean_tolerates_a_node_modules_symlink_vanishing() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().unwrap();
        let target = root.path().join("shared/chalk");
        fs::create_dir_all(&target).unwrap();
        let link = root.path().join("node_modules");
        symlink(target.parent().unwrap(), &link).unwrap();

        clear_javascript_dependencies_unlocked_with(
            root.path(),
            &mut |path| {
                if path.file_name().is_some_and(|name| name == "node_modules") {
                    fs::remove_file(path).unwrap();
                    Err(io::Error::new(io::ErrorKind::NotFound, "the link vanished"))
                } else {
                    fs::remove_file(path)
                }
            },
            &mut system_remove_dir_all,
        )
        .unwrap();

        assert!(!link.exists());
        assert!(target.is_dir());
        assert!(dependency_temporary_paths(root.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_clean_records_a_stuck_symlinked_node_modules() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().unwrap();
        let target = root.path().join("shared/chalk");
        fs::create_dir_all(&target).unwrap();
        let link = root.path().join("node_modules");
        symlink(target.parent().unwrap(), &link).unwrap();
        let failing_root = root.path().to_owned();
        let mut remove_file = |path: &Path| {
            if path.starts_with(&failing_root)
                && path.file_name().is_some_and(|name| name == "node_modules")
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "held by another process",
                ))
            } else {
                fs::remove_file(path)
            }
        };
        let successful = TempDir::new().unwrap();
        fs::write(successful.path().join("package.json"), b"disposable\n").unwrap();
        clear_javascript_dependencies_unlocked_with(
            successful.path(),
            &mut remove_file,
            &mut system_remove_dir_all,
        )
        .unwrap();

        let error = clear_javascript_dependencies_unlocked_with(
            root.path(),
            &mut remove_file,
            &mut system_remove_dir_all,
        )
        .unwrap_err();

        assert!(error.to_string().contains("node_modules"), "{error}");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(target.is_dir());
        assert!(dependency_temporary_paths(root.path()).is_empty());
    }

    #[test]
    fn test_clean_onexc_treats_an_already_gone_tree_as_success() {
        let root = TempDir::new().unwrap();
        let module = root.path().join("node_modules/chalk/index.js");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, b"module.exports = 1;\n").unwrap();
        let failing_root = root.path().to_owned();
        let mut remove_dir_all = |path: &Path| {
            if path.starts_with(&failing_root)
                && path.file_name().is_some_and(|name| name == "node_modules")
            {
                fs::remove_dir_all(path).unwrap();
                Err(io::Error::new(io::ErrorKind::NotFound, "the tree vanished"))
            } else {
                fs::remove_dir_all(path)
            }
        };
        let successful = TempDir::new().unwrap();
        fs::create_dir(successful.path().join("node_modules")).unwrap();
        clear_javascript_dependencies_unlocked_with(
            successful.path(),
            &mut system_remove_file,
            &mut remove_dir_all,
        )
        .unwrap();

        clear_javascript_dependencies_unlocked_with(
            root.path(),
            &mut system_remove_file,
            &mut remove_dir_all,
        )
        .unwrap();

        assert!(!root.path().join("node_modules").exists());
        assert!(dependency_temporary_paths(root.path()).is_empty());
    }

    #[test]
    fn test_clean_failure_message_verbatim() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("package.json"), b"{}\n").unwrap();
        let failing_root = root.path().to_owned();
        let mut remove_file = |path: &Path| {
            if path.starts_with(&failing_root)
                && path.file_name().is_some_and(|name| name == "package.json")
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "held by another process",
                ))
            } else {
                fs::remove_file(path)
            }
        };
        let successful = TempDir::new().unwrap();
        fs::write(successful.path().join("package.json"), b"disposable\n").unwrap();
        clear_javascript_dependencies_unlocked_with(
            successful.path(),
            &mut remove_file,
            &mut system_remove_dir_all,
        )
        .unwrap();

        let error = clear_javascript_dependencies_unlocked_with(
            root.path(),
            &mut remove_file,
            &mut system_remove_dir_all,
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Couldn't clear the old dependency environment: package.json: held by another process"
        );
    }

    #[test]
    fn a_partial_cleanup_failure_stays_quarantined_and_the_next_retry_repairs_it() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("package.json"), b"generated manifest\n").unwrap();
        fs::create_dir_all(root.path().join("node_modules/chalk")).unwrap();
        fs::write(
            root.path().join("node_modules/chalk/index.js"),
            b"module.exports = 1;\n",
        )
        .unwrap();
        fs::write(root.path().join("meta.toml"), b"name = \"Demo\"\n").unwrap();
        fs::write(root.path().join("script.js"), b"console.log(1);\n").unwrap();
        let failing_root = root.path().to_owned();
        let mut remove_dir_all = |path: &Path| {
            if path.starts_with(&failing_root)
                && path.file_name().is_some_and(|name| name == "node_modules")
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "tree is locked",
                ))
            } else {
                fs::remove_dir_all(path)
            }
        };
        let successful = TempDir::new().unwrap();
        fs::create_dir(successful.path().join("node_modules")).unwrap();
        clear_javascript_dependencies_unlocked_with(
            successful.path(),
            &mut system_remove_file,
            &mut remove_dir_all,
        )
        .unwrap();

        let error = clear_javascript_dependencies_unlocked_with(
            root.path(),
            &mut system_remove_file,
            &mut remove_dir_all,
        )
        .unwrap_err();

        assert!(matches!(error, DependencyError::ClearFailed { .. }));
        assert!(!root.path().join("package.json").exists());
        assert!(!root.path().join("node_modules").exists());
        assert_eq!(dependency_temporary_paths(root.path()).len(), 1);
        assert_eq!(
            fs::read(root.path().join("meta.toml")).unwrap(),
            b"name = \"Demo\"\n"
        );
        assert_eq!(
            fs::read(root.path().join("script.js")).unwrap(),
            b"console.log(1);\n"
        );

        clear_javascript_dependencies(root.path()).unwrap();
        assert!(dependency_temporary_paths(root.path()).is_empty());
    }

    #[test]
    fn a_partial_old_environment_cleanup_keeps_the_committed_new_environment() {
        let root = TempDir::new().unwrap();
        entry_with_previous_environment(root.path());
        let stage = staged_replacement(root.path());
        fs::create_dir(stage.join("node_modules")).unwrap();
        fs::write(stage.join("node_modules/new"), b"new module\n").unwrap();
        let failing_root = root.path().to_owned();
        let mut remove_dir_all = |path: &Path| {
            if path.starts_with(&failing_root)
                && path.file_name().is_some_and(|name| name == "node_modules")
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "old tree is locked",
                ))
            } else {
                fs::remove_dir_all(path)
            }
        };
        let successful = TempDir::new().unwrap();
        fs::create_dir(successful.path().join("node_modules")).unwrap();
        clear_javascript_dependencies_unlocked_with(
            successful.path(),
            &mut system_remove_file,
            &mut remove_dir_all,
        )
        .unwrap();

        let error = commit_dependency_stage_with_remover(
            root.path(),
            &stage,
            |entry_dir| Ok(unused_temporary_path(entry_dir)),
            &mut system_remove_file,
            &mut remove_dir_all,
        )
        .unwrap_err();

        assert!(matches!(error, DependencyError::ClearFailed { .. }));
        assert_eq!(
            fs::read(root.path().join("package.json")).unwrap(),
            b"new manifest\n"
        );
        assert_eq!(
            fs::read(root.path().join(STAMP_NAME)).unwrap(),
            b"new stamp\n"
        );
        assert_eq!(
            fs::read(root.path().join("node_modules/new")).unwrap(),
            b"new module\n"
        );
        assert!(!root.path().join("node_modules/old").exists());
        assert!(!root.path().join(BACKUP_NAME).exists());
        let quarantines = dependency_temporary_paths(root.path());
        assert_eq!(quarantines.len(), 1);
        assert_eq!(
            fs::read(quarantines[0].join("node_modules/old")).unwrap(),
            b"old module\n"
        );

        remove_staging_leftovers(root.path()).unwrap();
        assert!(dependency_temporary_paths(root.path()).is_empty());
        assert_eq!(
            fs::read(root.path().join("package.json")).unwrap(),
            b"new manifest\n"
        );
        assert_eq!(
            fs::read(root.path().join("node_modules/new")).unwrap(),
            b"new module\n"
        );
    }

    #[test]
    fn a_cleanup_root_that_vanishes_after_its_index_is_already_clean() {
        let root = TempDir::new().unwrap();
        let cleanup = root.path().join("cleanup");
        fs::create_dir(&cleanup).unwrap();
        fs::write(cleanup.join(BACKUP_INDEX), b"").unwrap();

        remove_dependency_cleanup(
            &cleanup,
            &mut |path| {
                fs::remove_file(path)?;
                fs::remove_dir(path.parent().unwrap())
            },
            &mut system_remove_dir_all,
        )
        .unwrap();
        assert!(!cleanup.exists());
    }

    #[test]
    fn a_nonempty_cleanup_root_is_a_typed_failure() {
        let root = TempDir::new().unwrap();
        let cleanup = root.path().join("cleanup");
        fs::create_dir(&cleanup).unwrap();
        fs::write(cleanup.join(BACKUP_INDEX), b"").unwrap();

        let failure =
            remove_dependency_cleanup(&cleanup, &mut |_| Ok(()), &mut system_remove_dir_all)
                .unwrap_err();
        assert!(failure.removed_any);
        assert!(matches!(
            failure.error,
            DependencyError::ClearFailed { ref item, .. } if item == BACKUP_NAME
        ));
    }

    #[test]
    fn a_missing_cleanup_backup_cannot_claim_that_rollback_succeeded() {
        let root = TempDir::new().unwrap();
        let error =
            recover_dependency_cleanup(root.path(), &root.path().join("missing")).unwrap_err();
        assert!(matches!(
            error,
            DependencyError::Io {
                operation: "restore dependency backup",
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn an_uninspectable_cleanup_item_is_an_io_refusal() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TempDir::new().unwrap();
        let blocked = root.path().join("blocked");
        fs::create_dir(&blocked).unwrap();
        let item = blocked.join("item");
        fs::write(&item, b"value\n").unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
        let error = remove_path_with(&item, &mut system_remove_file, &mut system_remove_dir_all)
            .unwrap_err();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn installer_failure_detail_keeps_the_last_cause_and_drops_noise() {
        let stderr = concat!(
            "npm error code E404\n",
            "npm error     at ignored stack frame\n",
            "npm error 404 \u{1b}[31mNot Found\u{1b}[0m - GET /missing\n",
            "npm error A complete log of this run can be found in: /tmp/debug.log\n",
        );
        assert_eq!(
            javascript_dependency_failure_detail(stderr.as_bytes()),
            "npm error 404 Not Found - GET /missing"
        );
        assert_eq!(javascript_dependency_failure_detail(&[]), "?");
        assert_eq!(
            javascript_dependency_failure_detail(b"detail \xff failed\n"),
            "detail \u{fffd} failed"
        );
    }

    #[test]
    fn atomic_dependency_writes_preserve_the_old_file_and_remove_failed_temps() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("package.json");
        fs::write(&target, b"old\n").unwrap();

        let sync_error = atomic_write_with(
            &target,
            b"new\n",
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "sync refused",
                ))
            },
            atomicwrites::replace_atomic,
        )
        .unwrap_err();
        assert!(sync_error.to_string().contains("sync refused"));
        assert_eq!(fs::read(&target).unwrap(), b"old\n");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);

        let replace_error = atomic_write_with(&target, b"new\n", File::sync_all, |_, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "replace refused",
            ))
        })
        .unwrap_err();
        assert!(replace_error.to_string().contains("replace refused"));
        assert_eq!(fs::read(&target).unwrap(), b"old\n");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);

        atomic_write_with(
            &target,
            b"new\n",
            File::sync_all,
            atomicwrites::replace_atomic,
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new\n");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn injected_sweep_uses_a_strict_one_hour_cutoff() {
        let root = TempDir::new().unwrap();
        let now = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(2 * 60 * 60))
            .unwrap();
        let cutoff = now.checked_sub(STALE_INJECTED_AGE).unwrap();
        let old = cutoff.checked_sub(Duration::from_secs(1)).unwrap();
        let write_at = |name: &str, modified: SystemTime| {
            let path = root.path().join(name);
            fs::write(&path, "value").unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(modified))
                .unwrap();
            path
        };
        let stale = write_at(".injected-stale.js", old);
        let edge = write_at(".injected-edge.js", cutoff);
        let fresh = write_at(".injected-fresh.js", now);
        let unrelated = write_at("keep.txt", old);

        sweep_stale_injected_at(root.path(), now);

        assert!(!stale.exists());
        assert!(edge.exists());
        assert!(fresh.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn test_sweep_keeps_a_file_exactly_at_the_cutoff() {
        let root = TempDir::new().unwrap();
        let now = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(2 * 60 * 60))
            .unwrap();
        let cutoff = now.checked_sub(STALE_INJECTED_AGE).unwrap();
        let edge = root.path().join(".injected-edge.js");
        fs::write(&edge, b"value\n").unwrap();
        File::options()
            .write(true)
            .open(&edge)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(cutoff))
            .unwrap();

        sweep_stale_injected_at(root.path(), now);

        assert!(edge.exists());
    }

    #[test]
    fn test_sweep_survives_one_failed_unlink_and_still_sweeps_the_rest() {
        let root = TempDir::new().unwrap();
        let cutoff = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(60))
            .unwrap();
        for name in [".injected-a.js", ".injected-b.js"] {
            let path = root.path().join(name);
            fs::write(&path, b"value\n").unwrap();
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
                .unwrap();
        }
        let mut calls = 0;

        sweep_stale_injected_before_with(root.path(), Some(cutoff), &mut |path| {
            calls += 1;
            if calls == 1 {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "held"))
            } else {
                fs::remove_file(path)
            }
        });

        let survivors = fs::read_dir(root.path())
            .unwrap()
            .flatten()
            .filter(|item| {
                item.file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(".injected-"))
            })
            .count();
        assert_eq!(calls, 2);
        assert_eq!(survivors, 1);
    }

    #[test]
    fn injected_sweep_is_inert_before_the_cutoff_exists_and_when_the_directory_is_gone() {
        let missing = TempDir::new().unwrap().path().join("gone");
        sweep_stale_injected_before(&missing, None);
        sweep_stale_injected_at(&missing, SystemTime::UNIX_EPOCH);

        let root = TempDir::new().unwrap();
        let candidate = root.path().join(".injected-young.js");
        fs::write(&candidate, b"keep\n").unwrap();
        sweep_stale_injected_at(root.path(), SystemTime::UNIX_EPOCH);
        assert_eq!(fs::read(candidate).unwrap(), b"keep\n");
    }

    #[test]
    fn backup_move_failure_uses_real_recovery_before_it_returns() {
        let root = TempDir::new().unwrap();
        entry_with_previous_environment(root.path());
        let mut moves = 0;

        let error = begin_dependency_backup_with(root.path(), |source, target| {
            moves += 1;
            if moves == 2 {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected rename refusal",
                ))
            } else {
                fs::rename(source, target)
            }
        })
        .unwrap_err();

        assert!(matches!(
            error,
            DependencyError::Io {
                operation: "backup",
                ..
            }
        ));
        assert_eq!(
            fs::read(root.path().join("package.json")).unwrap(),
            b"old manifest\n"
        );
        assert_eq!(
            fs::read(root.path().join(STAMP_NAME)).unwrap(),
            b"old stamp\n"
        );
        assert!(!root.path().join(BACKUP_NAME).exists());
    }

    #[test]
    fn backup_commit_failure_uses_real_recovery_before_it_returns() {
        let root = TempDir::new().unwrap();
        entry_with_previous_environment(root.path());
        begin_dependency_backup(root.path()).unwrap();

        let error = finish_dependency_backup_with(root.path(), |_, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected rename refusal",
            ))
        })
        .unwrap_err();

        assert!(matches!(
            error,
            DependencyError::Io {
                operation: "commit dependency backup",
                ..
            }
        ));
        assert_eq!(
            fs::read(root.path().join("package.json")).unwrap(),
            b"old manifest\n"
        );
        assert_eq!(
            fs::read(root.path().join(STAMP_NAME)).unwrap(),
            b"old stamp\n"
        );
        assert!(!root.path().join(BACKUP_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn backup_item_names_refuse_non_utf8_without_needing_filesystem_support() {
        use std::os::unix::ffi::OsStrExt as _;

        let name = std::ffi::OsStr::from_bytes(&[0xff, 0xfe]);
        let error = backup_item_name(name, Path::new("backup/item")).unwrap_err();

        assert!(error.to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn recovery_runs_even_when_removing_the_new_items_fails() {
        let recovered = Cell::new(false);
        let error = run_dependency_rollback(
            Path::new("entry"),
            || {
                Err(DependencyError::InvalidPackage {
                    value: "new items remain".to_owned(),
                })
            },
            || {
                recovered.set(true);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(recovered.get());
        assert!(matches!(error, DependencyError::InvalidPackage { .. }));

        let error = run_dependency_rollback(
            Path::new("entry"),
            || {
                Err(DependencyError::InvalidPackage {
                    value: "new items remain".to_owned(),
                })
            },
            || {
                Err(DependencyError::InvalidPackage {
                    value: "old items stay backed up".to_owned(),
                })
            },
        )
        .unwrap_err();
        assert!(matches!(error, DependencyError::Rollback { .. }));
    }

    #[test]
    fn cleanup_path_failure_restores_the_previous_environment() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("package.json"), b"old manifest\n").unwrap();
        fs::write(root.path().join(STAMP_NAME), b"old stamp\n").unwrap();
        fs::create_dir(root.path().join("node_modules")).unwrap();
        fs::write(root.path().join("node_modules/old"), b"old module\n").unwrap();
        let stage = root.path().join("stage");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("package.json"), b"new manifest\n").unwrap();
        fs::write(stage.join(STAMP_NAME), b"new stamp\n").unwrap();
        fs::create_dir(stage.join("node_modules")).unwrap();
        fs::write(stage.join("node_modules/new"), b"new module\n").unwrap();

        let error = commit_dependency_stage_with(root.path(), &stage, |path| {
            Err(DependencyError::Io {
                operation: "allocate cleanup",
                path: path.display().to_string(),
                reason: "injected late failure".to_owned(),
            })
        })
        .unwrap_err();

        assert!(matches!(error, DependencyError::Io { .. }));
        assert_eq!(
            fs::read(root.path().join("package.json")).unwrap(),
            b"old manifest\n"
        );
        assert_eq!(
            fs::read(root.path().join(STAMP_NAME)).unwrap(),
            b"old stamp\n"
        );
        assert_eq!(
            fs::read(root.path().join("node_modules/old")).unwrap(),
            b"old module\n"
        );
        assert!(!root.path().join("node_modules/new").exists());
        assert!(!root.path().join(BACKUP_NAME).exists());
    }

    /// Build one entry with a complete previous dependency environment.
    fn entry_with_previous_environment(root: &Path) {
        fs::write(root.join("package.json"), b"old manifest\n").unwrap();
        fs::write(root.join(STAMP_NAME), b"old stamp\n").unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::write(root.join("node_modules/old"), b"old module\n").unwrap();
    }

    fn staged_replacement(root: &Path) -> PathBuf {
        let stage = root.join("stage");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("package.json"), b"new manifest\n").unwrap();
        fs::write(stage.join(STAMP_NAME), b"new stamp\n").unwrap();
        stage
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_backup_move_failure_restores_the_previous_environment() {
        let root = TempDir::new().unwrap();
        entry_with_previous_environment(root.path());
        let stage = staged_replacement(root.path());
        // Moving a directory needs write permission on the directory itself.
        set_mode(&root.path().join("node_modules"), 0o555);

        let error = commit_dependency_stage(root.path(), &stage).unwrap_err();
        set_mode(&root.path().join("node_modules"), 0o755);

        assert!(matches!(error, DependencyError::Io { .. }));
        assert_eq!(
            fs::read(root.path().join("package.json")).unwrap(),
            b"old manifest\n"
        );
        assert_eq!(
            fs::read(root.path().join("node_modules/old")).unwrap(),
            b"old module\n"
        );
        assert!(!root.path().join(BACKUP_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_commit_move_failure_restores_the_previous_environment() {
        let root = TempDir::new().unwrap();
        entry_with_previous_environment(root.path());
        let stage = staged_replacement(root.path());
        fs::create_dir(stage.join("node_modules")).unwrap();
        fs::write(stage.join("node_modules/new"), b"new module\n").unwrap();
        set_mode(&stage.join("node_modules"), 0o555);

        let error = commit_dependency_stage(root.path(), &stage).unwrap_err();
        set_mode(&stage.join("node_modules"), 0o755);

        assert!(matches!(error, DependencyError::Io { .. }));
        assert_eq!(
            fs::read(root.path().join("package.json")).unwrap(),
            b"old manifest\n"
        );
        assert_eq!(
            fs::read(root.path().join("node_modules/old")).unwrap(),
            b"old module\n"
        );
        assert!(!root.path().join("node_modules/new").exists());
        assert!(!root.path().join(BACKUP_NAME).exists());
    }

    #[test]
    fn a_backup_cleanup_failure_restores_the_previous_environment() {
        let root = TempDir::new().unwrap();
        entry_with_previous_environment(root.path());
        let stage = staged_replacement(root.path());

        // The cleanup target names a directory that does not exist, so the move fails.
        let error = commit_dependency_stage_with(root.path(), &stage, |entry_dir| {
            Ok(entry_dir.join("absent").join("cleanup"))
        })
        .unwrap_err();

        assert!(matches!(error, DependencyError::Io { .. }));
        assert_eq!(
            fs::read(root.path().join("package.json")).unwrap(),
            b"old manifest\n"
        );
        assert_eq!(
            fs::read(root.path().join(STAMP_NAME)).unwrap(),
            b"old stamp\n"
        );
        assert!(!root.path().join(BACKUP_NAME).exists());
    }

    #[test]
    fn a_failed_rollback_reports_both_causes() {
        let root = TempDir::new().unwrap();
        entry_with_previous_environment(root.path());
        let stage = staged_replacement(root.path());

        // The cleanup step fails, and the backup then holds an item skit does not own.
        let error = commit_dependency_stage_with(root.path(), &stage, |entry_dir| {
            fs::write(entry_dir.join(BACKUP_NAME).join("mystery"), b"foreign\n").unwrap();
            Err(DependencyError::Io {
                operation: "allocate cleanup",
                path: entry_dir.display().to_string(),
                reason: "injected late failure".to_owned(),
            })
        })
        .unwrap_err();

        assert!(matches!(error, DependencyError::Rollback { .. }));
        let text = error.to_string();
        assert!(text.contains("rollback at"), "{text}");
        assert!(text.contains("injected late failure"), "{text}");
        assert!(text.contains("backup contains an unknown item"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_entry_directory_refuses_to_stage() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join(STAMP_NAME), b"old stamp\n").unwrap();
        set_mode(root.path(), 0o555);

        let error = clear_javascript_dependencies_unlocked(root.path()).unwrap_err();
        set_mode(root.path(), 0o755);

        assert!(matches!(
            error,
            DependencyError::Io {
                operation: "create",
                ..
            }
        ));
    }
}
