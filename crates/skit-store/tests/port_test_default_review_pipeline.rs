//! Cross-layer ports of the executable pipeline contracts in Python v0.4
//! `tests/test_default_semantics_review_fixes.py`.
//!
//! These tests intentionally cross form planning, ambient-value resolution, validation, delivery,
//! persistence policy, and source injection. A red assertion is a parity finding; this branch does
//! not patch product code or weaken the Python oracle.

use std::collections::BTreeMap;

use skit_application::{
    delivery::{PreparedValue, assemble},
    form_state::{prefill, remembered_values},
    tokens::TokenContext,
    value_preparation::prepare_values,
    value_resolution::{ValueResolutionError, resolve_values},
};
use skit_domain::{EntrySettings, parameters::ParameterValue};
use skit_form::form_plan;
use skit_language::inject_values;

const SECRET_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "API_KEY"
# kind = "const"
# type = "str"
# default = ""
# secret = true
# env_source = "MY_KEY"
# ///
API_KEY = ""
print(API_KEY)
"#;

const INPUT_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "input-1"
# kind = "input"
# type = "str"
# default = "Tim"
# order = 0
# prompt = "Your name? "
# ///
name = input("Your name? ")
print(name)
"#;

const MAIN_GUARD_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "HOST"
# kind = "const"
# type = "str"
# default = "localhost"
# ///
HOST = "localhost"

if __name__ == "__main__":
    HOST = "127.0.0.1"
    print(HOST)
"#;

fn token_context(env: &[(&str, &str)]) -> TokenContext {
    TokenContext {
        cwd: "/work".to_owned(),
        home: Some("/home/test".to_owned()),
        env: env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

fn assemble_raw(
    declarations: &[skit_domain::parameters::ParamDecl],
    raw: &BTreeMap<String, String>,
    context: &TokenContext,
) -> skit_application::delivery::Assembly {
    let resolved = resolve_values(declarations, raw, context).unwrap();
    let prepared = prepare_values(declarations, raw, &resolved).unwrap();
    assemble(declarations, &prepared, &[]).unwrap()
}

#[test]
fn test_secret_with_an_empty_source_literal_is_still_delivered() {
    let plan = form_plan("python", SECRET_SCRIPT, &EntrySettings::default());
    let [field] = plan.fields.as_slice() else {
        panic!("expected exactly one secret field: {plan:?}");
    };
    assert_eq!(field.declaration.name, "API_KEY");
    assert!(field.declaration.secret);
    assert_eq!(field.declaration.env_source, "MY_KEY");
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::String(String::new()))
    );

    let declarations = plan.declarations();
    let raw = prefill(&declarations, &BTreeMap::new(), None);
    assert!(raw.is_empty(), "a secret must never be prefilled: {raw:?}");

    let assembly = assemble_raw(
        &declarations,
        &raw,
        &token_context(&[("MY_KEY", "sk-live-XYZ")]),
    );
    assert_eq!(
        assembly.inject_values,
        BTreeMap::from([("API_KEY".to_owned(), "sk-live-XYZ".to_owned())])
    );
    assert_eq!(
        assembly.display,
        vec![("API_KEY".to_owned(), "•••".to_owned())]
    );
}

#[test]
fn test_secret_field_never_delivers_empty() {
    let plan = form_plan("python", SECRET_SCRIPT, &EntrySettings::default());
    let [field] = plan.fields.as_slice() else {
        panic!("expected exactly one secret field: {plan:?}");
    };
    assert!(!field.delivers_empty());

    let declarations = plan.declarations();
    let error = resolve_values(&declarations, &BTreeMap::new(), &token_context(&[])).unwrap_err();
    assert_eq!(
        error,
        ValueResolutionError::MissingSecretEnvironment {
            name: "API_KEY".to_owned(),
            environment: "MY_KEY".to_owned(),
        }
    );
}

#[test]
fn test_input_binding_with_a_default_is_delivered() {
    let plan = form_plan("python", INPUT_SCRIPT, &EntrySettings::default());
    let [field] = plan.fields.as_slice() else {
        panic!("expected exactly one input field: {plan:?}");
    };
    assert_eq!(field.declaration.name, "input-1");
    assert!(field.input_binding);
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::String("Tim".to_owned()))
    );
    assert!(
        plan.drift.is_empty(),
        "the stored prompt/order must still resolve"
    );

    let declarations = plan.declarations();
    let raw = prefill(&declarations, &BTreeMap::new(), None);
    assert_eq!(
        raw,
        BTreeMap::from([("input-1".to_owned(), "Tim".to_owned())])
    );
    let assembly = assemble_raw(&declarations, &raw, &token_context(&[]));
    assert_eq!(
        assembly.inject_values,
        BTreeMap::from([("input-1".to_owned(), "Tim".to_owned())])
    );

    let injected = inject_values(
        "python",
        INPUT_SCRIPT,
        &declarations,
        &assembly.inject_values,
    )
    .unwrap();
    assert!(injected.contains("_skit_i[0]("), "{injected}");
    assert!(!injected.contains("input(\"Your name? \")"), "{injected}");
}

#[test]
fn test_main_guard_override_receives_the_unchanged_default() {
    let plan = form_plan("python", MAIN_GUARD_SCRIPT, &EntrySettings::default());
    let declarations = plan.declarations();
    let raw = prefill(&declarations, &BTreeMap::new(), None);
    assert_eq!(
        raw,
        BTreeMap::from([("HOST".to_owned(), "localhost".to_owned())])
    );
    let assembly = assemble_raw(&declarations, &raw, &token_context(&[]));
    assert_eq!(
        assembly.inject_values,
        BTreeMap::from([("HOST".to_owned(), "localhost".to_owned())])
    );

    let injected = inject_values(
        "python",
        MAIN_GUARD_SCRIPT,
        &declarations,
        &assembly.inject_values,
    )
    .unwrap();
    assert_eq!(
        injected.matches("HOST = 'localhost'").count(),
        2,
        "{injected}"
    );
    assert!(!injected.contains("127.0.0.1"), "{injected}");
}

#[test]
fn test_last_used_filters_the_default_but_keeps_a_delivered_empty() {
    let source = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "GREETING"
# kind = "const"
# type = "str"
# default = "bonjour"
# ///
GREETING = "bonjour"
print(GREETING)
"#;
    let plan = form_plan("python", source, &EntrySettings::default());
    let [field] = plan.fields.as_slice() else {
        panic!("expected one defaulted field: {plan:?}");
    };
    assert!(field.delivers_empty());
    let declarations = plan.declarations();

    let accepted_default = BTreeMap::from([("GREETING".to_owned(), "bonjour".to_owned())]);
    assert!(remembered_values(&declarations, &accepted_default).is_empty());

    let delivered_empty = BTreeMap::from([("GREETING".to_owned(), String::new())]);
    assert_eq!(
        remembered_values(&declarations, &delivered_empty),
        delivered_empty
    );
}
