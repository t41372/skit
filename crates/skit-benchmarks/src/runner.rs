//! Shared suite context, tool discovery, and Hyperfine orchestration.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::Value;
use thiserror::Error;

use crate::{
    Metric, SuitePlan,
    dataset::DatasetManifest,
    environment::{EnvironmentError, bench_path, build_environment},
    hyperfine::{
        Case, HyperfineError, build_argv, metrics_from_export, parse_export, validate_case_names,
    },
    process::{ProcessError, ProcessSpec, run},
};

/// Long batch timeout.
pub const BENCHMARK_TIMEOUT: Duration = Duration::from_secs(1_800);
/// Build and installation timeout.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(600);
/// Single-probe timeout.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// Metrics and raw samples from one successful Hyperfine batch.
#[derive(Clone, Debug, PartialEq)]
pub struct HyperfineRun {
    /// Statistical metrics derived from the full sample set.
    pub metrics: BTreeMap<String, Metric>,
    /// Raw evidence keyed by stable payload name.
    pub raw: BTreeMap<String, Value>,
}

/// Discovered tools and isolated paths shared by all suites.
#[derive(Clone, Debug)]
pub struct RunContext {
    /// Harness checkout.
    pub repo_root: PathBuf,
    /// Durable run output.
    pub out_dir: PathBuf,
    /// External scratch directory.
    pub workdir: PathBuf,
    /// Shared generated libraries.
    pub datasets: BTreeMap<usize, DatasetManifest>,
    /// Benchmarked product binary.
    pub skit: PathBuf,
    /// This harness executable for fresh-process probes.
    pub harness: PathBuf,
    /// Python compatibility baseline.
    pub python: Option<PathBuf>,
    /// uv executable.
    pub uv: Option<PathBuf>,
    /// Bash resolved on the constructed PATH.
    pub bash: Option<PathBuf>,
    /// Node executable.
    pub node: Option<PathBuf>,
    /// Hyperfine executable.
    pub hyperfine: Option<PathBuf>,
    /// strace executable.
    pub strace: Option<PathBuf>,
    /// Cargo executable used by Maturin builds.
    pub cargo: Option<PathBuf>,
    /// Rust compiler used by Maturin builds.
    pub rustc: Option<PathBuf>,
}

/// Shared orchestration failed.
#[derive(Debug, Error)]
pub enum RunnerError {
    /// Required product binary is absent.
    #[error("skit binary is not an executable file: {0}")]
    MissingSkit(PathBuf),
    /// A dataset size was not prepared.
    #[error("dataset n{0} was not prepared")]
    MissingDataset(usize),
    /// Constructed environment failed.
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    /// Process failed.
    #[error(transparent)]
    Process(#[from] ProcessError),
    /// Hyperfine contract failed.
    #[error(transparent)]
    Hyperfine(#[from] HyperfineError),
    /// Hyperfine failed and each exact subject was rerun for a useful diagnosis.
    #[error("hyperfine batch {batch:?} exited {code}; single-shot diagnosis: {diagnosis}")]
    HyperfineBatch {
        /// Stable export label.
        batch: String,
        /// Hyperfine exit status or signal marker.
        code: String,
        /// Results from rerunning every subject once.
        diagnosis: String,
    },
    /// Tool output could not be read.
    #[error("could not read {path}: {source}")]
    Read {
        /// Output path.
        path: PathBuf,
        /// OS error.
        #[source]
        source: std::io::Error,
    },
}

impl RunContext {
    /// Discover optional tools without treating their absence as a crash.
    pub fn discover(
        repo_root: PathBuf,
        out_dir: PathBuf,
        workdir: PathBuf,
        datasets: BTreeMap<usize, DatasetManifest>,
        skit: PathBuf,
        harness: PathBuf,
    ) -> Result<Self, RunnerError> {
        if !skit.is_file() {
            return Err(RunnerError::MissingSkit(skit));
        }
        let uv = which::which("uv").ok();
        let node = which::which("node").ok();
        let path = bench_path(
            &skit.display().to_string(),
            uv.as_ref().map(|path| path.to_string_lossy()).as_deref(),
            node.as_ref().map(|path| path.to_string_lossy()).as_deref(),
        );
        let bash = which::which_in("bash", Some(path), &workdir).ok();
        let cargo = discover_rust_tool("cargo", &repo_root);
        let rustc = discover_rust_tool("rustc", &repo_root);
        Ok(Self {
            repo_root,
            out_dir,
            workdir,
            datasets,
            skit,
            harness,
            python: which::which("python3")
                .or_else(|_| which::which("python"))
                .ok(),
            uv,
            bash,
            node,
            hyperfine: which::which("hyperfine").ok(),
            strace: which::which("strace").ok(),
            cargo,
            rustc,
        })
    }

    /// Return one prepared dataset.
    pub fn dataset(&self, n: usize) -> Result<&DatasetManifest, RunnerError> {
        self.datasets.get(&n).ok_or(RunnerError::MissingDataset(n))
    }

    /// Construct the complete child environment for one dataset.
    pub fn environment(&self, n: usize) -> Result<BTreeMap<String, String>, RunnerError> {
        let dataset = self.dataset(n)?;
        self.environment_for(&dataset.root)
    }

    /// Construct the complete child environment for an ad-hoc generated library.
    pub fn environment_for(
        &self,
        dataset_root: &Path,
    ) -> Result<BTreeMap<String, String>, RunnerError> {
        let mut environment = build_environment(
            &self.skit.display().to_string(),
            self.uv
                .as_ref()
                .map(|path| path.to_string_lossy())
                .as_deref(),
            self.node
                .as_ref()
                .map(|path| path.to_string_lossy())
                .as_deref(),
            &self.workdir,
            dataset_root,
        )?;
        if let Some(cargo) = &self.cargo {
            environment.insert("CARGO".to_owned(), cargo.display().to_string());
        }
        if let Some(rustc) = &self.rustc {
            environment.insert("RUSTC".to_owned(), rustc.display().to_string());
        }
        Ok(environment)
    }
}

fn discover_rust_tool(name: &'static str, repo_root: &Path) -> Option<PathBuf> {
    let discovered = which::which(name).ok()?;
    let Ok(rustup) = which::which("rustup") else {
        return Some(discovered);
    };
    if !same_executable(&discovered, &rustup) {
        return Some(discovered);
    }
    let mut environment = BTreeMap::new();
    for variable in ["HOME", "PATH", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
        if let Ok(value) = env::var(variable) {
            environment.insert(variable.to_owned(), value);
        }
    }
    let Ok(output) = run(&ProcessSpec {
        argv: vec![path_arg(&rustup), "which".to_owned(), name.to_owned()],
        cwd: repo_root.to_path_buf(),
        env: environment,
        timeout: Duration::from_secs(30),
        check: false,
    }) else {
        return Some(discovered);
    };
    if !output.status.success() {
        return Some(discovered);
    }
    let resolved = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    resolved.is_file().then_some(resolved).or(Some(discovered))
}

fn same_executable(left: &Path, right: &Path) -> bool {
    same_file::is_same_file(left, right).unwrap_or(false)
}

/// Run one Hyperfine batch and retain its complete samples.
pub fn run_hyperfine(
    context: &RunContext,
    plan: &SuitePlan,
    cases: &[Case],
    environment: BTreeMap<String, String>,
    export_name: &str,
) -> Result<HyperfineRun, RunnerError> {
    let hyperfine = context
        .hyperfine
        .as_ref()
        .expect("callers record a skip when Hyperfine is unavailable");
    let export = context.workdir.join(format!("{export_name}.json"));
    let argv = build_argv(
        cases,
        plan.warmup,
        plan.minimum_runs,
        &export.display().to_string(),
        &hyperfine.display().to_string(),
    )?;
    let run_environment = environment.clone();
    let output = run(&ProcessSpec {
        argv,
        cwd: context.workdir.clone(),
        env: environment,
        timeout: BENCHMARK_TIMEOUT,
        check: false,
    })?;
    if !output.status.success() {
        return Err(RunnerError::HyperfineBatch {
            batch: export_name.to_owned(),
            code: output
                .status
                .code()
                .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
            diagnosis: diagnose_cases(&context.workdir, cases, &run_environment),
        });
    }
    let text = fs::read_to_string(&export).map_err(|source| RunnerError::Read {
        path: export,
        source,
    })?;
    let samples = parse_export(&text)?;
    validate_case_names(&samples, cases)?;
    let metrics = metrics_from_export(&samples)?;
    let raw = BTreeMap::from([(
        "times_s".to_owned(),
        serde_json::to_value(&samples).expect("finite Hyperfine samples serialize"),
    )]);
    Ok(HyperfineRun { metrics, raw })
}

fn diagnose_cases(cwd: &Path, cases: &[Case], environment: &BTreeMap<String, String>) -> String {
    cases
        .iter()
        .map(|case| {
            let probe = run(&ProcessSpec {
                argv: case.argv.clone(),
                cwd: cwd.to_path_buf(),
                env: environment.clone(),
                timeout: PROBE_TIMEOUT,
                check: false,
            });
            match probe {
                Ok(output) => {
                    let code = output
                        .status
                        .code()
                        .map_or_else(|| "signal".to_owned(), |code| code.to_string());
                    let detail = if output.status.success() {
                        String::new()
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        format!(" stderr: {:?}", tail(&stderr, 500))
                    };
                    format!("[{} rc={code}{detail}]", case.name)
                }
                Err(error) => format!("[{} probe failed: {error}]", case.name),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tail(text: &str, limit: usize) -> String {
    text.chars()
        .rev()
        .take(limit)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

/// Convert a path to a child argv token.
#[must_use]
pub fn path_arg(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::Path};

    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt as _};

    use crate::hyperfine::Case;
    #[cfg(unix)]
    use crate::{SuiteKind, suites::tests::plan};
    use tempfile::TempDir;

    #[cfg(unix)]
    fn executable(root: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = root.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn context_discovery_validates_the_product_and_keeps_optional_tools_optional() {
        let root = TempDir::new().unwrap();
        let missing = root.path().join("missing");
        assert!(matches!(
            super::RunContext::discover(
                root.path().to_path_buf(),
                root.path().join("out"),
                root.path().join("work"),
                BTreeMap::new(),
                missing,
                root.path().join("harness"),
            ),
            Err(super::RunnerError::MissingSkit(_))
        ));

        let skit = root.path().join("skit");
        std::fs::write(&skit, "product").unwrap();
        let context = super::RunContext::discover(
            root.path().to_path_buf(),
            root.path().join("out"),
            root.path().to_path_buf(),
            BTreeMap::new(),
            skit.clone(),
            root.path().join("harness"),
        )
        .unwrap();
        assert_eq!(context.skit, skit);
        assert!(matches!(
            context.dataset(99),
            Err(super::RunnerError::MissingDataset(99))
        ));
        assert!(context.environment_for(root.path()).is_err());
    }

    #[test]
    fn executable_identity_recognizes_hard_links() {
        let root = TempDir::new().unwrap();
        let original = root.path().join("original");
        let hard_link = root.path().join("hard-link");
        let distinct = root.path().join("distinct");
        std::fs::write(&original, b"tool").unwrap();
        std::fs::hard_link(&original, &hard_link).unwrap();
        std::fs::write(&distinct, b"tool").unwrap();

        assert!(super::same_executable(&original, &hard_link));
        assert!(!super::same_executable(&original, &distinct));
        assert!(!super::same_executable(
            &original,
            &root.path().join("missing")
        ));
    }

    #[test]
    fn rustup_proxy_resolves_to_the_selected_toolchain() {
        let Some(rustup) = which::which("rustup").ok() else {
            return;
        };
        let Some(cargo) = super::discover_rust_tool("cargo", Path::new(".")) else {
            panic!("cargo must be available when rustup is available");
        };
        if super::same_executable(&which::which("cargo").unwrap(), &rustup) {
            assert_eq!(
                cargo.file_name().and_then(|name| name.to_str()),
                Some("cargo")
            );
            assert!(!super::same_executable(
                &cargo,
                &which::which("rustup").unwrap()
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_hyperfine_batches_get_exact_single_shot_diagnostics() {
        let root = TempDir::new().unwrap();
        let cases = [
            Case::new("ok", ["/bin/sh", "-c", "test \"$ONLY\" = yes"]),
            Case::new("bad", ["/bin/sh", "-c", "printf problem >&2; exit 7"]),
        ];
        let report = super::diagnose_cases(
            root.path(),
            &cases,
            &BTreeMap::from([
                ("ONLY".to_owned(), "yes".to_owned()),
                ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            ]),
        );
        assert_eq!(report, "[ok rc=0] [bad rc=7 stderr: \"problem\"]");

        let missing = Case::new(
            "missing",
            [root.path().join("absent").display().to_string()],
        );
        assert!(
            super::diagnose_cases(root.path(), &[missing], &BTreeMap::new())
                .contains("probe failed")
        );
    }

    #[cfg(unix)]
    #[test]
    fn hyperfine_failures_keep_batch_diagnostics_and_missing_exports_typed() {
        let root = TempDir::new().unwrap();
        let skit = executable(root.path(), "skit", "#!/bin/sh\nexit 0\n");
        let failing = executable(root.path(), "failing", "#!/bin/sh\nexit 9\n");
        let silent = executable(root.path(), "silent", "#!/bin/sh\nexit 0\n");
        let mut context = super::RunContext {
            repo_root: root.path().to_path_buf(),
            out_dir: root.path().join("out"),
            workdir: root.path().to_path_buf(),
            datasets: BTreeMap::new(),
            skit,
            harness: root.path().join("harness"),
            python: None,
            uv: None,
            bash: None,
            node: None,
            hyperfine: Some(failing),
            strace: None,
            cargo: None,
            rustc: None,
        };
        let cases = [Case::new("subject", ["/bin/sh", "-c", "exit 0"])];
        let error = super::run_hyperfine(
            &context,
            &plan(SuiteKind::Imports, &[]),
            &cases,
            BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
            "failed",
        )
        .unwrap_err();
        assert!(matches!(error, super::RunnerError::HyperfineBatch { .. }));

        context.hyperfine = Some(silent);
        let error = super::run_hyperfine(
            &context,
            &plan(SuiteKind::Imports, &[]),
            &cases,
            BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
            "missing",
        )
        .unwrap_err();
        assert!(matches!(error, super::RunnerError::Read { .. }));
    }
}
