//! Cross-layer public-API ports for Python v0.4 source-default delivery semantics.
//!
//! These tests intentionally cross `skit-form` planning into `skit-application` delivery and
//! persistence. A red assertion means semantic information was lost between public layers; the
//! oracle-port branch does not patch production code to make it green.

use std::collections::BTreeMap;

use skit_application::{
    delivery::{PreparedValue, assemble},
    form_state::{preset_values, remembered_values},
};
use skit_domain::{EntrySettings, parameters::ParameterValue};
use skit_form::form_plan;

fn shell_envdefault(operator: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "envdefault"
# type = "str"
# default = "Taipei"
# ///
echo "${{CITY{operator}Taipei}}"
"#,
    )
}

fn empty_prepared() -> BTreeMap<String, PreparedValue> {
    BTreeMap::from([("CITY".to_owned(), PreparedValue::Scalar(String::new()))])
}

fn empty_submitted() -> BTreeMap<String, String> {
    BTreeMap::from([("CITY".to_owned(), String::new())])
}

#[test]
fn test_shell_colon_envdefault_does_not_export_empty_after_form_planning() {
    for operator in [":-", ":="] {
        let text = shell_envdefault(operator);
        let plan = form_plan("shell", &text, &EntrySettings::default());
        let [field] = plan.fields.as_slice() else {
            panic!("expected one field for {operator}: {plan:?}");
        };
        assert!(field.empty_uses_default, "operator {operator}");
        assert!(!field.delivers_empty(), "operator {operator}");

        let assembly = assemble(&plan.declarations(), &empty_prepared(), &[]).unwrap();
        assert!(
            assembly.env_values.is_empty(),
            "operator {operator} treats an empty environment value as unset: {assembly:?}"
        );
    }
}

#[test]
fn test_shell_noncolon_envdefault_exports_explicit_empty_after_form_planning() {
    for operator in ["-", "="] {
        let text = shell_envdefault(operator);
        let plan = form_plan("shell", &text, &EntrySettings::default());
        let [field] = plan.fields.as_slice() else {
            panic!("expected one field for {operator}: {plan:?}");
        };
        assert!(!field.empty_uses_default, "operator {operator}");
        assert!(field.delivers_empty(), "operator {operator}");

        let assembly = assemble(&plan.declarations(), &empty_prepared(), &[]).unwrap();
        assert_eq!(
            assembly.env_values,
            BTreeMap::from([("CITY".to_owned(), String::new())]),
            "operator {operator} genuinely delivers an explicit empty environment value"
        );
    }
}

#[test]
fn test_shell_colon_envdefault_does_not_remember_an_empty_value_that_was_not_delivered() {
    for operator in [":-", ":="] {
        let text = shell_envdefault(operator);
        let plan = form_plan("shell", &text, &EntrySettings::default());
        let remembered = remembered_values(&plan.declarations(), &empty_submitted());
        assert!(
            remembered.is_empty(),
            "operator {operator} must not persist an empty value that activates the source fallback"
        );
    }
}

#[test]
fn test_shell_noncolon_envdefault_remembers_an_explicit_empty_value() {
    for operator in ["-", "="] {
        let text = shell_envdefault(operator);
        let plan = form_plan("shell", &text, &EntrySettings::default());
        let remembered = remembered_values(&plan.declarations(), &empty_submitted());
        assert_eq!(remembered, empty_submitted(), "operator {operator}");
    }
}

#[test]
fn test_last_used_drops_values_equal_to_the_current_source_default_but_preset_pins_them() {
    let text = shell_envdefault("-");
    let plan = form_plan("shell", &text, &EntrySettings::default());
    let declarations = plan.declarations();
    assert_eq!(
        declarations[0].default,
        Some(ParameterValue::String("Taipei".to_owned()))
    );
    let submitted = BTreeMap::from([("CITY".to_owned(), "Taipei".to_owned())]);

    assert!(remembered_values(&declarations, &submitted).is_empty());
    assert_eq!(preset_values(&declarations, &submitted), submitted);
}
