use std::collections::BTreeMap;

use serde_json::{Value, json};
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};
use skit_domain::{EntryKind, EntryMeta, EntrySettings};

fn meta() -> EntryMeta {
    EntryMeta::minimal("Demo", EntryKind::parse("shell").unwrap())
}

#[test]
fn legacy_extra_fields_decode_to_one_typed_runtime_view() {
    let mut meta = meta();
    meta.extra = BTreeMap::from([
        ("template".to_owned(), json!("echo {name}")),
        ("dependencies".to_owned(), json!(["a", 7, "b"])),
        ("requires_python".to_owned(), json!(">=3.13")),
        ("params".to_owned(), json!(["name"])),
        ("interpreter".to_owned(), json!("zsh")),
        ("runner".to_owned(), json!("codex")),
        ("interpolate".to_owned(), json!(false)),
        ("needs".to_owned(), json!(["jq", "ffmpeg"])),
        (
            "parameters".to_owned(),
            json!([
                {
                    "name": "name",
                    "delivery": "env",
                    "type": "str",
                    "env_target": "NAME"
                },
                "bad-row"
            ]),
        ),
        ("future".to_owned(), json!({"keep": true})),
    ]);

    let settings = EntrySettings::from_meta(&meta);

    assert_eq!(settings.template, "echo {name}");
    assert_eq!(settings.dependencies, ["a", "b"]);
    assert_eq!(settings.requires_python, ">=3.13");
    assert_eq!(settings.params, ["name"]);
    assert_eq!(settings.interpreter, "zsh");
    assert_eq!(settings.runner, "codex");
    assert!(!settings.interpolate);
    assert_eq!(settings.needs, ["jq", "ffmpeg"]);
    assert_eq!(settings.parameters.len(), 1);
    assert_eq!(settings.parameters[0].name, "name");
    assert_eq!(settings.parameters[0].delivery, ParameterDelivery::Env);
    assert_eq!(settings.parameters[0].env_target, "NAME");
}

#[test]
fn malformed_optional_fields_use_safe_defaults() {
    let mut meta = meta();
    meta.extra = BTreeMap::from([
        ("template".to_owned(), json!(9)),
        ("dependencies".to_owned(), json!("bad")),
        ("requires_python".to_owned(), Value::Null),
        ("interpreter".to_owned(), json!(true)),
        ("runner".to_owned(), json!([])),
        ("interpolate".to_owned(), json!("false")),
        ("needs".to_owned(), json!([1, false])),
        ("parameters".to_owned(), json!({"bad": true})),
    ]);

    let settings = EntrySettings::from_meta(&meta);

    assert_eq!(settings, EntrySettings::default());
    assert!(settings.interpolate);
}

#[test]
fn writing_settings_preserves_unknown_extension_fields_and_legacy_omission_rules() {
    let mut meta = meta();
    meta.extra
        .insert("future".to_owned(), json!({"keep": true}));
    let mut parameter = ParamDecl::new("count");
    parameter.parameter_type = ParameterType::Int;
    parameter.delivery = ParameterDelivery::Flag;
    parameter.flag = "--count".to_owned();
    let settings = EntrySettings {
        template: "tool {count}".to_owned(),
        dependencies: vec!["pkg".to_owned()],
        requires_python: ">=3.13".to_owned(),
        params: vec!["count".to_owned()],
        interpreter: "bash".to_owned(),
        runner: String::new(),
        interpolate: true,
        needs: vec!["jq".to_owned()],
        parameters: vec![parameter],
    };

    settings.write_to_meta(&mut meta);

    assert_eq!(meta.extra["future"], json!({"keep": true}));
    assert_eq!(meta.extra["template"], json!("tool {count}"));
    assert_eq!(meta.extra["dependencies"], json!(["pkg"]));
    assert_eq!(meta.extra["requires_python"], json!(">=3.13"));
    assert_eq!(meta.extra["params"], json!(["count"]));
    assert_eq!(meta.extra["interpreter"], json!("bash"));
    assert_eq!(meta.extra["needs"], json!(["jq"]));
    assert!(!meta.extra.contains_key("runner"));
    assert!(!meta.extra.contains_key("interpolate"));
    assert_eq!(meta.extra["parameters"][0]["name"], json!("count"));
    assert_eq!(meta.extra["parameters"][0]["type"], json!("int"));
}

#[test]
fn writing_disabled_interpolation_keeps_the_explicit_legacy_false_value() {
    let mut meta = meta();
    EntrySettings {
        interpolate: false,
        ..EntrySettings::default()
    }
    .write_to_meta(&mut meta);

    assert_eq!(meta.extra["interpolate"], json!(false));
}
