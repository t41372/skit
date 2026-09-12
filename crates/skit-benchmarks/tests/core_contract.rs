use std::collections::{BTreeMap, BTreeSet};

use skit_benchmarks::{
    BenchmarkProfile, BudgetOutcome, GitInfo, HostInfo, Meta, Metric, PipelineError, Results, Skip,
    SuiteKind, SuiteOutput,
    budget::{
        Budget, BudgetReport, BudgetRowResult, BudgetTier, evaluate, format_number, load_budgets,
        propose, render_budgets, render_report,
    },
    compare::{Delta, compare, render_markdown as render_comparison},
    hyperfine::{
        Case, HyperfineError, build_argv, metrics_from_export, parse_export, validate_case_names,
    },
    merge,
    stats::{median, nearest_rank_p95, sample_stddev},
};

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
            pr: None,
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
        textual: "not-applicable".to_owned(),
        pyperf: "rust-harness-v1".to_owned(),
    }
}

fn results(metrics: &[(&str, f64, &str)]) -> Results {
    Results {
        schema_version: 1,
        meta: meta(),
        metrics: metrics
            .iter()
            .map(|(name, value, unit)| ((*name).to_owned(), metric(*value, unit)))
            .collect(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

#[test]
fn statistics_match_latest_main_definitions() {
    assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]).unwrap(), 2.5);
    assert_eq!(nearest_rank_p95(&[1.0, 2.0, 3.0, 4.0]).unwrap(), 4.0);
    assert_eq!(sample_stddev(&[1.0]).unwrap(), 0.0);
    assert!((sample_stddev(&[1.0, 2.0, 3.0]).unwrap() - 1.0).abs() < f64::EPSILON);
    assert!(median(&[]).is_err());
    assert!(median(&[f64::NAN]).is_err());
    assert!(nearest_rank_p95(&[f64::INFINITY]).is_err());
    assert!(sample_stddev(&[]).is_err());
    assert!(sample_stddev(&[f64::NEG_INFINITY]).is_err());
    assert_eq!(format_number(-0.0), "0");
}

#[test]
fn hyperfine_builder_and_parser_keep_real_argv_and_full_samples() {
    let argv = build_argv(
        &[Case::new(
            "startup.version",
            ["/tmp/skit path/skit", "--version"],
        )],
        3,
        15,
        "/tmp/result.json",
        "hyperfine",
    )
    .unwrap();
    assert_eq!(
        &argv[..8],
        [
            "hyperfine",
            "--shell=none",
            "--style",
            "basic",
            "--warmup",
            "3",
            "--min-runs",
            "15",
        ]
    );
    assert!(
        argv.iter()
            .any(|value| value.contains("'/tmp/skit path/skit' --version"))
    );

    let export = r#"{
      "results": [{
        "command": "startup.version",
        "times": [0.001, 0.002, 0.004],
        "exit_codes": [0, 0, 0]
      }]
    }"#;
    let samples = parse_export(export).unwrap();
    assert_eq!(samples["startup.version"], [0.001, 0.002, 0.004]);
    let metrics = metrics_from_export(&samples).unwrap();
    assert_eq!(metrics["startup.version.median_ms"].value, 2.0);
    assert_eq!(metrics["startup.version.median_ms"].p95, Some(4.0));
    assert!(parse_export(r#"{"results":[{"command":"x","times":[1],"exit_codes":[1]}]}"#).is_err());
    assert!(parse_export(r#"{"results":[{"command":"x","times":[-1]}]}"#).is_err());
    assert!(validate_case_names(&samples, &[Case::new("missing", ["true"])]).is_err());
    assert!(
        build_argv(
            &[Case::new("same", ["true"]), Case::new("same", ["false"]),],
            1,
            1,
            "/tmp/result.json",
            "hyperfine",
        )
        .is_err()
    );
}

#[test]
fn hyperfine_refuses_every_ambiguous_case_and_export_shape() {
    let build = |cases: &[Case]| build_argv(cases, 0, 1, "result.json", "hyperfine");
    assert!(matches!(build(&[]), Err(HyperfineError::EmptyCases)));
    assert!(matches!(
        build(&[Case::new("", ["true"])]),
        Err(HyperfineError::EmptyName)
    ));
    assert!(matches!(
        build(&[Case::new("empty", Vec::<String>::new())]),
        Err(HyperfineError::EmptyArgv(name)) if name == "empty"
    ));
    assert!(matches!(
        build(&[
            Case::new("duplicate", ["true"]),
            Case::new("duplicate", ["false"]),
        ]),
        Err(HyperfineError::DuplicateCase(name)) if name == "duplicate"
    ));
    assert!(matches!(
        build(&[Case::new("nul", ["bad\0argument"])]),
        Err(HyperfineError::Quote { case, .. }) if case == "nul"
    ));

    for export in [
        r#"{"results":[]}"#,
        r#"{"results":[{"command":"","times":[1]}]}"#,
        r#"{"results":[{"command":"missing-times","times":[]}]}"#,
    ] {
        assert!(matches!(
            parse_export(export),
            Err(HyperfineError::Shape(_))
        ));
    }
    assert!(matches!(
        parse_export(
            r#"{"results":[{"command":"same","times":[1]},{"command":"same","times":[2]}]}"#
        ),
        Err(HyperfineError::Shape(reason)) if reason.contains("duplicate hyperfine case")
    ));
}

#[test]
fn merge_rejects_duplicate_and_half_present_metric_contracts() {
    let output = |suite, metrics| SuiteOutput {
        suite,
        duration_seconds: 0.0,
        metrics,
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    assert!(matches!(
        merge(
            meta(),
            vec![
                output(
                    SuiteKind::Imports,
                    BTreeMap::from([("same.metric".to_owned(), metric(1.0, "count"))]),
                ),
                output(
                    SuiteKind::Footprint,
                    BTreeMap::from([("same.metric".to_owned(), metric(2.0, "count"))]),
                ),
            ],
            0.0,
        ),
        Err(PipelineError::DuplicateMetric(metric)) if metric == "same.metric"
    ));

    assert!(matches!(
        merge(
            meta(),
            vec![output(
                SuiteKind::Startup,
                BTreeMap::from([("startup.python.median_ms".to_owned(), metric(1.0, "ms"),)]),
            )],
            0.0,
        ),
        Err(PipelineError::HalfPresentDerivation {
            present: "startup.python.median_ms",
            absent: "startup.version.median_ms",
            ..
        })
    ));

    assert!(matches!(
        merge(
            meta(),
            vec![output(
                SuiteKind::Startup,
                BTreeMap::from([
                    ("startup.version.median_ms".to_owned(), metric(3.0, "ms")),
                    ("startup.python.median_ms".to_owned(), metric(1.0, "ms")),
                    ("startup.version.over_python_ms".to_owned(), metric(2.0, "ms")),
                ]),
            )],
            0.0,
        ),
        Err(PipelineError::DuplicateMetric(metric))
            if metric == "startup.version.over_python_ms"
    ));
}

#[test]
fn every_profile_and_suite_keeps_its_stable_artifact_token() {
    assert_eq!(BenchmarkProfile::Compare.as_str(), "compare");
    assert_eq!(SuiteKind::Rss.as_str(), "rss");
    assert_eq!(SuiteKind::Micro.as_str(), "micro");
    assert_eq!(SuiteKind::Syscalls.as_str(), "syscalls");
}

const ENFORCED: &str = r#"
[[budget]]
metric = "imports.version.modules"
max = 400
tier = "enforced"
ratchet = true
headroom = 0.1
context = { python = "3.13", commit = "abc", date = "2026-07-20" }
"#;

#[test]
fn budget_loader_and_evaluator_preserve_every_decay_channel() {
    let budgets = load_budgets(ENFORCED).unwrap();
    assert_eq!(
        evaluate(
            &budgets,
            &results(&[("imports.version.modules", 300.0, "count")])
        )
        .rows[0]
            .outcome,
        BudgetOutcome::Passed
    );
    assert_eq!(
        evaluate(
            &budgets,
            &results(&[("imports.version.modules", 500.0, "count")])
        )
        .rows[0]
            .outcome,
        BudgetOutcome::Violated
    );
    assert_eq!(
        evaluate(&budgets, &results(&[])).rows[0].outcome,
        BudgetOutcome::MetricMissing
    );

    let mut wrong_python = results(&[("imports.version.modules", 300.0, "count")]);
    wrong_python.meta.python = "3.14.1".to_owned();
    assert_eq!(
        evaluate(&budgets, &wrong_python).rows[0].outcome,
        BudgetOutcome::PythonMismatch
    );
    wrong_python.meta.host.ci_runner = None;
    assert_eq!(
        evaluate(&budgets, &wrong_python).rows[0].outcome,
        BudgetOutcome::NotApplicable
    );

    assert!(load_budgets("[[budget]]\nmetric='x'\nmax=1\ntier='enforced'").is_err());
    assert!(load_budgets("[[budget]]\nmetric='x'\nmax=1\ntier='target'\nbogus=1").is_err());
}

#[test]
fn test_loads_the_real_contract_file() {
    let budgets = load_budgets(include_str!("../../../benchmarks/budgets.toml")).unwrap();
    let enforced = budgets
        .iter()
        .filter(|budget| budget.tier == BudgetTier::Enforced)
        .collect::<Vec<_>>();
    assert!(enforced.iter().all(|budget| !budget.context.is_empty()));
    let skip_profiles = enforced
        .iter()
        .filter(|budget| budget.metric == "pipeline.skipped_count")
        .map(|budget| budget.profiles.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        skip_profiles,
        BTreeSet::from([vec!["full".to_owned()], vec!["pr".to_owned()]])
    );
    let wheel = enforced
        .iter()
        .find(|budget| budget.metric == "footprint.wheel_bytes")
        .unwrap();
    assert_eq!(wheel.profiles, ["pr", "full"]);
    let ratchets = enforced
        .iter()
        .filter(|budget| budget.ratchet)
        .collect::<Vec<_>>();
    assert!(!ratchets.is_empty());
    assert!(
        ratchets
            .iter()
            .all(|budget| budget.context.get("python").map(String::as_str) == Some("3.13"))
    );
}

#[test]
fn test_budgets_file_is_canonical() {
    let text = include_str!("../../../benchmarks/budgets.toml");
    assert_eq!(render_budgets(&load_budgets(text).unwrap()).unwrap(), text);
}

#[test]
fn test_budget_bounds_render_as_plain_numbers() {
    let budgets = load_budgets(
        "[[budget]]\nmetric='footprint.wheel_bytes'\nmax=1048576\ntier='enforced'\ncontext={commit='abc'}",
    )
    .unwrap();
    let report = evaluate(
        &budgets,
        &results(&[("footprint.wheel_bytes", 461_803.0, "bytes")]),
    );
    let text = render_report(&report);
    assert!(text.contains("461803 bytes ≤ 1048576"), "{text}");
    assert!(!text.contains("e+06"), "{text}");
}

#[test]
fn test_fractional_bounds_render_compactly() {
    let budgets = load_budgets("[[budget]]\nmetric='ratio'\nmax=0.5\ntier='target'").unwrap();
    let report = evaluate(&budgets, &results(&[("ratio", 0.25, "x")]));
    assert!(render_report(&report).contains("0.25 x ≤ 0.5"));
}

#[test]
fn budget_loader_rejects_every_malformed_field_without_defaulting() {
    for malformed in [
        "[",
        "unknown=1",
        "",
        "budget=[1]",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nunknown=1",
        "[[budget]]\nmax=1\ntier='target'",
        "[[budget]]\nmetric=''\nmax=1\ntier='target'",
        "[[budget]]\nmetric=1\nmax=1\ntier='target'",
        "[[budget]]\nmetric='x'\ntier='target'",
        "[[budget]]\nmetric='x'\nmax='1'\ntier='target'",
        "[[budget]]\nmetric='x'\nmax=nan\ntier='target'",
        "[[budget]]\nmetric='x'\nmax=1\ntier='other'",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nratchet='yes'",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nratchet=true",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nheadroom='small'",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nheadroom=0",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nprofiles='pr'",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nprofiles=['']",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nplatform=''",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nci_only='yes'",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\ncontext=[]",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\ncontext={date=1}",
        "[[budget]]\nmetric='x'\nmax=1\ntier='enforced'",
        "[[budget]]\nmetric='x'\nmax=1\ntier='target'\nnote=1",
    ] {
        assert!(
            load_budgets(malformed).is_err(),
            "malformed contract was accepted: {malformed}"
        );
    }

    let complete = load_budgets(
        "[[budget]]\nmetric='x'\nmax=1.5\ntier='target'\nheadroom=0.2\nprofiles=['pr']\nplatform='linux-x86_64'\nci_only=true\ncontext={date='2026-08-09'}\nnote='goal'",
    )
    .unwrap();
    assert_eq!(complete[0].max_value, 1.5);
    assert_eq!(
        load_budgets(&render_budgets(&complete).unwrap()).unwrap(),
        complete
    );
}

#[test]
fn budget_predicates_and_report_symbols_keep_all_decay_channels_visible() {
    let base = Budget {
        metric: "x".to_owned(),
        max_value: 10.0,
        tier: BudgetTier::Enforced,
        ratchet: false,
        headroom: 0.1,
        profiles: Vec::new(),
        platform: None,
        ci_only: false,
        context: BTreeMap::new(),
        note: String::new(),
    };
    let measured = results(&[("x", 5.0, "ms")]);

    let mut profile = base.clone();
    profile.profiles = vec!["full".to_owned()];
    assert_eq!(
        evaluate(&[profile], &measured).rows[0].outcome,
        BudgetOutcome::NotApplicable
    );

    let mut platform = base.clone();
    platform.platform = Some("darwin-aarch64".to_owned());
    assert_eq!(
        evaluate(&[platform.clone()], &measured).rows[0].outcome,
        BudgetOutcome::NotApplicable
    );
    let mut empty_platform = measured.clone();
    empty_platform.meta.host.platform_key.clear();
    assert_eq!(
        evaluate(&[platform], &empty_platform).rows[0].outcome,
        BudgetOutcome::PredicateUnevaluable
    );

    let mut empty_runner = measured.clone();
    empty_runner.meta.host.ci_runner = Some(String::new());
    assert_eq!(
        evaluate(std::slice::from_ref(&base), &empty_runner).rows[0].outcome,
        BudgetOutcome::PredicateUnevaluable
    );
    let mut ci_only = base.clone();
    ci_only.ci_only = true;
    let mut local = measured.clone();
    local.meta.host.ci_runner = None;
    assert_eq!(
        evaluate(&[ci_only], &local).rows[0].outcome,
        BudgetOutcome::NotApplicable
    );

    let rows = [
        (BudgetOutcome::Passed, BudgetTier::Target, "ok"),
        (BudgetOutcome::Violated, BudgetTier::Target, "△"),
        (BudgetOutcome::MetricMissing, BudgetTier::Enforced, "FAIL"),
        (BudgetOutcome::NotApplicable, BudgetTier::Enforced, "n/a"),
        (
            BudgetOutcome::PredicateUnevaluable,
            BudgetTier::Enforced,
            "FAIL",
        ),
        (BudgetOutcome::PythonMismatch, BudgetTier::Enforced, "FAIL"),
    ]
    .into_iter()
    .map(|(outcome, tier, _)| BudgetRowResult {
        budget: Budget {
            tier,
            ..base.clone()
        },
        outcome,
        value: None,
        detail: String::new(),
        stale: false,
    })
    .collect::<Vec<_>>();
    let rendered = render_report(&BudgetReport { rows });
    for (_, _, symbol) in [
        (BudgetOutcome::Passed, BudgetTier::Target, "ok"),
        (BudgetOutcome::Violated, BudgetTier::Target, "△"),
        (BudgetOutcome::MetricMissing, BudgetTier::Enforced, "FAIL"),
        (BudgetOutcome::NotApplicable, BudgetTier::Enforced, "n/a"),
    ] {
        assert!(rendered.contains(symbol));
    }
}

#[test]
fn propose_is_ci_only_dirty_safe_and_never_widens_without_consent() {
    let budgets = load_budgets(ENFORCED).unwrap();
    let measured = results(&[("imports.version.modules", 291.0, "count")]);
    let proposed = propose(&budgets, &measured, false).unwrap();
    let loaded = load_budgets(&proposed).unwrap();
    assert_eq!(loaded[0].max_value, 321.0);
    assert_eq!(loaded[0].context["commit"], "abcdef1234567890");

    let local = Results {
        meta: Meta {
            host: HostInfo {
                ci_runner: None,
                ..measured.meta.host.clone()
            },
            ..measured.meta.clone()
        },
        ..measured.clone()
    };
    let local_error = propose(&budgets, &local, false).unwrap_err().to_string();
    assert!(local_error.contains("platform- and python-dependent"));
    assert!(local_error.contains("benchmark-results-"));

    let mut dirty = measured.clone();
    dirty.meta.git.dirty = true;
    let dirty_error = propose(&budgets, &dirty, false).unwrap_err().to_string();
    assert!(dirty_error.contains("commit that does not describe what was measured"));

    let regressed = results(&[("imports.version.modules", 500.0, "count")]);
    let regression_error = propose(&budgets, &regressed, false)
        .unwrap_err()
        .to_string();
    assert!(regression_error.contains("measured 500"));
    assert!(regression_error.contains("--allow-regression"));
    assert!(regression_error.contains("say why in the row's note"));
    assert!(propose(&budgets, &regressed, true).is_ok());

    let rendered = render_budgets(&budgets).unwrap();
    assert!(rendered.contains("CI artifacts only"));
    assert!(rendered.contains("context.pr / context.commit"));
    assert!(rendered.contains("skit-bench check --propose"));
    assert!(!rendered.contains("uv run python"));
    assert_eq!(load_budgets(&rendered).unwrap(), budgets);

    let mut advisory = budgets[0].clone();
    advisory.ratchet = false;
    advisory.metric = "absent.advisory".to_owned();
    let preserved = load_budgets(&propose(&[advisory.clone()], &measured, false).unwrap()).unwrap();
    assert_eq!(preserved, [advisory]);

    let mut absent_ratchet = budgets[0].clone();
    absent_ratchet.metric = "absent.ratchet".to_owned();
    assert!(
        propose(&[absent_ratchet], &measured, false)
            .unwrap_err()
            .to_string()
            .contains("metric absent")
    );

    let mut pull_request = measured.clone();
    pull_request.meta.git.pr = Some("73".to_owned());
    let pull_request_proposal =
        load_budgets(&propose(&budgets, &pull_request, false).unwrap()).unwrap();
    assert_eq!(pull_request_proposal[0].context["pr"], "73");
    assert!(!pull_request_proposal[0].context.contains_key("commit"));
    assert_eq!(format_number(1.25), "1.25");
}

#[test]
fn budget_report_keeps_advisory_and_stale_context_actionable() {
    let budgets = load_budgets(&format!(
        "{ENFORCED}\n[[budget]]\nmetric='startup.version.median_ms'\nmax=10\ntier='target'"
    ))
    .unwrap();
    let report = evaluate(
        &budgets,
        &results(&[
            ("imports.version.modules", 300.0, "count"),
            ("startup.version.median_ms", 12.0, "ms"),
        ]),
    );
    let rendered = render_report(&report);
    assert!(rendered.contains("[target]    △  startup.version.median_ms"));
    assert!(rendered.contains("measured 300 sits below 85% of the bound 400"));
    assert!(rendered.contains("0 failed · target: 1 rows"));
}

#[test]
fn comparison_excludes_harness_metrics_and_flags_exact_changes_and_provenance() {
    let base = results(&[
        ("imports.version.modules", 100.0, "count"),
        ("startup.version.median_ms", 100.0, "ms"),
        ("pipeline.duration_s", 20.0, "s"),
    ]);
    let mut head = results(&[
        ("imports.version.modules", 101.0, "count"),
        ("startup.version.median_ms", 101.0, "ms"),
        ("pipeline.duration_s", 50.0, "s"),
    ]);
    head.meta.profile = BenchmarkProfile::Full;
    let comparison = compare(&base, &head);
    assert_eq!(comparison.deltas.len(), 2);
    assert_eq!(comparison.notable().len(), 1);
    assert_eq!(comparison.notable()[0].metric, "imports.version.modules");
    assert!(comparison.incomparable[0].contains("profile"));
    let markdown = render_comparison(&base, &head, &comparison);
    assert!(markdown.contains("not directly comparable"));
    assert!(markdown.contains("| Metric | Base | Head | Δ | Δ% |"));
    assert!(
        markdown.contains("| `imports.version.modules` | 100 count | 101 count | +1 | +1.0% |")
    );
    assert!(markdown.contains("|Δ| > max(5%, per-unit floor: 2 ms macro / 1 µs micro)"));
    assert!(markdown.contains("Deltas below mix apples and oranges."));
    assert!(!markdown.contains("pipeline.duration_s"));
}

#[test]
fn comparison_keeps_missing_unit_zero_floor_and_skip_evidence() {
    let mut base = results(&[
        ("only.base", 1.0, "count"),
        ("unit.changed", 1.0, "ms"),
        ("zero.base", 0.0, "ms"),
        ("micro.delta", 10.0, "us"),
    ]);
    let mut head = results(&[
        ("only.head", 2.0, "count"),
        ("unit.changed", 1.0, "s"),
        ("zero.base", 3.0, "ms"),
        ("micro.delta", 12.0, "us"),
    ]);
    head.meta.host.ci_image_version = None;
    base.skipped.push(Skip {
        suite: SuiteKind::Imports,
        case: "base-case".to_owned(),
        reason: "base reason".to_owned(),
    });
    head.skipped.push(Skip {
        suite: SuiteKind::Tui,
        case: "head-case".to_owned(),
        reason: "head reason".to_owned(),
    });

    let comparison = compare(&base, &head);
    assert_eq!(comparison.only_base, ["only.base"]);
    assert_eq!(comparison.only_head, ["only.head"]);
    assert!(
        comparison
            .incomparable
            .iter()
            .any(|row| row == "unit unit.changed: ms vs s")
    );
    assert!(
        comparison
            .incomparable
            .iter()
            .any(|row| row == "runner image: 20260808.1 vs None")
    );
    assert!(
        comparison
            .notable()
            .iter()
            .any(|row| row.metric == "zero.base")
    );
    assert!(
        comparison
            .notable()
            .iter()
            .any(|row| row.metric == "micro.delta")
    );

    let markdown = render_comparison(&base, &head, &comparison);
    for evidence in [
        "### Only in base",
        "### Only in head",
        "### Skipped in base (1)",
        "### Skipped in head (1)",
        "`imports/base-case`: base reason",
        "`tui/head-case`: head reason",
    ] {
        assert!(markdown.contains(evidence), "missing {evidence:?}");
    }

    assert!(
        Delta {
            metric: "unknown".to_owned(),
            unit: "widgets".to_owned(),
            base: 10.0,
            head: 11.0,
        }
        .is_notable()
    );
    assert!(
        !Delta {
            metric: "zero".to_owned(),
            unit: "ms".to_owned(),
            base: 0.0,
            head: 0.0,
        }
        .is_notable()
    );
}

#[test]
fn old_schema_defaults_missing_python_harness_versions() {
    let mut document: serde_json::Value =
        serde_json::from_str(&results(&[]).to_json().unwrap()).unwrap();
    document["meta"].as_object_mut().unwrap().remove("pyperf");
    document["meta"]["host"]
        .as_object_mut()
        .unwrap()
        .remove("ci_image_version");
    let restored = Results::from_json(&document.to_string()).unwrap();
    assert_eq!(restored.meta.pyperf, "unknown");
    assert_eq!(restored.meta.host.ci_image_version, None);
    assert_eq!(SuiteKind::RunOverhead.as_str(), "run_overhead");
}

#[test]
fn checked_in_budget_contract_is_strictly_valid_and_keeps_latest_main_rows() {
    let budgets =
        load_budgets(include_str!("../../../benchmarks/budgets.toml")).expect("valid budgets");
    let ids = budgets
        .iter()
        .map(|budget| budget.metric.as_str())
        .collect::<Vec<_>>();
    for required in [
        "footprint.wheel_bytes",
        "imports.version.modules",
        "imports.list_json.n100.has_tree_sitter",
        "pipeline.skipped_count",
        "startup.version.over_python_ms",
        "scale.list_json.per_entry_us",
        "tui.first_idle.n1000.median_ms",
        "rss.list_json.n1000.peak_kib",
        "footprint.closure_bytes",
        "syscalls.list_json.network",
    ] {
        assert!(ids.contains(&required), "missing budget row {required}");
    }
}
