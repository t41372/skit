//! Exact flag/multi/masking ports from Python v0.4 `tests/test_flows.py`.

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
        home: Some("/home/me".to_owned()),
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}
fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}
fn bool_decl(flag: &str, action: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new("v");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Bool;
    declaration.flag = flag.to_owned();
    declaration.action = action.to_owned();
    declaration
}
fn repeat_decl(repeat: bool) -> ParamDecl {
    let mut tag = ParamDecl::new("tag");
    tag.delivery = ParameterDelivery::Flag;
    tag.flag = "--tag".to_owned();
    tag.multiple = true;
    tag.repeat = repeat;
    tag
}

#[test]
fn test_assemble_tolerates_a_bool_field_missing_from_values() {
    let declaration = bool_decl("--fast", "store_true");
    let asm = assemble_run_inputs(&[declaration], &BTreeMap::new(), &[], true, &context(), &NoGlob).unwrap();
    assert!(asm.args.is_empty());
}

#[test]
fn test_assemble_store_false_fires_flag_when_unchecked() {
    let declaration = bool_decl("--color", "store_false");
    let checked = assemble_run_inputs(&[declaration.clone()], &values(&[("v", "true")]), &[], true, &context(), &NoGlob).unwrap();
    assert!(checked.args.is_empty());
    let unchecked = assemble_run_inputs(&[declaration], &values(&[("v", "false")]), &[], true, &context(), &NoGlob).unwrap();
    assert_eq!(unchecked.args, ["--color"]);
}

#[test]
fn test_assemble_repeat_emits_flag_before_each_piece() {
    let asm = assemble_run_inputs(&[repeat_decl(true)], &values(&[("tag", "a b")]), &[], true, &context(), &NoGlob).unwrap();
    assert_eq!(asm.args, ["--tag", "a", "--tag", "b"]);
}

#[test]
fn test_assemble_non_repeat_multi_keeps_one_flag_then_values() {
    let asm = assemble_run_inputs(&[repeat_decl(false)], &values(&[("tag", "a b")]), &[], true, &context(), &NoGlob).unwrap();
    assert_eq!(asm.args, ["--tag", "a", "b"]);
}

#[test]
fn test_assemble_repeat_single_piece() {
    let asm = assemble_run_inputs(&[repeat_decl(true)], &values(&[("tag", "a")]), &[], true, &context(), &NoGlob).unwrap();
    assert_eq!(asm.args, ["--tag", "a"]);
}

#[test]
fn test_assemble_repeat_shares_shlex_and_glob_split_with_non_repeat() {
    #[derive(Debug)]
    struct FrozenGlob;
    impl GlobExpander for FrozenGlob {
        fn expand_piece(&self, piece: &str) -> Vec<String> {
            match piece {
                "*.png" => vec!["1.png".to_owned(), "2.png".to_owned()],
                other => vec![other.to_owned()],
            }
        }
    }
    let mut repeated = repeat_decl(true);
    repeated.flag = "--src".to_owned();
    let rep = assemble_run_inputs(&[repeated], &values(&[("tag", "'a b' *.png")]), &[], true, &context(), &FrozenGlob).unwrap();
    assert_eq!(rep.args, ["--src", "a b", "--src", "1.png", "--src", "2.png"]);

    let mut plain = repeat_decl(false);
    plain.flag = "--src".to_owned();
    let plain = assemble_run_inputs(&[plain], &values(&[("tag", "'a b' *.png")]), &[], true, &context(), &FrozenGlob).unwrap();
    assert_eq!(plain.args, ["--src", "a b", "1.png", "2.png"]);
}

#[test]
fn test_assemble_bool_store_true_fires_only_when_checked() {
    let declaration = bool_decl("--v", "store_true");
    let checked = assemble_run_inputs(&[declaration.clone()], &values(&[("v", "true")]), &[], true, &context(), &NoGlob).unwrap();
    assert_eq!(checked.args, ["--v"]);
    let unchecked = assemble_run_inputs(&[declaration], &values(&[("v", "false")]), &[], true, &context(), &NoGlob).unwrap();
    assert!(unchecked.args.is_empty());
}

#[test]
fn test_assemble_bool_flagless_never_appends_empty_string() {
    let st = assemble_run_inputs(&[bool_decl("", "store_true")], &values(&[("v", "true")]), &[], true, &context(), &NoGlob).unwrap();
    assert!(st.args.is_empty());
    assert!(!st.args.iter().any(String::is_empty));
    let sf = assemble_run_inputs(&[bool_decl("", "store_false")], &values(&[("v", "false")]), &[], true, &context(), &NoGlob).unwrap();
    assert!(sf.args.is_empty());
    assert!(!sf.args.iter().any(String::is_empty));
}

#[test]
fn test_assemble_bool_empty_action_fires_in_neither_state() {
    let declaration = bool_decl("--v", "");
    let on = assemble_run_inputs(&[declaration.clone()], &values(&[("v", "true")]), &[], true, &context(), &NoGlob).unwrap();
    let off = assemble_run_inputs(&[declaration], &values(&[("v", "false")]), &[], true, &context(), &NoGlob).unwrap();
    assert!(on.args.is_empty());
    assert!(off.args.is_empty());
}

#[test]
fn test_assemble_expand_extra_false_passes_argv_untouched() {
    let asm = assemble_run_inputs(&[], &BTreeMap::new(), &["x*.txt".to_owned(), "{env:UNSET_VAR}".to_owned()], false, &context(), &NoGlob).unwrap();
    assert_eq!(asm.args, ["x*.txt", "{env:UNSET_VAR}"]);
}

#[test]
fn test_masked_args_hide_flag_source_secret_values() {
    let mut key = ParamDecl::new("api_key");
    key.delivery = ParameterDelivery::Flag;
    key.flag = "--api-key".to_owned();
    key.secret = true;
    let mut name = ParamDecl::new("name");
    name.delivery = ParameterDelivery::Flag;
    name.flag = "--name".to_owned();
    let asm = assemble_run_inputs(&[key, name], &values(&[("api_key", "sk-secret"), ("name", "ada")]), &[], true, &context(), &NoGlob).unwrap();
    assert_eq!(asm.args, ["--api-key", "sk-secret", "--name", "ada"]);
    assert_eq!(asm.masked_args, ["--api-key", "•••", "--name", "ada"]);
}

#[test]
fn test_typed_multi_value_field_validates_each_piece_not_the_whole_box() {
    let mut point = ParamDecl::new("point");
    point.delivery = ParameterDelivery::Flag;
    point.parameter_type = ParameterType::Int;
    point.multiple = true;
    assert!(validate_form_value(&point, "1 2").is_ok());
    assert!(validate_form_value(&point, "1 -2 30").is_ok());
    let error = validate_form_value(&point, "1 x").unwrap_err();
    assert!(error.to_string().contains("whole number"), "{error}");
    assert!(
        error.to_string().contains("'1 x'"),
        "the frozen error must quote the user's whole multi-value box, not only the failing piece: {error}"
    );
}

#[test]
fn test_single_value_field_still_validates_the_whole_string() {
    let mut n = ParamDecl::new("n");
    n.delivery = ParameterDelivery::Flag;
    n.parameter_type = ParameterType::Int;
    assert!(validate_form_value(&n, "1 2").is_err());
    assert!(validate_form_value(&n, "12").is_ok());
}
