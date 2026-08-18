//! Frozen parser, Hyperfine, and comparison contracts from `tests/test_benchmarks_tooling.py`.

use std::collections::BTreeMap;

use skit_benchmarks::{
    BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, Results, Skip, SuiteKind,
    compare::{compare, render_markdown},
    hyperfine::{Case, build_argv, metric_from_times, parse_export},
    parsers::{
        FILE_OP_SYSCALLS, NETWORK_SYSCALLS, bsd_time_max_kib, count_group, gnu_time_max_kib,
        strace_counts, vmhwm_kib,
    },
    stats::{median, nearest_rank_p95, sample_stddev},
};

fn meta() -> Meta {
    Meta {
        generated_at: "2026-07-20T12:00:00+00:00".to_owned(),
        profile: BenchmarkProfile::Pr,
        git: GitInfo {
            commit: "abcdef1234567890".to_owned(),
            dirty: false,
            pr: None,
        },
        skit_version: "0.2.1.dev0".to_owned(),
        host: HostInfo {
            os: "Linux".to_owned(),
            kernel: "6.8.0".to_owned(),
            cpu: "Test CPU".to_owned(),
            cpu_count: 8,
            mem_total_mib: 16_384,
            platform_key: "linux-x86_64".to_owned(),
            ci_runner: Some("ubuntu-24.04".to_owned()),
            ci_image_version: Some("20260719.1".to_owned()),
        },
        python: "3.13.5".to_owned(),
        uv: "0.11.26".to_owned(),
        textual: "8.2.8".to_owned(),
        pyperf: "2.10.0".to_owned(),
    }
}

fn metric(value: f64, unit: &str, n: usize) -> Metric {
    Metric {
        value,
        unit: unit.to_owned(),
        n,
        p95: None,
        stddev: None,
    }
}

fn results(metrics: &[(&str, f64, &str, usize)]) -> Results {
    Results {
        schema_version: 1,
        meta: meta(),
        metrics: metrics
            .iter()
            .map(|(name, value, unit, n)| ((*name).to_owned(), metric(*value, unit, *n)))
            .collect(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

#[test]
fn test_stats() {
    assert_eq!(median(&[3.0, 1.0, 2.0]).unwrap(), 2.0);
    assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]).unwrap(), 2.5);
    assert_eq!(
        nearest_rank_p95(&(1..=100).map(f64::from).collect::<Vec<_>>()).unwrap(),
        95.0
    );
    assert_eq!(nearest_rank_p95(&[5.0]).unwrap(), 5.0);
    assert!((sample_stddev(&[2.0, 4.0]).unwrap() - 1.414_213_562).abs() < 0.0015);
    assert_eq!(sample_stddev(&[7.0]).unwrap(), 0.0);
    assert!(median(&[]).is_err());
    assert!(nearest_rank_p95(&[]).is_err());
    assert!(sample_stddev(&[]).is_err());
}

#[test]
fn test_vmhwm() {
    assert_eq!(vmhwm_kib("VmPeak: 1 kB\nVmHWM:   4321 kB\n").unwrap(), 4_321);
    assert!(vmhwm_kib("VmPeak: 1 kB\n").is_err());
    assert!(vmhwm_kib("VmHWM: broken\n").is_err());
}

#[test]
fn test_maxrss() {
    assert_eq!(gnu_time_max_kib("2048").unwrap(), 2_048);
    assert_eq!(
        bsd_time_max_kib("2097152 maximum resident set size\n").unwrap(),
        2_048
    );
    assert!(gnu_time_max_kib("-1").is_err());
}

#[test]
fn test_strace() {
    let table = concat!(
        "% time     seconds  usecs/call     calls    errors syscall\n",
        "------ ----------- ----------- --------- --------- ----------------\n",
        " 50.00    0.001000           2       500           openat\n",
        " 30.00    0.000600           1       600        12 read\n",
        " 10.00    0.000200           1       200           newfstatat\n",
        "  5.00    0.000100           1         2           socket\n",
        "  5.00    0.000100           1         1           connect\n",
        "------ ----------- ----------- --------- --------- ----------------\n",
        "100.00    0.002000                  1303        12 total\n"
    );
    let counts = strace_counts(table).unwrap();
    assert_eq!(counts["openat"], 500);
    assert_eq!(count_group(&counts, FILE_OP_SYSCALLS), 1_300);
    assert_eq!(count_group(&counts, NETWORK_SYSCALLS), 3);
    assert!(strace_counts("no rows at all").is_err());
}

#[test]
fn test_build_argv() {
    let argv = build_argv(
        &[
            Case::new("a", ["skit", "--version"]),
            Case::new("b", ["skit", "list"]),
        ],
        3,
        15,
        "/tmp/x.json",
        "hyperfine",
    )
    .unwrap();
    assert_eq!(argv[0], "hyperfine");
    assert!(argv.iter().any(|arg| arg == "--shell=none"));
    let name_index = argv.iter().position(|arg| arg == "--command-name").unwrap();
    assert_eq!(argv[name_index + 1], "a");
    assert!(argv.iter().any(|arg| arg == "skit --version"));
    let error = build_argv(&[], 1, 1, "x", "hyperfine")
        .unwrap_err()
        .to_string();
    assert!(error.contains("no cases"));
}

#[test]
fn test_build_argv_quotes_awkward_paths() {
    let argv = build_argv(
        &[Case::new("a", ["/bin/echo", "a b"])],
        1,
        1,
        "x",
        "hyperfine",
    )
    .unwrap();
    assert!(argv.iter().any(|arg| arg == "/bin/echo 'a b'"));
}

#[test]
fn test_parse_export() {
    let text = r#"{"results":[{"command":"a","times":[0.1,0.2,0.3],"exit_codes":[0,0,0]}]}"#;
    assert_eq!(parse_export(text).unwrap()["a"], [0.1, 0.2, 0.3]);

    let nonzero = r#"{"results":[{"command":"a","times":[0.1,0.2,0.3],"exit_codes":[0,1,0]}]}"#;
    assert!(parse_export(nonzero).unwrap_err().to_string().contains("non-zero exit"));
    assert!(parse_export("{}").unwrap_err().to_string().contains("no results"));
    assert!(parse_export("{").unwrap_err().to_string().contains("not JSON"));
    let missing = r#"{"results":[{"command":"a","times":[]}]}"#;
    assert!(
        parse_export(missing)
            .unwrap_err()
            .to_string()
            .contains("missing command/times")
    );
}

#[test]
fn test_metric_from_times() {
    let metric = metric_from_times(&[0.1, 0.2, 0.3]).unwrap();
    assert!((metric.value - 200.0).abs() < f64::EPSILON);
    assert_eq!(metric.unit, "ms");
    assert_eq!(metric.n, 3);
    assert_eq!(metric.p95, Some(300.0));
}

#[test]
fn test_thresholds() {
    let base = results(&[
        ("startup.version.median_ms", 200.0, "ms", 15),
        ("small.wiggle_ms", 10.0, "ms", 15),
        ("imports.version.modules", 100.0, "count", 1),
        ("gone.metric", 1.0, "count", 1),
    ]);
    let head = results(&[
        ("startup.version.median_ms", 230.0, "ms", 15),
        ("small.wiggle_ms", 11.0, "ms", 15),
        ("imports.version.modules", 104.0, "count", 1),
        ("new.metric", 1.0, "count", 1),
    ]);
    let comparison = compare(&base, &head);
    let mut notable = comparison
        .notable()
        .iter()
        .map(|delta| delta.metric.as_str())
        .collect::<Vec<_>>();
    notable.sort();
    assert_eq!(
        notable,
        vec!["imports.version.modules", "startup.version.median_ms"]
    );
    assert_eq!(comparison.only_base, ["gone.metric"]);
    assert_eq!(comparison.only_head, ["new.metric"]);
}

#[test]
fn test_exact_units_ignore_the_noise_threshold() {
    for (unit, base_value, head_value) in [
        ("count", 315.0, 316.0),
        ("bytes", 508_392.0, 508_393.0),
        ("bool", 0.0, 1.0),
    ] {
        let base = results(&[("m.x", base_value, unit, 1)]);
        let head = results(&[("m.x", head_value, unit, 1)]);
        assert!(compare(&base, &head).deltas[0].is_notable(), "{unit}");
        assert!(!compare(&base, &base).deltas[0].is_notable(), "{unit}");
    }
}

#[test]
fn test_render_reports_each_side_skips() {
    let mut base = results(&[("m.x", 1.0, "count", 1)]);
    base.skipped.push(Skip {
        suite: SuiteKind::Micro,
        case: "bench_launch.py".to_owned(),
        reason: "exit 1: ImportError".to_owned(),
    });
    let head = results(&[("m.x", 1.0, "count", 1), ("m.y", 2.0, "count", 1)]);
    let text = render_markdown(&base, &head, &compare(&base, &head));
    assert!(text.contains("### Skipped in base (1)"));
    assert!(text.contains("exit 1: ImportError"));
    assert!(!text.contains("Skipped in head"));
}

#[test]
fn test_zero_base() {
    let base = results(&[("c.median_ms", 0.0, "ms", 15)]);
    let grown = results(&[("c.median_ms", 3.0, "ms", 15)]);
    let delta = &compare(&base, &grown).deltas[0];
    assert_eq!(delta.percent(), None);
    assert!(delta.is_notable());
    assert!(!compare(&base, &base).deltas[0].is_notable());
}

#[test]
fn test_render() {
    let base = results(&[("a.ms", 100.0, "ms", 5), ("b.ms", 10.0, "ms", 5)]);
    let head = results(&[("a.ms", 200.0, "ms", 5), ("b.ms", 10.5, "ms", 5)]);
    let text = render_markdown(&base, &head, &compare(&base, &head));
    assert!(text.contains("### Notable (1)"));
    assert!(text.contains("Within noise"));
    assert!(text.contains("`a.ms`"));
    let empty = render_markdown(&base, &base, &compare(&base, &base));
    assert!(empty.contains("### Notable (none)"));
}

#[test]
fn test_render_only_in_sections() {
    let base = results(&[
        ("shared.ms", 100.0, "ms", 5),
        ("gone.metric", 1.0, "count", 1),
    ]);
    let head = results(&[
        ("shared.ms", 100.0, "ms", 5),
        ("new.metric", 2.0, "count", 1),
    ]);
    let text = render_markdown(&base, &head, &compare(&base, &head));
    assert!(text.contains("### Only in base"));
    assert!(text.contains("- `gone.metric`"));
    assert!(text.contains("### Only in head"));
    assert!(text.contains("- `new.metric`"));
}

#[test]
fn test_unit_mismatch_is_loud_and_never_mints_a_false_delta() {
    let base = results(&[("elapsed", 1.0, "s", 5)]);
    let head = results(&[("elapsed", 1_000.0, "ms", 5)]);
    let comparison = compare(&base, &head);
    assert!(comparison.deltas.is_empty());
    assert_eq!(comparison.incomparable, ["unit elapsed: s vs ms"]);
    let text = render_markdown(&base, &head, &comparison);
    assert!(text.contains("not directly comparable"));
    assert!(!text.contains("+99900"));
}
