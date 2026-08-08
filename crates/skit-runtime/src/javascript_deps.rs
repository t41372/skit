//! Materialize private JavaScript dependencies beside a stored entry.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

use crate::ProgramProbe;

const STAMP_NAME: &str = ".skit-deps";
const OWNED_FILES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "bun.lock",
    "bun.lockb",
    "deno.lock",
    STAMP_NAME,
];

/// One package-manager process request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyCommand {
    /// Resolved package-manager executable.
    pub program: PathBuf,
    /// Arguments after the executable.
    pub args: Vec<String>,
    /// Private entry directory.
    pub cwd: PathBuf,
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
    /// The package manager returned a failure status.
    #[error("JavaScript package installation failed with {program}")]
    InstallFailed { program: String },
}

/// Build the deterministic private package.json document.
pub fn javascript_dependency_manifest(dependencies: &[String]) -> Result<String, DependencyError> {
    let mut rows = BTreeMap::new();
    for dependency in dependencies {
        let (name, version) = split_package_spec(dependency)?;
        rows.insert(name, version);
    }
    let mut output = String::from(
        "{\n  \"name\": \"skit-private-entry\",\n  \"private\": true,\n  \"dependencies\": {\n",
    );
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
    let _lock = dependency_lock(entry_dir)?;
    if dependencies.is_empty() {
        return clear_javascript_dependencies_unlocked(entry_dir);
    }
    let manifest = javascript_dependency_manifest(dependencies)?;
    let stamp = format!("v1\n{runtime}\n{:016x}\n", stable_hash(manifest.as_bytes()));
    let stamp_path = entry_dir.join(STAMP_NAME);
    if read_optional(&stamp_path)?.as_deref() == Some(stamp.as_bytes())
        && entry_dir.join("node_modules").is_dir()
    {
        return Ok(());
    }

    fs::create_dir_all(entry_dir).map_err(|error| io_error("create", entry_dir, error))?;
    atomic_write(&entry_dir.join("package.json"), manifest.as_bytes())?;
    let command = dependency_command(entry_dir, runtime, probe)?;
    let success = runner
        .run(&command)
        .map_err(|error| io_error("start package manager in", entry_dir, error))?;
    if !success {
        return Err(DependencyError::InstallFailed {
            program: command.program.display().to_string(),
        });
    }
    atomic_write(&stamp_path, stamp.as_bytes())
}

/// Remove only support artifacts that skit owns for one entry.
pub fn clear_javascript_dependencies(entry_dir: &Path) -> Result<(), DependencyError> {
    let _lock = dependency_lock(entry_dir)?;
    clear_javascript_dependencies_unlocked(entry_dir)
}

fn clear_javascript_dependencies_unlocked(entry_dir: &Path) -> Result<(), DependencyError> {
    if entry_dir.join(STAMP_NAME).exists() || generated_manifest(entry_dir)? {
        let modules = entry_dir.join("node_modules");
        if modules.exists() {
            if fs::symlink_metadata(&modules)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                fs::remove_file(&modules).map_err(|error| io_error("remove", &modules, error))?;
            } else {
                fs::remove_dir_all(&modules)
                    .map_err(|error| io_error("remove", &modules, error))?;
            }
        }
        for name in OWNED_FILES {
            let path = entry_dir.join(name);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error("remove", &path, error)),
            }
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
    let locks = parent.join(".locks");
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
    probe: &P,
) -> Result<DependencyCommand, DependencyError> {
    let (installer, args) = match runtime {
        "node" => (
            "npm",
            ["install", "--ignore-scripts", "--no-audit", "--no-fund"].as_slice(),
        ),
        "bun" => (
            "bun",
            ["install", "--ignore-scripts", "--production"].as_slice(),
        ),
        "deno" => (
            "deno",
            ["install", "--node-modules-dir=auto", "--prod"].as_slice(),
        ),
        _ => {
            return Err(DependencyError::UnsupportedRuntime {
                runtime: runtime.to_owned(),
            });
        }
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
    })
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

fn generated_manifest(entry_dir: &Path) -> Result<bool, DependencyError> {
    Ok(
        read_optional(&entry_dir.join("package.json"))?.is_some_and(|bytes| {
            bytes
                .windows(b"skit-private-entry".len())
                .any(|window| window == b"skit-private-entry")
        }),
    )
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
    fs::write(&temporary, bytes).map_err(|error| io_error("write", &temporary, error))?;
    if cfg!(windows) && path.exists() {
        fs::remove_file(path).map_err(|error| io_error("replace", path, error))?;
    }
    fs::rename(&temporary, path).map_err(|error| io_error("replace", path, error))
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
