//! Exact public-surface ports of the executable pipeline/persistence contracts in Python v0.4
//! `tests/test_source_default_semantics.py`.
//!
//! Python oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//! These tests deliberately cross the same public layers a run uses. They do not replace execution
//! semantics with a dry-run-only smoke test, and a behavioral mismatch stays red on this branch.

use std::collections::BTreeMap;

use skit_application::{
    delivery::{PreparedValue, assemble},
    form_state::{FormStateService, remembered_values},
    tokens::TokenContext,
    value_preparation::prepare_values,
    value_resolution::resolve_values,
};
use skit_domain::{
    EntrySettings, Slug,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::form_plan;
use skit_language::inject_values;
use skit_store::FileFormStateStore;
use tempfile::TempDir;

const REFRESH_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "GREETING"
# kind = "const"
# type = "str"
# default = "hello"
# ///
GREETING = 'bonjour'
print(GREETING)
"#;

fn text_default(name: &str, delivery: ParameterDelivery, default: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.parameter_type = ParameterType::Str;
    declaration.delivery = delivery;
    declaration.default = Some(ParameterValue::String(default.to_owned()));
    declaration
}

fn token_context() -> TokenContext {
    TokenContext {
        cwd: "/work/dir".to_owned(),
        home: Some("/home/u".to_owned()),
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

#[test]
fn test_assemble_injects_a_value_that_equals_the_source_default() {
    let plan = form_plan("python", REFRESH_SCRIPT, &EntrySettings::default());
    let declarations = plan.declarations();
    assert_eq!(
        declarations[0].default,
        Some(ParameterValue::String("bonjour".to_owned()))
    );

    for value in ["bonjour", "other"] {
        let prepared = BTreeMap::from([(
            "GREETING".to_owned(),
            PreparedValue::Scalar(value.to_owned()),
        )]);
        let assembly = assemble(&declarations, &prepared, &[]).unwrap();
        assert_eq!(
            assembly.inject_values,
            BTreeMap::from([("GREETING".to_owned(), value.to_owned())])
        );
        assert_eq!(
            assembly.display,
            vec![("GREETING".to_owned(), value.to_owned())]
        );

        let injected = inject_values(
            "python",
            REFRESH_SCRIPT,
            &declarations,
            &assembly.inject_values,
        )
        .unwrap();
        assert!(injected.contains(value), "{injected}");
    }
}

#[test]
fn test_assemble_injects_the_expansion_of_an_untouched_token_default() {
    let text = REFRESH_SCRIPT
        .replace("default = \"hello\"", "default = \"out_{today}.csv\"")
        .replace("GREETING = 'bonjour'", "GREETING = 'out_{today}.csv'");
    let plan = form_plan("python", &text, &EntrySettings::default());
    let declarations = plan.declarations();
    assert_eq!(
        declarations[0].default,
        Some(ParameterValue::String("out_{today}.csv".to_owned()))
    );

    let raw = BTreeMap::from([(
        "GREETING".to_owned(),
        "out_{today}.csv".to_owned(),
    )]);
    let resolved = resolve_values(&declarations, &raw, &token_context()).unwrap();
    assert_eq!(
        resolved,
        BTreeMap::from([(
            "GREETING".to_owned(),
            "out_2026-07-09.csv".to_owned()
        )])
    );
    let prepared = prepare_values(&declarations, &raw, &resolved).unwrap();
    let assembly = assemble(&declarations, &prepared, &[]).unwrap();
    assert_eq!(
        assembly.inject_values,
        BTreeMap::from([(
            "GREETING".to_owned(),
            "out_2026-07-09.csv".to_owned()
        )])
    );
}

#[test]
fn test_assemble_inject_delivers_empty_string_when_cleared() {
    let plan = form_plan("python", REFRESH_SCRIPT, &EntrySettings::default());
    let declarations = plan.declarations();
    let prepared = BTreeMap::from([(
        "GREETING".to_owned(),
        PreparedValue::Scalar(String::new()),
    )]);

    let assembly = assemble(&declarations, &prepared, &[]).unwrap();
    assert_eq!(
        assembly.inject_values,
        BTreeMap::from([("GREETING".to_owned(), String::new())])
    );
    assert!(
        assembly
            .display
            .iter()
            .any(|row| row == &("GREETING".to_owned(), "''".to_owned())),
        "{:?}",
        assembly.display
    );

    let injected = inject_values(
        "python",
        REFRESH_SCRIPT,
        &declarations,
        &assembly.inject_values,
    )
    .unwrap();
    assert!(!injected.contains("bonjour"), "{injected}");
}

#[test]
fn test_assemble_env_delivers_empty_string_when_cleared() {
    let mut declaration = text_default("CITY", ParameterDelivery::Env, "Taipei");
    declaration.env_target = "CITY".to_owned();
    let prepared = BTreeMap::from([(
        "CITY".to_owned(),
        PreparedValue::Scalar(String::new()),
    )]);

    let assembly = assemble(&[declaration], &prepared, &[]).unwrap();
    assert_eq!(
        assembly.env_values,
        BTreeMap::from([("CITY".to_owned(), String::new())])
    );
}

#[test]
fn test_assemble_flag_delivers_empty_string_when_cleared() {
    let mut declaration = text_default("x", ParameterDelivery::Flag, "def");
    declaration.flag = "--x".to_owned();
    let prepared = BTreeMap::from([(
        "x".to_owned(),
        PreparedValue::Scalar(String::new()),
    )]);

    let assembly = assemble(&[declaration], &prepared, &[]).unwrap();
    assert_eq!(assembly.args, ["--x", ""]);
    assert_eq!(assembly.masked_args, ["--x", ""]);
}

fn persist_declarations() -> Vec<ParamDecl> {
    let mut greeting = text_default("GREETING", ParameterDelivery::Inject, "bonjour");
    greeting.binding = ParameterBinding::Const;

    let mut width = ParamDecl::new("WIDTH");
    width.binding = ParameterBinding::Const;
    width.delivery = ParameterDelivery::Inject;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));
    vec![greeting, width]
}

#[test]
fn test_last_used_drops_values_equal_to_their_default() {
    let submitted = BTreeMap::from([
        ("GREETING".to_owned(), "bonjour".to_owned()),
        ("WIDTH".to_owned(), "800".to_owned()),
    ]);
    assert!(remembered_values(&persist_declarations(), &submitted).is_empty());
}

#[test]
fn test_last_used_keeps_a_cleared_empty_only_where_it_was_delivered() {
    let submitted = BTreeMap::from([
        ("GREETING".to_owned(), String::new()),
        ("WIDTH".to_owned(), String::new()),
    ]);
    assert_eq!(
        remembered_values(&persist_declarations(), &submitted),
        BTreeMap::from([("GREETING".to_owned(), String::new())])
    );
}

#[test]
fn test_save_after_run_persists_via_the_remembered_rule() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let service = FormStateService::new(repository);
    let slug = Slug::parse("rem".to_owned()).unwrap();
    let declarations = persist_declarations();
    let submitted = BTreeMap::from([
        ("GREETING".to_owned(), "bonjour".to_owned()),
        ("WIDTH".to_owned(), "900".to_owned()),
    ]);

    service
        .save_last(&slug, &declarations, Some(&submitted), Some(Vec::new()), false)
        .unwrap();
    service
        .record_run(
            &slug,
            0,
            "2026-07-09T14:30:05+00:00",
            &declarations,
            Some(&submitted),
        )
        .unwrap();

    let state = service.load(&slug);
    assert_eq!(
        state.values,
        BTreeMap::from([("WIDTH".to_owned(), "900".to_owned())])
    );
    assert_eq!(state.last_run.exit, Some(0));
    assert_eq!(state.last_run.values, Some(submitted));
}
