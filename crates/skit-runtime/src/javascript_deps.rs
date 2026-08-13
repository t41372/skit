//! Materialize private JavaScript dependencies beside a stored entry.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use skit_i18n::{Localize, Message};
use thiserror::Error;

use crate::ProgramProbe;

const STAMP_NAME: &str = ".skit-deps";
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
    /// Return true only when the child exits successfully.
    fn run(&self, command: &DependencyCommand) -> io::Result<bool>;
}

/// Start dependency commands on the local machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemDependencyCommandRunner;

impl DependencyCommandRunner for SystemDependencyCommandRunner {
    fn run(&self, command: &DependencyCommand) -> io::Result<bool> {
        Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.cwd)
            .envs(&command.environment)
            .status()
            .map(|status| status.success())
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
    #[error("required package manager was not found: {name}")]
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
    /// The package manager returned a failure status.
    #[error("JavaScript package installation failed with {program}")]
    InstallFailed { program: String },
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
            Self::InstallerNotFound { name } => {
                Message::new("required package manager was not found: {}").with(name)
            }
            Self::Io {
                operation,
                path,
                reason,
            } => Message::new("could not {} JavaScript dependencies at {}: {}")
                .nested(Message::term(operation))
                .with(path)
                .with(reason),
            Self::Rollback {
                path,
                primary,
                rollback,
            } => Message::new("rollback at {} failed after {}: {}")
                .with(path)
                .nested(primary.message())
                .nested(rollback.message()),
            Self::InstallFailed { program } => {
                Message::new("JavaScript package installation failed with {}").with(program)
            }
        }
    }
}

/// Build the deterministic private package.json document.
pub fn javascript_dependency_manifest(dependencies: &[String]) -> Result<String, DependencyError> {
    javascript_dependency_manifest_for_module(dependencies, None)
}

fn javascript_dependency_manifest_for_module(
    dependencies: &[String],
    module_type: Option<JavaScriptModuleType>,
) -> Result<String, DependencyError> {
    let mut rows = BTreeMap::new();
    for dependency in dependencies {
        if dependency.trim().is_empty() {
            continue;
        }
        let (name, version) = split_package_spec(dependency)?;
        rows.insert(name, version);
    }
    let mut output = String::from("{\n  \"name\": \"skit-private-entry\",\n  \"private\": true,\n");
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
    if dependencies.is_empty() && module_type.is_none() {
        return clear_javascript_dependencies_unlocked(entry_dir);
    }
    let manifest = javascript_dependency_manifest_for_module(dependencies, module_type)?;
    let stamp = format!("v1\n{runtime}\n{:016x}\n", stable_hash(manifest.as_bytes()));
    let stamp_path = entry_dir.join(STAMP_NAME);
    if read_optional(&stamp_path)?.as_deref() == Some(stamp.as_bytes())
        && (dependencies.is_empty() || entry_dir.join("node_modules").is_dir())
    {
        return Ok(());
    }

    let staged = TemporaryDependencyDirectory::new(entry_dir)?;
    atomic_write(&staged.path.join("package.json"), manifest.as_bytes())?;
    if !dependencies.is_empty() {
        let command = dependency_command(&staged.path, runtime, environment, probe)?;
        let success = runner
            .run(&command)
            .map_err(|error| io_error("start package manager in", entry_dir, error))?;
        if !success {
            return Err(DependencyError::InstallFailed {
                program: command.program.display().to_string(),
            });
        }
    }
    atomic_write(&staged.path.join(STAMP_NAME), stamp.as_bytes())?;
    commit_dependency_stage(entry_dir, &staged.path)
}

/// Remove JavaScript dependency artifacts from one private entry directory.
pub fn clear_javascript_dependencies(entry_dir: &Path) -> Result<(), DependencyError> {
    let _lock = dependency_lock(entry_dir)?;
    require_entry_directory(entry_dir)?;
    recover_dependency_backup(entry_dir)?;
    remove_staging_leftovers(entry_dir)?;
    clear_javascript_dependencies_unlocked(entry_dir)
}

fn clear_javascript_dependencies_unlocked(entry_dir: &Path) -> Result<(), DependencyError> {
    sweep_stale_injected_at(entry_dir, SystemTime::now());
    validate_dependency_item_shapes(entry_dir)?;
    if dependency_items().any(|name| path_exists(&entry_dir.join(name))) {
        let staged = TemporaryDependencyDirectory::new(entry_dir)?;
        commit_dependency_stage(entry_dir, &staged.path)?;
    }
    Ok(())
}

fn sweep_stale_injected_at(entry_dir: &Path, now: SystemTime) {
    let Some(cutoff) = now.checked_sub(STALE_INJECTED_AGE) else {
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
            let _ = fs::remove_file(path);
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
    let (installer, args) = match runtime {
        "node" => (
            "npm",
            ["install", "--no-audit", "--no-fund", "--ignore-scripts"].as_slice(),
        ),
        "bun" => ("bun", ["install", "--ignore-scripts"].as_slice()),
        "deno" => ("deno", ["install"].as_slice()),
        _ => (
            "npm",
            ["install", "--no-audit", "--no-fund", "--ignore-scripts"].as_slice(),
        ),
    };
    let program =
        probe
            .find_program(installer)
            .ok_or_else(|| DependencyError::InstallerNotFound {
                name: installer.to_owned(),
            })?;
    Ok(DependencyCommand {
        program,
        args: args.iter().map(|value| (*value).to_owned()).collect(),
        cwd: entry_dir.to_owned(),
        environment: environment.clone(),
    })
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

fn commit_dependency_stage(entry_dir: &Path, stage: &Path) -> Result<(), DependencyError> {
    commit_dependency_stage_with(entry_dir, stage, |entry_dir| {
        Ok(unused_temporary_path(entry_dir))
    })
}

fn commit_dependency_stage_with<F>(
    entry_dir: &Path,
    stage: &Path,
    cleanup_path: F,
) -> Result<(), DependencyError>
where
    F: FnOnce(&Path) -> Result<PathBuf, DependencyError>,
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
    let _ = remove_path(&cleanup);
    Ok(())
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
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|error| io_error("remove", path, error))
    } else {
        fs::remove_file(path).map_err(|error| io_error("remove", path, error))
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

fn split_package_spec(value: &str) -> Result<(String, String), DependencyError> {
    let value = value.trim();
    let version_at = if value.starts_with('@') {
        let slash = value.find('/').unwrap_or(value.len());
        value.rfind('@').filter(|index| *index > slash)
    } else {
        value.rfind('@').filter(|index| *index > 0)
    };
    let (name, version) =
        version_at.map_or((value, "*"), |index| (&value[..index], &value[index + 1..]));
    if !valid_package_name(name) || version.is_empty() {
        return Err(DependencyError::InvalidPackage {
            value: value.to_owned(),
        });
    }
    Ok((name.to_owned(), version.to_owned()))
}

fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains("..")
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '@' | '/' | '-' | '_' | '.')
        })
        && if let Some(scoped) = name.strip_prefix('@') {
            let mut parts = scoped.split('/');
            parts.next().is_some_and(|part| !part.is_empty())
                && parts.next().is_some_and(|part| !part.is_empty())
                && parts.next().is_none()
        } else {
            !name.contains('/')
        }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, DependencyError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("read", path, error)),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), DependencyError> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| io_error("write", &temporary, error))?;
    file.write_all(bytes)
        .map_err(|error| io_error("write", &temporary, error))?;
    file.sync_all()
        .map_err(|error| io_error("sync", &temporary, error))?;
    drop(file);
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(|error| io_error("replace", path, error))?;
    }
    fs::rename(&temporary, path).map_err(|error| io_error("replace", path, error))?;
    if let Some(parent) = path.parent() {
        let _ = sync_directory(parent);
    }
    Ok(())
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
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
