use std::collections::BTreeMap;

use skit_benchmarks::{
    BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, PipelineError, Results, ResultsError, Skip,
    SuiteKind, SuiteOutput, build_plan, merge,
};

#[test]
fn pr_profile_keeps_every_latest_main_suite_and_sampling_rule() {
    let plan = build_plan(BenchmarkProfile::Pr);

    assert_eq!(
        plan.iter().map(|suite| suite.kind).collect::<Vec<_>>(),
        [
            SuiteKind::Imports,
            SuiteKind::Footprint,
            SuiteKind::Rss,
            SuiteKind::Startup,
            SuiteKind::Scale,
            SuiteKind::RunOverhead,
            SuiteKind::Micro,
            SuiteKind::Tui,
        ]
    );
    assert_eq!(plan[0].library_sizes, [0, 100]);
    assert_eq!(plan[1].library_sizes, [0, 1_000]);
    assert!(!plan[1].measure_closure);
    assert_eq!(plan[2].samples, 5);
    assert_eq!(plan[3].warmup, 3);
    assert_eq!(plan[3].minimum_runs, 15);
    assert_eq!(plan[4].library_sizes, [0, 100, 1_000]);
    assert!(!plan[4].run_doctor);
    assert!(!plan[5].run_javascript_lane);
    assert!(plan[6].fast);
    assert_eq!(plan[7].samples, 5);
    assert!(plan.iter().all(|suite| !suite.compare_mode));
}

#[test]
fn full_profile_adds_nightly_only_measurements_without_dropping_a_suite() {
    let plan = build_plan(BenchmarkProfile::Full);

    assert_eq!(
        plan.iter().map(|suite| suite.kind).collect::<Vec<_>>(),
        [
            SuiteKind::Imports,
            SuiteKind::Footprint,
            SuiteKind::Rss,
            SuiteKind::Startup,
            SuiteKind::Scale,
            SuiteKind::RunOverhead,
            SuiteKind::Micro,
            SuiteKind::Tui,
            SuiteKind::Syscalls,
        ]
    );
    assert!(plan[1].measure_closure);
    assert_eq!(plan[2].samples, 10);
    assert_eq!(plan[3].warmup, 5);
    assert_eq!(plan[3].minimum_runs, 40);
    assert_eq!(plan[4].library_sizes, [0, 10, 100, 1_000]);
    assert!(plan[4].run_doctor);
    assert!(plan[5].run_javascript_lane);
    assert!(!plan[6].fast);
    assert_eq!(plan[7].samples, 10);
    assert_eq!(plan[8].library_sizes, [1_000]);
}

#[test]
fn compare_profile_uses_the_pr_grid_but_never_measures_the_harness_footprint() {
    let plan = build_plan(BenchmarkProfile::Compare);

    assert_eq!(
        plan.iter().map(|suite| suite.kind).collect::<Vec<_>>(),
        [
            SuiteKind::Imports,
            SuiteKind::Rss,
            SuiteKind::Startup,
            SuiteKind::Scale,
            SuiteKind::RunOverhead,
            SuiteKind::Micro,
            SuiteKind::Tui,
        ]
    );
    assert!(plan.iter().all(|suite| suite.compare_mode));
    assert!(!plan.iter().any(|suite| suite.kind == SuiteKind::Footprint));
    assert!(!plan.iter().any(|suite| suite.kind == SuiteKind::Syscalls));
}

#[test]
fn dataset_sizes_are_the_sorted_union_of_every_planned_suite() {
    assert_eq!(
        skit_benchmarks::dataset_sizes(&build_plan(BenchmarkProfile::Pr)),
        [0, 100, 1_000]
    );
    assert_eq!(
        skit_benchmarks::dataset_sizes(&build_plan(BenchmarkProfile::Full)),
        [0, 10, 100, 1_000]
    );
}

fn metric(value: f64, unit: &str) -> Metric {
    Metric {
        value,
        unit: unit.to_owned(),
        n: 1,
        p95: None,
        stddev: None,
    }
}

fn meta() -> Meta {
    Meta {
        generated_at: "2026-08-08T12:00:00Z".to_owned(),
        profile: BenchmarkProfile::Pr,
        git: GitInfo {
            commit: "abcdef1234567890".to_owned(),
            dirty: false,
            pr: Some("29".to_owned()),
        },
        skit_version: "0.5.0".to_owned(),
        host: HostInfo {
            os: "Linux".to_owned(),
            kernel: "6.8.0".to_owned(),
            cpu: "Test CPU".to_owned(),
            cpu_count: 8,
            mem_total_mib: 16_384,
            platform_key: "linux-x86_64".to_owned(),
            ci_runner: Some("ubuntu-24.04".to_owned()),
            ci_image_version: Some("20260808.1".to_owned()),
        },
        python: "3.13.5".to_owned(),
        uv: "0.11.26".to_owned(),
        textual: "8.2.8".to_owned(),
        pyperf: "2.10.0".to_owned(),
    }
}

fn output(suite: SuiteKind, duration_seconds: f64, metrics: &[(&str, f64, &str)]) -> SuiteOutput {
    SuiteOutput {
        suite,
        duration_seconds,
        metrics: metrics
            .iter()
            .map(|(name, value, unit)| ((*name).to_owned(), metric(*value, unit)))
            .collect(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

#[test]
fn merge_adds_pipeline_facts_and_derives_cross_suite_metrics() {
    let mut startup = output(
        SuiteKind::Startup,
        1.25,
        &[
            ("startup.version.median_ms", 18.5, "ms"),
            ("startup.python.median_ms", 7.25, "ms"),
        ],
    );
    startup.skipped.push(Skip {
        suite: SuiteKind::Startup,
        case: "platform-probe".to_owned(),
        reason: "not supported".to_owned(),
    });
    let scale = output(
        SuiteKind::Scale,
        2.5,
        &[
            ("scale.list_json.n0.median_ms", 5.0, "ms"),
            ("scale.list_json.n1000.median_ms", 32.0, "ms"),
        ],
    );

    let results = merge(meta(), vec![startup, scale], 4.0).unwrap();

    assert_eq!(results.metrics["pipeline.duration_s"], metric(4.0, "s"));
    assert_eq!(
        results.metrics["pipeline.suite.startup.duration_s"],
        metric(1.25, "s")
    );
    assert_eq!(
        results.metrics["pipeline.suite.scale.duration_s"],
        metric(2.5, "s")
    );
    assert_eq!(
        results.metrics["pipeline.skipped_count"],
        metric(1.0, "count")
    );
    assert_eq!(
        results.metrics["startup.version.over_python_ms"],
        metric(11.25, "ms")
    );
    assert_eq!(
        results.metrics["scale.list_json.per_entry_us"],
        metric(27.0, "us")
    );
    assert_eq!(results.skipped.len(), 1);
    assert_eq!(
        results.raw,
        BTreeMap::from([
            ("scale".to_owned(), serde_json::json!({})),
            ("startup".to_owned(), serde_json::json!({})),
        ])
    );
}

#[test]
fn merge_refuses_duplicate_and_reserved_metric_ids() {
    let first = output(SuiteKind::Imports, 0.1, &[("same", 1.0, "count")]);
    let second = output(SuiteKind::Scale, 0.2, &[("same", 2.0, "count")]);
    assert_eq!(
        merge(meta(), vec![first, second], 1.0),
        Err(PipelineError::DuplicateMetric("same".to_owned()))
    );

    let reserved = output(
        SuiteKind::Imports,
        0.1,
        &[("pipeline.user_supplied", 1.0, "count")],
    );
    assert_eq!(
        merge(meta(), vec![reserved], 1.0),
        Err(PipelineError::ReservedMetric(
            "pipeline.user_supplied".to_owned()
        ))
    );

    let first = output(SuiteKind::Imports, 0.1, &[("one", 1.0, "count")]);
    let second = output(SuiteKind::Imports, 0.2, &[("two", 2.0, "count")]);
    assert_eq!(
        merge(meta(), vec![first, second], 1.0),
        Err(PipelineError::DuplicateSuite("imports".to_owned()))
    );
}

#[test]
fn suite_outputs_and_merge_reject_skips_owned_by_another_suite() {
    let mut startup = output(SuiteKind::Startup, 0.1, &[]);
    startup.skipped.push(Skip {
        suite: SuiteKind::Scale,
        case: "all".to_owned(),
        reason: "not supported".to_owned(),
    });
    assert!(startup.to_json().is_err());
    let json = serde_json::to_string(&startup).unwrap();
    assert!(matches!(
        SuiteOutput::from_json(&json),
        Err(ResultsError::InvalidField { ref path, .. }) if path == "skipped[0].suite"
    ));
    assert_eq!(
        merge(meta(), vec![startup], 1.0),
        Err(PipelineError::SkipSuiteMismatch {
            output: "startup".to_owned(),
            skip: "scale".to_owned(),
        })
    );
}

#[test]
fn merge_rejects_invalid_suite_and_total_durations_before_publication() {
    let invalid_suite = output(SuiteKind::Imports, -0.1, &[]);
    assert!(matches!(
        merge(meta(), vec![invalid_suite], 1.0),
        Err(PipelineError::InvalidSuiteOutput { ref suite, .. }) if suite == "imports"
    ));
    assert!(matches!(
        merge(meta(), Vec::new(), f64::NAN),
        Err(PipelineError::InvalidTotalDuration)
    ));
    assert_eq!(
        merge(meta(), Vec::new(), f64::MAX).unwrap().metrics["pipeline.duration_s"].value,
        f64::MAX
    );

    let mut invalid_meta = meta();
    invalid_meta.git.commit.clear();
    assert!(matches!(
        merge(invalid_meta, Vec::new(), 1.0),
        Err(PipelineError::InvalidMeta(_))
    ));

    let overflowing_derivation = output(
        SuiteKind::Startup,
        0.1,
        &[
            ("startup.version.median_ms", f64::MAX, "ms"),
            ("startup.python.median_ms", -f64::MAX, "ms"),
        ],
    );
    assert!(matches!(
        merge(meta(), vec![overflowing_derivation], 1.0),
        Err(PipelineError::InvalidMergedResults(_))
    ));
}

#[test]
fn strict_derivations_refuse_half_present_pairs_but_scale_can_omit_one_endpoint() {
    let startup = output(
        SuiteKind::Startup,
        0.1,
        &[("startup.version.median_ms", 10.0, "ms")],
    );
    assert_eq!(
        merge(meta(), vec![startup], 1.0),
        Err(PipelineError::HalfPresentDerivation {
            target: "startup.version.over_python_ms",
            present: "startup.version.median_ms",
            absent: "startup.python.median_ms",
        })
    );

    let scale = output(
        SuiteKind::Scale,
        0.1,
        &[("scale.list_json.n0.median_ms", 2.0, "ms")],
    );
    let results = merge(meta(), vec![scale], 1.0).unwrap();
    assert!(!results.metrics.contains_key("scale.list_json.per_entry_us"));
}

#[test]
fn results_schema_round_trips_every_statistical_and_provenance_field() {
    let mut results = merge(
        meta(),
        vec![output(
            SuiteKind::Startup,
            1.25,
            &[
                ("startup.version.median_ms", 18.5, "ms"),
                ("startup.python.median_ms", 7.5, "ms"),
            ],
        )],
        1.5,
    )
    .unwrap();
    results
        .metrics
        .get_mut("startup.version.median_ms")
        .unwrap()
        .clone_from(&Metric {
            value: 18.5,
            unit: "ms".to_owned(),
            n: 15,
            p95: Some(20.0),
            stddev: Some(0.75),
        });

    let json = results.to_json().unwrap();
    let decoded = Results::from_json(&json).unwrap();

    assert_eq!(decoded, results);
    assert!(json.ends_with('\n'));
    assert_eq!(decoded.schema_version, 1);
    assert_eq!(decoded.meta.git.pr.as_deref(), Some("29"));
}

#[test]
fn results_validation_rejects_decay_instead_of_accepting_plausible_json() {
    let valid = merge(meta(), Vec::new(), 0.5).unwrap().to_json().unwrap();
    let mut document: serde_json::Value = serde_json::from_str(&valid).unwrap();

    document["schema_version"] = 99.into();
    assert!(matches!(
        Results::from_json(&document.to_string()),
        Err(ResultsError::SchemaVersion(99))
    ));

    document = serde_json::from_str(&valid).unwrap();
    document["meta"]["host"]["cpu_count"] = 0.into();
    assert!(matches!(
        Results::from_json(&document.to_string()),
        Err(ResultsError::InvalidField { ref path, .. }) if path == "meta.host.cpu_count"
    ));

    document = serde_json::from_str(&valid).unwrap();
    document["metrics"]["bad"] = serde_json::json!({
        "value": 1.0,
        "unit": "ms",
        "n": 0,
        "p95": null,
        "stddev": null
    });
    assert!(matches!(
        Results::from_json(&document.to_string()),
        Err(ResultsError::InvalidField { ref path, .. }) if path == "metrics.bad.n"
    ));

    document = serde_json::from_str(&valid).unwrap();
    document["raw"] = serde_json::json!([]);
    assert!(matches!(
        Results::from_json(&document.to_string()),
        Err(ResultsError::Json(_))
    ));
}

#[test]
fn result_validation_names_every_typed_field_invariant() {
    let valid = merge(meta(), Vec::new(), 0.5).unwrap();

    for field in [
        "generated_at",
        "skit_version",
        "python",
        "uv",
        "textual",
        "pyperf",
    ] {
        let mut candidate = valid.clone();
        match field {
            "generated_at" => candidate.meta.generated_at.clear(),
            "skit_version" => candidate.meta.skit_version.clear(),
            "python" => candidate.meta.python.clear(),
            "uv" => candidate.meta.uv.clear(),
            "textual" => candidate.meta.textual.clear(),
            "pyperf" => candidate.meta.pyperf.clear(),
            _ => unreachable!(),
        }
        assert!(candidate.to_json().is_err(), "accepted empty meta.{field}");
    }
    for field in ["commit", "os", "kernel", "cpu", "platform_key"] {
        let mut candidate = valid.clone();
        match field {
            "commit" => candidate.meta.git.commit.clear(),
            "os" => candidate.meta.host.os.clear(),
            "kernel" => candidate.meta.host.kernel.clear(),
            "cpu" => candidate.meta.host.cpu.clear(),
            "platform_key" => candidate.meta.host.platform_key.clear(),
            _ => unreachable!(),
        }
        assert!(
            candidate.to_json().is_err(),
            "accepted empty meta field {field}"
        );
    }
    let mut candidate = valid.clone();
    candidate.meta.git.pr = Some(String::new());
    assert!(candidate.to_json().is_err());

    let metric_cases = [
        ("", 1.0, "ms", 1, None, None),
        ("x", f64::NAN, "ms", 1, None, None),
        ("x", 1.0, "", 1, None, None),
        ("x", 1.0, "ms", 0, None, None),
        ("x", 1.0, "ms", 1, Some(f64::INFINITY), None),
        ("x", 1.0, "ms", 1, None, Some(f64::NEG_INFINITY)),
    ];
    for (id, value, unit, n, p95, stddev) in metric_cases {
        let mut candidate = valid.clone();
        candidate.metrics.insert(
            id.to_owned(),
            Metric {
                value,
                unit: unit.to_owned(),
                n,
                p95,
                stddev,
            },
        );
        assert!(
            candidate.to_json().is_err(),
            "accepted invalid metric {id:?}"
        );
    }

    for (case, reason) in [("", "reason"), ("case", "")] {
        let mut candidate = valid.clone();
        candidate.skipped.push(Skip {
            suite: SuiteKind::Startup,
            case: case.to_owned(),
            reason: reason.to_owned(),
        });
        assert!(candidate.to_json().is_err());
    }

    let skipped = SuiteOutput::skip_all(SuiteKind::Rss, "not available");
    assert_eq!(skipped.skipped[0].case, "all");
    assert!(skipped.to_json().is_ok());
}
