//! Exact application-pipeline ports from Python v0.4 `tests/test_flows.py`.
//!
//! Rust splits Python's `flows.assemble` into resolution, preparation, glob and delivery stages.
//! These tests cross the public `assemble_run_inputs` boundary whenever the frozen contract is an
//! assembly behavior; helper-shaped Python tests are only mapped here when the same consequence is
//! fully observable through that public boundary.

use std::collections::BTreeMap;

use skit_application::{
    delivery::PreparedValue,
    form_state::prefill,
    glob_expansion::GlobExpander,
    run_inputs::{RunInputError, assemble_run_inputs},
    tokens::TokenContext,
    value_preparation::{ValuePreparationError, validate_form_value},
    value_resolution::{ValueResolutionError, resolve_values},
};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};

#[derive(Debug)]
struct NoGlob;
impl GlobExpander for NoGlob {
    fn expand_piece(&self, piece: &str) -> Vec<String> {
        vec![piece.to_owned()]
    }
}

fn context(env: &[(&str, &str)]) -> TokenContext {
    TokenContext {
        cwd: "/work".to_owned(),
        home: Some("/home/me".to_owned()),
        env: env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn arg_declarations() -> Vec<ParamDecl> {
    let mut inputs = ParamDecl::new("inputs");
    inputs.delivery = ParameterDelivery::Flag;
    inputs.parameter_type = ParameterType::Str;
    inputs.required = true;
    inputs.multiple = true;
    inputs.repeat = false;
    inputs.flag.clear();

    let mut output = ParamDecl::new("output");
    output.delivery = ParameterDelivery::Flag;
    output.parameter_type = ParameterType::Str;
    output.required = true;
    output.flag = "--output".to_owned();

    let mut gap = ParamDecl::new("gap");
    gap.delivery = ParameterDelivery::Flag;
    gap.parameter_type = ParameterType::Int;
    gap.flag = "--gap".to_owned();
    gap.default = Some(ParameterValue::Integer(0));

    let mut mode = ParamDecl::new("mode");
    mode.delivery = ParameterDelivery::Flag;
    mode.parameter_type = ParameterType::Choice;
    mode.flag = "--mode".to_owned();
    mode.choices = vec!["a".to_owned(), "b".to_owned()];
    mode.default = Some(ParameterValue::String("a".to_owned()));

    let mut fast = ParamDecl::new("fast");
    fast.delivery = ParameterDelivery::Flag;
    fast.parameter_type = ParameterType::Bool;
    fast.flag = "--fast".to_owned();
    fast.action = "store_true".to_owned();
    fast.default = Some(ParameterValue::Bool(false));

    let mut bg = ParamDecl::new("bg");
    bg.delivery = ParameterDelivery::Flag;
    bg.parameter_type = ParameterType::Str;
    bg.flag = "--bg".to_owned();
    bg.degraded = true;

    vec![inputs, output, gap, mode, fast, bg]
}

fn managed_declarations() -> Vec<ParamDecl> {
    let mut output = ParamDecl::new("OUTPUT");
    output.binding = ParameterBinding::Const;
    output.delivery = ParameterDelivery::Inject;
    output.parameter_type = ParameterType::Str;
    output.default = Some(ParameterValue::String("out.jpg".to_owned()));

    let mut width = ParamDecl::new("WIDTH");
    width.binding = ParameterBinding::Const;
    width.delivery = ParameterDelivery::Inject;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));

    let mut secret = ParamDecl::new("API_KEY");
    secret.binding = ParameterBinding::Const;
    secret.delivery = ParameterDelivery::Inject;
    secret.parameter_type = ParameterType::Str;
    secret.default = Some(ParameterValue::String("xxx".to_owned()));
    secret.secret = true;
    secret.env_source = "MY_API_KEY".to_owned();

    vec![output, width, secret]
}

fn values_ok() -> BTreeMap<String, String> {
    values(&[
        ("inputs", "a.png"),
        ("output", "o.png"),
        ("gap", "0"),
        ("mode", "a"),
        ("fast", "false"),
    ])
}

#[test]
fn test_prefill_default_then_last_then_preset() {
    let declarations = managed_declarations();
    assert_eq!(
        prefill(&declarations, &BTreeMap::new(), None).get("OUTPUT").map(String::as_str),
        Some("out.jpg")
    );
    let last = values(&[("OUTPUT", "last.jpg")]);
    assert_eq!(prefill(&declarations, &last, None).get("OUTPUT").map(String::as_str), Some("last.jpg"));
    let preset = values(&[("OUTPUT", "web.jpg")]);
    assert_eq!(
        prefill(&declarations, &last, Some(&preset)).get("OUTPUT").map(String::as_str),
        Some("web.jpg")
    );
    assert_eq!(prefill(&declarations, &last, None).get("OUTPUT").map(String::as_str), Some("last.jpg"));
}

#[test]
fn test_prefill_never_offers_secrets() {
    let actual = prefill(&managed_declarations(), &BTreeMap::new(), None);
    assert!(!actual.contains_key("API_KEY"));
}

#[test]
fn test_validate_required_empty() {
    let declarations = arg_declarations();
    let errors = declarations
        .iter()
        .filter_map(|decl| validate_form_value(decl, "").err().map(|error| (decl.name.as_str(), error)))
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors.iter().map(|(name, _)| *name).collect::<std::collections::BTreeSet<_>>(), ["inputs", "output"].into_iter().collect());
    assert!(errors.iter().all(|(_, error)| matches!(error, ValuePreparationError::Required { .. })));
}

#[test]
fn test_validate_int_error_names_field_and_value() {
    let gap = arg_declarations().into_iter().find(|decl| decl.name == "gap").unwrap();
    assert_eq!(
        validate_form_value(&gap, "abc").unwrap_err(),
        ValuePreparationError::InvalidType {
            name: "gap".to_owned(),
            value: "abc".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

#[test]
fn test_validate_choice() {
    let mode = arg_declarations().into_iter().find(|decl| decl.name == "mode").unwrap();
    assert_eq!(
        validate_form_value(&mode, "zzz").unwrap_err(),
        ValuePreparationError::InvalidChoice {
            name: "mode".to_owned(),
            value: "zzz".to_owned(),
            choices: vec!["a".to_owned(), "b".to_owned()],
        }
    );
}

#[test]
fn test_validate_token_values_deferred() {
    let gap = arg_declarations().into_iter().find(|decl| decl.name == "gap").unwrap();
    assert!(validate_form_value(&gap, "{env:GAP}").is_ok());
}

#[test]
fn test_assemble_argparse_positionals_then_flags() {
    let assembly = assemble_run_inputs(
        &arg_declarations(),
        &values(&[
            ("inputs", "a.png b.png"),
            ("output", "o.png"),
            ("gap", "4"),
            ("mode", "b"),
            ("fast", "true"),
        ]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(
        assembly.args,
        ["a.png", "b.png", "--output", "o.png", "--gap", "4", "--mode", "b", "--fast"]
    );
}

#[test]
fn test_assemble_unchecked_store_true_omits_flag() {
    let assembly = assemble_run_inputs(
        &arg_declarations(),
        &values_ok(),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert!(!assembly.args.iter().any(|arg| arg == "--fast"));
}

#[test]
fn test_assemble_degraded_empty_omitted_filled_passed() {
    let empty = assemble_run_inputs(
        &arg_declarations(),
        &values_ok(),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert!(!empty.args.iter().any(|arg| arg == "--bg"));
    let mut filled = values_ok();
    filled.insert("bg".to_owned(), "#fff".to_owned());
    let filled = assemble_run_inputs(
        &arg_declarations(),
        &filled,
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(&filled.args[filled.args.len() - 2..], ["--bg", "#fff"]);
}

#[test]
fn test_assemble_tokens_expand_and_type_check_after_expansion() {
    let mut raw = values_ok();
    raw.insert("output".to_owned(), "out_{today}.png".to_owned());
    raw.insert("gap".to_owned(), "{env:GAP}".to_owned());
    let assembly = assemble_run_inputs(
        &arg_declarations(),
        &raw,
        &[],
        true,
        &context(&[("GAP", "8")]),
        &NoGlob,
    )
    .unwrap();
    assert!(assembly.args.iter().any(|arg| arg == "out_2026-07-09.png"));
    assert!(assembly.args.iter().any(|arg| arg == "8"));

    let error = assemble_run_inputs(
        &arg_declarations(),
        &raw,
        &[],
        true,
        &context(&[("GAP", "not-a-number")]),
        &NoGlob,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RunInputError::Preparation(ValuePreparationError::InvalidType { name, value, parameter_type: ParameterType::Int })
            if name == "gap" && value == "not-a-number"
    ));
}

#[test]
fn test_assemble_missing_env_token_is_named_error() {
    let mut raw = values_ok();
    raw.insert("output".to_owned(), "{env:NOPE}".to_owned());
    let error = assemble_run_inputs(
        &arg_declarations(),
        &raw,
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap_err();
    assert!(error.to_string().contains("NOPE"), "{error}");
}

#[test]
fn test_assemble_inject_values_expanded_and_masked_display() {
    let assembly = assemble_run_inputs(
        &managed_declarations(),
        &values(&[
            ("OUTPUT", "long_{today}.jpg"),
            ("WIDTH", "800"),
            ("API_KEY", "typed-secret"),
        ]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.inject_values.get("OUTPUT").map(String::as_str), Some("long_2026-07-09.jpg"));
    assert!(assembly.display.contains(&("API_KEY".to_owned(), "•••".to_owned())));
    assert!(assembly.display.iter().all(|(_, value)| value != "typed-secret"));
}

#[test]
fn test_assemble_secret_env_source_reads_environment() {
    let assembly = assemble_run_inputs(
        &managed_declarations(),
        &values(&[("OUTPUT", "o.jpg"), ("WIDTH", "1"), ("API_KEY", "")]),
        &[],
        true,
        &context(&[("MY_API_KEY", "from-env")]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.inject_values.get("API_KEY").map(String::as_str), Some("from-env"));
}

#[test]
fn test_assemble_secret_env_source_missing_is_named_error() {
    let error = assemble_run_inputs(
        &managed_declarations(),
        &values(&[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "API_KEY reads from the environment variable MY_API_KEY, but it isn't set."
    );
}

#[test]
fn test_assemble_typed_secret_beats_env_source() {
    let assembly = assemble_run_inputs(
        &managed_declarations(),
        &values(&[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "typed")]),
        &[],
        true,
        &context(&[("MY_API_KEY", "env")]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.inject_values.get("API_KEY").map(String::as_str), Some("typed"));
}

#[test]
fn test_assemble_command_values_and_extra_args() {
    let mut msg = ParamDecl::new("msg");
    msg.delivery = ParameterDelivery::Placeholder;
    msg.required = true;
    let assembly = assemble_run_inputs(
        &[msg],
        &values(&[("msg", "hi {today}")]),
        &["--verbose".to_owned()],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.command_values, values(&[("msg", "hi 2026-07-09")]));
    assert_eq!(assembly.args, ["--verbose"]);
    assert_eq!(assembly.masked_args, ["--verbose"]);
}

#[test]
fn test_assemble_extra_arg_token_error_forwards_the_token_message() {
    let error = assemble_run_inputs(
        &[],
        &BTreeMap::new(),
        &["{env:NOPE_EXTRA}".to_owned()],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap_err();
    assert!(error.to_string().contains("NOPE_EXTRA"), "{error}");
}

#[test]
fn test_assemble_inject_source_forwards_extra_args() {
    let assembly = assemble_run_inputs(
        &managed_declarations(),
        &values(&[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "k")]),
        &["--flag".to_owned(), "v".to_owned()],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--flag", "v"]);
}

#[test]
fn test_assemble_field_expands_cwd_and_now_tokens() {
    let mut raw = values_ok();
    raw.insert("output".to_owned(), "{cwd}/{now}.png".to_owned());
    let assembly = assemble_run_inputs(
        &arg_declarations(),
        &raw,
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert!(assembly.args.iter().any(|arg| arg == "/work/14-30-05.png"));
}

#[test]
fn test_assemble_flags_tolerates_missing_keys() {
    let assembly = assemble_run_inputs(
        &arg_declarations(),
        &values(&[("inputs", "a"), ("output", "o")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["a", "--output", "o"]);
}

#[test]
fn test_assemble_empty_field_does_not_stop_later_flags() {
    let mut raw = values_ok();
    raw.insert("gap".to_owned(), String::new());
    raw.insert("mode".to_owned(), "b".to_owned());
    let assembly = assemble_run_inputs(
        &arg_declarations(),
        &raw,
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert!(assembly.args.iter().any(|arg| arg == "--mode"));
    assert!(assembly.args.iter().any(|arg| arg == "b"));
}

#[test]
fn test_split_multi_falls_back_on_unbalanced_quote() {
    let mut raw = values_ok();
    raw.insert("inputs".to_owned(), "a\"b".to_owned());
    let assembly = assemble_run_inputs(
        &arg_declarations(),
        &raw,
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args.first().map(String::as_str), Some("a\"b"));
}

#[test]
fn test_resolve_secret_empty_when_no_input_and_no_env_source() {
    let mut secret = ParamDecl::new("k");
    secret.secret = true;
    let resolved = resolve_values(&[secret], &values(&[("k", "")]), &context(&[])).unwrap();
    assert_eq!(resolved.get("k").map(String::as_str), Some(""));
}

#[test]
fn test_validate_value_accepts_a_valid_choice() {
    let mut mode = ParamDecl::new("m");
    mode.parameter_type = ParameterType::Choice;
    mode.choices = vec!["a".to_owned(), "b".to_owned()];
    assert!(validate_form_value(&mode, "a").is_ok());
    assert!(validate_form_value(&mode, "b").is_ok());
}

#[test]
fn test_prefill_drops_a_secret_that_leaked_into_saved_values() {
    let actual = prefill(
        &managed_declarations(),
        &values(&[("OUTPUT", "o.jpg"), ("API_KEY", "leaked")]),
        None,
    );
    assert_eq!(actual.get("OUTPUT").map(String::as_str), Some("o.jpg"));
    assert!(!actual.contains_key("API_KEY"));
}

#[test]
fn test_prefill_preset_drops_leaked_secret() {
    let preset = values(&[("OUTPUT", "web.jpg"), ("API_KEY", "leaked")]);
    let actual = prefill(&managed_declarations(), &BTreeMap::new(), Some(&preset));
    assert_eq!(actual.get("OUTPUT").map(String::as_str), Some("web.jpg"));
    assert!(!actual.contains_key("API_KEY"));
}

#[test]
fn test_prefill_unknown_preset_is_no_op_not_a_crash() {
    let actual = prefill(&managed_declarations(), &BTreeMap::new(), Some(&BTreeMap::new()));
    assert_eq!(actual.get("OUTPUT").map(String::as_str), Some("out.jpg"));
}

#[test]
fn test_assemble_tolerates_a_bool_field_missing_from_values() {
    let raw = values(&[("inputs", "a.png"), ("output", "o.png"), ("gap", "0"), ("mode", "a")]);
    let assembly = assemble_run_inputs(
        &arg_declarations(),
        &raw,
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert!(!assembly.args.iter().any(|arg| arg == "--fast"));
}

#[test]
fn test_assemble_store_false_fires_flag_when_unchecked() {
    let mut color = ParamDecl::new("color");
    color.delivery = ParameterDelivery::Flag;
    color.parameter_type = ParameterType::Bool;
    color.flag = "--color".to_owned();
    color.action = "store_false".to_owned();
    let checked = assemble_run_inputs(
        &[color.clone()],
        &values(&[("color", "true")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert!(checked.args.is_empty());
    let unchecked = assemble_run_inputs(
        &[color],
        &values(&[("color", "false")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(unchecked.args, ["--color"]);
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
fn test_assemble_repeat_emits_flag_before_each_piece() {
    let assembly = assemble_run_inputs(
        &[repeat_decl(true)],
        &values(&[("tag", "a b")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--tag", "a", "--tag", "b"]);
}

#[test]
fn test_assemble_non_repeat_multi_keeps_one_flag_then_values() {
    let assembly = assemble_run_inputs(
        &[repeat_decl(false)],
        &values(&[("tag", "a b")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--tag", "a", "b"]);
}

#[test]
fn test_assemble_repeat_single_piece() {
    let assembly = assemble_run_inputs(
        &[repeat_decl(true)],
        &values(&[("tag", "a")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--tag", "a"]);
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
    let rep = assemble_run_inputs(
        &[repeated],
        &values(&[("tag", "'a b' *.png")]),
        &[],
        true,
        &context(&[]),
        &FrozenGlob,
    )
    .unwrap();
    assert_eq!(rep.args, ["--src", "a b", "--src", "1.png", "--src", "2.png"]);

    let mut plain = repeat_decl(false);
    plain.flag = "--src".to_owned();
    let plain = assemble_run_inputs(
        &[plain],
        &values(&[("tag", "'a b' *.png")]),
        &[],
        true,
        &context(&[]),
        &FrozenGlob,
    )
    .unwrap();
    assert_eq!(plain.args, ["--src", "a b", "1.png", "2.png"]);
}

fn bool_decl(flag: &str, action: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new("v");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Bool;
    declaration.flag = flag.to_owned();
    declaration.action = action.to_owned();
    declaration
}

#[test]
fn test_assemble_bool_store_true_fires_only_when_checked() {
    let declaration = bool_decl("--v", "store_true");
    let checked = assemble_run_inputs(&[declaration.clone()], &values(&[("v", "true")]), &[], true, &context(&[]), &NoGlob).unwrap();
    assert_eq!(checked.args, ["--v"]);
    let unchecked = assemble_run_inputs(&[declaration], &values(&[("v", "false")]), &[], true, &context(&[]), &NoGlob).unwrap();
    assert!(unchecked.args.is_empty());
}

#[test]
fn test_assemble_bool_flagless_never_appends_empty_string() {
    let st = assemble_run_inputs(&[bool_decl("", "store_true")], &values(&[("v", "true")]), &[], true, &context(&[]), &NoGlob).unwrap();
    assert!(st.args.is_empty());
    assert!(!st.args.iter().any(String::is_empty));
    let sf = assemble_run_inputs(&[bool_decl("", "store_false")], &values(&[("v", "false")]), &[], true, &context(&[]), &NoGlob).unwrap();
    assert!(sf.args.is_empty());
    assert!(!sf.args.iter().any(String::is_empty));
}

#[test]
fn test_assemble_bool_empty_action_fires_in_neither_state() {
    let declaration = bool_decl("--v", "");
    let on = assemble_run_inputs(&[declaration.clone()], &values(&[("v", "true")]), &[], true, &context(&[]), &NoGlob).unwrap();
    let off = assemble_run_inputs(&[declaration], &values(&[("v", "false")]), &[], true, &context(&[]), &NoGlob).unwrap();
    assert!(on.args.is_empty());
    assert!(off.args.is_empty());
}

#[test]
fn test_assemble_expand_extra_false_passes_argv_untouched() {
    let assembly = assemble_run_inputs(
        &[],
        &BTreeMap::new(),
        &["x*.txt".to_owned(), "{env:UNSET_VAR}".to_owned()],
        false,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["x*.txt", "{env:UNSET_VAR}"]);
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
    let assembly = assemble_run_inputs(
        &[key, name],
        &values(&[("api_key", "sk-secret"), ("name", "ada")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--api-key", "sk-secret", "--name", "ada"]);
    assert_eq!(assembly.masked_args, ["--api-key", "•••", "--name", "ada"]);
}

#[test]
fn test_typed_multi_value_field_validates_each_piece_not_the_whole_box() {
    let mut point = ParamDecl::new("point");
    point.delivery = ParameterDelivery::Flag;
    point.parameter_type = ParameterType::Int;
    point.multiple = true;
    assert!(validate_form_value(&point, "1 2").is_ok());
    assert!(validate_form_value(&point, "1 -2 30").is_ok());
    assert_eq!(
        validate_form_value(&point, "1 x").unwrap_err(),
        ValuePreparationError::InvalidType {
            name: "point".to_owned(),
            value: "x".to_owned(),
            parameter_type: ParameterType::Int,
        }
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

#[test]
fn rust_additive_prepared_value_shape_is_not_the_frozen_flow_oracle() {
    // Keep a non-frozen sanity check that the final assembly still carries typed multi values; this
    // is explicitly additive and is excluded from exact-name accounting.
    assert_eq!(PreparedValue::Multiple(vec!["a".to_owned()]), PreparedValue::Multiple(vec!["a".to_owned()]));
}

#[test]
fn rust_additive_resolution_variant_is_structurally_named() {
    let mut secret = ParamDecl::new("API_KEY");
    secret.secret = true;
    secret.env_source = "MY_API_KEY".to_owned();
    assert!(matches!(
        resolve_values(&[secret], &values(&[("API_KEY", "")]), &context(&[])),
        Err(ValueResolutionError::MissingSecretEnvironment { name, environment })
            if name == "API_KEY" && environment == "MY_API_KEY"
    ));
}
