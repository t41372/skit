//! Constructed benchmark environments and reproducibility metadata.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use sysinfo::System;
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    BenchmarkProfile, GitInfo, HostInfo, Meta,
    process::{ProcessError, ProcessSpec, run},
};

/// CI runner label exported by benchmark workflows.
pub const CI_RUNNER_VAR: &str = "BENCH_CI_RUNNER";
/// GitHub-hosted image version.
pub const CI_IMAGE_VERSION_VAR: &str = "ImageVersion";

/// Environment construction failed.
#[derive(Debug, Error)]
pub enum EnvironmentError {
    /// Dataset roots must carry a manifest, including n=0.
    #[error("{0} is not a generated dataset (no manifest.json)")]
    NotDataset(PathBuf),
    /// Current-directory resolution failed.
    #[error("could not resolve {path}: {source}")]
    Resolve {
        /// Target path.
        path: PathBuf,
        /// OS error.
        #[source]
        source: std::io::Error,
    },
    /// Scratch directory creation failed.
    #[error("could not create {path}: {source}")]
    Create {
        /// Target path.
        path: PathBuf,
        /// OS error.
        #[source]
        source: std::io::Error,
    },
    /// A bounded host probe failed.
    #[error(transparent)]
    Process(#[from] ProcessError),
    /// A required host tool is absent.
    #[error("required host tool was not found: {0}")]
    MissingTool(&'static str),
    /// The run timestamp could not be formatted.
    #[error("could not format benchmark timestamp: {0}")]
    Time(#[from] time::error::Format),
}

/// Build the POSIX PATH used by children and tool discovery.
#[must_use]
pub fn bench_path(skit: &str, uv: Option<&str>, node: Option<&str>) -> String {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for program in [Some(skit), uv, node].into_iter().flatten() {
        let parent = Path::new(program)
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = parent.display().to_string();
        if seen.insert(parent.clone()) {
            ordered.push(parent);
        }
    }
    for system in ["/usr/bin", "/bin"] {
        if seen.insert(system.to_owned()) {
            ordered.push(system.to_owned());
        }
    }
    ordered.join(":")
}

/// Build, rather than scrub, the complete child environment.
pub fn build_environment(
    skit: &str,
    uv: Option<&str>,
    node: Option<&str>,
    workdir: &Path,
    dataset_root: &Path,
) -> Result<BTreeMap<String, String>, EnvironmentError> {
    if !dataset_root.join("manifest.json").is_file() {
        return Err(EnvironmentError::NotDataset(dataset_root.to_path_buf()));
    }
    let dataset_root = absolute(dataset_root)?;
    let workdir = absolute(workdir)?;
    let home = workdir.join("home");
    std::fs::create_dir_all(&home).map_err(|source| EnvironmentError::Create {
        path: home.clone(),
        source,
    })?;
    Ok(BTreeMap::from([
        ("PATH".to_owned(), bench_path(skit, uv, node)),
        ("HOME".to_owned(), home.display().to_string()),
        (
            "XDG_DATA_HOME".to_owned(),
            workdir.join("xdg-data").display().to_string(),
        ),
        (
            "XDG_STATE_HOME".to_owned(),
            workdir.join("xdg-state").display().to_string(),
        ),
        (
            "XDG_CONFIG_HOME".to_owned(),
            workdir.join("xdg-config").display().to_string(),
        ),
        (
            "XDG_CACHE_HOME".to_owned(),
            workdir.join("xdg-cache").display().to_string(),
        ),
        (
            "UV_CACHE_DIR".to_owned(),
            workdir.join("uv-cache").display().to_string(),
        ),
        (
            "SKIT_DATA_DIR".to_owned(),
            dataset_root.join("data").display().to_string(),
        ),
        (
            "SKIT_STATE_DIR".to_owned(),
            dataset_root.join("state").display().to_string(),
        ),
        (
            "SKIT_CONFIG_DIR".to_owned(),
            dataset_root.join("config").display().to_string(),
        ),
        ("SKIT_LANG".to_owned(), "en".to_owned()),
        ("PYTHONUTF8".to_owned(), "1".to_owned()),
        ("LC_ALL".to_owned(), "C.UTF-8".to_owned()),
        ("TERM".to_owned(), "dumb".to_owned()),
        ("COLUMNS".to_owned(), "100".to_owned()),
        ("LINES".to_owned(), "40".to_owned()),
    ]))
}

/// Normalize OS and architecture for budget predicates.
#[must_use]
pub fn platform_key(system: &str, machine: &str) -> String {
    let machine = match machine.to_ascii_lowercase().as_str() {
        "amd64" => "x86_64".to_owned(),
        "arm64" => "aarch64".to_owned(),
        other => other.to_owned(),
    };
    format!("{}-{machine}", system.to_ascii_lowercase())
}

/// Extract a durable pull-request number from a GitHub merge ref.
#[must_use]
pub fn pull_request_number(reference: &str) -> Option<String> {
    let parts = reference.split('/').collect::<Vec<_>>();
    if parts.len() >= 4
        && parts[0] == "refs"
        && parts[1] == "pull"
        && parts[2].chars().all(|character| character.is_ascii_digit())
    {
        Some(parts[2].to_owned())
    } else {
        None
    }
}

/// Extract the stable version token from common `PROGRAM --version` output.
#[must_use]
pub fn version_from_output(output: &str) -> String {
    let words = output.split_whitespace().collect::<Vec<_>>();
    match words.as_slice() {
        [] => "unknown".to_owned(),
        [version] => (*version).to_owned(),
        [_, version, ..] => (*version).to_owned(),
    }
}

/// Collect the reproducibility manifest from the measured checkout and host.
pub fn collect_meta(
    profile: BenchmarkProfile,
    repo_root: &Path,
    skit: &Path,
    python: Option<&Path>,
    uv: Option<&Path>,
) -> Result<Meta, EnvironmentError> {
    let system = System::new_all();
    let os = System::name().unwrap_or_else(|| env::consts::OS.to_owned());
    let architecture = System::cpu_arch();
    let cpu = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim())
        .filter(|brand| !brand.is_empty())
        .unwrap_or(&architecture)
        .to_owned();
    let git = which::which("git").map_err(|_| EnvironmentError::MissingTool("git"))?;
    let host_env = host_probe_environment();
    let commit = probe_stdout(
        vec![path_token(&git), "rev-parse".to_owned(), "HEAD".to_owned()],
        repo_root,
        &host_env,
    )?;
    let status = probe_stdout(
        vec![
            path_token(&git),
            "status".to_owned(),
            "--porcelain".to_owned(),
        ],
        repo_root,
        &host_env,
    )?;
    let skit_version = version_probe(skit, repo_root, &host_env)?;
    let python_version = python.map_or_else(
        || Ok("unknown".to_owned()),
        |program| version_probe(program, repo_root, &host_env),
    )?;
    let uv_version = uv.map_or_else(
        || Ok("unknown".to_owned()),
        |program| version_probe(program, repo_root, &host_env),
    )?;
    let generated_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
    Ok(Meta {
        generated_at,
        profile,
        git: GitInfo {
            commit,
            dirty: !status.trim().is_empty(),
            pr: env::var("GITHUB_REF")
                .ok()
                .and_then(|reference| pull_request_number(&reference)),
        },
        skit_version,
        host: HostInfo {
            os: os.clone(),
            kernel: System::kernel_version().unwrap_or_else(|| "unknown".to_owned()),
            cpu,
            cpu_count: system.cpus().len().max(1),
            mem_total_mib: usize::try_from(system.total_memory() / (1024 * 1024))
                .unwrap_or(usize::MAX),
            platform_key: platform_key(&os, &architecture),
            ci_runner: non_empty_env(CI_RUNNER_VAR),
            ci_image_version: non_empty_env(CI_IMAGE_VERSION_VAR),
        },
        python: python_version,
        uv: uv_version,
        textual: "not-applicable".to_owned(),
        pyperf: "rust-harness-v1".to_owned(),
    })
}

fn version_probe(
    program: &Path,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<String, EnvironmentError> {
    probe_stdout(
        vec![path_token(program), "--version".to_owned()],
        cwd,
        environment,
    )
    .map(|output| version_from_output(&output))
}

fn probe_stdout(
    argv: Vec<String>,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<String, EnvironmentError> {
    let output = run(&ProcessSpec {
        argv,
        cwd: cwd.to_path_buf(),
        env: environment.clone(),
        timeout: Duration::from_secs(30),
        check: true,
    })?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn host_probe_environment() -> BTreeMap<String, String> {
    let mut output = BTreeMap::new();
    for name in ["PATH", "SystemRoot", "SYSTEMROOT", "COMSPEC", "PATHEXT"] {
        if let Ok(value) = env::var(name) {
            output.insert(name.to_owned(), value);
        }
    }
    output.insert("LC_ALL".to_owned(), "C.UTF-8".to_owned());
    output
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn path_token(path: &Path) -> String {
    path.display().to_string()
}

fn absolute(path: &Path) -> Result<PathBuf, EnvironmentError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|source| EnvironmentError::Resolve {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(all(test, unix))]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt as _, path::Path};

    use tempfile::TempDir;

    use crate::BenchmarkProfile;

    fn executable(root: &Path, name: &str, output: &str, status: i32) -> std::path::PathBuf {
        let path = root.join(name);
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{output}'\nexit {status}\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn real_host_metadata_uses_bounded_git_and_version_probes() {
        let tools = TempDir::new().unwrap();
        let skit = executable(tools.path(), "skit", "skit 0.5.0", 0);
        let python = executable(tools.path(), "python", "Python 3.13.5", 0);
        let uv = executable(tools.path(), "uv", "uv 0.11.26", 0);
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

        let meta = super::collect_meta(
            BenchmarkProfile::Compare,
            &repo,
            &skit,
            Some(&python),
            Some(&uv),
        )
        .unwrap();
        assert_eq!(meta.profile, BenchmarkProfile::Compare);
        assert_eq!(meta.skit_version, "0.5.0");
        assert_eq!(meta.python, "3.13.5");
        assert_eq!(meta.uv, "0.11.26");
        assert!(!meta.git.commit.is_empty());
        assert!(!meta.host.os.is_empty());
        assert!(!meta.host.kernel.is_empty());
        assert!(!meta.host.cpu.is_empty());
        assert!(meta.host.cpu_count > 0);
        assert!(!meta.host.platform_key.is_empty());

        let without_optional =
            super::collect_meta(BenchmarkProfile::Pr, &repo, &skit, None, None).unwrap();
        assert_eq!(without_optional.python, "unknown");
        assert_eq!(without_optional.uv, "unknown");
    }

    #[test]
    fn host_helpers_preserve_unknown_architectures_and_probe_failures() {
        assert_eq!(super::platform_key("Plan9", "riscv64"), "plan9-riscv64");
        assert_eq!(super::version_from_output(""), "unknown");
        assert_eq!(super::version_from_output("1.2.3"), "1.2.3");

        let root = TempDir::new().unwrap();
        let failing = executable(root.path(), "failing", "no version", 7);
        assert!(
            super::version_probe(&failing, root.path(), &super::host_probe_environment()).is_err()
        );
        assert!(
            super::absolute(Path::new("relative"))
                .unwrap()
                .is_absolute()
        );
        assert_eq!(super::path_token(&failing), failing.display().to_string());
    }
}
