//! Exact public-surface ports of Python v0.4 `tests/test_shell_analyzer_mut.py`.
//!
//! Thirty contracts are observable through the published onboarding analysis: candidate spans,
//! declarations, demotion, candidate ordering, and uses_argv. Four Python tests inspect the private
//! `ReadFlags` helper shape beyond what any public candidate exposes; the companion manifest keeps
//! those explicitly blocked rather than replacing them with weaker assertions.

use skit_domain::parameters::{ParameterBinding, ParameterType};
use skit_form::{OnboardingCandidate, OnboardingPlan, onboarding_plan};
use skit_language::DegradationReason;

fn plan(source: &str) -> OnboardingPlan {
    onboarding_plan("shell", source)
}

fn candidates(source: &str) -> Vec<OnboardingCandidate> {
    plan(source).candidates
}

fn candidate(source: &str, name: &str) -> OnboardingCandidate {
    candidates(source)
        .into_iter()
        .find(|candidate| candidate.declaration.name == name)
        .unwrap_or_else(|| panic!("missing candidate {name} in {source:?}"))
}

fn names(source: &str) -> Vec<String> {
    candidates(source)
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

fn reads(source: &str) -> Vec<OnboardingCandidate> {
    candidates(source)
        .into_iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .collect()
}

fn demoted(source: &str) -> Vec<String> {
    candidates(source)
        .into_iter()
        .filter(|candidate| candidate.demotion.is_some())
        .map(|candidate| candidate.declaration.name)
        .collect()
}

fn bindings(source: &str, name: &str) -> Vec<ParameterBinding> {
    candidates(source)
        .into_iter()
        .filter(|candidate| candidate.declaration.name == name)
        .map(|candidate| candidate.declaration.binding)
        .collect()
}

#[test]
fn test_const_candidate_carries_exact_lineno_and_secret() {
    let actual = candidate("\nAPI_TOKEN=abcdef\n", "API_TOKEN");
    assert_eq!(actual.span.start_line, 2);
    assert!(actual.declaration.secret);
}

#[test]
fn test_const_non_secret_lineno_deeper_in_file() {
    let actual = candidate("X=1\nY=2\nCITY=Taipei\n", "CITY");
    assert_eq!(actual.span.start_line, 3);
    assert!(!actual.declaration.secret);
}

#[test]
fn test_envdefault_candidate_carries_exact_lineno_and_secret() {
    let actual = candidate("\n: \"${API_TOKEN:-x}\"\n", "API_TOKEN");
    assert_eq!(actual.span.start_line, 2);
    assert_eq!(actual.declaration.binding, ParameterBinding::EnvDefault);
    assert!(actual.declaration.secret);
}

#[test]
fn test_read_candidate_carries_exact_lineno_and_str_type() {
    let actual = reads("\nread NAME\n");
    let [field] = actual.as_slice() else {
        panic!("expected one read candidate: {actual:?}");
    };
    assert_eq!(field.span.start_line, 2);
    assert_eq!(field.declaration.parameter_type, ParameterType::Str);
}

#[test]
fn test_demoted_const_reports_accumulator_reason() {
    let actual = candidate("N=0\nN+=1\n", "N");
    assert_eq!(actual.demotion, Some(DegradationReason::Accumulator));
    assert!(!actual.selected_by_default());
}

#[test]
fn test_plain_loop_body_reassignment_is_the_only_demotion_path() {
    assert_eq!(demoted("X=1\nfor i in 1 2; do X=5; done\n"), ["X"]);
}

#[test]
fn test_plain_while_loop_reassignment_demotes() {
    assert_eq!(demoted("Y=1\nwhile true; do Y=9; done\n"), ["Y"]);
}

#[test]
fn test_bare_assigned_skips_subscript_then_records_later_clobber() {
    assert_eq!(
        bindings("arr[0]=1\nPORT=8080\necho \"${PORT:-9090}\"\n", "PORT"),
        [ParameterBinding::Const]
    );
}

#[test]
fn test_bare_assigned_self_read_continues_to_later_clobber() {
    assert_eq!(
        bindings(
            "PORT=${PORT:-8080}\nMODE=production\necho \"${MODE:-dev}\"\n",
            "MODE"
        ),
        [ParameterBinding::Const]
    );
}

#[test]
fn test_const_scan_skips_plus_equals_then_finds_later_const() {
    assert_eq!(names("N+=1\nCITY=Taipei\n"), ["CITY"]);
}

#[test]
fn test_envdefault_scan_skips_nondefault_operator_then_finds_default() {
    let actual = candidates("echo \"${VAR:?err}\"\necho \"${PORT:-8080}\"\n")
        .into_iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::EnvDefault)
        .map(|candidate| candidate.declaration.name)
        .collect::<Vec<_>>();
    assert_eq!(actual, ["PORT"]);
}

#[test]
fn test_envdefault_scan_skips_subscript_then_finds_scalar() {
    let actual = candidates("echo \"${ARR[0]:-x}\"\necho \"${PORT:-8080}\"\n")
        .into_iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::EnvDefault)
        .map(|candidate| candidate.declaration.name)
        .collect::<Vec<_>>();
    assert_eq!(actual, ["PORT"]);
}

#[test]
fn test_toplevel_skips_local_then_yields_later_const() {
    assert_eq!(names("local X=1\nCITY=Taipei\n"), ["CITY"]);
}

#[test]
fn test_injectable_reads_skip_reframing_then_include_normal_read() {
    assert_eq!(reads("read -n 3 CODE\nread NAME\n").len(), 1);
}

#[test]
fn test_dash_p_prompt_consumes_only_a_real_next_arg() {
    assert!(reads("read -p Enter\n").is_empty());
}

#[test]
fn test_dash_p_prompt_then_varname() {
    let actual = reads("read -p \"Question: \" ANSWER");
    let [field] = actual.as_slice() else {
        panic!("expected one read candidate: {actual:?}");
    };
    assert_eq!(field.declaration.prompt, "Question: ");
    assert_eq!(field.declaration.name, "input-1");
}

#[test]
fn test_command_prefix_without_read_is_not_a_read() {
    assert!(reads("command ls FILE\n").is_empty());
}

#[test]
fn test_builtin_prefix_without_read_is_not_a_read() {
    assert!(reads("builtin echo VALUE\n").is_empty());
}

#[test]
fn test_builtin_read_still_recognized() {
    let actual = reads("builtin read TOWN\n");
    let [field] = actual.as_slice() else {
        panic!("expected one read candidate: {actual:?}");
    };
    assert_eq!(field.declaration.name, "input-1");
}

#[test]
fn test_non_ifs_var_prefix_does_not_exclude_the_read() {
    assert_eq!(reads("FOO=bar read NAME\n").len(), 1);
}

#[test]
fn test_ifs_prefix_still_excludes_the_read() {
    assert!(reads("IFS=: read A B\n").is_empty());
}

#[test]
fn test_non_let_command_arguments_are_not_scanned_for_targets() {
    assert!(demoted("COUNT=0\necho COUNT=99\n").is_empty());
}

#[test]
fn test_let_command_targets_are_demoted() {
    assert_eq!(demoted("M=1\nlet M=M+1\n"), ["M"]);
}

#[test]
fn test_long_option_containing_r_is_not_treated_as_readonly() {
    assert_eq!(names("declare --xr Y=1\n"), ["Y"]);
}

#[test]
fn test_short_r_flag_is_readonly() {
    assert!(candidates("declare -r LOCKED=1\n").is_empty());
}

#[test]
fn test_uses_argv_at_alone() {
    assert!(plan("echo \"$@\"\n").uses_argv);
}

#[test]
fn test_uses_argv_star_alone() {
    assert!(plan("echo \"$*\"\n").uses_argv);
}

#[test]
fn test_uses_argv_hash_alone() {
    assert!(plan("echo \"$#\"\n").uses_argv);
}

#[test]
fn test_secret_flag_after_another_cluster_letter_terminates() {
    let actual = reads("read -rs X");
    let [field] = actual.as_slice() else {
        panic!("expected one read candidate: {actual:?}");
    };
    assert!(field.declaration.secret);
}

#[test]
fn test_unknown_flag_letter_after_another_terminates() {
    let actual = reads("read -re X");
    let [field] = actual.as_slice() else {
        panic!("expected one read candidate: {actual:?}");
    };
    assert_eq!(field.declaration.name, "input-1");
}
