use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::{
    bundle::{
        CoverageSummary, EffectCoverage, coverage_summary_bytes, decode_coverage_summary,
        validate_coverage_summary,
    },
    canonical_json_bytes,
};

const STATICALLY_UNSERVED: [&str; 8] = [
    "none",
    "preferences.close",
    "preferences.confirm_discard",
    "preferences.none",
    "quit",
    "submit.add",
    "submit.preferences",
    "submit.runners",
];

fn names() -> Vec<String> {
    STATICALLY_UNSERVED.map(str::to_owned).to_vec()
}

fn canonical(value: &Value) -> Vec<u8> {
    canonical_json_bytes(value).unwrap()
}

#[test]
fn unmeasured_coverage_has_exact_canonical_bytes_and_round_trips() {
    let summary = CoverageSummary::unmeasured_effects(names()).unwrap();
    let cloned = summary.clone();
    let bytes = coverage_summary_bytes(&summary).unwrap();

    assert_eq!(cloned, summary);
    assert!(format!("{cloned:?}").contains("CoverageSummary"));
    assert_eq!(
        bytes,
        br#"{"effects":{"measured":false,"nested_occurrences":null,"served_requests":null,"statically_unserved":["none","preferences.close","preferences.confirm_discard","preferences.none","quit","submit.add","submit.preferences","submit.runners"]},"schema":1}"#,
    );
    assert_eq!(decode_coverage_summary(&bytes).unwrap(), summary);
}

#[test]
fn coverage_decoder_rejects_shape_schema_and_noncanonical_bytes() {
    let valid = json!({
        "schema": 1,
        "effects": {
            "measured": false,
            "served_requests": null,
            "nested_occurrences": null,
            "statically_unserved": names(),
        },
    });
    let cases = [
        (json!([]), "not an object"),
        (
            {
                let mut value = valid.clone();
                value.as_object_mut().unwrap().remove("schema");
                value
            },
            "missing",
        ),
        (
            {
                let mut value = valid.clone();
                value.as_object_mut().unwrap().remove("effects");
                value
            },
            "missing",
        ),
        (
            {
                let mut value = valid.clone();
                value["effects"] = json!([]);
                value
            },
            "not an object",
        ),
        (
            {
                let mut value = valid.clone();
                value["schema"] = json!(2);
                value
            },
            "unsupported schema",
        ),
        (
            {
                let mut value = valid.clone();
                value["schema"] = json!("1");
                value
            },
            "invalid type",
        ),
        (
            {
                let mut value = valid.clone();
                value["extra"] = json!(true);
                value
            },
            "unknown field",
        ),
        (
            {
                let mut value = valid.clone();
                value["effects"].as_object_mut().unwrap().remove("measured");
                value
            },
            "missing",
        ),
        (
            {
                let mut value = valid.clone();
                value["effects"]
                    .as_object_mut()
                    .unwrap()
                    .remove("served_requests");
                value
            },
            "missing",
        ),
        (
            {
                let mut value = valid.clone();
                value["effects"]
                    .as_object_mut()
                    .unwrap()
                    .remove("nested_occurrences");
                value
            },
            "missing",
        ),
        (
            {
                let mut value = valid.clone();
                value["effects"]
                    .as_object_mut()
                    .unwrap()
                    .remove("statically_unserved");
                value
            },
            "missing",
        ),
        (
            {
                let mut value = valid.clone();
                value["effects"]["extra"] = json!(true);
                value
            },
            "unknown field",
        ),
    ];

    for (value, message) in cases {
        let error = decode_coverage_summary(&canonical(&value))
            .unwrap_err()
            .to_string();
        assert!(error.contains(message), "{error}");
    }

    let mut noncanonical = vec![b' '];
    noncanonical.extend(canonical(&valid));
    assert_eq!(
        decode_coverage_summary(&noncanonical)
            .unwrap_err()
            .to_string(),
        "JSON bytes are not canonical"
    );
}

#[test]
fn coverage_validator_rejects_count_state_name_and_exclusion_mutations() {
    let measured = || CoverageSummary {
        schema: 1,
        effects: EffectCoverage {
            measured: true,
            served_requests: Some(BTreeMap::from([("add".to_owned(), 1)])),
            nested_occurrences: Some(BTreeMap::from([("add.inspect_source".to_owned(), 2)])),
            statically_unserved: names(),
        },
    };
    validate_coverage_summary(&measured()).unwrap();

    let cases = [
        (
            {
                let mut value = measured();
                value.effects.measured = false;
                value
            },
            "unmeasured",
        ),
        (
            {
                let mut value = measured();
                value.effects.served_requests = None;
                value
            },
            "measured",
        ),
        (
            {
                let mut value = measured();
                value.effects.nested_occurrences = None;
                value
            },
            "measured",
        ),
        (
            {
                let mut value = measured();
                value
                    .effects
                    .served_requests
                    .as_mut()
                    .unwrap()
                    .insert("reload".to_owned(), 0);
                value
            },
            "positive",
        ),
        (
            {
                let mut value = measured();
                value
                    .effects
                    .nested_occurrences
                    .as_mut()
                    .unwrap()
                    .insert("Add.Save".to_owned(), 1);
                value
            },
            "name",
        ),
        (
            {
                let mut value = measured();
                value.effects.statically_unserved = vec!["none".to_owned(), "none".to_owned()];
                value
            },
            "sorted and unique",
        ),
        (
            {
                let mut value = measured();
                value.effects.statically_unserved = vec!["quit".to_owned(), "none".to_owned()];
                value
            },
            "sorted and unique",
        ),
        (
            {
                let mut value = measured();
                value.effects.statically_unserved = vec!["none".to_owned(), "open..run".to_owned()];
                value
            },
            "name",
        ),
        (
            {
                let mut value = measured();
                value
                    .effects
                    .served_requests
                    .as_mut()
                    .unwrap()
                    .insert("quit".to_owned(), 1);
                value
            },
            "statically unserved",
        ),
        (
            {
                let mut value = measured();
                value
                    .effects
                    .nested_occurrences
                    .as_mut()
                    .unwrap()
                    .insert("preferences.none".to_owned(), 1);
                value
            },
            "statically unserved",
        ),
    ];

    for (summary, message) in cases {
        let error = validate_coverage_summary(&summary).unwrap_err().to_string();
        assert!(error.contains(message), "{error}");
    }

    let unmeasured = CoverageSummary {
        schema: 1,
        effects: EffectCoverage {
            measured: false,
            served_requests: None,
            nested_occurrences: Some(BTreeMap::new()),
            statically_unserved: names(),
        },
    };
    assert!(
        validate_coverage_summary(&unmeasured)
            .unwrap_err()
            .to_string()
            .contains("unmeasured")
    );
    let unmeasured = CoverageSummary {
        schema: 1,
        effects: EffectCoverage {
            measured: false,
            served_requests: Some(BTreeMap::new()),
            nested_occurrences: None,
            statically_unserved: names(),
        },
    };
    assert!(
        validate_coverage_summary(&unmeasured)
            .unwrap_err()
            .to_string()
            .contains("unmeasured")
    );
}

#[test]
fn coverage_validator_rejects_empty_static_list_and_each_noncanonical_name_form() {
    let mut summary = CoverageSummary::unmeasured_effects(names()).unwrap();
    summary.effects.statically_unserved.clear();
    assert!(
        validate_coverage_summary(&summary)
            .unwrap_err()
            .to_string()
            .contains("empty")
    );

    assert!(CoverageSummary::unmeasured_effects(Vec::new()).is_err());

    for name in [
        "",
        ".none",
        "none.",
        "1reload",
        "save-runner",
        "save_runner_",
        "save__runner",
        " save_runner",
        "save_runner ",
    ] {
        let mut summary = CoverageSummary::unmeasured_effects(names()).unwrap();
        summary.effects.statically_unserved = vec![name.to_owned()];
        assert!(
            validate_coverage_summary(&summary)
                .unwrap_err()
                .to_string()
                .contains("name"),
            "{name:?}"
        );
    }

    let mut unsupported = CoverageSummary::unmeasured_effects(names()).unwrap();
    unsupported.schema = 2;
    assert!(coverage_summary_bytes(&unsupported).is_err());
}
