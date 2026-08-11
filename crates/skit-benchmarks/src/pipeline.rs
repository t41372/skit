//! End-to-end benchmark run orchestration.

use std::{
    collections::BTreeMap,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    time::Instant,
};

use thiserror::Error;

use crate::{
    BenchmarkProfile, Meta, Results, ResultsError, SuiteOutput, SuitePlan,
    budget::Budget,
    build_plan,
    dataset::{
        DEFAULT_SEED, DEFAULT_STATE_FRACTION, DatasetError, DatasetManifest, check_reusable,
        generate,
    },
    dataset_sizes,
    environment::{EnvironmentError, collect_meta},
    report::{RunRecord, SummaryError, summarize_directory},
    runner::{RunContext, RunnerError},
    suites::{self, SuiteError},
};

/// A complete run could not be prepared, measured, or published.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// Dataset generation or reuse failed.
    #[error(transparent)]
    Dataset(#[from] DatasetError),
    /// Tool discovery or shared execution failed.
    #[error(transparent)]
    Runner(#[from] RunnerError),
    /// Host provenance collection failed.
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    /// One suite crashed.
    #[error(transparent)]
    Suite(#[from] SuiteError),
    /// Result publication failed.
    #[error(transparent)]
    Summary(#[from] SummaryError),
    /// A suite artifact failed schema validation.
    #[error(transparent)]
    Results(#[from] ResultsError),
    /// Run-record JSON serialization failed.
    #[error("could not serialize run.json: {0}")]
    Json(#[from] serde_json::Error),
    /// A filesystem operation failed.
    #[error("could not {operation} {path}: {source}")]
    Io {
        /// Operation.
        operation: &'static str,
        /// Target.
        path: PathBuf,
        /// Operating-system error.
        #[source]
        source: std::io::Error,
    },
    /// A suite mislabeled its output.
    #[error("suite {expected} returned output labeled {actual}")]
    SuiteLabel {
        /// Planned suite.
        expected: &'static str,
        /// Returned suite.
        actual: &'static str,
    },
    /// A cleanup target was not a normal benchmark directory.
    #[error("refusing to remove unexpected filesystem object {0}")]
    UnsafeCleanup(PathBuf),
}

/// Generate or reuse all datasets needed by a profile.
pub fn prepare_datasets(
    bench_dir: &Path,
    sizes: &[usize],
) -> Result<BTreeMap<usize, DatasetManifest>, ExecutionError> {
    ensure_normal_directory(bench_dir)?;
    let datasets_dir = bench_dir.join("datasets");
    ensure_normal_directory(&datasets_dir)?;
    let mut datasets = BTreeMap::new();
    for n in sizes {
        let root = datasets_dir.join(format!("n{n}"));
        let manifest_path = root.join("manifest.json");
        let root_exists = normal_directory_exists(&root)?;
        let manifest_exists = if root_exists {
            normal_file_exists(&manifest_path)?
        } else {
            false
        };
        let manifest = if manifest_exists {
            let manifest = DatasetManifest::load(&root)?;
            check_reusable(&manifest, *n)?;
            manifest
        } else {
            if root_exists {
                remove_generated_directory(&root)?;
            }
            generate(
                &root,
                isize::try_from(*n).map_err(|_| {
                    DatasetError::Invalid(format!("dataset size {n} does not fit isize"))
                })?,
                DEFAULT_SEED,
                DEFAULT_STATE_FRACTION,
            )?
        };
        datasets.insert(*n, manifest);
    }
    Ok(datasets)
}

/// Inputs for one complete benchmark profile.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionRequest<'a> {
    /// Profile to measure.
    pub profile: BenchmarkProfile,
    /// Durable benchmark output directory.
    pub bench_dir: &'a Path,
    /// Checkout that owns the Rust benchmark harness.
    pub repo_root: &'a Path,
    /// Checkout whose Git identity belongs in the artifact.
    pub measured_repo: Option<&'a Path>,
    /// Product binary to measure.
    pub skit: &'a Path,
    /// Current benchmark harness executable.
    pub harness: &'a Path,
    /// Optional budget contract rendered into the report.
    pub budgets: Option<&'a [Budget]>,
}

/// Execute one complete profile and publish its result artifacts.
pub fn execute(request: ExecutionRequest<'_>) -> Result<Results, ExecutionError> {
    let started = Instant::now();
    let bench_dir = absolute(request.bench_dir)?;
    let repo_root = absolute(request.repo_root)?;
    let measured_repo = absolute(request.measured_repo.unwrap_or(&repo_root))?;
    let skit = absolute(request.skit)?;
    let harness = absolute(request.harness)?;
    ensure_normal_directory(&bench_dir)?;
    clear_stale_outputs(&bench_dir)?;

    let plan = build_plan(request.profile);
    let datasets = prepare_datasets(&bench_dir, &dataset_sizes(&plan))?;
    let work = tempfile::Builder::new()
        .prefix("skit-bench-")
        .tempdir()
        .map_err(|source| io("create", &std::env::temp_dir(), source))?;
    let context = RunContext::discover(
        repo_root,
        bench_dir.clone(),
        work.path().to_path_buf(),
        datasets,
        skit,
        harness,
    )?;
    let meta = collect_meta(
        request.profile,
        &measured_repo,
        &context.skit,
        context.python.as_deref(),
        context.uv.as_deref(),
    )?;
    run_and_publish(
        started,
        &bench_dir,
        &context,
        &plan,
        meta,
        request.budgets,
        suites::run,
    )
}

fn run_and_publish(
    started: Instant,
    bench_dir: &Path,
    context: &RunContext,
    plan: &[SuitePlan],
    meta: Meta,
    budgets: Option<&[Budget]>,
    mut run_suite: impl FnMut(&RunContext, &SuitePlan) -> Result<SuiteOutput, SuiteError>,
) -> Result<Results, ExecutionError> {
    let suites_dir = bench_dir.join("suites");
    for suite_plan in plan {
        let suite_started = Instant::now();
        let mut output = run_suite(context, suite_plan)?;
        if output.suite != suite_plan.kind {
            return Err(ExecutionError::SuiteLabel {
                expected: suite_plan.kind.as_str(),
                actual: output.suite.as_str(),
            });
        }
        output.duration_seconds = suite_started.elapsed().as_secs_f64();
        atomic_write(
            &suites_dir.join(format!("{}.json", suite_plan.kind.as_str())),
            &output.to_json()?,
        )?;
    }
    let record = RunRecord {
        meta,
        total_duration_s: started.elapsed().as_secs_f64(),
        suites: plan.iter().map(|suite| suite.kind).collect(),
    };
    let mut run_json = serde_json::to_string_pretty(&record)?;
    run_json.push('\n');
    atomic_write(&bench_dir.join("run.json"), &run_json)?;
    summarize_directory(bench_dir, budgets).map_err(Into::into)
}

fn clear_stale_outputs(bench_dir: &Path) -> Result<(), ExecutionError> {
    ensure_normal_directory(bench_dir)?;
    for name in ["run.json", "results.json", "results.md"] {
        let path = bench_dir.join(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(io("remove", &path, source)),
        }
    }
    let suites = bench_dir.join("suites");
    ensure_normal_directory(&suites)?;
    let reader = fs::read_dir(&suites).map_err(|source| io("scan", &suites, source))?;
    for item in reader {
        let item = item.map_err(|source| io("scan", &suites, source))?;
        let path = item.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            fs::remove_file(&path).map_err(|source| io("remove", &path, source))?;
        }
    }
    Ok(())
}

fn remove_generated_directory(path: &Path) -> Result<(), ExecutionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io("inspect", path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ExecutionError::UnsafeCleanup(path.to_path_buf()));
    }
    fs::remove_dir_all(path).map_err(|source| io("remove", path, source))
}

fn ensure_normal_directory(path: &Path) -> Result<(), ExecutionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(ExecutionError::UnsafeCleanup(path.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|source| io("create", path, source))?;
            if normal_directory_exists(path)? {
                Ok(())
            } else {
                Err(ExecutionError::UnsafeCleanup(path.to_path_buf()))
            }
        }
        Err(source) => Err(io("inspect", path, source)),
    }
}

fn normal_directory_exists(path: &Path) -> Result<bool, ExecutionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(ExecutionError::UnsafeCleanup(path.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io("inspect", path, source)),
    }
}

fn normal_file_exists(path: &Path) -> Result<bool, ExecutionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ExecutionError::UnsafeCleanup(path.to_path_buf()))
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io("inspect", path, source)),
    }
}

fn atomic_write(path: &Path, text: &str) -> Result<(), ExecutionError> {
    let parent = path
        .parent()
        .ok_or_else(|| ExecutionError::UnsafeCleanup(path.to_path_buf()))?;
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| io("create", parent, source))?;
    staged
        .write_all(text.as_bytes())
        .map_err(|source| io("write", staged.path(), source))?;
    staged
        .persist(path)
        .map_err(|error| io("commit", path, error.error))?;
    Ok(())
}

fn absolute(path: &Path) -> Result<PathBuf, ExecutionError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|source| io("resolve", path, source))
}

fn io(operation: &'static str, path: &Path, source: std::io::Error) -> ExecutionError {
    ExecutionError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{fs, os::unix::fs::symlink, path::Path, time::Instant};

    use tempfile::TempDir;

    use crate::{
        BenchmarkProfile, GitInfo, HostInfo, Meta, SuiteKind, SuiteOutput,
        suites::tests::{Fixture, plan},
    };

    fn meta() -> Meta {
        Meta {
            generated_at: "2026-08-09T00:00:00Z".to_owned(),
            profile: BenchmarkProfile::Pr,
            git: GitInfo {
                commit: "abcdef1234567890".to_owned(),
                dirty: false,
                pr: None,
            },
            skit_version: "0.5.0".to_owned(),
            host: HostInfo {
                os: "Linux".to_owned(),
                kernel: "test".to_owned(),
                cpu: "Test CPU".to_owned(),
                cpu_count: 1,
                mem_total_mib: 1,
                platform_key: "linux-x86_64".to_owned(),
                ci_runner: None,
                ci_image_version: None,
            },
            python: "3.13".to_owned(),
            uv: "0.11".to_owned(),
            textual: "not-applicable".to_owned(),
            pyperf: "rust-harness-v1".to_owned(),
        }
    }

    #[test]
    fn stale_output_cleanup_refuses_a_symlinked_suite_directory() {
        let bench = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let evidence = outside.path().join("keep.json");
        fs::write(&evidence, "keep").unwrap();
        symlink(outside.path(), bench.path().join("suites")).unwrap();

        assert!(super::clear_stale_outputs(bench.path()).is_err());
        assert_eq!(fs::read_to_string(evidence).unwrap(), "keep");
    }

    #[test]
    fn publication_runs_the_planned_suite_and_writes_strict_artifacts() {
        let fixture = Fixture::new();
        super::clear_stale_outputs(&fixture.context.out_dir).unwrap();
        let suite_plan = [plan(SuiteKind::Imports, &[0, 100])];
        let results = super::run_and_publish(
            Instant::now(),
            &fixture.context.out_dir,
            &fixture.context,
            &suite_plan,
            meta(),
            None,
            crate::suites::run,
        )
        .unwrap();

        assert!(results.metrics.contains_key("imports.version.modules"));
        assert!(fixture.context.out_dir.join("run.json").is_file());
        assert!(fixture.context.out_dir.join("results.json").is_file());
        assert!(fixture.context.out_dir.join("results.md").is_file());
        assert!(
            fixture
                .context
                .out_dir
                .join("suites/imports.json")
                .is_file()
        );
    }

    #[test]
    fn publication_rejects_a_mislabeled_suite_before_writing_a_run_record() {
        let fixture = Fixture::new();
        super::clear_stale_outputs(&fixture.context.out_dir).unwrap();
        let suite_plan = [plan(SuiteKind::Imports, &[0])];
        let error = super::run_and_publish(
            Instant::now(),
            &fixture.context.out_dir,
            &fixture.context,
            &suite_plan,
            meta(),
            None,
            |_, _| Ok(SuiteOutput::skip_all(SuiteKind::Footprint, "test")),
        )
        .unwrap_err();

        assert!(matches!(error, super::ExecutionError::SuiteLabel { .. }));
        assert!(!fixture.context.out_dir.join("run.json").exists());
    }

    #[test]
    fn filesystem_helpers_keep_cleanup_scoped_to_normal_directories() {
        let root = TempDir::new().unwrap();
        let created = root.path().join("created");
        super::ensure_normal_directory(&created).unwrap();
        super::ensure_normal_directory(&created).unwrap();
        assert!(super::normal_directory_exists(&created).unwrap());
        assert!(!super::normal_directory_exists(&root.path().join("missing")).unwrap());

        let ordinary_file = root.path().join("ordinary");
        fs::write(&ordinary_file, "keep").unwrap();
        assert!(super::ensure_normal_directory(&ordinary_file).is_err());
        assert!(!super::normal_file_exists(&created).unwrap());
        assert!(super::normal_file_exists(&ordinary_file).unwrap());
        assert!(!super::normal_file_exists(&root.path().join("missing-file")).unwrap());
        assert!(super::remove_generated_directory(&ordinary_file).is_err());

        let generated = root.path().join("generated");
        fs::create_dir(&generated).unwrap();
        fs::write(generated.join("payload"), "data").unwrap();
        super::remove_generated_directory(&generated).unwrap();
        assert!(!generated.exists());

        let output = root.path().join("atomic.txt");
        super::atomic_write(&output, "complete\n").unwrap();
        assert_eq!(fs::read_to_string(output).unwrap(), "complete\n");
        assert!(super::atomic_write(Path::new("/"), "no").is_err());
    }
}
