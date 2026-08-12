//! Exact language-layer ports of Python v0.4 `tests/test_path_type.py` reconciliation contracts.
//!
//! Frozen oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{ParseOutcome, parse_document};

const SCRIPT: &str = "SRC = \"./data.csv\"\nRETRIES = 3\nprint(SRC, RETRIES)\n";

fn path_decl(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Path;
    declaration
}

#[test]
fn test_reconcile_path_over_str_const_is_refinement() {
    let ParseOutcome::Parsed(document) = parse_document("python", SCRIPT) else {
        panic!("expected source to parse");
    };
    let report = document.reconcile(&[path_decl("SRC")]);
    assert!(!report.has_drift());
    assert!(report.changed.is_empty());
    assert_eq!(
        report
            .usable()
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["SRC"]
    );
}

#[test]
fn test_reconcile_path_over_int_const_is_drift() {
    let ParseOutcome::Parsed(document) = parse_document("python", SCRIPT) else {
        panic!("expected source to parse");
    };
    let report = document.reconcile(&[path_decl("RETRIES")]);
    assert!(report.has_drift());
    assert_eq!(report.changed.len(), 1);
    assert_eq!(report.changed[0].stored.name, "RETRIES");
    assert_eq!(
        report.changed[0].current.declaration.parameter_type,
        ParameterType::Int
    );
}

#[test]
fn rust_additive_current_string_default_refreshes_a_path_refinement_as_text() {
    let ParseOutcome::Parsed(document) = parse_document("python", SCRIPT) else {
        panic!("expected source to parse");
    };
    let report = document.reconcile(&[path_decl("SRC")]);
    assert_eq!(
        report.current_defaults.get("SRC"),
        Some(&skit_domain::parameters::ParameterValue::String(
            "./data.csv".to_owned()
        ))
    );
}
