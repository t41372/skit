//! Public-API ports of Python v0.4 Fish analyzer and argparse contracts.
//!
//! Fish has a deliberately narrow source analyzer: only the inherited-env default idiom is offered
//! as a managed source value. Its built-in `argparse` reader is projected independently as a static
//! CLI surface when every spec string is literal.

use skit_domain::parameters::{ParameterBinding, ParameterType, ParameterValue};
use skit_form::{CliFormProjection, onboarding_plan};
use skit_language::DegradationReason;

fn plan(source: &str) -> skit_form::OnboardingPlan {
    onboarding_plan("fish", source)
}

fn candidate<'a>(plan: &'a skit_form::OnboardingPlan, name: &str) -> &'a skit_form::OnboardingCandidate {
    plan.candidates
        .iter()
        .find(|candidate| candidate.declaration.name == name)
        .unwrap_or_else(|| panic!("missing candidate {name}: {plan:?}"))
}

fn static_fields(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    let plan = plan(source);
    match plan.cli_surface {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, "argparse");
            fields
        }
        other => panic!("expected static fish argparse surface: {other:?}"),
    }
}

#[test]
fn test_fish_one_line_envdefault_idiom_infers_int_and_env_target() {
    let plan = plan("set -q PORT; or set PORT 8080\n");
    let port = candidate(&plan, "PORT");
    assert_eq!(port.declaration.binding, ParameterBinding::EnvDefault);
    assert_eq!(port.declaration.parameter_type, ParameterType::Int);
    assert_eq!(port.declaration.default, Some(ParameterValue::Integer(8080)));
    assert_eq!(port.declaration.env_target, "PORT");
}

#[test]
fn test_fish_newline_or_float_and_string_defaults() {
    let plan = plan(concat!(
        "set -q PORT\nor set PORT 8080\n",
        "set -q RATE; or set RATE 2.5\n",
        "set -q REGION; or set REGION us-east-1\n",
    ));
    assert_eq!(candidate(&plan, "PORT").declaration.default, Some(ParameterValue::Integer(8080)));
    assert_eq!(candidate(&plan, "RATE").declaration.default, Some(ParameterValue::Float(2.5)));
    assert_eq!(
        candidate(&plan, "REGION").declaration.default,
        Some(ParameterValue::String("us-east-1".to_owned()))
    );
}

#[test]
fn test_fish_guarded_set_scope_flags_still_form_one_envdefault() {
    let plan = plan("set -q LOG; or set -gx LOG /var/log\n");
    assert_eq!(
        candidate(&plan, "LOG").declaration.default,
        Some(ParameterValue::String("/var/log".to_owned()))
    );
}

#[test]
fn test_fish_secret_name_is_marked_and_private_underscore_is_skipped() {
    let plan = plan(concat!(
        "set -q API_TOKEN; or set API_TOKEN x\n",
        "set -q _P; or set _P 1\n",
    ));
    assert!(candidate(&plan, "API_TOKEN").declaration.secret);
    assert!(plan.candidates.iter().all(|candidate| candidate.declaration.name != "_P"));
}

#[test]
fn test_fish_plain_clobber_suppresses_same_envdefault_regardless_of_order() {
    for source in [
        "set -q PORT; or set PORT 8080\nset PORT 9090\n",
        "set PORT 9090\nset -q PORT; or set PORT 8080\n",
    ] {
        assert!(plan(source).candidates.is_empty(), "{source:?}");
    }
}

#[test]
fn test_fish_unrelated_clobber_does_not_suppress_and_first_duplicate_default_wins() {
    let plan = plan(concat!(
        "set OTHER 1\n",
        "set -q PORT; or set PORT 8080\n",
        "set -q PORT; or set PORT 1\n",
    ));
    assert_eq!(plan.candidates.len(), 1);
    assert_eq!(
        candidate(&plan, "PORT").declaration.default,
        Some(ParameterValue::Integer(8080))
    );
}

#[test]
fn test_fish_malformed_or_mismatched_guarded_set_is_not_a_candidate() {
    for source in [
        "set -q PORT\necho done\n",
        "set -q; or set PORT 8080\n",
        "set -q PORT; or set PORT\n",
        "set -q PORT; or set OTHER 8080\n",
        "set -q X; set X 1\n",
    ] {
        assert!(plan(source).candidates.is_empty(), "{source:?}");
    }
}

#[test]
fn test_fish_idiom_inside_blocks_is_ignored_but_toplevel_after_block_is_detected() {
    for source in [
        "function f\n  set -q P; or set P 1\nend\n",
        "if true\n  set -q P; or set P 1\nend\n",
        "while true\n  set -q P; or set P 1\nend\n",
        "for x in 1\n  set -q P; or set P 1\nend\n",
        "begin\n  set -q P; or set P 1\nend\n",
        "switch $x\n  set -q P; or set P 1\nend\n",
    ] {
        assert!(plan(source).candidates.is_empty(), "{source:?}");
    }
    let outer = plan("function f\n  echo hi\nend\nset -q P; or set P 1\n");
    assert_eq!(candidate(&outer, "P").declaration.default, Some(ParameterValue::Integer(1)));
}

#[test]
fn test_fish_argv_and_self_location_hints_ignore_comments() {
    assert!(plan("echo $argv\n").uses_argv);
    assert!(!plan("# uses $argv here\necho hi\n").uses_argv);
    assert!(plan("set d (status dirname)\n").uses_self_location);
    assert!(plan("set f (status filename)\n").uses_self_location);
    assert!(!plan("echo hi\n").uses_self_location);
}

#[test]
fn test_fish_argparse_valueless_specs_are_bool_flags() {
    let fields = static_fields("argparse 'h/help' 'v/verbose' -- $argv\n");
    let by = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(by["help"].flag, "--help");
    assert_eq!(by["help"].parameter_type, ParameterType::Bool);
    assert_eq!(by["help"].action, "store_true");
    assert_eq!(by["help"].default, Some(ParameterValue::Bool(false)));
    assert_eq!(by["verbose"].parameter_type, ParameterType::Bool);
}

#[test]
fn test_fish_argparse_value_suffixes_and_repeat_grammar() {
    let fields = static_fields("argparse 'n/name=' 'r/retries=?' 'f/file=+' 'g/glob=*' -- $argv\n");
    let by = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(by["name"].parameter_type, ParameterType::Str);
    assert!(!by["name"].multiple);
    assert!(!by["name"].repeat);
    assert_eq!(by["retries"].parameter_type, ParameterType::Str);
    assert!(by["file"].multiple && by["file"].repeat);
    assert!(by["glob"].multiple && by["glob"].repeat);
}

#[test]
fn test_fish_argparse_long_short_dummy_short_and_numeric_hash_shapes() {
    let fields = static_fields("argparse 'dry-run' 'x' 'x-long' 'm#max' -- $argv\n");
    let by = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(by["dry-run"].flag, "--dry-run");
    assert_eq!(by["x"].flag, "-x");
    assert_eq!(by["long"].flag, "--long");
    assert_eq!(by["max"].flag, "--max");
    assert!(by["max"].degraded);
}

#[test]
fn test_fish_argparse_validator_is_stripped_and_secret_name_is_marked() {
    let fields = static_fields("argparse 'v/verbose!_check_it' 'token=' -- $argv\n");
    let by = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(!by["verbose"].degraded);
    assert_eq!(by["verbose"].parameter_type, ParameterType::Bool);
    assert!(by["token"].secret);
}

#[test]
fn test_fish_argparse_own_options_are_skipped_before_specs() {
    let fields = static_fields("argparse -n tool -x 'h,help' -i 'c/city=' -- $argv\n");
    assert!(fields.iter().all(|field| field.name != "tool"));
}

#[test]
fn test_fish_dynamic_argparse_spec_degrades_the_whole_surface() {
    let plan = plan("set spec 'x/foo'\nargparse $spec -- $argv\n");
    assert!(matches!(
        plan.cli_surface,
        CliFormProjection::Dynamic {
            framework,
            reason: DegradationReason::DynamicDeclaration,
        } if framework == "argparse"
    ));
}
