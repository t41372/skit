//! Public-API ports of Python v0.4 reconciliation/default regressions.
//!
//! These pin the semantic report itself rather than only its later form projection: current source
//! defaults belong to unchanged, public bindings; drift and unfit/secret values must not leak into
//! that refresh map.

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{ParseOutcome, ReconcileReport, parse_document};

fn reconcile(kind: &str, source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    let ParseOutcome::Parsed(document) = parse_document(kind, source) else {
        panic!("expected {kind} source to parse");
    };
    document.reconcile(stored)
}

fn const_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

fn envdefault(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::EnvDefault;
    declaration.delivery = ParameterDelivery::Env;
    declaration.parameter_type = parameter_type;
    declaration
}

#[test]
fn test_reconcile_records_current_default_for_an_ok_const() {
    let report = reconcile(
        "python",
        "CITY = \"Taipei\"\nprint(CITY)\n",
        &[const_decl("CITY", ParameterType::Str)],
    );

    assert_eq!(report.ok.len(), 1);
    assert_eq!(report.ok[0].stored.name, "CITY");
    assert_eq!(
        report.current_defaults,
        [(
            "CITY".to_owned(),
            ParameterValue::String("Taipei".to_owned())
        )]
        .into_iter()
        .collect()
    );
}

#[test]
fn test_reconcile_records_current_default_for_an_ok_envdefault() {
    let report = reconcile(
        "shell",
        "echo \"${PORT:-8080}\"\n",
        &[envdefault("PORT", ParameterType::Int)],
    );

    assert_eq!(report.ok.len(), 1);
    assert_eq!(report.ok[0].stored.name, "PORT");
    assert_eq!(
        report.current_defaults,
        [("PORT".to_owned(), ParameterValue::Integer(8080))]
            .into_iter()
            .collect()
    );
}

#[test]
fn test_reconcile_omits_current_default_for_a_type_changed_const() {
    let mut stored = const_decl("RETRIES", ParameterType::Int);
    stored.default = Some(ParameterValue::Integer(3));
    let report = reconcile("python", "RETRIES = \"three\"\nprint(RETRIES)\n", &[stored]);

    assert!(report.ok.is_empty());
    assert_eq!(report.changed.len(), 1);
    assert_eq!(report.changed[0].stored.name, "RETRIES");
    assert!(report.current_defaults.is_empty());
}

#[test]
fn test_reconcile_secret_const_never_publishes_its_source_literal() {
    let mut stored = const_decl("TOKEN", ParameterType::Str);
    stored.secret = true;
    let report = reconcile(
        "python",
        "TOKEN = \"sk-live-source\"\nprint(TOKEN)\n",
        &[stored],
    );

    assert_eq!(report.ok.len(), 1);
    assert!(report.current_defaults.is_empty());
}

#[test]
fn test_envdefault_default_that_no_longer_fits_the_declared_type_is_not_published() {
    let mut stored = envdefault("PORT", ParameterType::Int);
    stored.default = Some(ParameterValue::Integer(8080));
    let report = reconcile(
        "shell",
        "PORT=${PORT:-$FALLBACK}\necho \"$PORT\"\n",
        &[stored],
    );

    assert_eq!(report.ok.len(), 1);
    assert!(report.current_defaults.is_empty());
}

#[test]
fn test_int_shaped_literal_can_refresh_a_string_envdefault() {
    let report = reconcile(
        "shell",
        "PORT=${PORT:-8080}\necho \"$PORT\"\n",
        &[envdefault("PORT", ParameterType::Str)],
    );

    assert_eq!(report.ok.len(), 1);
    assert_eq!(
        report.current_defaults,
        [("PORT".to_owned(), ParameterValue::Integer(8080))]
            .into_iter()
            .collect()
    );
}

#[test]
fn test_input_prompt_movement_is_rebound_by_position_without_losing_the_field() {
    let source = "who = input(\"Name: \")\npw = input(\"New label: \")\nprint(who, pw)\n";

    let mut first = ParamDecl::new("input-1");
    first.binding = ParameterBinding::Input;
    first.delivery = ParameterDelivery::Inject;
    first.order = 0;
    first.prompt = "Name: ".to_owned();

    let mut second = ParamDecl::new("input-2");
    second.binding = ParameterBinding::Input;
    second.delivery = ParameterDelivery::Inject;
    second.order = 1;
    second.prompt = "Old label: ".to_owned();

    let report = reconcile("python", source, &[first, second]);

    assert_eq!(report.ok.len(), 1);
    assert_eq!(report.ok[0].stored.name, "input-1");
    assert_eq!(report.rebound.len(), 1);
    assert_eq!(report.rebound[0].stored.name, "input-2");
    assert_eq!(report.rebound[0].current.declaration.order, 1);
    assert_eq!(report.rebound[0].current.declaration.prompt, "New label: ");
}

#[test]
fn test_shell_colon_operator_marks_empty_as_using_the_default_in_reconcile_report() {
    for operator in [":-", ":="] {
        let source = format!("CITY=${{CITY{operator}Taipei}}\necho \"$CITY\"\n");
        let report = reconcile("shell", &source, &[envdefault("CITY", ParameterType::Str)]);
        assert!(
            report.empty_uses_default.contains("CITY"),
            "operator {operator}"
        );
    }
}

#[test]
fn test_shell_noncolon_operator_does_not_mark_empty_as_using_the_default() {
    for operator in ["-", "="] {
        let source = format!("CITY=${{CITY{operator}Taipei}}\necho \"$CITY\"\n");
        let report = reconcile("shell", &source, &[envdefault("CITY", ParameterType::Str)]);
        assert!(
            !report.empty_uses_default.contains("CITY"),
            "operator {operator}"
        );
    }
}
