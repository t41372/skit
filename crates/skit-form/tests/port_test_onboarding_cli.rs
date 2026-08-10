//! Public-API ports of Python v0.4 static argparse reader contracts (`tests/test_argspec.py`).
//!
//! The add lane consumes this projection before any frontend review. Static fields must stay in
//! source/runtime order; unsupported individual fields degrade locally, while subcommands or loop-
//! generated declarations degrade the whole CLI surface rather than pretending one form is sound.

use skit_domain::parameters::{ParameterType, ParameterValue};
use skit_form::{CliFormProjection, OnboardingParseState, onboarding_plan};
use skit_language::DegradationReason;

const STITCH: &str = r#"
import argparse
from pathlib import Path

def parse_color(value):
    return (0, 0, 0, 0)

def main():
    ap = argparse.ArgumentParser(description="stitch images")
    ap.add_argument("inputs", nargs="+", type=Path, help="input images")
    ap.add_argument("-o", "--output", type=Path, required=True, help="output path")
    ap.add_argument("-d", "--direction", choices=["vertical", "horizontal"],
                    default="vertical", help="direction")
    ap.add_argument("--gap", type=int, default=0, help="gap in px")
    ap.add_argument("--bg", type=parse_color, default=(255, 255, 255, 255), help="bg color")
    ap.add_argument("--match-size", action="store_true", help="unify sizes first")
    ap.add_argument("--align", choices=["start", "center", "end"], default="center")
    ap.add_argument("--no-sort", action="store_true", help="keep argv order")
    args = ap.parse_args()

if __name__ == "__main__":
    main()
"#;

fn static_fields(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    let plan = onboarding_plan("python", source);
    match plan.cli_surface {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, "argparse");
            fields
        }
        other => panic!("expected static argparse surface: {other:?}"),
    }
}

#[test]
fn test_no_argparse_surface_is_absent_even_when_the_module_is_only_imported() {
    for source in ["print('hi')\n", "import argparse\n"] {
        let plan = onboarding_plan("python", source);
        assert_eq!(plan.parse_state, OnboardingParseState::Parsed);
        assert!(matches!(plan.cli_surface, CliFormProjection::Absent));
    }
}

#[test]
fn test_syntax_error_does_not_publish_a_cli_surface() {
    let plan = onboarding_plan("python", "def broken(:\n");
    assert!(matches!(
        plan.parse_state,
        OnboardingParseState::SyntaxError { .. }
    ));
    assert!(matches!(plan.cli_surface, CliFormProjection::Absent));
    assert!(plan.cli_fields.is_empty());
}

#[test]
fn test_stitch_reads_eight_fields_in_source_order() {
    let fields = static_fields(STITCH);
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        [
            "inputs",
            "output",
            "direction",
            "gap",
            "bg",
            "match_size",
            "align",
            "no_sort",
        ]
    );
}

#[test]
fn test_stitch_positional_multiple_path_and_required_shape() {
    let fields = static_fields(STITCH);
    let inputs = &fields[0];
    assert!(inputs.flag.is_empty());
    assert!(inputs.multiple);
    assert!(!inputs.repeat);
    assert!(inputs.required);
    assert_eq!(inputs.parameter_type, ParameterType::Path);
    assert_eq!(inputs.help, "input images");
}

#[test]
fn test_argparse_path_type_spellings_are_modeled_without_degradation() {
    let source = concat!(
        "import argparse\nimport pathlib\n",
        "ap = argparse.ArgumentParser()\n",
        "ap.add_argument('--a', type=Path)\n",
        "ap.add_argument('--b', type=pathlib.Path)\n",
        "ap.add_argument('--c', type=argparse.FileType('w'))\n",
        "ap.add_argument('--d', type=FileType())\n",
        "ap.parse_args()\n",
    );
    let fields = static_fields(source);
    assert_eq!(
        fields.iter().map(|field| field.parameter_type).collect::<Vec<_>>(),
        [ParameterType::Path; 4]
    );
    assert!(fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_choices_beat_path_type_and_preserve_literal_default() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--m', choices=['a', 'b'], type=Path, default='a')\n",
        "ap.parse_args()\n",
    ));
    let [field] = fields.as_slice() else {
        panic!("expected one field");
    };
    assert_eq!(field.parameter_type, ParameterType::Choice);
    assert_eq!(field.choices, ["a", "b"]);
    assert_eq!(
        field.default,
        Some(ParameterValue::String("a".to_owned()))
    );
}

#[test]
fn test_stitch_required_long_flag_choices_int_and_bool_shapes() {
    let fields = static_fields(STITCH);

    assert_eq!(fields[1].flag, "--output");
    assert!(fields[1].required);
    assert_eq!(fields[1].parameter_type, ParameterType::Path);

    assert_eq!(fields[2].parameter_type, ParameterType::Choice);
    assert_eq!(fields[2].choices, ["vertical", "horizontal"]);
    assert_eq!(
        fields[2].default,
        Some(ParameterValue::String("vertical".to_owned()))
    );

    assert_eq!(fields[3].parameter_type, ParameterType::Int);
    assert_eq!(fields[3].default, Some(ParameterValue::Integer(0)));

    assert_eq!(fields[5].parameter_type, ParameterType::Bool);
    assert_eq!(fields[5].action, "store_true");
    assert_eq!(fields[5].default, Some(ParameterValue::Bool(false)));
    assert_eq!(fields[5].flag, "--match-size");
}

#[test]
fn test_custom_type_degrades_only_that_field_and_keeps_help() {
    let plan = onboarding_plan("python", STITCH);
    let fields = match &plan.cli_surface {
        CliFormProjection::Static { fields, .. } => fields,
        other => panic!("expected static surface: {other:?}"),
    };
    assert!(fields[4].degraded);
    assert_eq!(fields[4].help, "bg color");
    assert!(plan.cli_fields[4].degradation.is_some());
    assert!(plan
        .cli_fields
        .iter()
        .enumerate()
        .all(|(index, field)| index == 4 || field.degradation.is_none()));
}

#[test]
fn test_store_false_defaults_on() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--no-color', action='store_false')\n",
    ));
    let [field] = fields.as_slice() else {
        panic!("expected one field");
    };
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.action, "store_false");
    assert_eq!(field.default, Some(ParameterValue::Bool(true)));
}

#[test]
fn test_subparsers_degrade_the_whole_cli_surface() {
    let plan = onboarding_plan(
        "python",
        concat!(
            "import argparse\nap = argparse.ArgumentParser()\n",
            "sub = ap.add_subparsers()\n",
            "p = sub.add_parser('x')\np.add_argument('--y')\n",
        ),
    );
    assert!(matches!(
        plan.cli_surface,
        CliFormProjection::Dynamic {
            framework,
            reason: DegradationReason::Subcommands,
        } if framework == "argparse"
    ));
}

#[test]
fn test_loop_generated_arguments_degrade_the_whole_cli_surface() {
    let plan = onboarding_plan(
        "python",
        concat!(
            "import argparse\nap = argparse.ArgumentParser()\n",
            "for name in NAMES:\n    ap.add_argument(name)\n",
        ),
    );
    assert!(matches!(
        plan.cli_surface,
        CliFormProjection::Dynamic {
            framework,
            reason: DegradationReason::DynamicDeclaration,
        } if framework == "argparse"
    ));
}

#[test]
fn test_append_action_and_dynamic_choices_degrade_only_their_fields() {
    let plan = onboarding_plan(
        "python",
        concat!(
            "import argparse\nap = argparse.ArgumentParser()\n",
            "ap.add_argument('--tag', action='append')\n",
            "ap.add_argument('--mode', choices=MODES)\n",
            "ap.add_argument('--n', type=int)\n",
        ),
    );
    let fields = match &plan.cli_surface {
        CliFormProjection::Static { fields, .. } => fields,
        other => panic!("expected static surface: {other:?}"),
    };
    assert!(fields[0].degraded);
    assert!(fields[1].degraded);
    assert!(!fields[2].degraded);
    assert!(plan.cli_fields[0].degradation.is_some());
    assert!(plan.cli_fields[1].degradation.is_some());
    assert!(plan.cli_fields[2].degradation.is_none());
}

#[test]
fn test_help_and_version_actions_are_not_form_fields() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser(add_help=False)\n",
        "ap.add_argument('--version', action='version', version='1.0')\n",
        "ap.add_argument('--real')\n",
    ));
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["real"]
    );
}

#[test]
fn test_secret_name_dest_override_and_short_flag_are_preserved() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--api-key')\n",
        "ap.add_argument('--out-file', dest='target')\n",
        "ap.add_argument('-v')\n",
    ));
    assert!(fields[0].secret);
    assert_eq!(fields[0].name, "api_key");
    assert_eq!(fields[1].name, "target");
    assert_eq!(fields[1].flag, "--out-file");
    assert_eq!(fields[2].name, "v");
    assert_eq!(fields[2].flag, "-v");
}

#[test]
fn test_fixed_nargs_greater_than_one_is_multi_but_nargs_one_is_scalar() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--point', nargs=2, type=int)\n",
        "ap.add_argument('--one', nargs=1)\n",
    ));
    assert!(fields[0].multiple);
    assert!(!fields[0].repeat);
    assert!(!fields[1].multiple);
}

#[test]
fn test_non_literal_argument_name_skips_only_that_call() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument(FLAG_NAME)\n",
        "ap.add_argument('--real')\n",
    ));
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["real"]
    );
}

#[test]
fn test_computed_default_degrades_field_without_inventing_a_default() {
    let fields = static_fields(concat!(
        "import argparse\nap = argparse.ArgumentParser()\n",
        "ap.add_argument('--size', default=(1, 2))\n",
    ));
    let [field] = fields.as_slice() else {
        panic!("expected one field");
    };
    assert!(field.degraded);
    assert_eq!(field.default, None);
}
