use std::collections::BTreeMap;

use serde_json::{Value, json};
use skit_domain::parameters::{
    ParamDecl, ParameterDelivery, ParameterType, ParameterValue, declared_for_template,
    declared_from_meta,
};

fn map(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}

fn row(name: &str, delivery: ParameterDelivery) -> BTreeMap<String, Value> {
    ParamDecl {
        delivery,
        ..ParamDecl::new(name)
    }
    .to_meta_map()
}

#[test]
fn declared_from_meta_drops_nameless_rows_but_preserves_order_and_duplicates() {
    let rows = vec![
        map(json!({"delivery": "flag"})),
        row("same", ParameterDelivery::Placeholder),
        row("env", ParameterDelivery::Env),
        row("same", ParameterDelivery::Env),
    ];

    let declarations = declared_from_meta(Some(&rows));

    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["same", "env", "same"]
    );
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.delivery)
            .collect::<Vec<_>>(),
        [
            ParameterDelivery::Placeholder,
            ParameterDelivery::Env,
            ParameterDelivery::Env,
        ]
    );
    assert!(declared_from_meta(None).is_empty());
}

#[test]
fn undeclared_placeholders_synthesize_the_historical_schema_in_template_order() {
    let placeholders = vec!["input".to_owned(), "api_key".to_owned()];

    let declarations = declared_for_template(None, &placeholders);

    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["input", "api_key"]
    );
    assert!(
        declarations
            .iter()
            .all(|declaration| declaration.delivery == ParameterDelivery::Placeholder)
    );
    assert!(declarations.iter().all(|declaration| declaration.required));
    assert!(!declarations[0].secret);
    assert!(declarations[1].secret);
}

#[test]
fn declared_placeholder_rows_override_the_entire_synthesized_schema() {
    let rows = vec![
        ParamDecl {
            delivery: ParameterDelivery::Placeholder,
            parameter_type: ParameterType::Choice,
            default: Some(ParameterValue::String("m".to_owned())),
            required: false,
            choices: vec!["s".to_owned(), "m".to_owned()],
            secret: false,
            prompt: "Size".to_owned(),
            ..ParamDecl::new("size")
        }
        .to_meta_map(),
        ParamDecl {
            delivery: ParameterDelivery::Placeholder,
            default: Some(ParameterValue::String("creds.json".to_owned())),
            required: false,
            secret: false,
            ..ParamDecl::new("token_file")
        }
        .to_meta_map(),
    ];
    let placeholders = vec!["token_file".to_owned(), "size".to_owned(), "host".to_owned()];

    let declarations = declared_for_template(Some(&rows), &placeholders);

    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["token_file", "size", "host"]
    );
    assert_eq!(
        declarations[0].default,
        Some(ParameterValue::String("creds.json".to_owned()))
    );
    assert!(!declarations[0].required);
    assert!(!declarations[0].secret);
    assert_eq!(declarations[1].parameter_type, ParameterType::Choice);
    assert_eq!(declarations[1].choices, ["s", "m"]);
    assert_eq!(declarations[1].prompt, "Size");
    assert!(declarations[2].required);
}

#[test]
fn wrong_delivery_for_a_placeholder_is_replaced_and_not_readded_as_an_env_rider() {
    let rows = vec![ParamDecl {
        delivery: ParameterDelivery::Env,
        default: Some(ParameterValue::String("wrong channel".to_owned())),
        ..ParamDecl::new("file")
    }
    .to_meta_map()];
    let placeholders = vec!["file".to_owned()];

    let declarations = declared_for_template(Some(&rows), &placeholders);

    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].name, "file");
    assert_eq!(declarations[0].delivery, ParameterDelivery::Placeholder);
    assert_eq!(declarations[0].default, None);
    assert!(declarations[0].required);
}

#[test]
fn env_riders_follow_placeholders_while_flag_and_stray_placeholder_rows_are_dropped() {
    let rows = vec![
        row("width", ParameterDelivery::Flag),
        row("RETRIES", ParameterDelivery::Env),
        row("unused_slot", ParameterDelivery::Placeholder),
        row("DEBUG", ParameterDelivery::Env),
    ];
    let placeholders = vec!["file".to_owned()];

    let declarations = declared_for_template(Some(&rows), &placeholders);

    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["file", "RETRIES", "DEBUG"]
    );
    assert_eq!(declarations[1].delivery, ParameterDelivery::Env);
    assert_eq!(declarations[2].delivery, ParameterDelivery::Env);
}

#[test]
fn duplicate_names_use_the_last_schema_without_moving_the_first_insertion_slot() {
    let rows = vec![
        ParamDecl {
            delivery: ParameterDelivery::Env,
            default: Some(ParameterValue::Integer(1)),
            ..ParamDecl::new("A")
        }
        .to_meta_map(),
        ParamDecl {
            delivery: ParameterDelivery::Env,
            default: Some(ParameterValue::Integer(2)),
            ..ParamDecl::new("B")
        }
        .to_meta_map(),
        ParamDecl {
            delivery: ParameterDelivery::Env,
            default: Some(ParameterValue::Integer(3)),
            ..ParamDecl::new("A")
        }
        .to_meta_map(),
    ];
    let placeholders = vec!["slot".to_owned()];

    let declarations = declared_for_template(Some(&rows), &placeholders);

    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["slot", "A", "B"]
    );
    assert_eq!(declarations[1].default, Some(ParameterValue::Integer(3)));
    assert_eq!(declarations[2].default, Some(ParameterValue::Integer(2)));
}

#[test]
fn placeholder_multiplicity_and_case_are_preserved_exactly() {
    let rows = vec![ParamDecl {
        delivery: ParameterDelivery::Placeholder,
        required: false,
        ..ParamDecl::new("name")
    }
    .to_meta_map()];
    let placeholders = vec!["name".to_owned(), "Name".to_owned(), "name".to_owned()];

    let declarations = declared_for_template(Some(&rows), &placeholders);

    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["name", "Name", "name"]
    );
    assert!(!declarations[0].required);
    assert!(declarations[1].required);
    assert!(!declarations[2].required);
}
