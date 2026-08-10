//! Direct ports from Python `tests/test_source_default_semantics.py`
//! (`origin/main@206f9ef`). The Python implementation is the behavioral oracle.

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{ParseOutcome, ParsedDocument, parse_document};

fn parsed(kind: &str, source: &str) -> ParsedDocument {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected parsed {kind} source, got {other:?}"),
    }
}

fn constant(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

fn env_default(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::EnvDefault;
    declaration.delivery = ParameterDelivery::Env;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

#[test]
fn test_reconcile_records_current_default_for_an_ok_const() {
    let report = parsed("python", "CITY = \"Taipei\"\nprint(CITY)\n")
        .reconcile(&[constant("CITY", ParameterType::Str)]);
    assert_eq!(
        report
            .ok
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<Vec<_>>(),
        ["CITY"]
    );
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
    // The Python oracle treats a source fallback shaped like 8080 as the current value of a stored
    // string environment default. The value remains environment-delivered text at runtime.
    let report = parsed("shell", "echo \"${PORT:-8080}\"\n").reconcile(&[env_default("PORT")]);
    assert_eq!(
        report
            .ok
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<Vec<_>>(),
        ["PORT"]
    );
    assert_eq!(
        report.current_defaults,
        [("PORT".to_owned(), ParameterValue::Integer(8080))]
            .into_iter()
            .collect()
    );
}

#[test]
fn test_reconcile_omits_current_default_for_a_type_changed_const() {
    let report = parsed("python", "RETRIES = \"three\"\nprint(RETRIES)\n")
        .reconcile(&[constant("RETRIES", ParameterType::Int)]);
    assert_eq!(
        report
            .changed
            .iter()
            .map(|pair| pair.stored.name.as_str())
            .collect::<Vec<_>>(),
        ["RETRIES"]
    );
    assert!(report.current_defaults.is_empty());
}
