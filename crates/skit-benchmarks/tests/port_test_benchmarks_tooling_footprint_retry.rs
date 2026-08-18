#![cfg(unix)]
//! Frozen install-closure retry, bound, timeout, and environment-isolation contract.

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

fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("missing function signature {signature:?}"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap();
    let mut depth = 0_usize;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated function body for {signature:?}")
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
    assert!(
        Command::new("git")
            .arg("init")
            .arg(&repo)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "skit/runtime.py"])
            .status()
            .unwrap()
            .success()
    );

    let tools = root.path().join("tools");
    fs::create_dir_all(&tools).unwrap();
    let skit = executable(tools.join("skit"), "#!/bin/sh\nexit 0\n");
    let harness = executable(tools.join("harness"), "#!/bin/sh\nprintf '1.0\\n'\n");
    let pip_count = root.path().join("pip-count");
    let log = root.path().join("uv-log");
    let uv_source = r#"#!/bin/sh
set -eu
COUNT='__COUNT__'
LOG='__LOG__'
printf '%s|%s|%s|%s|%s\n' "$*" "$PWD" "${SKIT_DATA_DIR-}" "${HOME-}" "${UV_INDEX_URL-unset}" >> "$LOG"

case "$1" in
  build)
    shift
    out=
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --out-dir)
          out=$2
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    [ -n "$out" ]
    mkdir -p "$out"
    printf wheel > "$out/skit_cli-0.0.0-py3-none-any.whl"
    printf sdist > "$out/skit_cli-0.0.0.tar.gz"
    ;;
  venv)
    venv=$2
    mkdir -p "$venv/bin" "$venv/lib/python3.13/site-packages"
    printf '#!/bin/sh\nexit 0\n' > "$venv/bin/python"
    chmod +x "$venv/bin/python"
    ;;
  pip)
    shift
    python=
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --python)
          python=$2
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    [ -n "$python" ]
    n=0
    [ ! -f "$COUNT" ] || n=$(cat "$COUNT")
    n=$((n + 1))
    printf '%s' "$n" > "$COUNT"
    if [ "$n" -eq 1 ]; then
      printf 'temporary network error\n' >&2
      exit 1
    fi

    venv=${python%/bin/python}
    site="$venv/lib/python3.13/site-packages"
    dist="$site/skit_cli-0.0.0.dist-info"
    mkdir -p "$site/skit" "$dist"
    printf installed > "$site/skit/runtime.py"
    printf 'Metadata-Version: 2.4\nName: skit-cli\n' > "$dist/METADATA"
    printf 'skit/runtime.py,,\n' > "$dist/RECORD"
    ;;
  *)
    exit 2
    ;;
esac
"#
    .replace("__COUNT__", &pip_count.display().to_string())
    .replace("__LOG__", &log.display().to_string());
    let uv = executable(tools.join("uv"), &uv_source);

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

    let closure = &output.metrics["footprint.closure_bytes"];
    assert!(closure.value > 0.0);
    assert_eq!(closure.unit, "bytes");
    assert_eq!(closure.n, 1);
    assert!(output.metrics["footprint.skit_installed_bytes"].value > 0.0);
    assert_eq!(output.metrics["footprint.distributions"].value, 1.0);

    let expected = context.environment(0).unwrap();
    let calls = fs::read_to_string(&log).unwrap();
    let closure_calls = calls
        .lines()
        .filter(|line| line.starts_with("venv ") || line.starts_with("pip "))
        .collect::<Vec<_>>();
    assert_eq!(
        closure_calls.len(),
        3,
        "one venv plus two bounded install attempts were expected: {calls}"
    );
    assert!(closure_calls[0].starts_with("venv "));
    assert!(closure_calls[1].starts_with("pip "));
    assert!(closure_calls[2].starts_with("pip "));

    let expected_workdir = context.workdir.display().to_string();
    for call in closure_calls {
        let fields = call.splitn(5, '|').collect::<Vec<_>>();
        assert_eq!(fields.len(), 5, "malformed fake-uv evidence row: {call}");
        assert_eq!(fields[1], expected_workdir.as_str());
        assert_eq!(fields[2], expected["SKIT_DATA_DIR"].as_str());
        assert_eq!(fields[3], expected["HOME"].as_str());
        assert_eq!(
            fields[4], "unset",
            "ambient UV_INDEX_URL leaked into a measured child"
        );
    }

    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/suites/footprint.rs"),
    )
    .unwrap();
    assert!(source.contains("const INSTALL_ATTEMPTS: usize = 3"));
    let body = function_body(&source, "fn measure_closure");
    assert!(body.contains("for attempt in 1..=INSTALL_ATTEMPTS"));
    assert!(body.contains("thread::sleep"));
    assert!(body.contains("(attempt * 2) as u64"));
    assert_eq!(
        body.matches("run_process(&ProcessSpec").count(),
        2,
        "the closure has exactly one venv spawn and one bounded install spawn site"
    );
    assert_eq!(
        body.matches("timeout: TOOL_TIMEOUT").count(),
        2,
        "every closure spawn must remain bounded"
    );
    assert_eq!(
        body.matches("cwd: context.workdir.clone()").count(),
        2,
        "every closure spawn must remain in the isolated benchmark workdir"
    );
    assert_eq!(
        body.matches("env: environment.clone()").count(),
        2,
        "every closure spawn must receive the constructed benchmark environment"
    );
}
