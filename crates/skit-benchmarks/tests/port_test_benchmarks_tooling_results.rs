//! Frozen result-schema contracts from `tests/test_benchmarks_tooling.py`.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use skit_benchmarks::{
    BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, Results, Skip, SuiteKind, SuiteOutput,
    budget::python_major_minor,
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

fn results(metrics: BTreeMap<String, Metric>) -> Results {
    Results {
        schema_version: 1,
        meta: meta(),
        metrics,
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

fn metric() -> Metric {
    Metric {
        value: 1.0,
        unit: "ms".to_owned(),
        n: 1,
        p95: None,
        stddev: None,
    }
}

fn results_doc() -> Value {
    serde_json::from_str(&results(BTreeMap::new()).to_json().unwrap()).unwrap()
}

fn set_path(mut doc: Value, path: &[&str], value: Value) -> String {
    let mut node = &mut doc;
    for key in &path[..path.len() - 1] {
        node = node
            .as_object_mut()
            .and_then(|object| object.get_mut(*key))
            .unwrap_or_else(|| panic!("fixture path {path:?} is not an object path"));
    }
    node.as_object_mut()
        .expect("fixture parent is an object")
        .insert(path[path.len() - 1].to_owned(), value);
    serde_json::to_string(&doc).unwrap()
}

fn error_for(text: &str) -> String {
    Results::from_json(text).unwrap_err().to_string()
}

#[test]
fn test_round_trip() {
    let mut metrics = BTreeMap::new();
    metrics.insert(
        "a.b".to_owned(),
        Metric {
            value: 1.5,
            unit: "ms".to_owned(),
            n: 3,
            p95: Some(2.0),
            stddev: Some(0.1),
        },
    );
    let mut value = results(metrics);
    value.skipped.push(Skip {
        suite: SuiteKind::Startup,
        case: "c".to_owned(),
        reason: "r".to_owned(),
    });
    assert_eq!(Results::from_json(&value.to_json().unwrap()).unwrap(), value);
}

#[test]
fn test_rejects_wrong_schema_version() {
    let text = set_path(results_doc(), &["schema_version"], json!(99));
    assert!(error_for(&text).contains("schema_version"));
}

#[test]
fn test_rejects_non_json() {
    let error = error_for("{nope");
    assert!(
        error.contains("not valid JSON"),
        "frozen diagnostic lost `not valid JSON`: {error}"
    );
}

#[test]
fn test_rejects_non_object() {
    let error = error_for("[1]");
    assert!(
        error.contains("expected a JSON object"),
        "frozen diagnostic lost `expected a JSON object`: {error}"
    );
}

#[test]
fn test_rejects_bad_metric_fields() {
    let cases = [
        ("value", json!("fast")),
        ("value", json!(true)),
        ("unit", json!("")),
        ("n", json!(0)),
        ("n", json!(1.5)),
        ("p95", json!("high")),
        ("stddev", json!("low")),
    ];
    for (field, bad) in cases {
        let mut doc = results_doc();
        doc["metrics"] = json!({"m.x": metric()});
        doc["metrics"]["m.x"][field] = bad;
        let error = error_for(&serde_json::to_string(&doc).unwrap());
        assert!(
            error.contains("m.x") && error.contains(field),
            "bad metric field {field:?} lost its frozen path diagnostic: {error}"
        );
    }
}

#[test]
fn test_rejects_bad_skip_entry() {
    let mut doc = results_doc();
    doc["skipped"] = json!([{"suite": "startup", "case": "", "reason": "r"}]);
    let error = error_for(&serde_json::to_string(&doc).unwrap());
    assert!(error.contains("case"), "bad skip diagnostic lost `case`: {error}");
}

#[test]
fn test_rejects_empty_meta_strings() {
    let text = set_path(results_doc(), &["meta", "host", "platform_key"], json!(""));
    let error = error_for(&text);
    assert!(
        error.contains("platform_key"),
        "empty meta diagnostic lost `platform_key`: {error}"
    );
}

#[test]
fn test_ci_runner_null_is_valid() {
    let text = set_path(results_doc(), &["meta", "host", "ci_runner"], Value::Null);
    assert_eq!(Results::from_json(&text).unwrap().meta.host.ci_runner, None);
}

#[test]
fn test_meta_from_dict_matches_round_trip() {
    let expected = meta();
    let value = results(BTreeMap::new());
    let text = value.to_json().unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    let meta_doc = doc.get("meta").expect("serialized results contain meta");
    let reparsed: Meta = serde_json::from_value(meta_doc.clone()).unwrap();
    assert_eq!(reparsed, expected);
    assert_eq!(Results::from_json(&text).unwrap().meta, expected);
}

#[test]
fn test_python_major_minor() {
    assert_eq!(python_major_minor("3.13.7"), "3.13");
    assert_eq!(python_major_minor("3.14"), "3.14");
}

#[test]
fn test_suite_output_round_trip() {
    let output = SuiteOutput {
        suite: SuiteKind::Startup,
        duration_seconds: 1.25,
        metrics: BTreeMap::from([(
            "startup.version.median_ms".to_owned(),
            Metric {
                value: 200.0,
                unit: "ms".to_owned(),
                n: 15,
                p95: None,
                stddev: None,
            },
        )]),
        skipped: vec![Skip {
            suite: SuiteKind::Startup,
            case: "x".to_owned(),
            reason: "y".to_owned(),
        }],
        raw: BTreeMap::from([("times".to_owned(), json!([1, 2]))]),
    };
    assert_eq!(
        SuiteOutput::from_json(&output.to_json().unwrap()).unwrap(),
        output
    );
}

#[test]
fn test_suite_output_rejects_bad_duration() {
    let output = SuiteOutput {
        suite: SuiteKind::Startup,
        duration_seconds: 0.0,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    let mut doc: Value = serde_json::from_str(&output.to_json().unwrap()).unwrap();
    doc["duration_s"] = json!("long");
    let error = SuiteOutput::from_json(&serde_json::to_string(&doc).unwrap())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("duration_s"),
        "bad duration diagnostic lost `duration_s`: {error}"
    );
}

#[test]
fn test_suite_output_rejects_non_object_raw() {
    let output = SuiteOutput {
        suite: SuiteKind::Startup,
        duration_seconds: 0.0,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    let mut doc: Value = serde_json::from_str(&output.to_json().unwrap()).unwrap();
    doc["raw"] = json!([]);
    let error = SuiteOutput::from_json(&serde_json::to_string(&doc).unwrap())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("raw: expected an object"),
        "frozen raw diagnostic changed: {error}"
    );
}

#[test]
fn test_rejects_bad_results_structure() {
    let cases = [
        (vec!["metrics"], json!([1]), "metrics: expected an object"),
        (vec!["metrics"], json!({"m.x": 5}), "metrics.m.x: expected an object"),
        (vec!["skipped"], json!({}), "skipped: expected an array"),
        (vec!["skipped"], json!([5]), "skipped[0]: expected an object"),
        (vec!["raw"], json!([]), "raw: expected an object"),
        (vec!["meta"], Value::Null, "meta: expected an object"),
        (vec!["meta", "git"], json!(5), "meta.git: expected an object"),
        (
            vec!["meta", "git", "dirty"],
            json!("yes"),
            "meta.git.dirty: expected a boolean",
        ),
        (
            vec!["meta", "git", "pr"],
            json!(""),
            "meta.git.pr: expected a non-empty string or null",
        ),
        (
            vec!["meta", "git", "pr"],
            json!(29),
            "meta.git.pr: expected a non-empty string or null",
        ),
        (vec!["meta", "host"], json!(5), "meta.host: expected an object"),
        (
            vec!["meta", "host", "cpu_count"],
            json!(0),
            "meta.host.cpu_count: expected a positive integer",
        ),
        (
            vec!["meta", "host", "cpu_count"],
            json!(true),
            "meta.host.cpu_count: expected a positive integer",
        ),
        (
            vec!["meta", "host", "cpu_count"],
            json!("8"),
            "meta.host.cpu_count: expected a positive integer",
        ),
        (
            vec!["meta", "host", "mem_total_mib"],
            json!(-1),
            "meta.host.mem_total_mib: expected a non-negative integer",
        ),
        (
            vec!["meta", "host", "mem_total_mib"],
            json!(true),
            "meta.host.mem_total_mib: expected a non-negative integer",
        ),
        (
            vec!["meta", "host", "mem_total_mib"],
            json!("x"),
            "meta.host.mem_total_mib: expected a non-negative integer",
        ),
        (
            vec!["meta", "host", "ci_runner"],
            json!(5),
            "meta.host.ci_runner: expected a string or null",
        ),
    ];

    for (path, value, fragment) in cases {
        let text = set_path(results_doc(), &path, value);
        let error = error_for(&text);
        assert!(
            error.contains(fragment),
            "bad structure at {path:?} lost frozen diagnostic {fragment:?}: {error}"
        );
    }
}
