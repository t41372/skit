//! Complete benchmark suite dispatch.

mod footprint;
mod imports;
mod micro;
mod rss;
mod run_overhead;
mod scale;
mod startup;
mod syscalls;
mod tui;

use thiserror::Error;

use crate::{SuiteKind, SuiteOutput, SuitePlan, runner::RunContext};

/// A suite crashed after pre-spawn skip decisions were complete.
#[derive(Debug, Error)]
pub enum SuiteError {
    /// Shared orchestration failed.
    #[error(transparent)]
    Runner(#[from] crate::runner::RunnerError),
    /// A direct suite probe failed.
    #[error(transparent)]
    Process(#[from] crate::process::ProcessError),
    /// Tool output parse failed.
    #[error(transparent)]
    Parse(#[from] crate::parsers::ParseError),
    /// Statistical summary failed.
    #[error(transparent)]
    Stats(#[from] crate::stats::StatsError),
    /// Dataset generation failed.
    #[error(transparent)]
    Dataset(#[from] crate::dataset::DatasetError),
    /// Suite-specific contract failed.
    #[error("{0}")]
    Contract(String),
}

/// Execute one suite implementation.
pub fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    match plan.kind {
        SuiteKind::Imports => imports::run(context, plan),
        SuiteKind::Footprint => footprint::run(context, plan),
        SuiteKind::Startup => startup::run(context, plan),
        SuiteKind::Scale => scale::run(context, plan),
        SuiteKind::RunOverhead => run_overhead::run(context, plan),
        SuiteKind::Rss => rss::run(context, plan),
        SuiteKind::Micro => micro::run(context, plan),
        SuiteKind::Syscalls => syscalls::run(context, plan),
        SuiteKind::Tui => tui::run(context, plan),
    }
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        os::unix::fs::PermissionsExt as _,
        path::{Path, PathBuf},
    };

    use tempfile::TempDir;

    use crate::{
        SuiteKind, SuitePlan,
        dataset::{DEFAULT_SEED, DEFAULT_STATE_FRACTION, generate},
        runner::RunContext,
    };

    pub(crate) fn executable(path: &Path, body: &str) -> PathBuf {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        path.to_path_buf()
    }

    pub(crate) fn plan(kind: SuiteKind, library_sizes: &[usize]) -> SuitePlan {
        SuitePlan {
            kind,
            library_sizes: library_sizes.to_vec(),
            warmup: 1,
            minimum_runs: 2,
            samples: 1,
            fast: true,
            measure_closure: false,
            run_javascript_lane: false,
            run_doctor: false,
            compare_mode: false,
        }
    }

    pub(crate) struct Fixture {
        _root: TempDir,
        pub(crate) context: RunContext,
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            let root = TempDir::new().unwrap();
            let workdir = root.path().join("work");
            let out_dir = root.path().join("out");
            let tools = root.path().join("tools");
            fs::create_dir_all(&workdir).unwrap();
            fs::create_dir_all(&out_dir).unwrap();
            fs::create_dir_all(&tools).unwrap();

            let dataset0 = generate(
                &root.path().join("dataset-0"),
                0,
                DEFAULT_SEED,
                DEFAULT_STATE_FRACTION,
            )
            .unwrap();
            let dataset100 = generate(
                &root.path().join("dataset-100"),
                100,
                DEFAULT_SEED,
                DEFAULT_STATE_FRACTION,
            )
            .unwrap();
            let datasets = BTreeMap::from([(0, dataset0), (100, dataset100)]);

            let skit = executable(&tools.join("skit"), "#!/bin/sh\nprintf 'skit 0.5.0\\n'\n");
            let subject = executable(&tools.join("subject"), "#!/bin/sh\nexit 0\n");
            let harness = executable(
                &tools.join("skit-bench"),
                r#"#!/bin/sh
case " $* " in
  *" probe tui "*)
    case " $* " in
      *" --entries 0 "*) selected=null ;;
      *) selected=0.25 ;;
    esac
    printf '{"first_idle_ms":1.5,"select_ms":%s,"search_ms":0.75,"status_text":"VmHWM:\\t1234 kB\\n"}\n' "$selected"
    ;;
  *) printf '1.25\n' ;;
esac
"#,
            );
            let hyperfine = executable(
                &tools.join("hyperfine"),
                r#"#!/bin/sh
out=
rows=
separator=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --export-json) out=$2; shift 2 ;;
    --command-name)
      rows="${rows}${separator}{\"command\":\"$2\",\"times\":[0.001,0.002],\"exit_codes\":[0,0]}"
      separator=,
      shift 3
      ;;
    *) shift ;;
  esac
done
printf '{"results":[%s]}\n' "$rows" > "$out"
"#,
            );
            let uv = executable(
                &tools.join("uv"),
                r#"#!/bin/sh
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
    shift
    while [ "$#" -gt 0 ]; do
      case "$1" in --python) python=$2; shift 2 ;; *) shift ;; esac
    done
    venv=${python%/bin/python}
    site="$venv/lib/python3.13/site-packages"
    dist="$site/skit_cli-0.5.0.dist-info"
    mkdir -p "$dist"
    printf payload > "$site/skit_payload"
    printf '#!/bin/sh\nexit 0\n' > "$venv/bin/skit"
    chmod +x "$venv/bin/skit"
    printf 'skit_payload,,\n../../../bin/skit,,\n' > "$dist/RECORD"
    printf 'Metadata-Version: 2.4\nName: skit-cli\n' > "$dist/METADATA"
    ;;
  --version) printf 'uv 0.11.26\n' ;;
esac
"#,
            );
            let strace = executable(
                &tools.join("strace"),
                r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
  case "$1" in -o) out=$2; shift 2 ;; *) shift ;; esac
done
printf ' 80.00 0.008 8 9 openat\n 20.00 0.002 2 1 socket\n' > "$out"
"#,
            );
            let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap();
            Self {
                context: RunContext {
                    repo_root,
                    out_dir,
                    workdir,
                    datasets,
                    skit,
                    harness,
                    python: Some(subject.clone()),
                    uv: Some(uv),
                    bash: Some(subject.clone()),
                    node: Some(subject),
                    hyperfine: Some(hyperfine),
                    strace: Some(strace),
                    cargo: None,
                    rustc: None,
                },
                _root: root,
            }
        }
    }

    #[test]
    fn every_suite_dispatches_and_preserves_its_public_metric_and_raw_contract() {
        let fixture = Fixture::new();
        let context = &fixture.context;

        let imports = super::run(context, &plan(SuiteKind::Imports, &[0, 100])).unwrap();
        assert_eq!(imports.metrics["imports.version.modules"].value, 0.0);
        assert!(imports.raw.contains_key("importtime_top"));
        assert!(
            fs::read_to_string(context.out_dir.join("artifacts/importtime.txt"))
                .unwrap()
                .contains("native Rust binary")
        );

        let mut footprint_plan = plan(SuiteKind::Footprint, &[0, 100]);
        footprint_plan.measure_closure = true;
        let footprint = super::run(context, &footprint_plan).unwrap();
        assert!(footprint.metrics["footprint.wheel_bytes"].value > 0.0);
        assert!(footprint.metrics["footprint.closure_bytes"].value > 0.0);
        assert_eq!(
            footprint.metrics["repository.python_implementation_files"].value,
            0.0
        );

        let rss = super::run(context, &plan(SuiteKind::Rss, &[0])).unwrap();
        assert_eq!(rss.metrics["rss.version.peak_kib"].n, 1);

        let startup = super::run(context, &plan(SuiteKind::Startup, &[0])).unwrap();
        assert_eq!(startup.metrics["startup.version.median_ms"].value, 1.5);
        assert_eq!(startup.metrics["startup.import_skit.median_ms"].value, 0.0);

        let mut scale_plan = plan(SuiteKind::Scale, &[0, 100]);
        scale_plan.run_doctor = true;
        let scale = super::run(context, &scale_plan).unwrap();
        assert!(scale.metrics.contains_key("scale.show.n100.median_ms"));
        assert!(scale.metrics.contains_key("scale.doctor_json.n0.median_ms"));

        let mut overhead_plan = plan(SuiteKind::RunOverhead, &[]);
        overhead_plan.run_javascript_lane = true;
        let overhead = super::run(context, &overhead_plan).unwrap();
        assert!(
            overhead
                .metrics
                .contains_key("run_overhead.js.skit.median_ms")
        );

        let micro = super::run(context, &plan(SuiteKind::Micro, &[100])).unwrap();
        assert!(
            micro
                .metrics
                .contains_key("micro.store.list_entries.n100.median_us")
        );
        assert!(micro.raw.contains_key("analyzers"));

        let tui = super::run(context, &plan(SuiteKind::Tui, &[0, 100])).unwrap();
        assert_eq!(tui.metrics["tui.first_idle.n100.median_ms"].value, 1.5);
        assert!(
            !tui.raw["n100"]
                .as_object()
                .unwrap()
                .contains_key("import_ms")
        );

        let syscalls = super::run(context, &plan(SuiteKind::Syscalls, &[100])).unwrap();
        assert_eq!(syscalls.metrics["syscalls.list_json.file_ops"].value, 9.0);
        assert_eq!(syscalls.metrics["syscalls.list_json.network"].value, 1.0);
    }

    #[test]
    fn optional_tool_absence_is_a_typed_skip_at_each_suite_boundary() {
        let mut fixture = Fixture::new();

        fixture.context.uv = None;
        let footprint = super::footprint::run(
            &fixture.context,
            &plan(SuiteKind::Footprint, &[0]),
        )
        .unwrap();
        assert_eq!(footprint.skipped[0].reason, "uv not found");

        fixture.context.hyperfine = None;
        let startup =
            super::startup::run(&fixture.context, &plan(SuiteKind::Startup, &[0])).unwrap();
        let scale = super::scale::run(&fixture.context, &plan(SuiteKind::Scale, &[0])).unwrap();
        let overhead =
            super::run_overhead::run(&fixture.context, &plan(SuiteKind::RunOverhead, &[])).unwrap();
        assert_eq!(startup.skipped[0].reason, "hyperfine not found");
        assert_eq!(scale.skipped[0].reason, "hyperfine not found");
        assert_eq!(overhead.skipped[0].reason, "hyperfine not found");

        fixture.context.hyperfine = Some(fixture.context.workdir.join("not-used"));
        fixture.context.python = None;
        let startup =
            super::startup::run(&fixture.context, &plan(SuiteKind::Startup, &[0])).unwrap();
        assert_eq!(startup.skipped[0].reason, "python not found");

        fixture.context.strace = None;
        let syscalls =
            super::syscalls::run(&fixture.context, &plan(SuiteKind::Syscalls, &[100])).unwrap();
        assert_eq!(syscalls.skipped[0].reason, "strace not found");
    }

    #[test]
    fn optional_cases_degrade_individually_without_erasing_other_measurements() {
        let mut fixture = Fixture::new();
        fixture.context.python = None;
        let mut footprint_plan = plan(SuiteKind::Footprint, &[0, 100]);
        footprint_plan.measure_closure = true;
        let footprint = super::footprint::run(&fixture.context, &footprint_plan).unwrap();
        assert_eq!(footprint.skipped[0].case, "closure");
        assert!(footprint.metrics.contains_key("footprint.wheel_bytes"));

        let mut scale_plan = plan(SuiteKind::Scale, &[0]);
        scale_plan.run_doctor = true;
        fixture.context.uv = None;
        let scale = super::scale::run(&fixture.context, &scale_plan).unwrap();
        assert_eq!(scale.skipped[0].case, "doctor_json.n0");

        fixture.context.python = Some(fixture.context.skit.clone());
        let overhead =
            super::run_overhead::run(&fixture.context, &plan(SuiteKind::RunOverhead, &[])).unwrap();
        assert_eq!(overhead.skipped[0].reason, "uv not found");

        fixture.context.uv = Some(fixture.context.skit.clone());
        fixture.context.python = None;
        let overhead =
            super::run_overhead::run(&fixture.context, &plan(SuiteKind::RunOverhead, &[])).unwrap();
        assert_eq!(overhead.skipped[0].reason, "python not found");

        let mut fixture = Fixture::new();
        fixture.context.bash = None;
        fixture.context.node = None;
        let mut overhead_plan = plan(SuiteKind::RunOverhead, &[]);
        overhead_plan.run_javascript_lane = true;
        let overhead = super::run_overhead::run(&fixture.context, &overhead_plan).unwrap();
        assert!(overhead.skipped.iter().any(|skip| skip.case == "shell"));
        assert!(overhead.skipped.iter().any(|skip| skip.case == "js"));
        assert!(
            overhead
                .metrics
                .contains_key("run_overhead.python.skit.median_ms")
        );
    }

    #[test]
    fn malformed_suite_plans_fail_before_starting_a_subject() {
        let fixture = Fixture::new();
        assert!(super::imports::run(&fixture.context, &plan(SuiteKind::Imports, &[])).is_err());
        assert!(
            super::footprint::run(&fixture.context, &plan(SuiteKind::Footprint, &[])).is_err()
        );
        assert!(super::startup::run(&fixture.context, &plan(SuiteKind::Startup, &[])).is_err());
        assert!(super::syscalls::run(
            &fixture.context,
            &plan(SuiteKind::Syscalls, &[0, 100])
        )
        .is_err());
    }
}
