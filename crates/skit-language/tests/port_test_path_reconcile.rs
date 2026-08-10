//! Public-API ports of Python v0.4 path-type reconciliation contracts.

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
fn test_reconcile_path_over_str_const_is_refinement_not_drift() {
    let ParseOutcome::Parsed(document) = parse_document("python", SCRIPT) else {
        panic!("expected source to parse");
    };
    let report = document.reconcile(&[path_decl("SRC")]);

    assert!(!report.has_drift());
    assert!(report.changed.is_empty());
    assert_eq!(report.usable()[0].name, "SRC");
}

#[test]
fn test_reconcile_path_over_int_const_is_real_type_drift() {
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
    assert_eq!(report.usable()[0].parameter_type, ParameterType::Path);
}

#[test]
fn test_current_string_default_refreshes_a_path_refinement_as_text() {
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
