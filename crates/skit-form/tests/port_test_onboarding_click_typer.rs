//! Public-API ports of Python v0.4 Click/Typer static-reader contracts.
//!
//! These assertions pin only frontend-neutral onboarding facts. A reader may degrade one field when
//! its exact runtime grammar is not representable; multi-command surfaces degrade as a whole.

use skit_domain::parameters::{ParameterType, ParameterValue};
use skit_form::{CliFormProjection, onboarding_plan};
use skit_language::DegradationReason;

const CLICK_SCRIPT: &str = r#"
import click

@click.command()
@click.option("--output", "-o", required=True, help="output path")
@click.option("--gap", type=int, default=0)
@click.option("--mode", type=click.Choice(["a", "b"]), default="a")
@click.option("--fast", is_flag=True)
@click.argument("inputs", nargs=-1)
def main(output, gap, mode, fast, inputs):
    pass
"#;

const TYPER_SCRIPT: &str = r#"
import typer

app = typer.Typer()

@app.command()
def main(
    inputs: str = typer.Argument(...),
    output: str = typer.Option(..., "--output", "-o", help="output path"),
    gap: int = typer.Option(0),
    fast: bool = typer.Option(False, "--fast"),
    label: str = "x",
):
    pass
"#;

fn static_fields(source: &str, expected_framework: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    let plan = onboarding_plan("python", source);
    match plan.cli_surface {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, expected_framework);
            fields
        }
        other => panic!("expected static {expected_framework} surface: {other:?}"),
    }
}

#[test]
fn test_click_fields_follow_bottom_up_decorator_runtime_order() {
    let fields = static_fields(CLICK_SCRIPT, "click");
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["inputs", "fast", "mode", "gap", "output"]
    );
}

#[test]
fn test_click_variadic_argument_is_optional_multiple_positional_not_repeated_flag() {
    let fields = static_fields(CLICK_SCRIPT, "click");
    let inputs = &fields[0];
    assert!(inputs.flag.is_empty());
    assert!(inputs.multiple);
    assert!(!inputs.required);
    assert!(!inputs.repeat);
}

#[test]
fn test_click_flag_choice_int_and_required_option_shapes() {
    let fields = static_fields(CLICK_SCRIPT, "click");
    let by = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(by["fast"].parameter_type, ParameterType::Bool);
    assert_eq!(by["fast"].action, "store_true");
    assert_eq!(by["fast"].default, Some(ParameterValue::Bool(false)));

    assert_eq!(by["mode"].parameter_type, ParameterType::Choice);
    assert_eq!(by["mode"].choices, ["a", "b"]);
    assert_eq!(
        by["mode"].default,
        Some(ParameterValue::String("a".to_owned()))
    );

    assert_eq!(by["gap"].parameter_type, ParameterType::Int);
    assert_eq!(by["gap"].default, Some(ParameterValue::Integer(0)));

    assert!(by["output"].required);
    assert_eq!(by["output"].flag, "--output");
    assert_eq!(by["output"].help, "output path");
}

#[test]
fn test_click_plain_argument_is_required() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.argument('name')\ndef m(name): pass\n",
        "click",
    );
    assert!(fields[0].required);
    assert!(fields[0].flag.is_empty());
}

#[test]
fn test_click_group_degrades_as_subcommands() {
    let plan = onboarding_plan(
        "python",
        concat!(
            "import click\n@click.group()\ndef cli(): pass\n",
            "@click.command()\n@click.option('--x')\ndef sub(x): pass\n",
        ),
    );
    assert!(matches!(
        plan.cli_surface,
        CliFormProjection::Dynamic {
            framework,
            reason: DegradationReason::Subcommands,
        } if framework == "click"
    ));
}

#[test]
fn test_click_count_and_non_choice_callable_degrade_only_the_field() {
    let plan = onboarding_plan(
        "python",
        concat!(
            "import click\n@click.command()\n",
            "@click.option('-v', '--verbose', count=True)\n",
            "@click.option('--c', type=Wrapper(['a', 'b']))\n",
            "@click.option('--ok', type=int)\n",
            "def m(verbose, c, ok): pass\n",
        ),
    );
    let fields = match &plan.cli_surface {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, "click");
            fields
        }
        other => panic!("expected static click: {other:?}"),
    };
    assert!(fields[0].degraded);
    assert!(fields[1].degraded);
    assert!(!fields[2].degraded);
}

#[test]
fn test_click_multiple_option_repeats_the_flag() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--tag', multiple=True)\ndef m(tag): pass\n",
        "click",
    );
    let [tag] = fields.as_slice() else {
        panic!("expected one field");
    };
    assert!(tag.multiple);
    assert!(tag.repeat);
}

#[test]
fn test_click_multiple_with_fixed_nargs_greater_than_one_is_not_modeled() {
    let fields = static_fields(
        concat!(
            "import click\n@click.command()\n",
            "@click.option('--point', nargs=2, type=int, multiple=True)\n",
            "@click.option('--tag', multiple=True)\n",
            "def main(point, tag): pass\n",
        ),
        "click",
    );
    assert!(fields.iter().all(|field| field.name != "point"));
    assert!(fields.iter().any(|field| field.name == "tag"));
}

#[test]
fn test_click_fixed_nargs_is_one_flag_many_values() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--pair', nargs=2)\ndef main(pair): pass\n",
        "click",
    );
    let [pair] = fields.as_slice() else {
        panic!("expected one pair field");
    };
    assert!(pair.multiple);
    assert!(!pair.repeat);
}

#[test]
fn test_click_secret_precheck_and_short_flag_help_are_preserved() {
    let fields = static_fields(
        concat!(
            "import click\n@click.command()\n",
            "@click.option('--api-key')\n",
            "@click.option('-v', help='verbosity')\n",
            "def m(api_key, v): pass\n",
        ),
        "click",
    );
    let by = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(by["api_key"].secret);
    assert_eq!(by["v"].flag, "-v");
    assert_eq!(by["v"].help, "verbosity");
}

#[test]
fn test_click_flag_defaulting_on_degrades_instead_of_inventing_a_negative_flag() {
    let fields = static_fields(
        concat!(
            "import click\n@click.command()\n",
            "@click.option('--color', is_flag=True, default=True)\n",
            "def m(color): pass\n",
        ),
        "click",
    );
    let [color] = fields.as_slice() else {
        panic!("expected one color field");
    };
    assert!(color.degraded);
    assert!(color.action.is_empty());
}

#[test]
fn test_typer_signature_order_and_core_field_shapes() {
    let fields = static_fields(TYPER_SCRIPT, "typer");
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["inputs", "output", "gap", "fast", "label"]
    );
    let by = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert!(by["inputs"].flag.is_empty());
    assert!(by["inputs"].required);
    assert_eq!(by["output"].flag, "--output");
    assert!(by["output"].required);
    assert_eq!(by["output"].help, "output path");
    assert_eq!(by["gap"].parameter_type, ParameterType::Int);
    assert_eq!(by["gap"].default, Some(ParameterValue::Integer(0)));
    assert_eq!(by["gap"].flag, "--gap");
    assert_eq!(by["fast"].parameter_type, ParameterType::Bool);
    assert_eq!(by["fast"].action, "store_true");
    assert_eq!(
        by["label"].default,
        Some(ParameterValue::String("x".to_owned()))
    );
    assert_eq!(by["label"].flag, "--label");
}

#[test]
fn test_typer_run_pattern_reads_the_target_function() {
    let fields = static_fields(
        "import typer\n\ndef main(n: int = 3):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    let [field] = fields.as_slice() else {
        panic!("expected one field");
    };
    assert_eq!(field.name, "n");
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert_eq!(field.default, Some(ParameterValue::Integer(3)));
}

#[test]
fn test_typer_bool_default_true_degrades_not_guesses() {
    let fields = static_fields(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    let [field] = fields.as_slice() else {
        panic!("expected one field");
    };
    assert!(field.degraded);
    assert!(field.action.is_empty());
}

#[test]
fn test_typer_underscored_parameter_uses_kebab_flag() {
    let fields = static_fields(
        "import typer\n\ndef main(max_size: int = 1):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(fields[0].flag, "--max-size");
}

#[test]
fn test_typer_two_commands_degrade_as_subcommands() {
    let plan = onboarding_plan(
        "python",
        concat!(
            "import typer\napp = typer.Typer()\n",
            "@app.command()\ndef a(x: int = 1): pass\n",
            "@app.command()\ndef b(y: int = 2): pass\n",
        ),
    );
    assert!(matches!(
        plan.cli_surface,
        CliFormProjection::Dynamic {
            framework,
            reason: DegradationReason::Subcommands,
        } if framework == "typer"
    ));
}

#[test]
fn test_argparse_wins_when_multiple_cli_frameworks_are_imported() {
    let fields = static_fields(
        concat!(
            "import argparse\nimport click\n",
            "ap = argparse.ArgumentParser()\nap.add_argument('--x')\nap.parse_args()\n",
        ),
        "argparse",
    );
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["x"]
    );
}

#[test]
fn test_typer_bare_required_param_and_unannotated_default_shapes() {
    let required = static_fields(
        "import typer\n\ndef main(n: int): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(required[0].flag.is_empty());
    assert!(required[0].required);
    assert_eq!(required[0].parameter_type, ParameterType::Int);
    assert!(!required[0].degraded);

    let plain = static_fields(
        "import typer\n\ndef main(x=1): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(plain[0].parameter_type, ParameterType::Str);
    assert!(!plain[0].degraded);
    assert_eq!(plain[0].default, Some(ParameterValue::Integer(1)));
}

#[test]
fn test_typer_unmodelable_annotation_degrades_and_none_option_default_is_clean() {
    let degraded = static_fields(
        "import typer\n\ndef main(xs: list = None): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(degraded[0].degraded);

    let clean = static_fields(
        "import typer\n\ndef main(x: str = typer.Option(None, '--x')): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(!clean[0].degraded);
    assert_eq!(clean[0].default, None);
    assert!(!clean[0].required);
}

#[test]
fn test_typer_secret_name_precheck() {
    let fields = static_fields(
        "import typer\n\ndef main(api_token: str = ''): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(fields[0].secret);
}
