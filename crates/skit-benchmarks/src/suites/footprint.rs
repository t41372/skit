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
    let venv = context.workdir.join("footprint-venv");
    let venv_argv = vec![
        path_arg(uv),
        "venv".to_owned(),
        path_arg(&venv),
        "--python".to_owned(),
        path_arg(context.python.as_ref().expect("caller checked python")),
    ];
    run_process(&ProcessSpec {
        argv: venv_argv,
        cwd: context.workdir.clone(),
        env: environment.clone(),
        timeout: TOOL_TIMEOUT,
        check: true,
    })?;
    let python = venv_python(&venv);
    let mut last_error = String::new();
    for attempt in 1..=INSTALL_ATTEMPTS {
        let install = run_process(&ProcessSpec {
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
        if install.status.success() {
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
            thread::sleep(Duration::from_secs((attempt * 2) as u64));
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
            let mut rows = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_path(&record)
                .map_err(|error| {
                    SuiteError::Contract(format!("could not read {}: {error}", record.display()))
                })?;
            for row in rows.records() {
                let row = row.map_err(|error| {
                    SuiteError::Contract(format!(
                        "invalid wheel RECORD {}: {error}",
                        record.display()
                    ))
                })?;
                let Some(relative) = row.get(0) else {
                    continue;
                };
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

fn venv_python(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts/python.exe")
    } else {
        venv.join("bin/python")
    }
}

fn venv_skit(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts/skit.exe")
    } else {
        venv.join("bin/skit")
    }
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
    use std::{collections::BTreeMap, fs};

    use tempfile::TempDir;

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
        fs::write(dist.join("RECORD"), "payload,,\n../../../bin/skit,,\n").unwrap();
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
    }
}
