//! Exact behavioral ports of Python v0.4 `tests/test_shell_getopts.py` at
//! `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//!
//! These tests use the published Rust semantic projection, form planner, and delivery assembler.
//! They do not reimplement the getopts parser in test code. Red assertions are parity findings.

use std::collections::BTreeMap;

use skit_application::delivery::{PreparedValue, assemble};
use skit_domain::{
    EntrySettings,
    parameters::{ParameterType, ParameterValue},
};
use skit_form::{CliFormProjection, FormSource, cli_form_projection, form_plan};
use skit_language::DegradationReason;

fn static_fields(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    match cli_form_projection("shell", source) {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, "getopts");
            fields
        }
        other => panic!("expected static getopts projection, got {other:?}"),
    }
}

fn by_name(
    fields: &[skit_domain::parameters::ParamDecl],
) -> BTreeMap<&str, &skit_domain::parameters::ParamDecl> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

#[test]
fn test_value_and_bool_flags() {
    let fields = static_fields("while getopts \"n:v\" opt; do :; done\n");
    let fields = by_name(&fields);

    assert_eq!(fields["n"].parameter_type, ParameterType::Str);
    assert_eq!(fields["n"].flag, "-n");
    assert_eq!(fields["n"].action, "");

    assert_eq!(fields["v"].parameter_type, ParameterType::Bool);
    assert_eq!(fields["v"].flag, "-v");
    assert_eq!(fields["v"].action, "store_true");
    assert_eq!(fields["v"].default, Some(ParameterValue::Bool(false)));
}

#[test]
fn test_leading_colon_silent_mode_is_skipped() {
    let fields = static_fields("while getopts \":ab:c:\" opt; do :; done\n");
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(fields[0].parameter_type, ParameterType::Bool);
    assert_eq!(fields[1].parameter_type, ParameterType::Str);
}

#[test]
fn test_non_alphanumeric_characters_are_skipped() {
    let fields = static_fields("while getopts \"a-b\" opt; do :; done\n");
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(
        fields
            .iter()
            .all(|field| field.parameter_type == ParameterType::Bool)
    );
}

#[test]
fn test_repeated_letter_keeps_first() {
    let fields = static_fields("while getopts \"vv\" opt; do :; done\n");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "v");
    assert_eq!(fields[0].flag, "-v");
}

#[test]
fn test_dynamic_optstring_degrades_to_dynamic() {
    assert_eq!(
        cli_form_projection("shell", "getopts \"$OPTS\" opt\n"),
        CliFormProjection::Dynamic {
            framework: "getopts".to_owned(),
            reason: DegradationReason::DynamicDeclaration,
        }
    );
}

#[test]
fn test_getopts_without_optstring_is_none() {
    assert_eq!(
        cli_form_projection("shell", "getopts\n"),
        CliFormProjection::Absent
    );
}

#[test]
fn test_no_getopts_is_none() {
    assert_eq!(
        cli_form_projection("shell", "echo hello\n"),
        CliFormProjection::Absent
    );
}

#[test]
fn test_unparseable_script_is_none() {
    assert_eq!(
        cli_form_projection("shell", "if\n"),
        CliFormProjection::Absent
    );
}

#[test]
fn test_secret_letter_is_not_special() {
    let fields = static_fields("while getopts \"k:\" opt; do :; done\n");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "k");
    assert!(!fields[0].secret);
}

#[test]
fn test_plan_reads_getopts_and_assembles_flags() {
    let source = "#!/usr/bin/env bash\nwhile getopts \"n:v\" opt; do :; done\n";
    let plan = form_plan("shell", source, &EntrySettings::default());

    assert_eq!(plan.source, FormSource::Reader);
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["n", "v"]
    );
    assert!(plan.degradation.is_none());

    let declarations = plan.declarations();
    let values = BTreeMap::from([
        ("n".to_owned(), PreparedValue::Scalar("Ada".to_owned())),
        ("v".to_owned(), PreparedValue::Scalar("true".to_owned())),
    ]);
    let assembly = assemble(&declarations, &values, &[]).unwrap();
    assert_eq!(assembly.args, ["-n", "Ada", "-v"]);
}

#[test]
fn test_plan_dynamic_getopts_degrades_with_reason() {
    let source = "#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" opt; do :; done\n";
    let plan = form_plan("shell", source, &EntrySettings::default());

    assert_eq!(plan.source, FormSource::Reader);
    assert_eq!(plan.degradation, Some(DegradationReason::DynamicDeclaration));
    assert!(plan.fields.is_empty());
}
