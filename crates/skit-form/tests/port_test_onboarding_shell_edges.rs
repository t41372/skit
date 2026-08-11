//! Public-API ports of Python v0.4 shell analyzer mutation-kill edges.
//!
//! These cases pin scanner continuation and read-command gating without depending on private tree
//! walks: skipping one unsupported node must never abandon later valid candidates.

use skit_domain::parameters::ParameterBinding;
use skit_form::onboarding_plan;
use skit_language::DegradationReason;

fn plan(source: &str) -> skit_form::OnboardingPlan {
    onboarding_plan("shell", source)
}

fn names(source: &str) -> Vec<String> {
    plan(source)
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

fn input_candidates(source: &str) -> Vec<skit_form::OnboardingCandidate> {
    plan(source)
        .candidates
        .into_iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .collect()
}

#[test]
fn test_shell_argv_special_vars_are_detected_individually() {
    for source in ["echo \"$@\"\n", "echo \"$*\"\n", "echo \"$#\"\n"] {
        assert!(plan(source).uses_argv, "{source:?}");
    }
}

#[test]
fn test_shell_command_or_builtin_prefix_without_read_is_not_an_input() {
    for source in ["command ls FILE\n", "builtin echo VALUE\n"] {
        assert!(input_candidates(source).is_empty(), "{source:?}");
    }
    assert_eq!(input_candidates("builtin read TOWN\n").len(), 1);
}

#[test]
fn test_non_ifs_assignment_prefix_keeps_read_interactive_but_ifs_excludes_it() {
    assert_eq!(input_candidates("FOO=bar read NAME\n").len(), 1);
    assert!(input_candidates("IFS=: read A B\n").is_empty());
}

#[test]
fn test_plain_loop_body_reassignment_demotes_without_self_reference() {
    for source in [
        "X=1\nfor i in 1 2; do X=5; done\n",
        "Y=1\nwhile true; do Y=9; done\n",
    ] {
        let plan = plan(source);
        let candidate = plan
            .candidates
            .iter()
            .find(|candidate| candidate.declaration.binding == ParameterBinding::Const)
            .unwrap();
        assert_eq!(
            candidate.demotion,
            Some(DegradationReason::Accumulator),
            "{source:?}"
        );
    }
}

#[test]
fn test_skipped_plus_equals_still_allows_a_later_constant() {
    assert_eq!(names("N+=1\nCITY=Taipei\n"), ["CITY"]);
}

#[test]
fn test_nondefault_and_subscript_expansions_do_not_stop_later_envdefault_scan() {
    for source in [
        "echo \"${VAR:?err}\"\necho \"${PORT:-8080}\"\n",
        "echo \"${ARR[0]:-x}\"\necho \"${PORT:-8080}\"\n",
    ] {
        let envs = plan(source)
            .candidates
            .into_iter()
            .filter(|candidate| candidate.declaration.binding == ParameterBinding::EnvDefault)
            .map(|candidate| candidate.declaration.name)
            .collect::<Vec<_>>();
        assert_eq!(envs, ["PORT"], "{source:?}");
    }
}

#[test]
fn test_top_level_local_declaration_does_not_stop_later_constant_scan() {
    assert_eq!(names("local X=1\nCITY=Taipei\n"), ["CITY"]);
}

#[test]
fn test_reframing_read_is_skipped_without_dropping_a_later_normal_read() {
    let reads = input_candidates("read -n 3 CODE\nread NAME\n");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].declaration.name, "input-1");
}

#[test]
fn test_dash_p_consumes_the_last_argument_as_prompt_not_varname() {
    assert!(input_candidates("read -p Enter\n").is_empty());
    let reads = input_candidates("read -p \"Question: \" ANSWER\n");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].declaration.prompt, "Question: ");
}

#[test]
fn test_long_option_containing_r_is_not_readonly_but_short_r_is() {
    assert_eq!(names("declare --xr Y=1\n"), ["Y"]);
    assert!(names("declare -r LOCKED=1\n").is_empty());
}

#[test]
fn test_read_flag_clusters_terminate_and_keep_secret_or_varname() {
    let secret = input_candidates("read -rs X\n");
    assert_eq!(secret.len(), 1);
    assert!(secret[0].declaration.secret);

    let unknown = input_candidates("read -re X\n");
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].declaration.name, "input-1");
}

#[test]
fn test_subscript_assignment_is_skipped_without_hiding_later_const() {
    let source = "arr[0]=1\nPORT=8080\necho \"${PORT:-9090}\"\n";
    let port = plan(source)
        .candidates
        .into_iter()
        .filter(|candidate| candidate.declaration.name == "PORT")
        .collect::<Vec<_>>();
    assert_eq!(port.len(), 1);
    assert_eq!(port[0].declaration.binding, ParameterBinding::Const);
}
