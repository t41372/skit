//! Rust-additive strength checks for frozen flows contracts whose Python tests also include
//! implementation-specific helper identity assertions. These do not count toward exact-name parity.

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
    fn expand_piece(&self, piece: &str) -> Vec<String> { vec![piece.to_owned()] }
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
fn rust_additive_validate_choice_preserves_the_frozen_human_choice_list() {
    let mut mode = ParamDecl::new("mode");
    mode.parameter_type = ParameterType::Choice;
    mode.choices = vec!["a".to_owned(), "b".to_owned()];
    let error = validate_form_value(&mode, "zzz").unwrap_err();
    assert!(error.to_string().contains("mode"), "{error}");
    assert!(error.to_string().contains("a, b"), "the frozen choice list rendering changed: {error}");
}

#[test]
fn rust_additive_truthy_spellings_drive_the_same_store_true_flag_rule() {
    let mut flag = ParamDecl::new("v");
    flag.delivery = ParameterDelivery::Flag;
    flag.parameter_type = ParameterType::Bool;
    flag.flag = "--v".to_owned();
    flag.action = "store_true".to_owned();

    for spelling in ["true", "1", "yes", "y", "on", " TRUE ", "On"] {
        let values = BTreeMap::from([("v".to_owned(), spelling.to_owned())]);
        let assembly = assemble_run_inputs(&[flag.clone()], &values, &[], true, &context(), &NoGlob).unwrap();
        assert_eq!(assembly.args, ["--v"], "truthy spelling {spelling:?} stopped firing");
    }
    for spelling in ["false", "0", "no", "n", "off", "", "garbage"] {
        let values = BTreeMap::from([("v".to_owned(), spelling.to_owned())]);
        let assembly = assemble_run_inputs(&[flag.clone()], &values, &[], true, &context(), &NoGlob).unwrap();
        assert!(assembly.args.is_empty(), "falsey spelling {spelling:?} unexpectedly fired: {:?}", assembly.args);
    }
}
