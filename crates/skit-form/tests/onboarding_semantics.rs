use skit_form::{CliFormProjection, OnboardingParseState, cli_form_projection, onboarding_plan};
use skit_language::DegradationReason;

#[test]
fn one_python_parse_preserves_every_onboarding_signal_and_candidate_identity() {
    let source = concat!(
        "import argparse\n",
        "import sys\n",
        "COUNT = 0\n",
        "COUNT += 1\n",
        "OUTPUT = 'result.png'\n",
        "p = argparse.ArgumentParser()\n",
        "p.add_argument('--count', type=int)\n",
        "print('input.csv', sys.argv, __file__)\n",
    );

    let plan = onboarding_plan("python", source);

    assert_eq!(plan.parse_state, OnboardingParseState::Parsed);
    assert_eq!(plan.frameworks, ["argparse"]);
    assert!(plan.uses_argv);
    assert_eq!(plan.filename_literals, ["input.csv"]);
    assert!(plan.uses_self_location);
    assert!(plan.uses_cli_framework());
    assert!(matches!(
        plan.cli_surface,
        CliFormProjection::Static { ref framework, ref fields }
            if framework == "argparse"
                && fields.len() == 1
                && fields[0].name == "count"
    ));
    assert_eq!(plan.cli_fields.len(), 1);
    assert_eq!(plan.cli_fields[0].identity.key, "count");
    assert_eq!(plan.cli_fields[0].degradation, None);
    assert_eq!(
        &source[plan.cli_fields[0].span.start..plan.cli_fields[0].span.end],
        "p.add_argument('--count', type=int)"
    );

    let count = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "COUNT")
        .expect("COUNT candidate");
    assert_eq!(count.identity.key, "COUNT");
    assert_eq!(&source[count.span.start..count.span.end], "COUNT = 0");
    assert_eq!(count.demotion, Some(DegradationReason::Accumulator));
    assert!(!count.selected_by_default());

    let output = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "OUTPUT")
        .expect("OUTPUT candidate");
    assert_eq!(output.demotion, None);
    assert!(output.selected_by_default());
}

#[test]
fn onboarding_keeps_absent_static_zero_dynamic_and_parse_failure_distinct() {
    assert!(matches!(
        onboarding_plan("python", "print('plain')\n").cli_surface,
        CliFormProjection::Absent
    ));

    let zero = onboarding_plan("python", "p.add_argument('--help', action='help')\n");
    assert!(matches!(
        zero.cli_surface,
        CliFormProjection::Static { ref framework, ref fields }
            if framework == "argparse" && fields.is_empty()
    ));

    let dynamic = onboarding_plan("python", "p.add_argument('--x')\np.add_subparsers()\n");
    assert!(matches!(
        dynamic.cli_surface,
        CliFormProjection::Dynamic {
            ref framework,
            reason: DegradationReason::Subcommands,
        } if framework == "argparse"
    ));

    let invalid = onboarding_plan("python", "def broken(:\n");
    assert!(matches!(
        invalid.parse_state,
        OnboardingParseState::SyntaxError {
            line: Some(1),
            column: Some(_),
        }
    ));
    assert!(invalid.candidates.is_empty());
    assert!(matches!(invalid.cli_surface, CliFormProjection::Absent));

    let unavailable = onboarding_plan("ruby", "puts 'ok'\n");
    assert_eq!(
        unavailable.parse_state,
        OnboardingParseState::ParserUnavailable
    );

    let degraded = onboarding_plan("python", "p.add_argument('--value', type=factory())\n");
    assert_eq!(
        degraded.cli_fields[0].degradation,
        Some(DegradationReason::DynamicType)
    );
    assert!(degraded.cli_fields[0].declaration.degraded);
}

#[test]
fn only_a_nonempty_static_reader_replaces_the_source_candidate_offer() {
    let modeled = onboarding_plan("python", "VALUE = 1\np.add_argument('--name')\n");
    assert_eq!(modeled.candidates.len(), 1);
    assert!(modeled.offered_candidates().is_empty());

    let zero = onboarding_plan(
        "python",
        "VALUE = 1\np.add_argument('--help', action='help')\n",
    );
    assert_eq!(zero.offered_candidates().len(), 1);

    let dynamic = onboarding_plan(
        "python",
        "VALUE = 1\np.add_argument('--name')\np.add_subparsers()\n",
    );
    assert_eq!(dynamic.offered_candidates().len(), 1);

    let absent = onboarding_plan("python", "VALUE = 1\n");
    assert_eq!(absent.offered_candidates().len(), 1);
}

#[test]
fn every_parser_adapter_keeps_absent_static_zero_and_dynamic_states_typed() {
    assert!(matches!(
        cli_form_projection("shell", "echo ok\n"),
        CliFormProjection::Absent
    ));
    assert!(matches!(
        cli_form_projection("powershell", "param()\n"),
        CliFormProjection::Static { framework, fields }
            if framework == "param" && fields.is_empty()
    ));
    assert!(matches!(
        cli_form_projection("js", "parseArgs({ options: spread });\n"),
        CliFormProjection::Dynamic {
            framework,
            reason: DegradationReason::DynamicDeclaration,
        } if framework == "parseArgs"
    ));
    assert!(matches!(
        cli_form_projection("fish", "argparse 'v/verbose' -- $argv\n"),
        CliFormProjection::Static { framework, fields }
            if framework == "argparse" && fields.len() == 1
    ));
}
