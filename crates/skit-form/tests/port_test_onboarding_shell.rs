//! Public-API ports of Python v0.4 shell analyzer contracts.
//!
//! This target pins the add-time candidate projection only: plain literals and faithful terminal
//! reads are candidates; shell data reads and dynamic values are not; mutable constants are offered
//! conservatively via an explicit demotion rather than silently treated as stable configuration.

use skit_domain::parameters::{ParameterBinding, ParameterType, ParameterValue};
use skit_form::onboarding_plan;
use skit_language::DegradationReason;

fn plan(source: &str) -> skit_form::OnboardingPlan {
    onboarding_plan("shell", source)
}

fn candidate_names(source: &str) -> Vec<String> {
    plan(source)
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

#[test]
fn test_shell_literal_constants_infer_scalar_types_and_values() {
    let plan = plan("A=plain\nB=42\nC='raw text'\nD=\"double q\"\n");
    let by = plan
        .candidates
        .iter()
        .map(|candidate| (candidate.declaration.name.as_str(), &candidate.declaration))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(by["A"].parameter_type, ParameterType::Str);
    assert_eq!(
        by["A"].default,
        Some(ParameterValue::String("plain".to_owned()))
    );
    assert_eq!(by["B"].parameter_type, ParameterType::Int);
    assert_eq!(by["B"].default, Some(ParameterValue::Integer(42)));
    assert_eq!(
        by["C"].default,
        Some(ParameterValue::String("raw text".to_owned()))
    );
    assert_eq!(
        by["D"].default,
        Some(ParameterValue::String("double q".to_owned()))
    );
}

#[test]
fn test_shell_dynamic_or_empty_assignments_are_not_literal_candidates() {
    let source = concat!(
        "EMPTY=\nQUOTED_EMPTY=''\nARR=(1 2 3)\n",
        "CONCAT=a$B\nSUBBED=$(date)\nEXPANDED=${OTHER}\n",
    );
    assert!(candidate_names(source).is_empty());
}

#[test]
fn test_shell_private_leading_underscore_is_skipped() {
    assert_eq!(candidate_names("_HIDDEN=1\nSHOWN=2\n"), ["SHOWN"]);
}

#[test]
fn test_shell_const_last_write_wins_but_keeps_first_candidate_slot() {
    let plan = plan("X=1\nY=5\nX=2\n");
    assert_eq!(
        plan.candidates
            .iter()
            .map(|candidate| candidate.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["X", "Y"]
    );
    assert_eq!(
        plan.candidates[0].declaration.default,
        Some(ParameterValue::Integer(2))
    );
}

#[test]
fn test_shell_export_declare_and_typeset_literals_are_candidates_but_readonly_is_not() {
    let source = concat!(
        "export A=1\n",
        "declare -i B=2\n",
        "typeset C=three\n",
        "readonly LOCKED=1\n",
        "declare -r FROZEN=2\n",
        "typeset -rx ALSO=3\n",
        "OPEN=4\n",
    );
    assert_eq!(
        candidate_names(source)
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "A".to_owned(),
            "B".to_owned(),
            "C".to_owned(),
            "OPEN".to_owned(),
        ])
    );
}

#[test]
fn test_shell_all_four_default_expansion_operators_are_envdefault_candidates() {
    let plan = plan(": \"${A:-x}\"\n: \"${B:=y}\"\n: \"${C-z}\"\n: \"${D=w}\"\n");
    assert_eq!(
        plan.candidates
            .iter()
            .map(|candidate| candidate.declaration.name.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["A", "B", "C", "D"])
    );
    assert!(
        plan.candidates
            .iter()
            .all(|candidate| { candidate.declaration.binding == ParameterBinding::EnvDefault })
    );
    let a = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "A")
        .unwrap();
    assert_eq!(
        a.declaration.default,
        Some(ParameterValue::String("x".to_owned()))
    );
}

#[test]
fn test_shell_non_default_parameter_expansions_are_ignored() {
    assert!(candidate_names(": \"${VAR:?missing}\"\necho \"${#LIST}\"\n").is_empty());
}

#[test]
fn test_shell_envdefault_infers_int_float_text_and_empty_defaults() {
    let plan = plan(concat!(
        ": \"${PORT:-8080}\"\n",
        ": \"${RATIO:-1.5}\"\n",
        ": \"${NAME:-guest}\"\n",
        ": \"${OPT:-}\"\n",
    ));
    let by = plan
        .candidates
        .iter()
        .map(|candidate| (candidate.declaration.name.as_str(), &candidate.declaration))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(by["PORT"].parameter_type, ParameterType::Int);
    assert_eq!(by["PORT"].default, Some(ParameterValue::Integer(8080)));
    assert_eq!(by["RATIO"].parameter_type, ParameterType::Float);
    assert_eq!(by["RATIO"].default, Some(ParameterValue::Float(1.5)));
    assert_eq!(by["NAME"].parameter_type, ParameterType::Str);
    assert_eq!(
        by["OPT"].default,
        Some(ParameterValue::String(String::new()))
    );
}

#[test]
fn test_shell_envdefault_first_default_wins_and_subscript_is_ignored() {
    let plan = plan(concat!(
        "echo \"${MODE:-first}\"\n",
        "echo \"${MODE:-second}\"\n",
        "echo \"${ARR[0]:-x}\"\n",
    ));
    let [mode] = plan.candidates.as_slice() else {
        panic!("expected only MODE candidate: {plan:?}");
    };
    assert_eq!(mode.declaration.name, "MODE");
    assert_eq!(
        mode.declaration.default,
        Some(ParameterValue::String("first".to_owned()))
    );
}

#[test]
fn test_shell_plain_assignment_suppresses_later_envdefault_for_that_name_only() {
    let plan = plan(concat!(
        "PORT=8080\n",
        "echo \"${PORT:-9090}\"\n",
        "echo \"${MODE:-auto}\"\n",
    ));
    let by = plan
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.declaration.name.as_str(),
                candidate.declaration.binding,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(by["PORT"], ParameterBinding::Const);
    assert_eq!(by["MODE"], ParameterBinding::EnvDefault);
}

#[test]
fn test_shell_dynamic_assignment_suppresses_envdefault_without_becoming_a_const() {
    assert!(candidate_names("HOST=$(hostname)\necho \"${HOST:-local}\"\n").is_empty());
}

#[test]
fn test_shell_self_envdefault_idiom_is_not_suppressed() {
    let plan = plan("PORT=\"${PORT:-8080}\"\nNAME=${NAME:-guest}\n");
    assert!(
        plan.candidates
            .iter()
            .all(|candidate| { candidate.declaration.binding == ParameterBinding::EnvDefault })
    );
    assert_eq!(
        plan.candidates
            .iter()
            .map(|candidate| candidate.declaration.name.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["NAME", "PORT"])
    );
}

#[test]
fn test_shell_read_prompt_order_and_secret_flag_are_preserved() {
    let plan = plan(concat!(
        "read -p \"Name: \" NAME\n",
        "read -s -p \"PIN: \" PIN\n",
    ));
    let reads = plan
        .candidates
        .iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0].declaration.name, "input-1");
    assert_eq!(reads[0].declaration.order, 0);
    assert_eq!(reads[0].declaration.prompt, "Name: ");
    assert!(!reads[0].declaration.secret);
    assert_eq!(reads[1].declaration.name, "input-2");
    assert_eq!(reads[1].declaration.order, 1);
    assert_eq!(reads[1].declaration.prompt, "PIN: ");
    assert!(reads[1].declaration.secret);
}

#[test]
fn test_shell_multi_variable_read_shares_prompt_and_dynamic_prompt_becomes_empty() {
    let plan = plan(concat!(
        "read -p \"Two: \" FIRST LAST\n",
        "read -p \"$MSG\" V\n",
    ));
    let reads = plan
        .candidates
        .iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 3);
    assert_eq!(reads[0].declaration.prompt, "Two: ");
    assert_eq!(reads[1].declaration.prompt, "Two: ");
    assert_eq!(reads[2].declaration.prompt, "");
}

#[test]
fn test_shell_reframing_and_custom_ifs_reads_are_not_candidates() {
    for source in [
        "read -n 3 X\n",
        "read -N 5 X\n",
        "read -d : X\n",
        "read -n3 X\n",
        "IFS=: read A B\n",
        "IFS= read -r LINE\n",
    ] {
        assert!(
            plan(source)
                .candidates
                .iter()
                .all(|candidate| candidate.declaration.binding != ParameterBinding::Input),
            "{source:?}"
        );
    }
}

#[test]
fn test_shell_data_reads_from_pipe_or_stdin_redirection_are_excluded() {
    for source in [
        "cat f | while read -r line; do echo $line; done\n",
        "a | b | read Z\n",
        "while read -r x; do echo $x; done < f\n",
        "read -r x < input.txt\n",
        "read -r x <<< \"$data\"\n",
    ] {
        assert!(
            plan(source)
                .candidates
                .iter()
                .all(|candidate| candidate.declaration.binding != ParameterBinding::Input),
            "{source:?}"
        );
    }
}

#[test]
fn test_shell_read_as_pipe_head_or_with_stdout_redirect_remains_interactive() {
    for source in ["read X | cat\n", "read -r x > out.log\n"] {
        assert!(
            plan(source)
                .candidates
                .iter()
                .any(|candidate| { candidate.declaration.binding == ParameterBinding::Input })
        );
    }
}

#[test]
fn test_shell_mutated_constants_are_demoted_as_accumulators() {
    for source in [
        "N=0\nN+=1\n",
        "TOTAL=100\nTOTAL=$((TOTAL - 1))\n",
        "N=0\n((N++))\n",
        "N=0\n((N += 5))\n",
        "M=1\nlet M=M+1\n",
        "SUM=0\nfor i in 1 2; do SUM=$((SUM + i)); done\n",
    ] {
        let plan = plan(source);
        let candidate = plan
            .candidates
            .iter()
            .find(|candidate| candidate.declaration.binding == ParameterBinding::Const)
            .unwrap_or_else(|| panic!("expected a constant candidate for {source:?}: {plan:?}"));
        assert_eq!(
            candidate.demotion,
            Some(DegradationReason::Accumulator),
            "{source:?}"
        );
        assert!(!candidate.selected_by_default(), "{source:?}");
    }
}

#[test]
fn test_shell_read_only_arithmetic_use_does_not_demote_constant() {
    let plan = plan("N=3\n(( N > 5 )) && echo big\n");
    let [candidate] = plan.candidates.as_slice() else {
        panic!("expected one N candidate: {plan:?}");
    };
    assert_eq!(candidate.declaration.name, "N");
    assert_eq!(candidate.demotion, None);
    assert!(candidate.selected_by_default());
}
