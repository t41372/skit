//! Mechanical port of the Python oracle module `tests/test_argspec_click_typer.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name so it
//! traces back to its origin, and the Python "WHY" comment is preserved above it.
//!
//! Concept mapping used throughout:
//! - Python `argspec.read_cli(src)` -> `parsed(src).cli_surface()` (the unified reflection that
//!   tries argparse, then click, then typer).
//! - Python `spec is None` (a plain script) -> `CliSurface::Absent`; a syntax error is
//!   `ParseOutcome::SyntaxError` (no parsed document), the faithful mapping of `read_cli`
//!   returning `None` on `SyntaxError`.
//! - Python `spec.ok is True` -> `CliSurface::Static(surface)`.
//! - Python `spec.ok is False` + `spec.reason == "subparsers"` ->
//!   `CliSurface::Dynamic(surface)` with `DegradationReason::Subcommands`.
//! - Python `spec.fields[i]` -> `surface.fields[i].declaration` (a `ParamDecl`).
//! - Python field `type` strings -> `ParameterType`; Python `default` -> `Option<ParameterValue>`
//!   (`ParameterValue::String`, not `Text`); Python tuple `choices` -> `Vec<String>`.
//! - Python `f.flag == ""` -> `field.flag.is_empty()` (positional); Python `f.action == ""` ->
//!   `field.action.is_empty()`.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_language::{CliSurface, DegradationReason, ParseOutcome, ParsedDocument, parse_document};

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid Python, got {other:?}"),
    }
}

/// Python `spec.ok` static path: the fields of a `CliSurface::Static` surface.
fn static_fields(source: &str) -> Vec<ParamDecl> {
    let CliSurface::Static(surface) = parsed(source).cli_surface() else {
        panic!("expected a static CLI surface");
    };
    surface
        .fields
        .into_iter()
        .map(|field| field.declaration)
        .collect()
}

/// Python `[f.name for f in spec.fields]`.
fn names(fields: &[ParamDecl]) -> Vec<&str> {
    fields.iter().map(|field| field.name.as_str()).collect()
}

/// Python `{f.name: f for f in spec.fields}`.
fn by_name(fields: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

/// Python `spec is None`: no CLI parser is present at all (a plain script).
fn is_absent(source: &str) -> bool {
    matches!(parsed(source).cli_surface(), CliSurface::Absent)
}

/// Python `spec.ok is False`: the typed whole-surface degradation reason.
fn dynamic_reason(source: &str) -> DegradationReason {
    match parsed(source).cli_surface() {
        CliSurface::Dynamic(surface) => surface.reason,
        _ => panic!("expected a dynamic CLI surface"),
    }
}

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

// --------------------------------------------------------------------------
// click
// --------------------------------------------------------------------------

#[test]
fn test_click_fields_bottom_up_order_matches_runtime() {
    // click applies decorators bottom-up: the bottom @click.argument is param #1.
    let fields = static_fields(CLICK_SCRIPT);
    assert_eq!(names(&fields), ["inputs", "fast", "mode", "gap", "output"]);
}

#[test]
fn test_click_argument_variadic_is_multiple_not_required() {
    let fields = static_fields(CLICK_SCRIPT);
    let inputs = &fields[0];
    assert!(inputs.flag.is_empty());
    assert!(inputs.multiple);
    assert!(!inputs.required); // nargs=-1 lifts click's argument-required default
    // nargs=-1 is variadic-positional grammar (`--flag a b`), NOT the repeated-option shape:
    // a positional has no flag to repeat, so repeat stays False even though multiple is True.
    assert!(!inputs.repeat);
}

#[test]
fn test_click_is_flag_choice_int_and_required() {
    let fields = static_fields(CLICK_SCRIPT);
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
    let fields = static_fields(
        "import click\n@click.command()\n@click.argument('name')\ndef m(name): pass\n",
    );
    assert!(fields[0].required);
}

#[test]
fn test_click_group_degrades_as_subcommands() {
    assert_eq!(
        dynamic_reason(concat!(
            "import click\n@click.group()\ndef cli(): pass\n",
            "@click.command()\n@click.option('--x')\ndef sub(x): pass\n",
        )),
        DegradationReason::Subcommands
    );
}

#[test]
fn test_click_count_option_degrades_field() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n@click.option('-v', '--verbose', count=True)\n",
        "def m(verbose): pass\n",
    ));
    assert!(fields[0].degraded);
}

// --------------------------------------------------------------------------
// typer
// --------------------------------------------------------------------------

#[test]
fn test_typer_signature_order_and_kinds() {
    let fields = static_fields(TYPER_SCRIPT);
    assert_eq!(names(&fields), ["inputs", "output", "gap", "fast", "label"]);
    let by = by_name(&fields);
    assert!(by["inputs"].flag.is_empty()); // Argument -> positional
    assert!(by["inputs"].required); // Ellipsis default
    assert_eq!(by["output"].flag, "--output");
    assert!(by["output"].required);
    assert_eq!(by["output"].help, "output path");
    assert_eq!(by["gap"].parameter_type, ParameterType::Int);
    assert_eq!(by["gap"].default, Some(ParameterValue::Integer(0)));
    assert_eq!(by["gap"].flag, "--gap"); // derived from the parameter name
    assert_eq!(by["fast"].parameter_type, ParameterType::Bool);
    assert_eq!(by["fast"].action, "store_true");
    assert_eq!(
        by["label"].default,
        Some(ParameterValue::String("x".to_owned()))
    ); // plain literal default becomes an option
    assert_eq!(by["label"].flag, "--label");
}

#[test]
fn test_typer_run_pattern_reads_the_function() {
    let fields =
        static_fields("import typer\n\ndef main(n: int = 3):\n    pass\n\ntyper.run(main)\n");
    assert_eq!(fields[0].name, "n");
    assert_eq!(fields[0].parameter_type, ParameterType::Int);
    assert_eq!(fields[0].default, Some(ParameterValue::Integer(3)));
}

#[test]
fn test_typer_bool_default_true_degrades_not_guesses() {
    let fields = static_fields(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n",
    );
    let field = &fields[0];
    assert!(field.degraded); // --color/--no-color pairing cannot be assembled faithfully
    assert!(field.action.is_empty());
}

#[test]
fn test_typer_underscored_param_gets_kebab_flag() {
    let fields = static_fields(
        "import typer\n\ndef main(max_size: int = 1):\n    pass\n\ntyper.run(main)\n",
    );
    assert_eq!(fields[0].flag, "--max-size");
}

#[test]
fn test_typer_two_commands_degrade_as_subcommands() {
    assert_eq!(
        dynamic_reason(concat!(
            "import typer\napp = typer.Typer()\n",
            "@app.command()\ndef a(x: int = 1): pass\n",
            "@app.command()\ndef b(y: int = 2): pass\n",
        )),
        DegradationReason::Subcommands
    );
}

#[test]
fn test_argparse_still_wins_when_present() {
    let fields = static_fields(concat!(
        "import argparse\nimport click\n",
        "ap = argparse.ArgumentParser()\nap.add_argument('--x')\nap.parse_args()\n",
    ));
    assert_eq!(names(&fields), ["x"]);
}

#[test]
fn test_read_cli_none_for_plain_scripts() {
    assert!(is_absent("print('hi')\n"));
    assert!(matches!(
        parse_document("python", "def broken(:\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

// --------------------------------------------------------------------------
// mutation hardening: exact contracts for the click/typer readers
// --------------------------------------------------------------------------

#[test]
fn test_click_field_orders_increment_by_one() {
    // Order is list position now: five decorators -> five fields, in decorator-runtime order.
    let fields = static_fields(CLICK_SCRIPT);
    assert_eq!(fields.len(), 5);
}

#[test]
fn test_click_from_import_form_is_recognized() {
    let fields = static_fields(
        "from click import command, option\n@command()\n@option('--x', type=int)\ndef m(x): pass\n",
    );
    assert_eq!(names(&fields), ["x"]);
}

#[test]
fn test_click_dotted_import_is_recognized() {
    let fields = static_fields(concat!(
        "import click.decorators\nimport click\n",
        "@click.command()\n@click.option('--x')\ndef m(x): pass\n",
    ));
    assert_eq!(names(&fields), ["x"]);
}

#[test]
fn test_click_secret_name_precheck_and_flag_default() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('--api-key')\n@click.option('--fast', is_flag=True)\n",
        "def m(api_key, fast): pass\n",
    ));
    let by = by_name(&fields);
    assert!(by["api_key"].secret);
    assert!(!by["fast"].secret);
    assert_eq!(by["fast"].default, Some(ParameterValue::Bool(false))); // an is_flag option starts unchecked
}

#[test]
fn test_click_uppercase_type_constants() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('--n', type=click.INT)\n",
        "@click.option('--r', type=click.FLOAT)\n",
        "@click.option('--s', type=click.STRING)\n",
        "def m(n, r, s): pass\n",
    ));
    // Bottom-up decorator order: STRING, FLOAT, INT.
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
    // A random callable taking a list is NOT a Choice -- it must degrade, never be
    // mistaken for one (kills the and->or mutant in the Choice detection).
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('--c', type=Wrapper(['a', 'b']))\ndef m(c): pass\n",
    ));
    assert!(fields[0].degraded);
    assert!(fields[0].choices.is_empty());
}

#[test]
fn test_click_non_literal_default_degrades() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('--bg', default=(255, 255, 255))\ndef m(bg): pass\n",
    ));
    assert!(fields[0].degraded);
}

#[test]
fn test_click_multiple_option_flag() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--tag', multiple=True)\ndef m(tag): pass\n",
    );
    assert!(fields[0].multiple);
    // click's multiple=True consumes ONE value per occurrence, so assembly must REPEAT the flag
    // (`--tag a --tag b`); `--tag a b` is an exit-2 usage error to click. repeat records that.
    assert!(fields[0].repeat);
}

#[test]
fn test_click_short_flag_only_and_help() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('-v', help='verbosity')\ndef m(v): pass\n",
    );
    assert_eq!(fields[0].flag, "-v");
    assert_eq!(fields[0].name, "v");
    assert_eq!(fields[0].help, "verbosity");
}

#[test]
fn test_typer_from_import_form_is_recognized() {
    let fields = static_fields(
        "from typer import Typer\napp = Typer()\n@app.command()\ndef m(x: int = 1): pass\n",
    );
    assert_eq!(names(&fields), ["x"]);
}

#[test]
fn test_typer_orders_match_signature_positions() {
    // Order is list position now: five parameters -> five fields, in signature order.
    let fields = static_fields(TYPER_SCRIPT);
    assert_eq!(fields.len(), 5);
}

#[test]
fn test_typer_bare_positional_no_default() {
    let fields = static_fields("import typer\n\ndef main(n: int): pass\n\ntyper.run(main)\n");
    let field = &fields[0];
    assert!(field.flag.is_empty()); // positional
    assert!(field.required);
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert!(!field.degraded);
}

#[test]
fn test_typer_unannotated_param_is_plain_text_not_degraded() {
    let fields = static_fields("import typer\n\ndef main(x=1): pass\n\ntyper.run(main)\n");
    let field = &fields[0];
    assert_eq!(field.parameter_type, ParameterType::Str);
    assert!(!field.degraded);
    assert_eq!(field.default, Some(ParameterValue::Integer(1)));
}

#[test]
fn test_typer_unmodelable_annotation_degrades() {
    let fields =
        static_fields("import typer\n\ndef main(xs: list = None): pass\n\ntyper.run(main)\n");
    assert!(fields[0].degraded);
}

#[test]
fn test_typer_option_none_default_is_clean() {
    let fields = static_fields(
        "import typer\n\ndef main(x: str = typer.Option(None, '--x')): pass\n\ntyper.run(main)\n",
    );
    let field = &fields[0];
    assert!(!field.degraded);
    assert!(field.default.is_none());
    assert!(!field.required);
}

#[test]
fn test_typer_secret_param_name_precheck() {
    let fields =
        static_fields("import typer\n\ndef main(api_token: str = ''): pass\n\ntyper.run(main)\n");
    assert!(fields[0].secret);
}

// WHY: `_decorator_name` returns "" for a callee that is itself a Call (e.g. `(f())()`).
#[test]
#[ignore = "UNMAPPED: argspec._decorator_name is a private Python helper called directly on a hand-built ast.Call node; the Rust reflection exposes no equivalent function. The analogous whole-surface consequence (an unnameable-callable decorator yields a CliSurface::Absent surface) is covered by the oracle test origin_main_click_boolean_and_dynamic_type_oracle_degrades_without_guessing."]
fn test_decorator_name_unnameable_callable_is_empty() {
    // Faithful port impossible: no public counterpart to `argspec._decorator_name`.
}

#[test]
fn test_click_is_flag_defaulting_on_degrades_not_guesses() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('--color', is_flag=True, default=True)\ndef m(color): pass\n",
    ));
    let field = &fields[0];
    assert!(field.degraded); // --color/--no-color pairing cannot be assembled faithfully
    assert!(field.action.is_empty());
}

// --------------------------------------------------------------------------
// mutation tail: exact contracts for import guards, orders, dests, defaults
// --------------------------------------------------------------------------

#[test]
fn test_click_dotted_only_import_counts() {
    // `import click.testing` (no plain `import click`) binds the click name at runtime;
    // the import guard must dot-split module paths on BOTH import forms.
    let fields = static_fields(
        "import click.testing\n@click.command()\n@click.option('--x')\ndef m(x): pass\n",
    );
    assert_eq!(names(&fields), ["x"]);
}

#[test]
fn test_click_from_dotted_module_counts() {
    let fields = static_fields(
        "from click.decorators import command, option\n@command()\n@option('--x')\ndef m(x): pass\n",
    );
    assert_eq!(names(&fields), ["x"]);
}

#[test]
fn test_typer_dotted_only_import_counts() {
    let fields =
        static_fields("import typer.main\n\ndef main(n: int = 1):\n    pass\n\ntyper.run(main)\n");
    assert_eq!(names(&fields), ["n"]);
}

#[test]
fn test_typer_from_dotted_module_counts() {
    let fields = static_fields(
        "from typer.main import Typer\napp = Typer()\n@app.command()\ndef m(x: int = 1): pass\n",
    );
    assert_eq!(names(&fields), ["x"]);
}

#[test]
fn test_click_two_commands_without_group_degrade() {
    assert_eq!(
        dynamic_reason(concat!(
            "import click\n",
            "@click.command()\n@click.option('--x')\ndef a(x): pass\n",
            "@click.command()\n@click.option('--y')\ndef b(y): pass\n",
        )),
        DegradationReason::Subcommands
    );
}

#[test]
fn test_click_foreign_decorators_between_options_are_skipped_not_fatal() {
    // A bare decorator AND a non-click call decorator sit between two options: the
    // reader must skip them and keep walking (a `break` would silently drop fields).
    let fields = static_fields(concat!(
        "import click\nimport functools\n",
        "@click.command()\n",
        "@click.option('--first')\n",
        "@functools.cache\n",
        "@other.thing()\n",
        "@click.option('--second')\n",
        "def m(first, second): pass\n",
    ));
    let mut got = names(&fields);
    got.sort_unstable();
    assert_eq!(got, ["first", "second"]);
}

#[test]
fn test_click_non_literal_name_skips_that_call_only() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option(FLAG_CONST)\n@click.option('--real')\ndef m(real): pass\n",
    ));
    assert_eq!(names(&fields), ["real"]);
}

#[test]
fn test_click_partly_non_literal_names_skip_that_call_only() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('-x', EXTRA)\n@click.option('--real')\ndef m(x, real): pass\n",
    ));
    assert_eq!(names(&fields), ["real"]);
}

#[test]
fn test_click_short_first_declaration_still_prefers_long_flag() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('-o', '--output')\ndef m(output): pass\n",
    );
    assert_eq!(fields[0].flag, "--output");
    assert_eq!(fields[0].name, "output");
}

#[test]
fn test_click_dest_strips_dashes_not_letters() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--Xray')\ndef m(xray): pass\n",
    );
    assert_eq!(fields[0].name, "Xray");
}

#[test]
fn test_click_default_none_is_clean() {
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--x', default=None)\ndef m(x): pass\n",
    );
    assert!(!fields[0].degraded);
    assert!(fields[0].default.is_none());
}

#[test]
fn test_click_bare_float_and_str_types() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('--r', type=float)\n@click.option('--s', type=str)\n",
        "def m(r, s): pass\n",
    ));
    // bottom-up: s first, then r
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
    let fields = static_fields(
        "import click\n@click.command()\n@click.option('--p', type=parse_color)\ndef m(p): pass\n",
    );
    assert!(fields[0].degraded); // an arbitrary callable we will not execute
}

#[test]
fn test_click_path_and_file_types() {
    // click.Path(...)/click.File(...) calls and a bare pathlib Path callable are all
    // path signals; keyword refinements (exists=, dir_okay=, ...) are not modelled.
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('--src', type=click.Path(exists=True))\n",
        "@click.option('--out', type=click.File('w'))\n",
        "@click.option('--raw', type=Path)\n",
        "def m(src, out, raw): pass\n",
    ));
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
    // The long flag may not be the FIRST declaration after the default.
    let fields = static_fields(concat!(
        "import typer\n\ndef main(out: str = typer.Option('x', '-o', '--renamed')):\n",
        "    pass\n\ntyper.run(main)\n",
    ));
    assert_eq!(fields[0].flag, "--renamed");
    assert_eq!(
        fields[0].default,
        Some(ParameterValue::String("x".to_owned()))
    );
}

#[test]
fn test_typer_path_annotation_is_path() {
    // A Path annotation is a path signal, in both the legacy-default and the
    // Annotated spellings.
    let fields = static_fields(concat!(
        "import typer\nfrom pathlib import Path\nfrom typing import Annotated\n\n",
        "def main(src: Path, out: Annotated[Path, typer.Option()] = Path('o.txt')):\n",
        "    pass\n\ntyper.run(main)\n",
    ));
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
    let fields = static_fields(concat!(
        "import typer\n\ndef main(out: str = typer.Option('x', SOME_DECL)):\n",
        "    pass\n\ntyper.run(main)\n",
    ));
    assert_eq!(fields[0].flag, "--out"); // falls back to the derived flag
    assert_eq!(
        fields[0].default,
        Some(ParameterValue::String("x".to_owned()))
    );
}

#[test]
fn test_typer_computed_plain_default_degrades() {
    let fields = static_fields(
        "import typer\n\ndef main(x: int = make_default()):\n    pass\n\ntyper.run(main)\n",
    );
    assert!(fields[0].degraded);
}

#[test]
fn test_typer_option_computed_first_arg_degrades() {
    let fields = static_fields(
        "import typer\n\ndef main(x: str = typer.Option(CONST_REF)):\n    pass\n\ntyper.run(main)\n",
    );
    assert!(fields[0].degraded);
}

#[test]
fn test_typer_bool_true_degrade_renders_as_text() {
    let fields = static_fields(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n",
    );
    let field = &fields[0];
    assert!(field.degraded);
    assert_eq!(field.parameter_type, ParameterType::Str); // the degrade path pins the free-text kind exactly
}

#[test]
fn test_typer_bool_false_flag_contract_exact() {
    let fields = static_fields(
        "import typer\n\ndef main(fast: bool = False):\n    pass\n\ntyper.run(main)\n",
    );
    let field = &fields[0];
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.action, "store_true");
    assert_eq!(field.default, Some(ParameterValue::Bool(false)));
    assert!(!field.degraded);
}

#[test]
fn test_click_non_literal_choice_list_degrades() {
    let fields = static_fields(concat!(
        "import click\n@click.command()\n",
        "@click.option('--mode', type=click.Choice(MODES))\ndef m(mode): pass\n",
    ));
    assert!(fields[0].degraded);
    assert_ne!(fields[0].parameter_type, ParameterType::Choice);
}

#[test]
fn test_typer_unmodelable_annotation_degrades_despite_literal_default() {
    // The annotation-driven degrade must hold on its own -- a clean literal default
    // (which does NOT degrade) must not mask it.
    let fields =
        static_fields("import typer\n\ndef main(mode: dict = 'x'):\n    pass\n\ntyper.run(main)\n");
    assert!(fields[0].degraded);
}

#[test]
fn test_typer_option_single_extra_decl_is_read() {
    // The declaration list starts right AFTER the default (args[1:], not args[2:]).
    let fields = static_fields(concat!(
        "import typer\n\ndef main(out: str = typer.Option('x', '--renamed')):\n",
        "    pass\n\ntyper.run(main)\n",
    ));
    assert_eq!(fields[0].flag, "--renamed");
}

// --------------------------------------------------------------------------
// typer Annotated[...] (A6) -- the modern style AI-written typer scripts use
// --------------------------------------------------------------------------

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

#[test]
fn test_annotated_reads_type_and_metadata() {
    let fields = static_fields(ANNOTATED_SCRIPT);
    let by = by_name(&fields);
    // Argument -> required positional, type from Annotated's first arg
    assert!(by["name"].flag.is_empty());
    assert!(by["name"].required);
    assert_eq!(by["name"].parameter_type, ParameterType::Str);
    assert_eq!(by["name"].help, "who");
    // Option with a literal `= default`, type int
    assert_eq!(by["count"].parameter_type, ParameterType::Int);
    assert_eq!(by["count"].default, Some(ParameterValue::Integer(3)));
    assert_eq!(by["count"].flag, "--count");
    assert_eq!(by["count"].help, "how many");
    assert!(!by["count"].degraded);
    // Explicit flag declarations inside the Annotated Option
    assert_eq!(by["mode"].flag, "--mode");
    assert_eq!(
        by["mode"].default,
        Some(ParameterValue::String("fast".to_owned()))
    );
    // bool option defaulting False -> checkbox
    assert_eq!(by["fast"].parameter_type, ParameterType::Bool);
    assert_eq!(by["fast"].action, "store_true");
}

#[test]
fn test_annotated_option_without_default_is_required() {
    let fields = static_fields(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[int, typer.Option()]):\n    pass\n\ntyper.run(main)\n",
    ));
    let field = &fields[0];
    assert_eq!(field.flag, "--x"); // still an option, not a positional
    assert!(field.required);
    assert_eq!(field.parameter_type, ParameterType::Int);
}

#[test]
fn test_annotated_argument_with_default_is_optional_positional() {
    let fields = static_fields(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(name: Annotated[str, typer.Argument()] = 'anon'):\n    pass\n\ntyper.run(main)\n",
    ));
    let field = &fields[0];
    assert!(field.flag.is_empty());
    assert!(!field.required);
    assert_eq!(
        field.default,
        Some(ParameterValue::String("anon".to_owned()))
    );
}

#[test]
fn test_annotated_unmodelable_inner_type_degrades() {
    let fields = static_fields(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(tags: Annotated[list, typer.Option()] = None):\n    pass\n\ntyper.run(main)\n",
    ));
    assert!(fields[0].degraded);
}

#[test]
fn test_annotated_bool_default_true_degrades() {
    let fields = static_fields(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(color: Annotated[bool, typer.Option()] = True):\n    pass\n\ntyper.run(main)\n",
    ));
    let field = &fields[0];
    assert!(field.degraded);
    assert!(field.action.is_empty()); // not a store_true -- the --no-color pairing cannot be assembled
}

#[test]
fn test_annotated_choice_via_typing_qualified_name() {
    // `typing.Annotated` (attribute form) must be recognized too, not just a bare name.
    let fields = static_fields(concat!(
        "import typer\nimport typing\n",
        "def main(gap: typing.Annotated[int, typer.Option()] = 0):\n    pass\n\ntyper.run(main)\n",
    ));
    assert_eq!(fields[0].parameter_type, ParameterType::Int);
    assert_eq!(fields[0].default, Some(ParameterValue::Integer(0)));
}

#[test]
fn test_annotated_help_kwarg_survives_on_degraded_field() {
    let fields = static_fields(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[MyType, typer.Option(help='hint')] = None):\n",
        "    pass\n\ntyper.run(main)\n",
    ));
    assert!(fields[0].degraded);
    assert_eq!(fields[0].help, "hint"); // the user's hint still reaches the form
}

#[test]
fn test_legacy_typer_style_still_works_after_annotated_refactor() {
    // Regression: the non-Annotated `x: int = typer.Option(5, '--renamed')` path must be
    // untouched by the Annotated addition.
    let fields = static_fields(TYPER_SCRIPT);
    let by = by_name(&fields);
    assert!(by["output"].required);
    assert_eq!(by["gap"].default, Some(ParameterValue::Integer(0)));
    assert_eq!(by["fast"].action, "store_true");
}

#[test]
fn test_annotated_only_recognizes_the_real_annotated_name() {
    // A qualified subscript that is NOT typing.Annotated must not be unwrapped as one.
    let fields = static_fields(concat!(
        "import typer\nimport mod\n",
        "def main(x: mod.Wrapper[int, typer.Option()] = 1):\n    pass\n\ntyper.run(main)\n",
    ));
    assert!(fields[0].degraded); // unknown subscript type -> free text, not int
}

#[test]
fn test_annotated_without_typer_metadata_reads_as_plain_type() {
    // Annotated with only a doc string (no typer.Option/Argument): the field is still a
    // plain int with its signature default -- the meta search must return None, not raise.
    let fields = static_fields(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[int, 'a note'] = 5):\n    pass\n\ntyper.run(main)\n",
    ));
    let field = &fields[0];
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert_eq!(field.default, Some(ParameterValue::Integer(5)));
    assert!(!field.degraded);
}

#[test]
fn test_annotated_picks_the_typer_call_among_several() {
    // Two calls in the metadata: the typer one must be chosen, not the first call.
    let fields = static_fields(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[int, Validator(), typer.Option(help='H')] = 5):\n",
        "    pass\n\ntyper.run(main)\n",
    ));
    assert_eq!(fields[0].help, "H");
}

#[test]
fn test_annotated_option_positional_decl_is_a_flag_not_a_default() {
    // In the Annotated style, Option's positional strings are flag DECLARATIONS, never a
    // value default (that lives in the `= value`). A required Annotated Option with two
    // long decls keeps the first as the flag and stays default-less.
    let fields = static_fields(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[str, typer.Option('--primary', '--secondary')]):\n",
        "    pass\n\ntyper.run(main)\n",
    ));
    let field = &fields[0];
    assert_eq!(field.flag, "--primary");
    assert!(field.default.is_none()); // not '--primary' (the True mutant would read it as a default)
    assert!(field.required);
}
