#![cfg(unix)]
//! Frozen install-closure retry/isolation contract from `tests/test_benchmarks_tooling.py`.

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
};

use skit_benchmarks::{
    SuiteKind, SuitePlan,
    dataset::{DEFAULT_SEED, DEFAULT_STATE_FRACTION, generate},
    runner::RunContext,
    suites,
};
use tempfile::TempDir;

fn executable(path: PathBuf, source: &str) -> PathBuf {
    fs::write(&path, source).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[test]
fn test_footprint_closure_bounds_and_isolates_retries() {
    let root = TempDir::new().unwrap();
    let dataset = generate(
        &root.path().join("dataset-0"),
        0,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join("skit")).unwrap();
    fs::write(repo.join("skit/runtime.py"), "x = 1\n").unwrap();
    assert!(Command::new("git").arg("init").arg(&repo).status().unwrap().success());
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "skit/runtime.py"])
            .status()
            .unwrap()
            .success()
    );

    let skit = executable(root.path().join("skit-bin"), "#!/bin/sh\nexit 0\n");
    let harness = executable(root.path().join("harness"), "#!/bin/sh\nprintf '1.0\\n'\n");
    let pip_count = root.path().join("pip-count");
    let log = root.path().join("uv-log");
    let uv = executable(
        root.path().join("uv"),
        &format!(
            r#"#!/bin/sh
set -eu
COUNT='{count}'
LOG='{log}'
printf '%s|%s|%s|%s\n' "$*" "$PWD" "${{SKIT_DATA_DIR-}}" "${{HOME-}}" >> "$LOG"
case "$1" in
  build)
    mkdir -p "$PWD/dist"
    printf wheel > "$PWD/dist/skit-0.0.0-py3-none-any.whl"
    printf sdist > "$PWD/dist/skit-0.0.0.tar.gz"
    ;;
  venv)
    for VENV do :; done
    mkdir -p "$VENV/bin" "$VENV/site"
    : > "$VENV/bin/python"
    ;;
  run)
    VENV=$(dirname "$(dirname "$3")")
    printf '%s\n' "$VENV/site"
    ;;
  pip)
    n=0
    [ ! -f "$COUNT" ] || n=$(cat "$COUNT")
    n=$((n + 1))
    printf '%s' "$n" > "$COUNT"
    if [ "$n" -eq 1 ]; then exit 1; fi
    VENV=$(dirname "$(dirname "$3")")
    mkdir -p "$VENV/site/skit"
    printf installed > "$VENV/site/skit/runtime.py"
    ;;
esac
"#,
            count = pip_count.display(),
            log = log.display(),
        ),
    );
    let workdir = root.path().join("work");
    let out_dir = root.path().join("out");
    fs::create_dir_all(&workdir).unwrap();
    fs::create_dir_all(&out_dir).unwrap();
    let context = RunContext {
        repo_root: repo,
        out_dir,
        workdir,
        datasets: BTreeMap::from([(0, dataset)]),
        skit,
        harness,
        python: Some(PathBuf::from("/bin/sh")),
        uv: Some(uv),
        bash: Some(PathBuf::from("/bin/sh")),
        node: None,
        hyperfine: None,
        strace: None,
        cargo: None,
        rustc: None,
    };
    let plan = SuitePlan {
        kind: SuiteKind::Footprint,
        library_sizes: vec![0],
        warmup: 0,
        minimum_runs: 1,
        samples: 1,
        fast: true,
        measure_closure: true,
        run_javascript_lane: false,
        run_doctor: false,
        compare_mode: false,
    };

    let output = suites::run(&context, &plan).unwrap();
    assert_eq!(fs::read_to_string(&pip_count).unwrap(), "2");
    assert!(output.metrics["footprint.install_closure.bytes"].value > 0.0);
    assert_eq!(output.metrics["footprint.install_closure.bytes"].unit, "bytes");

    let expected = context.environment(0).unwrap();
    let calls = fs::read_to_string(&log).unwrap();
    let closure_calls = calls
        .lines()
        .filter(|line| line.starts_with("venv ") || line.starts_with("run ") || line.starts_with("pip "))
        .collect::<Vec<_>>();
    assert!(closure_calls.len() >= 5, "retry did not create isolated child calls: {calls}");
    for call in closure_calls {
        assert!(call.contains(&format!("|{}|", context.workdir.display())));
        assert!(call.contains(&format!("|{}|", expected["SKIT_DATA_DIR"])));
        assert!(call.ends_with(&format!("|{}", expected["HOME"])));
    }

    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/suites/footprint.rs"),
    )
    .unwrap();
    assert!(source.contains("const INSTALL_ATTEMPTS: usize = 3"));
    assert!(source.matches("timeout: TOOL_TIMEOUT").count() >= 4);
}
