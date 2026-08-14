//! Exact scalar-validation and bool-spelling ports from Python v0.4 `tests/test_flows.py`.
//!
//! Python also asserted that a private compatibility alias was the same function object as
//! `flows.truthy`. Rust has no equivalent public function-identity seam, so the exact executable
//! owner pins every observable spelling through the shared delivery pipeline instead.

use std::collections::BTreeMap;

use skit_application::{
    glob_expansion::GlobExpander,
    run_inputs::assemble_run_inputs,
    tokens::TokenContext,
    value_preparation::validate_form_value,
};
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};

#[derive(Debug)]
struct NoGlob;
impl GlobExpander for NoGlob {
    fn expand_piece(&self, piece: &str) -> Vec<String> {
        vec![piece.to_owned()]
    }
}

fn context() -> TokenContext {
    TokenContext {
        cwd: "/work".to_owned(),
        home: None,
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

#[test]
fn test_truthy_accepts_every_truthy_spelling() {
    let mut declaration = ParamDecl::new("v");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Bool;
    declaration.flag = "--v".to_owned();
    declaration.action = "store_true".to_owned();

    for spelling in ["true", "1", "yes", "y", "on", " TRUE ", "On"] {
        let values = BTreeMap::from([("v".to_owned(), spelling.to_owned())]);
        let assembly = assemble_run_inputs(
            std::slice::from_ref(&declaration),
            &values,
            &[],
            true,
            &context(),
            &NoGlob,
        )
        .unwrap();
        assert_eq!(
            assembly.args,
            ["--v"],
            "frozen truthy spelling {spelling:?} stopped firing the flag"
        );
    }

    for spelling in ["false", "0", "no", "n", "off", "", "garbage"] {
        let values = BTreeMap::from([("v".to_owned(), spelling.to_owned())]);
        let assembly = assemble_run_inputs(
            std::slice::from_ref(&declaration),
            &values,
            &[],
            true,
            &context(),
            &NoGlob,
        )
        .unwrap();
        assert!(
            assembly.args.is_empty(),
            "frozen falsey spelling {spelling:?} unexpectedly fired: {:?}",
            assembly.args
        );
    }
}

#[test]
fn test_type_error_messages_exact() {
    let mut int_field = ParamDecl::new("gap");
    int_field.parameter_type = ParameterType::Int;
    assert_eq!(
        validate_form_value(&int_field, "abc").unwrap_err().to_string(),
        "gap needs a whole number — you typed 'abc'."
    );

    let mut float_field = ParamDecl::new("r");
    float_field.prompt = "ratio".to_owned();
    float_field.parameter_type = ParameterType::Float;
    assert_eq!(
        validate_form_value(&float_field, "x").unwrap_err().to_string(),
        "ratio needs a number — you typed 'x'."
    );
    assert!(validate_form_value(&float_field, "1.5").is_ok());

    let mut bool_field = ParamDecl::new("b");
    bool_field.prompt = "fast".to_owned();
    bool_field.parameter_type = ParameterType::Bool;
    assert_eq!(
        validate_form_value(&bool_field, "maybe").unwrap_err().to_string(),
        "fast needs on or off — you typed 'maybe'."
    );
    assert!(validate_form_value(&bool_field, "yes").is_ok());

    let mut choice_field = ParamDecl::new("m");
    choice_field.prompt = "mode".to_owned();
    choice_field.parameter_type = ParameterType::Choice;
    choice_field.choices = vec!["a".to_owned(), "b".to_owned()];
    assert_eq!(
        validate_form_value(&choice_field, "z").unwrap_err().to_string(),
        "mode must be one of: a, b"
    );

    let mut required_field = ParamDecl::new("o");
    required_field.prompt = "output".to_owned();
    required_field.required = true;
    assert_eq!(
        validate_form_value(&required_field, "  ").unwrap_err().to_string(),
        "output is required."
    );
}
