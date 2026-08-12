//! Exact public-surface ports of the 66 executable contracts in Python v0.4
//! `tests/test_argspec_click_typer.py`.
//!
//! Python oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//! The one Python-private `_decorator_name` helper contract is deliberately absent and guarded as
//! blocked by a manifest. These tests use the real parser-owned `cli_form_projection`.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_form::{CliFormProjection, cli_form_projection};
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

const ANNOTATED_SCRIPT: &str = r#"
import typer
from typing import Annotated

app = typer.Typer()

@app.command()
def main(
    name: Annotated[str, typer.Argument(help="who")],
    count: Annotated[int, typer.Option(help="how many")] = 3,
    mode: Annotated[str, typer.Option("-m", "--mode")] = "fast",
    fast: Annotated[bool, typer.Option()] = False,
):
    pass
"#;

fn static_fields(source: &str, framework: &str) -> Vec<ParamDecl> {
    match cli_form_projection("python", source) {
        CliFormProjection::Static {
            framework: actual,
            fields,
        } => {
            assert_eq!(actual, framework, "source:\n{source}");
            fields
        }
        other => panic!("expected static {framework} surface, got {other:?}\nsource:\n{source}"),
    }
}

fn one(source: &str, framework: &str) -> ParamDecl {
    let fields = static_fields(source, framework);
    let [field] = fields.as_slice() else {
        panic!("expected one {framework} field, got {fields:?}\nsource:\n{source}");
    };
    field.clone()
}

fn by_name(fields: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

fn assert_dynamic(source: &str, framework: &str) {
    assert!(matches!(
        cli_form_projection("python", source),
        CliFormProjection::Dynamic {
            framework: actual,
            reason: DegradationReason::Subcommands,
        } if actual == framework
    ));
}

#[test]
fn test_click_fields_bottom_up_order_matches_runtime() {
    let fields = static_fields(CLICK_SCRIPT, "click");
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["inputs", "fast", "mode", "gap", "output"]
    );
}

#[test]
fn test_click_argument_variadic_is_multiple_not_required() {
    let fields = static_fields(CLICK_SCRIPT, "click");
    let input = &fields[0];
    assert_eq!(input.flag, "");
    assert!(input.multiple);
    assert!(!input.required);
    assert!(!input.repeat);
}

#[test]
fn test_click_is_flag_choice_int_and_required() {
    let fields = static_fields(CLICK_SCRIPT, "click");
    let by = by_name(&fields);
    assert_eq!(by["fast"].parameter_type, ParameterType::Bool);
    assert_eq!(by["fast"].action, "store_true");
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
    let field = one(
        "import click\n@click.command()\n@click.argument('name')\ndef m(name): pass\n",
        "click",
    );
    assert!(field.required);
}

#[test]
fn test_click_group_degrades_as_subcommands() {
    assert_dynamic(
        "import click\n@click.group()\ndef cli(): pass\n@click.command()\n@click.option('--x')\ndef sub(x): pass\n",
        "click",
    );
}

#[test]
fn test_click_count_option_degrades_field() {
    let field = one(
        "import click\n@click.command()\n@click.option('-v', '--verbose', count=True)\ndef m(verbose): pass\n",
        "click",
    );
    assert!(field.degraded);
}

#[test]
fn test_typer_signature_order_and_kinds() {
    let fields = static_fields(TYPER_SCRIPT, "typer");
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["inputs", "output", "gap", "fast", "label"]
    );
    let by = by_name(&fields);
    assert_eq!(by["inputs"].flag, "");
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
fn test_typer_run_pattern_reads_the_function() {
    let field = one(
        "import typer\n\ndef main(n: int = 3):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.name, "n");
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert_eq!(field.default, Some(ParameterValue::Integer(3)));
}

#[test]
fn test_typer_bool_default_true_degrades_not_guesses() {
    let field = one(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
    assert_eq!(field.action, "");
}

#[test]
fn test_typer_underscored_param_gets_kebab_flag() {
    let field = one(
        "import typer\n\ndef main(max_size: int = 1):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.flag, "--max-size");
}

#[test]
fn test_typer_two_commands_degrade_as_subcommands() {
    assert_dynamic(
        "import typer\napp = typer.Typer()\n@app.command()\ndef a(x: int = 1): pass\n@app.command()\ndef b(y: int = 2): pass\n",
        "typer",
    );
}

#[test]
fn test_argparse_still_wins_when_present() {
    let fields = static_fields(
        "import argparse\nimport click\nap = argparse.ArgumentParser()\nap.add_argument('--x')\nap.parse_args()\n",
        "argparse",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["x"]
    );
}

#[test]
fn test_read_cli_none_for_plain_scripts() {
    for source in ["print('hi')\n", "def broken(:\n"] {
        assert!(matches!(
            cli_form_projection("python", source),
            CliFormProjection::Absent
        ));
    }
}

#[test]
fn test_click_field_orders_increment_by_one() {
    let fields = static_fields(CLICK_SCRIPT, "click");
    assert_eq!((0..fields.len()).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
}

#[test]
fn test_click_from_import_form_is_recognized() {
    let fields = static_fields(
        "from click import command, option\n@command()\n@option('--x', type=int)\ndef m(x): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["x"]
    );
}

#[test]
fn test_click_dotted_import_is_recognized() {
    let fields = static_fields(
        "import click.decorators\nimport click\n@click.command()\n@click.option('--x')\ndef m(x): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["x"]
    );
}

#[test]
fn test_click_secret_name_precheck_and_flag_default() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--api-key')\n@click.option('--fast', is_flag=True)\ndef m(api_key, fast): pass\n",
        "click",
    );
    let by = by_name(&fields);
    assert!(by["api_key"].secret);
    assert!(!by["fast"].secret);
    assert_eq!(by["fast"].default, Some(ParameterValue::Bool(false)));
}

#[test]
fn test_click_uppercase_type_constants() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--n', type=click.INT)\n@click.option('--r', type=click.FLOAT)\n@click.option('--s', type=click.STRING)\ndef m(n, r, s): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [ParameterType::Str, ParameterType::Float, ParameterType::Int]
    );
    assert!(fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_click_non_choice_call_type_degrades_even_with_list_arg() {
    let field = one(
        "import click\n@click.command()\n@click.option('--c', type=Wrapper(['a', 'b']))\ndef m(c): pass\n",
        "click",
    );
    assert!(field.degraded);
    assert!(field.choices.is_empty());
}

#[test]
fn test_click_non_literal_default_degrades() {
    let field = one(
        "import click\n@click.command()\n@click.option('--bg', default=(255, 255, 255))\ndef m(bg): pass\n",
        "click",
    );
    assert!(field.degraded);
}

#[test]
fn test_click_multiple_option_flag() {
    let field = one(
        "import click\n@click.command()\n@click.option('--tag', multiple=True)\ndef m(tag): pass\n",
        "click",
    );
    assert!(field.multiple);
    assert!(field.repeat);
}

#[test]
fn test_click_short_flag_only_and_help() {
    let field = one(
        "import click\n@click.command()\n@click.option('-v', help='verbosity')\ndef m(v): pass\n",
        "click",
    );
    assert_eq!(field.flag, "-v");
    assert_eq!(field.name, "v");
    assert_eq!(field.help, "verbosity");
}

#[test]
fn test_typer_from_import_form_is_recognized() {
    let fields = static_fields(
        "from typer import Typer\napp = Typer()\n@app.command()\ndef m(x: int = 1): pass\n",
        "typer",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["x"]
    );
}

#[test]
fn test_typer_orders_match_signature_positions() {
    let fields = static_fields(TYPER_SCRIPT, "typer");
    assert_eq!((0..fields.len()).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
}

#[test]
fn test_typer_bare_positional_no_default() {
    let field = one(
        "import typer\n\ndef main(n: int): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.flag, "");
    assert!(field.required);
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert!(!field.degraded);
}

#[test]
fn test_typer_unannotated_param_is_plain_text_not_degraded() {
    let field = one(
        "import typer\n\ndef main(x=1): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.parameter_type, ParameterType::Str);
    assert!(!field.degraded);
    assert_eq!(field.default, Some(ParameterValue::Integer(1)));
}

#[test]
fn test_typer_unmodelable_annotation_degrades() {
    let field = one(
        "import typer\n\ndef main(xs: list = None): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
}

#[test]
fn test_typer_option_none_default_is_clean() {
    let field = one(
        "import typer\n\ndef main(x: str = typer.Option(None, '--x')): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(!field.degraded);
    assert_eq!(field.default, None);
    assert!(!field.required);
}

#[test]
fn test_typer_secret_param_name_precheck() {
    let field = one(
        "import typer\n\ndef main(api_token: str = ''): pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.secret);
}

#[test]
fn test_click_is_flag_defaulting_on_degrades_not_guesses() {
    let field = one(
        "import click\n@click.command()\n@click.option('--color', is_flag=True, default=True)\ndef m(color): pass\n",
        "click",
    );
    assert!(field.degraded);
    assert_eq!(field.action, "");
}

#[test]
fn test_click_dotted_only_import_counts() {
    let fields = static_fields(
        "import click.testing\n@click.command()\n@click.option('--x')\ndef m(x): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["x"]
    );
}

#[test]
fn test_click_from_dotted_module_counts() {
    let fields = static_fields(
        "from click.decorators import command, option\n@command()\n@option('--x')\ndef m(x): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["x"]
    );
}

#[test]
fn test_typer_dotted_only_import_counts() {
    let fields = static_fields(
        "import typer.main\n\ndef main(n: int = 1):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["n"]
    );
}

#[test]
fn test_typer_from_dotted_module_counts() {
    let fields = static_fields(
        "from typer.main import Typer\napp = Typer()\n@app.command()\ndef m(x: int = 1): pass\n",
        "typer",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["x"]
    );
}

#[test]
fn test_click_two_commands_without_group_degrade() {
    assert_dynamic(
        "import click\n@click.command()\n@click.option('--x')\ndef a(x): pass\n@click.command()\n@click.option('--y')\ndef b(y): pass\n",
        "click",
    );
}

#[test]
fn test_click_foreign_decorators_between_options_are_skipped_not_fatal() {
    let fields = static_fields(
        "import click\nimport functools\n@click.command()\n@click.option('--first')\n@functools.cache\n@other.thing()\n@click.option('--second')\ndef m(first, second): pass\n",
        "click",
    );
    let mut names = fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, ["first", "second"]);
}

#[test]
fn test_click_non_literal_name_skips_that_call_only() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option(FLAG_CONST)\n@click.option('--real')\ndef m(real): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["real"]
    );
}

#[test]
fn test_click_partly_non_literal_names_skip_that_call_only() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('-x', EXTRA)\n@click.option('--real')\ndef m(x, real): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["real"]
    );
}

#[test]
fn test_click_short_first_declaration_still_prefers_long_flag() {
    let field = one(
        "import click\n@click.command()\n@click.option('-o', '--output')\ndef m(output): pass\n",
        "click",
    );
    assert_eq!(field.flag, "--output");
    assert_eq!(field.name, "output");
}

#[test]
fn test_click_dest_strips_dashes_not_letters() {
    let field = one(
        "import click\n@click.command()\n@click.option('--Xray')\ndef m(xray): pass\n",
        "click",
    );
    assert_eq!(field.name, "Xray");
}

#[test]
fn test_click_default_none_is_clean() {
    let field = one(
        "import click\n@click.command()\n@click.option('--x', default=None)\ndef m(x): pass\n",
        "click",
    );
    assert!(!field.degraded);
    assert_eq!(field.default, None);
}

#[test]
fn test_click_bare_float_and_str_types() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--r', type=float)\n@click.option('--s', type=str)\ndef m(r, s): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [ParameterType::Str, ParameterType::Float]
    );
    assert!(fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_click_unknown_name_type_degrades() {
    let field = one(
        "import click\n@click.command()\n@click.option('--p', type=parse_color)\ndef m(p): pass\n",
        "click",
    );
    assert!(field.degraded);
}

#[test]
fn test_click_path_and_file_types() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--src', type=click.Path(exists=True))\n@click.option('--out', type=click.File('w'))\n@click.option('--raw', type=Path)\ndef m(src, out, raw): pass\n",
        "click",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [
            ParameterType::Path,
            ParameterType::Path,
            ParameterType::Path
        ]
    );
    assert!(fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_typer_option_extra_decl_positions() {
    let field = one(
        "import typer\n\ndef main(out: str = typer.Option('x', '-o', '--renamed')):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.flag, "--renamed");
    assert_eq!(field.default, Some(ParameterValue::String("x".to_owned())));
}

#[test]
fn test_typer_path_annotation_is_path() {
    let fields = static_fields(
        "import typer\nfrom pathlib import Path\nfrom typing import Annotated\n\ndef main(src: Path, out: Annotated[Path, typer.Option()] = Path('o.txt')):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(
        fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [ParameterType::Path, ParameterType::Path]
    );
}

#[test]
fn test_typer_non_constant_decl_is_ignored_not_fatal() {
    let field = one(
        "import typer\n\ndef main(out: str = typer.Option('x', SOME_DECL)):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.flag, "--out");
    assert_eq!(field.default, Some(ParameterValue::String("x".to_owned())));
}

#[test]
fn test_typer_computed_plain_default_degrades() {
    let field = one(
        "import typer\n\ndef main(x: int = make_default()):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
}

#[test]
fn test_typer_option_computed_first_arg_degrades() {
    let field = one(
        "import typer\n\ndef main(x: str = typer.Option(CONST_REF)):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
}

#[test]
fn test_typer_bool_true_degrade_renders_as_text() {
    let field = one(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
    assert_eq!(field.parameter_type, ParameterType::Str);
}

#[test]
fn test_typer_bool_false_flag_contract_exact() {
    let field = one(
        "import typer\n\ndef main(fast: bool = False):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.action, "store_true");
    assert_eq!(field.default, Some(ParameterValue::Bool(false)));
    assert!(!field.degraded);
}

#[test]
fn test_click_non_literal_choice_list_degrades() {
    let field = one(
        "import click\n@click.command()\n@click.option('--mode', type=click.Choice(MODES))\ndef m(mode): pass\n",
        "click",
    );
    assert!(field.degraded);
    assert_ne!(field.parameter_type, ParameterType::Choice);
}

#[test]
fn test_typer_unmodelable_annotation_degrades_despite_literal_default() {
    let field = one(
        "import typer\n\ndef main(mode: dict = 'x'):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
}

#[test]
fn test_typer_option_single_extra_decl_is_read() {
    let field = one(
        "import typer\n\ndef main(out: str = typer.Option('x', '--renamed')):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.flag, "--renamed");
}

#[test]
fn test_annotated_reads_type_and_metadata() {
    let fields = static_fields(ANNOTATED_SCRIPT, "typer");
    let by = by_name(&fields);
    assert_eq!(by["name"].flag, "");
    assert!(by["name"].required);
    assert_eq!(by["name"].parameter_type, ParameterType::Str);
    assert_eq!(by["name"].help, "who");
    assert_eq!(by["count"].parameter_type, ParameterType::Int);
    assert_eq!(by["count"].default, Some(ParameterValue::Integer(3)));
    assert_eq!(by["count"].flag, "--count");
    assert_eq!(by["count"].help, "how many");
    assert!(!by["count"].degraded);
    assert_eq!(by["mode"].flag, "--mode");
    assert_eq!(
        by["mode"].default,
        Some(ParameterValue::String("fast".to_owned()))
    );
    assert_eq!(by["fast"].parameter_type, ParameterType::Bool);
    assert_eq!(by["fast"].action, "store_true");
}

#[test]
fn test_annotated_option_without_default_is_required() {
    let field = one(
        "import typer\nfrom typing import Annotated\ndef main(x: Annotated[int, typer.Option()]):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.flag, "--x");
    assert!(field.required);
    assert_eq!(field.parameter_type, ParameterType::Int);
}

#[test]
fn test_annotated_argument_with_default_is_optional_positional() {
    let field = one(
        "import typer\nfrom typing import Annotated\ndef main(name: Annotated[str, typer.Argument()] = 'anon'):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.flag, "");
    assert!(!field.required);
    assert_eq!(
        field.default,
        Some(ParameterValue::String("anon".to_owned()))
    );
}

#[test]
fn test_annotated_unmodelable_inner_type_degrades() {
    let field = one(
        "import typer\nfrom typing import Annotated\ndef main(tags: Annotated[list, typer.Option()] = None):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
}

#[test]
fn test_annotated_bool_default_true_degrades() {
    let field = one(
        "import typer\nfrom typing import Annotated\ndef main(color: Annotated[bool, typer.Option()] = True):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
    assert_eq!(field.action, "");
}

#[test]
fn test_annotated_choice_via_typing_qualified_name() {
    let field = one(
        "import typer\nimport typing\ndef main(gap: typing.Annotated[int, typer.Option()] = 0):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert_eq!(field.default, Some(ParameterValue::Integer(0)));
}

#[test]
fn test_annotated_help_kwarg_survives_on_degraded_field() {
    let field = one(
        "import typer\nfrom typing import Annotated\ndef main(x: Annotated[MyType, typer.Option(help='hint')] = None):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
    assert_eq!(field.help, "hint");
}

#[test]
fn test_legacy_typer_style_still_works_after_annotated_refactor() {
    let fields = static_fields(TYPER_SCRIPT, "typer");
    let by = by_name(&fields);
    assert!(by["output"].required);
    assert_eq!(by["gap"].default, Some(ParameterValue::Integer(0)));
    assert_eq!(by["fast"].action, "store_true");
}

#[test]
fn test_annotated_only_recognizes_the_real_annotated_name() {
    let field = one(
        "import typer\nimport mod\ndef main(x: mod.Wrapper[int, typer.Option()] = 1):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert!(field.degraded);
}

#[test]
fn test_annotated_without_typer_metadata_reads_as_plain_type() {
    let field = one(
        "import typer\nfrom typing import Annotated\ndef main(x: Annotated[int, 'a note'] = 5):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert_eq!(field.default, Some(ParameterValue::Integer(5)));
    assert!(!field.degraded);
}

#[test]
fn test_annotated_picks_the_typer_call_among_several() {
    let field = one(
        "import typer\nfrom typing import Annotated\ndef main(x: Annotated[int, Validator(), typer.Option(help='H')] = 5):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.help, "H");
}

#[test]
fn test_annotated_option_positional_decl_is_a_flag_not_a_default() {
    let field = one(
        "import typer\nfrom typing import Annotated\ndef main(x: Annotated[str, typer.Option('--primary', '--secondary')]):\n    pass\n\ntyper.run(main)\n",
        "typer",
    );
    assert_eq!(field.flag, "--primary");
    assert_eq!(field.default, None);
    assert!(field.required);
}
