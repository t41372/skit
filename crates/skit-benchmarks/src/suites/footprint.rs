//! Installed distribution and product-library footprint.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use walkdir::WalkDir;

use crate::{
    Metric, Skip, SuiteKind, SuiteOutput, SuitePlan,
    dataset::dataset_dirs,
    process::{ProcessSpec, run as run_process},
    runner::{PROBE_TIMEOUT, RunContext, TOOL_TIMEOUT, path_arg},
};

use super::SuiteError;

const INSTALL_ATTEMPTS: usize = 3;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    let Some(uv) = &context.uv else {
        return Ok(SuiteOutput::skip_all(SuiteKind::Footprint, "uv not found"));
    };
    let first = *plan
        .library_sizes
        .first()
        .ok_or_else(|| SuiteError::Contract("footprint plan needs a dataset".to_owned()))?;
    let environment = context.environment(first)?;
    let dist = context.workdir.join("dist");
    fs::create_dir_all(&dist).map_err(|error| {
        SuiteError::Contract(format!("could not create {}: {error}", dist.display()))
    })?;
    let build = run_process(&ProcessSpec {
        argv: vec![
            path_arg(uv),
            "build".to_owned(),
            "--out-dir".to_owned(),
            path_arg(&dist),
        ],
        cwd: context.repo_root.clone(),
        env: environment.clone(),
        timeout: TOOL_TIMEOUT,
        check: true,
    })?;
    let wheel = one_artifact(&dist, |path| {
        path.extension().is_some_and(|extension| extension == "whl")
    })?;
    let sdist = one_artifact(&dist, |path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".tar.gz"))
    })?;
    let mut output = SuiteOutput {
        suite: SuiteKind::Footprint,
        duration_seconds: 0.0,
        metrics: BTreeMap::from([
            (
                "footprint.wheel_bytes".to_owned(),
                Metric::single(file_size(&wheel)?, "bytes"),
            ),
            (
                "footprint.sdist_bytes".to_owned(),
                Metric::single(file_size(&sdist)?, "bytes"),
            ),
            (
                "binary.release_bytes".to_owned(),
                Metric::single(file_size(&context.skit)?, "bytes"),
            ),
            (
                "repository.python_implementation_files".to_owned(),
                Metric::single(python_implementation_files(context)? as f64, "count"),
            ),
        ]),
        skipped: Vec::new(),
        raw: BTreeMap::from([
            (
                "build".to_owned(),
                serde_json::json!({
                    "wheel": wheel.file_name().map(|name| name.to_string_lossy()),
                    "sdist": sdist.file_name().map(|name| name.to_string_lossy()),
                    "stdout": String::from_utf8_lossy(&build.stdout),
                    "stderr": String::from_utf8_lossy(&build.stderr),
                }),
            ),
            (
                "uv_pinned_version".to_owned(),
                serde_json::json!(skit_runtime::UV_VERSION),
            ),
        ]),
    };
    measure_libraries(context, plan, &mut output)?;
    if plan.measure_closure {
        if context.python.is_none() {
            output.skipped.push(Skip {
                suite: SuiteKind::Footprint,
                case: "closure".to_owned(),
                reason: "python not found".to_owned(),
            });
        } else {
            measure_closure(context, uv, &wheel, &environment, &mut output)?;
        }
    }
    Ok(output)
}

fn measure_libraries(
    context: &RunContext,
    plan: &SuitePlan,
    output: &mut SuiteOutput,
) -> Result<(), SuiteError> {
    for n in &plan.library_sizes {
        let dirs = dataset_dirs(&context.dataset(*n)?.root)?;
        let store = tree_bytes(&dirs.data).map_err(walk_error)?;
        let state = tree_bytes(&dirs.state).map_err(walk_error)?;
        let total = store + state;
        output.metrics.insert(
            format!("footprint.library_bytes.n{n}"),
            Metric::single(store as f64, "bytes"),
        );
        output.metrics.insert(
            format!("footprint.library_state_bytes.n{n}"),
            Metric::single(state as f64, "bytes"),
        );
        output.metrics.insert(
            format!("footprint.library_total_bytes.n{n}"),
            Metric::single(total as f64, "bytes"),
        );
        if *n > 0 {
            output.metrics.insert(
                format!("footprint.library_bytes_per_entry.n{n}"),
                Metric::single(total as f64 / *n as f64, "bytes"),
            );
        }
    }
    Ok(())
}

fn measure_closure(
    context: &RunContext,
    uv: &Path,
    wheel: &Path,
    environment: &BTreeMap<String, String>,
    output: &mut SuiteOutput,
) -> Result<(), SuiteError> {
    measure_closure_with_ops(
        context,
        uv,
        wheel,
        environment,
        output,
        |spec| {
            let output = run_process(spec)?;
            Ok(ClosureProcessOutput {
                stderr: output.stderr,
                success: output.status.success(),
            })
        },
        thread::sleep,
    )
}

struct ClosureProcessOutput {
    stderr: Vec<u8>,
    success: bool,
}

fn measure_closure_with_ops(
    context: &RunContext,
    uv: &Path,
    wheel: &Path,
    environment: &BTreeMap<String, String>,
    output: &mut SuiteOutput,
    mut run: impl FnMut(&ProcessSpec) -> Result<ClosureProcessOutput, SuiteError>,
    mut sleep: impl FnMut(Duration),
) -> Result<(), SuiteError> {
    let venv = context.workdir.join("footprint-venv");
    let venv_argv = vec![
        path_arg(uv),
        "venv".to_owned(),
        path_arg(&venv),
        "--python".to_owned(),
        path_arg(context.python.as_ref().expect("caller checked python")),
    ];
    run(&ProcessSpec {
        argv: venv_argv,
        cwd: context.workdir.clone(),
        env: environment.clone(),
        timeout: TOOL_TIMEOUT,
        check: true,
    })?;
    let python = venv_python(&venv);
    let mut last_error = String::new();
    for attempt in 1..=INSTALL_ATTEMPTS {
        let install = run(&ProcessSpec {
            argv: vec![
                path_arg(uv),
                "pip".to_owned(),
                "install".to_owned(),
                "--python".to_owned(),
                path_arg(&python),
                path_arg(wheel),
            ],
            cwd: context.workdir.clone(),
            env: environment.clone(),
            timeout: TOOL_TIMEOUT,
            check: false,
        })?;
        if install.success {
            last_error.clear();
            break;
        }
        last_error = String::from_utf8_lossy(&install.stderr)
            .chars()
            .rev()
            .take(2_000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if attempt < INSTALL_ATTEMPTS {
            sleep(Duration::from_secs((attempt * 2) as u64));
        }
    }
    if !last_error.is_empty() {
        return Err(SuiteError::Contract(format!(
            "closure install failed {INSTALL_ATTEMPTS} times: {last_error}"
        )));
    }
    let site = find_site_packages(&venv)?;
    let executable = venv_skit(&venv);
    let mut closure = tree_bytes(&site).map_err(walk_error)?;
    if executable.is_file() {
        closure += file_size_u64(&executable)?;
    }
    let distributions = distribution_sizes(&site, &venv)?;
    let skit_bytes = distributions
        .iter()
        .find(|(name, _)| is_skit_distribution(name))
        .map_or(0, |(_, size)| *size);
    let mut largest = distributions.clone();
    largest.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    largest.truncate(10);
    output.metrics.extend([
        (
            "footprint.closure_bytes".to_owned(),
            Metric::single(closure as f64, "bytes"),
        ),
        (
            "footprint.skit_installed_bytes".to_owned(),
            Metric::single(skit_bytes as f64, "bytes"),
        ),
        (
            "footprint.distributions".to_owned(),
            Metric::single(distributions.len() as f64, "count"),
        ),
    ]);
    output.raw.insert(
        "largest_distributions".to_owned(),
        serde_json::Value::Object(
            largest
                .into_iter()
                .map(|(name, size)| (name, serde_json::json!(size)))
                .collect(),
        ),
    );
    Ok(())
}

fn distribution_sizes(site: &Path, venv: &Path) -> Result<Vec<(String, u64)>, SuiteError> {
    let mut sizes = Vec::new();
    for item in fs::read_dir(site).map_err(|error| contract_io("scan", site, error))? {
        let item = item.map_err(|error| contract_io("scan", site, error))?;
        let name = item.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".dist-info") || !item.path().is_dir() {
            continue;
        }
        let distribution = distribution_name(&item.path(), &name)?;
        let record = item.path().join("RECORD");
        let mut total = 0_u64;
        let mut counted = HashSet::new();
        if record.is_file() {
            let reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_path(&record);
            let mut rows = open_record(&record, reader)?;
            for row in rows.records() {
                let row = row.map_err(|error| {
                    SuiteError::Contract(format!(
                        "invalid wheel RECORD {}: {error}",
                        record.display()
                    ))
                })?;
                let relative = row.get(0).unwrap_or_default();
                let installed = site.join(relative);
                if installed.is_file() {
                    total += file_size_u64(&installed)?;
                    if let Ok(path) = fs::canonicalize(&installed) {
                        counted.insert(path);
                    }
                }
            }
        } else {
            total = tree_bytes(&item.path()).map_err(walk_error)?;
        }
        if is_skit_distribution(&distribution) {
            let executable = venv_skit(venv);
            let executable_was_counted =
                fs::canonicalize(&executable).is_ok_and(|path| counted.contains(&path));
            if executable.is_file() && !executable_was_counted {
                total += file_size_u64(&executable)?;
            }
        }
        sizes.push((distribution, total));
    }
    sizes.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(sizes)
}

fn open_record(
    record: &Path,
    reader: Result<csv::Reader<fs::File>, csv::Error>,
) -> Result<csv::Reader<fs::File>, SuiteError> {
    reader.map_err(|error| {
        SuiteError::Contract(format!("could not read {}: {error}", record.display()))
    })
}

fn distribution_name(dist_info: &Path, directory_name: &str) -> Result<String, SuiteError> {
    let metadata = dist_info.join("METADATA");
    if metadata.is_file() {
        let text =
            fs::read_to_string(&metadata).map_err(|error| contract_io("read", &metadata, error))?;
        if let Some(name) = text.lines().find_map(|line| {
            let (field, value) = line.split_once(':')?;
            (field.eq_ignore_ascii_case("name") && !value.trim().is_empty())
                .then(|| value.trim().to_owned())
        }) {
            return Ok(name);
        }
    }
    Ok(directory_name.trim_end_matches(".dist-info").to_owned())
}

fn is_skit_distribution(name: &str) -> bool {
    let normalized = name
        .trim_end_matches(".dist-info")
        .replace('_', "-")
        .to_ascii_lowercase();
    normalized == "skit-cli" || normalized.starts_with("skit-cli-")
}

fn find_site_packages(venv: &Path) -> Result<PathBuf, SuiteError> {
    let windows = venv.join("Lib/site-packages");
    if windows.is_dir() {
        return Ok(windows);
    }
    let lib = venv.join("lib");
    let mut candidates = Vec::new();
    for item in fs::read_dir(&lib).map_err(|error| contract_io("scan", &lib, error))? {
        let path = item
            .map_err(|error| contract_io("scan", &lib, error))?
            .path()
            .join("site-packages");
        if path.is_dir() {
            candidates.push(path);
        }
    }
    candidates.sort();
    match candidates.as_slice() {
        [site] => Ok(site.clone()),
        _ => Err(SuiteError::Contract(format!(
            "expected one site-packages directory under {}, found {}",
            lib.display(),
            candidates.len()
        ))),
    }
}

#[cfg(windows)]
fn venv_python(venv: &Path) -> PathBuf {
    venv.join("Scripts/python.exe")
}

#[cfg(not(windows))]
fn venv_python(venv: &Path) -> PathBuf {
    venv.join("bin/python")
}

#[cfg(windows)]
fn venv_skit(venv: &Path) -> PathBuf {
    venv.join("Scripts/skit.exe")
}

#[cfg(not(windows))]
fn venv_skit(venv: &Path) -> PathBuf {
    venv.join("bin/skit")
}

fn one_artifact(
    directory: &Path,
    predicate: impl Fn(&Path) -> bool,
) -> Result<PathBuf, SuiteError> {
    let mut artifacts = Vec::new();
    for item in fs::read_dir(directory).map_err(|error| contract_io("scan", directory, error))? {
        let path = item
            .map_err(|error| contract_io("scan", directory, error))?
            .path();
        if path.is_file() && predicate(&path) {
            artifacts.push(path);
        }
    }
    artifacts.sort();
    match artifacts.as_slice() {
        [artifact] => Ok(artifact.clone()),
        _ => Err(SuiteError::Contract(format!(
            "expected one matching artifact in {}, found {}",
            directory.display(),
            artifacts.len()
        ))),
    }
}

fn python_implementation_files(context: &RunContext) -> Result<usize, SuiteError> {
    let git = which::which("git")
        .map_err(|_| SuiteError::Contract("git not found for repository census".to_owned()))?;
    let output = run_process(&ProcessSpec {
        argv: vec![
            path_arg(&git),
            "ls-files".to_owned(),
            "--cached".to_owned(),
            "--others".to_owned(),
            "--exclude-standard".to_owned(),
            "--".to_owned(),
            "*.py".to_owned(),
        ],
        cwd: context.repo_root.clone(),
        env: context.environment(*context.datasets.keys().next().ok_or_else(|| {
            SuiteError::Contract("footprint needs a generated dataset".to_owned())
        })?)?,
        timeout: PROBE_TIMEOUT,
        check: true,
    })?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|path| context.repo_root.join(path).is_file())
        .filter(|path| is_python_implementation(path))
        .count())
}

fn is_python_implementation(path: &str) -> bool {
    !path.starts_with("tests/corpus/")
        && !path.starts_with("docs/assets/demo/scripts/")
        && !path.starts_with("benchmarks/fixtures/")
}

fn tree_bytes(root: &Path) -> Result<u64, walkdir::Error> {
    if !root.exists() {
        return Ok(0);
    }
    WalkDir::new(root)
        .into_iter()
        .try_fold(0_u64, |total, item| {
            let item = item?;
            if item.file_type().is_file() {
                Ok(total + item.metadata()?.len())
            } else {
                Ok(total)
            }
        })
}

fn file_size(path: &Path) -> Result<f64, SuiteError> {
    file_size_u64(path).map(|size| size as f64)
}

fn file_size_u64(path: &Path) -> Result<u64, SuiteError> {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| contract_io("inspect", path, error))
}

fn walk_error(error: walkdir::Error) -> SuiteError {
    SuiteError::Contract(format!("could not measure file tree: {error}"))
}

fn contract_io(operation: &str, path: &Path, error: std::io::Error) -> SuiteError {
    SuiteError::Contract(format!("could not {operation} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::Path};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    use tempfile::TempDir;

    fn context(workdir: &Path) -> crate::runner::RunContext {
        crate::runner::RunContext {
            repo_root: workdir.to_path_buf(),
            out_dir: workdir.join("out"),
            workdir: workdir.to_path_buf(),
            datasets: BTreeMap::new(),
            skit: workdir.join("skit"),
            harness: workdir.join("skit-bench"),
            python: Some(workdir.join("python")),
            uv: Some(workdir.join("uv")),
            bash: None,
            node: None,
            hyperfine: None,
            strace: None,
            cargo: None,
            rustc: None,
        }
    }

    #[test]
    fn test_footprint_closure_bounds_and_isolates_retries() {
        use crate::{SuiteKind, SuiteOutput, process::ProcessSpec};

        let root = TempDir::new().unwrap();
        let workdir = root.path().join("work");
        fs::create_dir(&workdir).unwrap();
        let context = context(&workdir);
        let environment = BTreeMap::from([("PATH".to_owned(), "benchmark-path".to_owned())]);
        let wheel = context.workdir.join("skit.whl");
        fs::write(&wheel, "wheel").unwrap();
        let mut output = SuiteOutput {
            suite: SuiteKind::Footprint,
            duration_seconds: 0.0,
            metrics: BTreeMap::new(),
            skipped: Vec::new(),
            raw: BTreeMap::new(),
        };
        let mut calls = Vec::new();
        let mut install_attempts = 0;
        let venv = context.workdir.join("footprint-venv");
        let mut sleeps = Vec::new();
        {
            let mut run = |spec: &ProcessSpec| {
                calls.push(spec.clone());
                let operation = spec.argv.get(1).map(String::as_str);
                if operation == Some("venv") {
                    let site = if cfg!(windows) {
                        venv.join("Lib/site-packages")
                    } else {
                        venv.join("lib/python3.13/site-packages")
                    };
                    fs::create_dir_all(site).unwrap();
                }
                let success = if operation == Some("pip") {
                    install_attempts += 1;
                    install_attempts == 2
                } else {
                    true
                };
                Ok(super::ClosureProcessOutput {
                    stderr: if success {
                        Vec::new()
                    } else {
                        b"temporary network error".to_vec()
                    },
                    success,
                })
            };

            super::measure_closure_with_ops(
                &context,
                context.uv.as_ref().unwrap(),
                &wheel,
                &environment,
                &mut output,
                &mut run,
                &mut |duration| sleeps.push(duration),
            )
            .unwrap();
        }

        assert_eq!(install_attempts, 2);
        assert_eq!(sleeps, [std::time::Duration::from_secs(2)]);
        assert_eq!(calls.len(), 3, "one venv and two install processes");
        assert!(calls.iter().all(|spec| spec.cwd == context.workdir));
        assert!(calls.iter().all(|spec| spec.env == environment));
        assert!(calls.iter().all(|spec| spec.timeout == super::TOOL_TIMEOUT));
        assert!(calls[0].check);
        assert!(calls[1..].iter().all(|spec| !spec.check));
        assert_eq!(
            calls
                .iter()
                .filter(|spec| spec.argv.get(1).is_some_and(|value| value == "pip"))
                .count(),
            2
        );
        assert!(output.metrics.contains_key("footprint.closure_bytes"));
        let serialized = output.to_json().unwrap();
        assert_eq!(SuiteOutput::from_json(&serialized).unwrap(), output);

        let mut failure_calls = Vec::new();
        let mut failure_attempts = 0;
        let mut failure_sleeps = Vec::new();
        let mut failure_output = SuiteOutput {
            suite: SuiteKind::Footprint,
            duration_seconds: 0.0,
            metrics: BTreeMap::new(),
            skipped: Vec::new(),
            raw: BTreeMap::new(),
        };
        let error = super::measure_closure_with_ops(
            &context,
            context.uv.as_ref().unwrap(),
            &wheel,
            &environment,
            &mut failure_output,
            |spec| {
                failure_calls.push(spec.clone());
                let is_install = spec.argv.get(1).is_some_and(|value| value == "pip");
                if is_install {
                    failure_attempts += 1;
                }
                Ok(super::ClosureProcessOutput {
                    stderr: if is_install {
                        format!("temporary failure {failure_attempts}").into_bytes()
                    } else {
                        Vec::new()
                    },
                    success: !is_install,
                })
            },
            |duration| failure_sleeps.push(duration),
        )
        .unwrap_err();

        assert_eq!(failure_attempts, 3);
        assert_eq!(
            failure_sleeps,
            [
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(4)
            ]
        );
        assert_eq!(
            failure_calls.len(),
            4,
            "one venv and three install processes"
        );
        assert!(failure_calls.iter().all(|spec| spec.cwd == context.workdir));
        assert!(failure_calls.iter().all(|spec| spec.env == environment));
        assert!(
            failure_calls
                .iter()
                .all(|spec| spec.timeout == super::TOOL_TIMEOUT)
        );
        assert!(error.to_string().contains("closure install failed 3 times"));
        assert!(error.to_string().ends_with("temporary failure 3"));
        assert!(failure_output.metrics.is_empty());
    }

    #[test]
    fn test_the_library_footprint_metrics_divide_into_each_other() {
        use crate::{
            SuiteKind, SuiteOutput, SuitePlan,
            dataset::{DEFAULT_SEED, DEFAULT_STATE_FRACTION, generate},
        };

        let root = TempDir::new().unwrap();
        let manifest = generate(
            &root.path().join("dataset"),
            3,
            DEFAULT_SEED,
            DEFAULT_STATE_FRACTION,
        )
        .unwrap();
        let mut context = context(root.path());
        context.datasets.insert(3, manifest);
        let plan = SuitePlan {
            kind: SuiteKind::Footprint,
            library_sizes: vec![3],
            warmup: 1,
            minimum_runs: 1,
            samples: 1,
            fast: true,
            measure_closure: false,
            run_javascript_lane: false,
            run_doctor: false,
            compare_mode: false,
        };
        let mut output = SuiteOutput {
            suite: SuiteKind::Footprint,
            duration_seconds: 0.0,
            metrics: BTreeMap::new(),
            skipped: Vec::new(),
            raw: BTreeMap::new(),
        };
        super::measure_libraries(&context, &plan, &mut output).unwrap();
        let output = SuiteOutput::from_json(&output.to_json().unwrap()).unwrap();

        let store = output.metrics["footprint.library_bytes.n3"].value;
        let state = output.metrics["footprint.library_state_bytes.n3"].value;
        let total = output.metrics["footprint.library_total_bytes.n3"].value;
        let per_entry = output.metrics["footprint.library_bytes_per_entry.n3"].value;
        assert_eq!(total, store + state);
        assert_eq!(per_entry, total / 3.0);
    }

    #[test]
    fn tree_size_counts_files_once_and_ignores_directories() {
        let root = TempDir::new().unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        fs::write(root.path().join("a"), b"abc").unwrap();
        fs::write(root.path().join("nested/b"), b"12345").unwrap();
        assert_eq!(super::tree_bytes(root.path()).unwrap(), 8);
        assert_eq!(super::tree_bytes(&root.path().join("absent")).unwrap(), 0);
    }

    #[test]
    fn python_contract_excludes_only_allowed_subject_files() {
        assert!(!super::is_python_implementation("tests/corpus/exact.py"));
        assert!(!super::is_python_implementation(
            "docs/assets/demo/scripts/demo.py"
        ));
        assert!(!super::is_python_implementation(
            "benchmarks/fixtures/noop.py"
        ));
        assert!(!super::is_python_implementation(
            "benchmarks/fixtures/future-subject.py"
        ));
        assert!(super::is_python_implementation("tools/release.py"));
    }

    #[test]
    fn record_distribution_sizes_count_an_external_binary_exactly_once() {
        let root = TempDir::new().unwrap();
        let venv = root.path().join("venv");
        let site = venv.join("lib/python3.13/site-packages");
        let dist = site.join("skit_cli-0.5.0.dist-info");
        fs::create_dir_all(&dist).unwrap();
        fs::create_dir_all(venv.join("bin")).unwrap();
        fs::write(site.join("payload"), b"abc").unwrap();
        fs::write(venv.join("bin/skit"), b"12345").unwrap();
        fs::write(dist.join("RECORD"), "payload,,\n").unwrap();
        fs::write(
            dist.join("METADATA"),
            "Metadata-Version: 2.4\nName: skit-cli\n",
        )
        .unwrap();

        let sizes = super::distribution_sizes(&site, &venv).unwrap();
        assert_eq!(sizes, [("skit-cli".to_owned(), 8)]);
    }

    #[test]
    fn artifact_site_package_and_distribution_discovery_are_strict() {
        let root = TempDir::new().unwrap();
        assert!(super::one_artifact(root.path(), |_| true).is_err());
        fs::write(root.path().join("one.whl"), "one").unwrap();
        fs::write(root.path().join("two.whl"), "two").unwrap();
        assert!(
            super::one_artifact(root.path(), |path| path
                .extension()
                .is_some_and(|x| x == "whl"))
            .is_err()
        );

        let windows = root.path().join("windows/Lib/site-packages");
        fs::create_dir_all(&windows).unwrap();
        assert_eq!(
            super::find_site_packages(&root.path().join("windows")).unwrap(),
            windows
        );

        let unix = root.path().join("unix/lib/python3.13/site-packages");
        fs::create_dir_all(&unix).unwrap();
        assert_eq!(
            super::find_site_packages(&root.path().join("unix")).unwrap(),
            unix
        );
        let second = root.path().join("unix/lib/python3.14/site-packages");
        fs::create_dir_all(second).unwrap();
        assert!(super::find_site_packages(&root.path().join("unix")).is_err());
        assert!(super::find_site_packages(&root.path().join("absent")).is_err());

        let fallback = root.path().join("fallback-1.0.dist-info");
        fs::create_dir(&fallback).unwrap();
        assert_eq!(
            super::distribution_name(&fallback, "fallback-1.0.dist-info").unwrap(),
            "fallback-1.0"
        );
        assert!(super::is_skit_distribution("skit_cli-0.5.0.dist-info"));
        assert!(!super::is_skit_distribution("not-skit-1.0.dist-info"));
    }

    #[test]
    fn footprint_io_and_missing_dataset_failures_keep_context() {
        let root = TempDir::new().unwrap();
        let missing = root.path().join("missing");
        assert!(super::file_size(&missing).is_err());
        assert!(
            super::contract_io("read", &missing, std::io::Error::other("failure"))
                .to_string()
                .contains("could not read")
        );
        let walk_failure = walkdir::WalkDir::new(&missing)
            .into_iter()
            .next()
            .expect("a walk of a missing path reports one entry")
            .expect_err("a walk of a missing path fails");
        assert!(
            super::walk_error(walk_failure)
                .to_string()
                .contains("file tree")
        );

        #[cfg(unix)]
        {
            use crate::suites::tests::Fixture;

            let mut fixture = Fixture::new();
            fixture.context.datasets = BTreeMap::new();
            assert!(super::python_implementation_files(&fixture.context).is_err());
        }
    }

    #[test]
    fn distribution_census_falls_back_without_record_and_rejects_bad_csv() {
        let root = TempDir::new().unwrap();
        let venv = root.path().join("venv");
        let site = venv.join("lib/python3.13/site-packages");
        let fallback = site.join("example-1.0.dist-info");
        fs::create_dir_all(&fallback).unwrap();
        fs::write(fallback.join("payload"), b"1234").unwrap();
        let sizes = super::distribution_sizes(&site, &venv).unwrap();
        assert_eq!(sizes, [("example-1.0".to_owned(), 4)]);

        let broken = site.join("broken-1.0.dist-info");
        fs::create_dir(&broken).unwrap();
        fs::write(broken.join("RECORD"), [0xff, b',', b',', b'\n']).unwrap();
        assert!(super::distribution_sizes(&site, &venv).is_err());

        let unreadable = site.join("unreadable-1.0.dist-info");
        fs::create_dir(&unreadable).unwrap();
        fs::create_dir(unreadable.join("RECORD")).unwrap();
        assert!(super::distribution_sizes(&site, &venv).is_err());

        let vanished = site.join("vanished.RECORD");
        let error = super::open_record(
            &vanished,
            csv::ReaderBuilder::new()
                .has_headers(false)
                .from_path(&vanished),
        )
        .unwrap_err();
        assert!(error.to_string().contains(&vanished.display().to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn footprint_names_dist_creation_and_retried_install_failures() {
        use crate::{
            SuiteKind,
            suites::tests::{Fixture, plan},
        };

        let fixture = Fixture::new();
        let dist = fixture.context.workdir.join("dist");
        fs::write(&dist, "unchanged").unwrap();
        let error = super::run(&fixture.context, &plan(SuiteKind::Footprint, &[0])).unwrap_err();
        assert!(error.to_string().contains(&dist.display().to_string()));
        assert_eq!(fs::read_to_string(&dist).unwrap(), "unchanged");

        let fixture = Fixture::new();
        let uv = fixture.context.uv.clone().unwrap();
        let log = fixture.context.workdir.join("uv-invocations");
        let script = format!(
            r#"#!/bin/sh
printf '%s|%s\n' "$PWD" "$*" >> '{}'
case "$1" in
  build)
    shift
    while [ "$#" -gt 0 ]; do
      case "$1" in --out-dir) out=$2; shift 2 ;; *) shift ;; esac
    done
    mkdir -p "$out"
    printf wheel > "$out/skit_cli-0.5.0-py3-none-any.whl"
    printf source > "$out/skit_cli-0.5.0.tar.gz"
    ;;
  venv)
    venv=$2
    mkdir -p "$venv/bin" "$venv/lib/python3.13/site-packages"
    printf '#!/bin/sh\nexit 0\n' > "$venv/bin/python"
    chmod +x "$venv/bin/python"
    ;;
  pip)
    i=0
    while [ "$i" -lt 2100 ]; do printf x >&2; i=$((i + 1)); done
    printf 'TAIL\n' >&2
    exit 9
    ;;
esac
"#,
            log.display()
        );
        fs::write(&uv, script).unwrap();
        fs::set_permissions(&uv, fs::Permissions::from_mode(0o755)).unwrap();
        let mut closure_plan = plan(SuiteKind::Footprint, &[0]);
        closure_plan.measure_closure = true;
        let error = super::run(&fixture.context, &closure_plan).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("closure install failed 3 times"));
        assert!(message.trim_end().ends_with("TAIL"));
        assert!(message.len() < 2_100);

        let invocations = fs::read_to_string(&log).unwrap();
        let lines = invocations.lines().collect::<Vec<_>>();
        let physical_workdir = fixture.context.workdir.canonicalize().unwrap();
        assert_eq!(lines.len(), 5);
        assert_eq!(
            lines[0],
            format!(
                "{}|build --out-dir {}",
                fixture.context.repo_root.display(),
                fixture.context.workdir.join("dist").display()
            )
        );
        assert_eq!(
            lines[1],
            format!(
                "{}|venv {} --python {}",
                physical_workdir.display(),
                fixture.context.workdir.join("footprint-venv").display(),
                fixture.context.python.as_ref().unwrap().display()
            )
        );
        let install = format!(
            "{}|pip install --python {} {}",
            physical_workdir.display(),
            fixture
                .context
                .workdir
                .join("footprint-venv/bin/python")
                .display(),
            fixture
                .context
                .workdir
                .join("dist/skit_cli-0.5.0-py3-none-any.whl")
                .display()
        );
        assert_eq!(
            &lines[2..],
            [install.as_str(), install.as_str(), install.as_str()]
        );
    }
}
