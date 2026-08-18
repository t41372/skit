//! Frozen benchmark budget contracts from `tests/test_benchmarks_tooling.py`.

use std::{collections::BTreeMap, fs, path::Path};

use skit_benchmarks::{
    BenchmarkProfile, BudgetOutcome, GitInfo, HostInfo, Meta, Metric, Results,
    budget::{BudgetTier, evaluate, load_budgets, propose, render_budgets, render_report},
};

const ENFORCED_ROW: &str = r#"
[[budget]]
metric = "imports.version.modules"
max = 400
tier = "enforced"
ratchet = true
context = { python = "3.13", commit = "abc", date = "2026-07-20" }
"#;

fn make_meta() -> Meta {
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

fn make_results(metrics: &[(&str, f64, &str)]) -> Results {
    Results {
        schema_version: 1,
        meta: make_meta(),
        metrics: metrics
            .iter()
            .map(|(name, value, unit)| {
                (
                    (*name).to_owned(),
                    Metric {
                        value: *value,
                        unit: (*unit).to_owned(),
                        n: 1,
                        p95: None,
                        stddev: None,
                    },
                )
            })
            .collect(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

fn real_budget_text() -> String {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-benchmarks lives under <repo>/crates/skit-benchmarks");
    fs::read_to_string(repo.join("benchmarks/budgets.toml")).expect("read real benchmark contract")
}

#[test]
fn test_loads_the_real_contract_file() {
    let budgets = load_budgets(&real_budget_text()).unwrap();
    let enforced = budgets
        .iter()
        .filter(|budget| budget.tier == BudgetTier::Enforced)
        .collect::<Vec<_>>();
    assert!(enforced.iter().all(|budget| !budget.context.is_empty()));

    let mut skip_rows = enforced
        .iter()
        .filter(|budget| budget.metric == "pipeline.skipped_count")
        .map(|budget| budget.profiles.clone())
        .collect::<Vec<_>>();
    skip_rows.sort();
    assert_eq!(
        skip_rows,
        vec![vec!["full".to_owned()], vec!["pr".to_owned()]]
    );

    let wheel = enforced
        .iter()
        .filter(|budget| budget.metric == "footprint.wheel_bytes")
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(wheel.len(), 1, "wheel budget must be unique");
    assert_eq!(
        wheel[0].profiles,
        vec!["pr".to_owned(), "full".to_owned()]
    );

    let ratchets = enforced
        .iter()
        .filter(|budget| budget.ratchet)
        .copied()
        .collect::<Vec<_>>();
    assert!(!ratchets.is_empty(), "the import ratchets must exist");
    assert!(
        ratchets
            .iter()
            .all(|budget| budget.context.get("python").map(String::as_str) == Some("3.13"))
    );
}

#[test]
fn test_rejects_malformed_rows() {
    let cases = vec![
        (String::new(), "no [[budget]] rows"),
        ("x = 1".to_owned(), "unknown top-level"),
        ("[[budget]]\nmetric = 1\nmax = 1\ntier = 'target'".to_owned(), "metric"),
        ("[[budget]]\nmetric = 'm'\nmax = 'big'\ntier = 'target'".to_owned(), "max"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'hard'".to_owned(), "tier"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nratchet = true".to_owned(), "ratchet"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'enforced'".to_owned(), "context"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nbogus = 1".to_owned(), "unknown keys"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nprofiles = ['']".to_owned(), "profiles"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nprofiles = 'pr'".to_owned(), "profiles"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nplatform = ''".to_owned(), "platform"),
        ("[[budget]]\nmetric = 'm'\nmax = inf\ntier = 'target'".to_owned(), "max must be finite"),
        (format!("{ENFORCED_ROW}headroom = 2.0"), "headroom"),
        ("not toml [".to_owned(), "not valid TOML"),
        ("budget = [1]".to_owned(), "expected a table"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nratchet = 1".to_owned(), "ratchet must be a boolean"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nheadroom = 'big'".to_owned(), "headroom must be a number"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nci_only = 1".to_owned(), "ci_only must be a boolean"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\ncontext = { python = 1 }".to_owned(), "context must be a table of strings"),
        ("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'\nnote = 5".to_owned(), "note must be a string"),
    ];
    for (text, fragment) in cases {
        let error = load_budgets(&text).unwrap_err().to_string();
        assert!(
            error.contains(fragment),
            "malformed budget lost frozen diagnostic {fragment:?}: {error}\ninput:\n{text}"
        );
    }
}

#[test]
fn test_pass_and_violation() {
    let budgets = load_budgets(ENFORCED_ROW).unwrap();
    let ok = evaluate(
        &budgets,
        &make_results(&[("imports.version.modules", 300.0, "count")]),
    );
    assert_eq!(ok.rows[0].outcome, BudgetOutcome::Passed);
    assert!(ok.failures().is_empty());

    let bad = evaluate(
        &budgets,
        &make_results(&[("imports.version.modules", 500.0, "count")]),
    );
    assert_eq!(bad.rows[0].outcome, BudgetOutcome::Violated);
    assert!(!bad.failures().is_empty());
}

#[test]
fn test_missing_metric_fails_enforced() {
    let report = evaluate(&load_budgets(ENFORCED_ROW).unwrap(), &make_results(&[]));
    assert_eq!(report.rows[0].outcome, BudgetOutcome::MetricMissing);
    assert!(!report.failures().is_empty());
}

#[test]
fn test_missing_metric_reported_not_failed_for_target() {
    let budgets = load_budgets("[[budget]]\nmetric = 'm'\nmax = 1\ntier = 'target'").unwrap();
    let report = evaluate(&budgets, &make_results(&[]));
    assert_eq!(report.rows[0].outcome, BudgetOutcome::MetricMissing);
    assert!(report.failures().is_empty());
}

#[test]
fn test_profile_predicate_scopes_row() {
    let budgets = load_budgets(&format!("{ENFORCED_ROW}profiles = ['full']")).unwrap();
    let report = evaluate(
        &budgets,
        &make_results(&[("imports.version.modules", 300.0, "count")]),
    );
    assert_eq!(report.rows[0].outcome, BudgetOutcome::NotApplicable);
    assert_eq!(report.enforced_evaluated(), 0);
}

#[test]
fn test_platform_predicate() {
    let budgets = load_budgets(&format!("{ENFORCED_ROW}platform = 'linux-x86_64'")).unwrap();
    let measured = make_results(&[("imports.version.modules", 300.0, "count")]);
    assert_eq!(evaluate(&budgets, &measured).rows[0].outcome, BudgetOutcome::Passed);

    let mut other = measured;
    other.meta.host.platform_key = "darwin-aarch64".to_owned();
    assert_eq!(
        evaluate(&budgets, &other).rows[0].outcome,
        BudgetOutcome::NotApplicable
    );
}

#[test]
fn test_empty_platform_key_is_unevaluable() {
    let budgets = load_budgets(&format!("{ENFORCED_ROW}platform = 'linux-x86_64'")).unwrap();
    let mut measured = make_results(&[("imports.version.modules", 300.0, "count")]);
    measured.meta.host.platform_key.clear();
    let report = evaluate(&budgets, &measured);
    assert_eq!(
        report.rows[0].outcome,
        BudgetOutcome::PredicateUnevaluable
    );
    assert!(!report.failures().is_empty());
}

#[test]
fn test_empty_ci_runner_is_unevaluable() {
    let budgets = load_budgets(ENFORCED_ROW).unwrap();
    let mut measured = make_results(&[("imports.version.modules", 300.0, "count")]);
    measured.meta.host.ci_runner = Some(String::new());
    assert_eq!(
        evaluate(&budgets, &measured).rows[0].outcome,
        BudgetOutcome::PredicateUnevaluable
    );
}

#[test]
fn test_ci_only_row_not_applicable_locally() {
    let budgets = load_budgets(&format!("{ENFORCED_ROW}ci_only = true")).unwrap();
    let mut measured = make_results(&[("imports.version.modules", 300.0, "count")]);
    measured.meta.host.ci_runner = None;
    measured.meta.host.ci_image_version = None;
    let report = evaluate(&budgets, &measured);
    assert_eq!(report.rows[0].outcome, BudgetOutcome::NotApplicable);
    assert!(report.failures().is_empty());
}

#[test]
fn test_python_mismatch_fails_on_ci_only() {
    let budgets = load_budgets(ENFORCED_ROW).unwrap();
    let mut on_ci = make_results(&[("imports.version.modules", 300.0, "count")]);
    on_ci.meta.python = "3.14.2".to_owned();
    let report = evaluate(&budgets, &on_ci);
    assert_eq!(report.rows[0].outcome, BudgetOutcome::PythonMismatch);
    assert!(!report.failures().is_empty());

    on_ci.meta.host.ci_runner = None;
    on_ci.meta.host.ci_image_version = None;
    let row = &evaluate(&budgets, &on_ci).rows[0];
    assert_eq!(row.outcome, BudgetOutcome::NotApplicable);
    assert!(row.detail.contains("3.14"));
}

#[test]
fn test_stale_ceiling_warns_on_ratchet_rows_only() {
    let ratchet = evaluate(
        &load_budgets(ENFORCED_ROW).unwrap(),
        &make_results(&[("imports.version.modules", 100.0, "count")]),
    );
    assert!(ratchet.rows[0].stale);

    let hand_set = load_budgets(
        "[[budget]]\nmetric = 'footprint.wheel_bytes'\nmax = 1048576\ntier = 'enforced'\ncontext = { commit = 'abc' }",
    )
    .unwrap();
    let report = evaluate(
        &hand_set,
        &make_results(&[("footprint.wheel_bytes", 400_000.0, "bytes")]),
    );
    assert!(!report.rows[0].stale);
}

#[test]
fn test_render_report_tally_and_stale_nudge() {
    let report = evaluate(
        &load_budgets(ENFORCED_ROW).unwrap(),
        &make_results(&[("imports.version.modules", 100.0, "count")]),
    );
    let text = render_report(&report);
    assert!(text.contains("enforced: 1 rows, 1 evaluated, 1 passed, 0 failed"));
    assert!(text.contains("ceiling is stale"));
}

#[test]
fn test_enforced_evaluated_counts_verdicts_not_na() {
    let mut budgets = load_budgets(&format!("{ENFORCED_ROW}profiles = ['full']")).unwrap();
    budgets.extend(load_budgets(ENFORCED_ROW).unwrap());
    let report = evaluate(
        &budgets,
        &make_results(&[("imports.version.modules", 300.0, "count")]),
    );
    assert_eq!(report.enforced_evaluated(), 1);
}

#[test]
fn test_refreshes_ratchet_rows_only() {
    let text = format!(
        "{ENFORCED_ROW}\n[[budget]]\nmetric = 'footprint.wheel_bytes'\nmax = 1048576\ntier = 'enforced'\ncontext = {{ commit = 'old' }}\n\n[[budget]]\nmetric = 'startup.version.over_python_ms'\nmax = 75\ntier = 'target'\n"
    );
    let measured = make_results(&[
        ("imports.version.modules", 291.0, "count"),
        ("footprint.wheel_bytes", 462_000.0, "bytes"),
    ]);
    let refreshed = load_budgets(&propose(&load_budgets(&text).unwrap(), &measured, false).unwrap())
        .unwrap();
    let by_metric = refreshed
        .iter()
        .map(|budget| (budget.metric.as_str(), budget))
        .collect::<BTreeMap<_, _>>();
    let ratchet = by_metric["imports.version.modules"];
    assert_eq!(ratchet.max_value, 321.0);
    assert_eq!(
        ratchet.context,
        BTreeMap::from([
            ("python".to_owned(), "3.13".to_owned()),
            ("commit".to_owned(), "abcdef1234567890".to_owned()),
            ("date".to_owned(), "2026-07-20".to_owned()),
        ])
    );
    assert_eq!(by_metric["footprint.wheel_bytes"].max_value, 1_048_576.0);
    assert_eq!(
        by_metric["footprint.wheel_bytes"].context,
        BTreeMap::from([("commit".to_owned(), "old".to_owned())])
    );
    assert_eq!(by_metric["startup.version.over_python_ms"].max_value, 75.0);
}

#[test]
fn test_propose_anchors_a_pr_artifact_on_the_pr_number() {
    let budgets = load_budgets(ENFORCED_ROW).unwrap();
    let mut measured = make_results(&[("imports.version.modules", 291.0, "count")]);
    measured.meta.git.pr = Some("29".to_owned());
    let proposed = load_budgets(&propose(&budgets, &measured, false).unwrap()).unwrap();
    assert_eq!(proposed.len(), 1);
    assert_eq!(
        proposed[0].context,
        BTreeMap::from([
            ("python".to_owned(), "3.13".to_owned()),
            ("pr".to_owned(), "29".to_owned()),
            ("date".to_owned(), "2026-07-20".to_owned()),
        ])
    );
    assert!(!proposed[0].context.contains_key("commit"));
}

#[test]
fn test_propose_requires_the_metric() {
    let error = propose(&load_budgets(ENFORCED_ROW).unwrap(), &make_results(&[]), false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot propose"));
}

#[test]
fn test_propose_refuses_a_local_artifact() {
    let mut measured = make_results(&[("imports.version.modules", 291.0, "count")]);
    measured.meta.host.ci_runner = None;
    measured.meta.host.ci_image_version = None;
    let error = propose(&load_budgets(ENFORCED_ROW).unwrap(), &measured, false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot propose from a local run"));
}

#[test]
fn test_propose_refuses_a_dirty_tree() {
    let mut measured = make_results(&[("imports.version.modules", 291.0, "count")]);
    measured.meta.git.commit = "abc".to_owned();
    measured.meta.git.dirty = true;
    let error = propose(&load_budgets(ENFORCED_ROW).unwrap(), &measured, false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot propose from a dirty tree"));
}

#[test]
fn test_propose_refuses_to_widen_a_bound() {
    let measured = make_results(&[("imports.version.modules", 400.0, "count")]);
    let error = propose(&load_budgets(ENFORCED_ROW).unwrap(), &measured, false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("refusing to loosen enforced bounds"));
    assert!(error.contains("imports.version.modules: 400 -> 441"));
    assert!(error.contains("--allow-regression"));
}

#[test]
fn test_propose_widens_when_the_increase_is_declared() {
    let measured = make_results(&[("imports.version.modules", 400.0, "count")]);
    let proposed = load_budgets(
        &propose(&load_budgets(ENFORCED_ROW).unwrap(), &measured, true).unwrap(),
    )
    .unwrap();
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0].max_value, 441.0);
}

#[test]
fn test_propose_keeps_a_hand_set_headroom_on_a_non_ratchet_row() {
    let text = format!(
        "{ENFORCED_ROW}\n[[budget]]\nmetric = 'startup.version.over_python_ms'\nmax = 75\ntier = 'target'\nheadroom = 0.25\n"
    );
    let measured = make_results(&[("imports.version.modules", 291.0, "count")]);
    let proposed = load_budgets(
        &propose(&load_budgets(&text).unwrap(), &measured, false).unwrap(),
    )
    .unwrap();
    let row = proposed
        .iter()
        .find(|budget| budget.metric == "startup.version.over_python_ms")
        .expect("target row survives propose");
    assert_eq!(row.headroom, 0.25);
}

#[test]
fn test_render_budgets_round_trips() {
    let budgets = load_budgets(&real_budget_text()).unwrap();
    let rendered = render_budgets(&budgets).unwrap();
    assert_eq!(load_budgets(&rendered).unwrap(), budgets);
}
