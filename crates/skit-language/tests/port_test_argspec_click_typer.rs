//! Mechanical port of the Python oracle module `tests/test_argspec_click_typer.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name and the Python
//! "WHY" comment is preserved verbatim above it.
//!
//! Concept mapping:
//! - Python `argspec.read_cli` -> `parse_document("python", source)` followed by
//!   `ParsedDocument::cli_surface()`.
//! - Python `None` -> a syntax failure or `CliSurface::Absent`.
//! - Python `ArgSpec(ok=False, reason="subparsers")` -> `CliSurface::Dynamic` with
//!   `DegradationReason::Subcommands`.
//! - Python `ArgSpec.fields` -> the `ParamDecl`s inside `CliSurface::Static` in runtime order.
//! - Python's private `_decorator_name((f())()) == ""` check is mapped to its public consequence:
//!   an unnameable decorator does not produce a Click CLI surface.

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_language::{CliSurface, DegradationReason, ParseOutcome, parse_document};

#[derive(Debug)]
struct Spec {
    framework: String,
    ok: bool,
    reason: Option<DegradationReason>,
    fields: Vec<ParamDecl>,
}

fn read_cli(source: &str) -> Option<Spec> {
    let document = match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        _ => return None,
    };
    match document.cli_surface() {
        CliSurface::Absent => None,
        CliSurface::Static(surface) => Some(Spec {
            framework: surface.framework,
            ok: true,
            reason: None,
            fields: surface
                .fields
                .into_iter()
                .map(|field| field.declaration)
                .collect(),
        }),
        CliSurface::Dynamic(surface) => Some(Spec {
            framework: surface.framework,
            ok: false,
            reason: Some(surface.reason),
            fields: Vec::new(),
        }),
    }
}

fn names(spec: &Spec) -> Vec<&str> {
    spec.fields
        .iter()
        .map(|field| field.name.as_str())
        .collect()
}

fn field<'a>(spec: &'a Spec, name: &str) -> &'a ParamDecl {
    spec.fields
        .iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("missing field {name}"))
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
    let spec = read_cli(CLICK_SCRIPT).unwrap();
    assert!(spec.ok);
    // click applies decorators bottom-up: the bottom @click.argument is param #1.
    assert_eq!(names(&spec), ["inputs", "fast", "mode", "gap", "output"]);
}

#[test]
fn test_click_argument_variadic_is_multiple_not_required() {
    let spec = read_cli(CLICK_SCRIPT).unwrap();
    let inputs = &spec.fields[0];
    assert!(inputs.flag.is_empty());
    assert!(inputs.multiple);
    assert!(!inputs.required); // nargs=-1 lifts click's argument-required default
    // nargs=-1 is variadic-positional grammar (`--flag a b`), NOT the repeated-option shape:
    // a positional has no flag to repeat, so repeat stays False even though multiple is True.
    assert!(!inputs.repeat);
}

#[test]
fn test_click_is_flag_choice_int_and_required() {
    let spec = read_cli(CLICK_SCRIPT).unwrap();
    assert_eq!(field(&spec, "fast").parameter_type, ParameterType::Bool);
    assert_eq!(field(&spec, "fast").action, "store_true");
    assert_eq!(field(&spec, "mode").parameter_type, ParameterType::Choice);
    assert_eq!(field(&spec, "mode").choices, ["a", "b"]);
    assert_eq!(
        field(&spec, "mode").default,
        Some(ParameterValue::String("a".to_owned()))
    );
    assert_eq!(field(&spec, "gap").parameter_type, ParameterType::Int);
    assert_eq!(
        field(&spec, "gap").default,
        Some(ParameterValue::Integer(0))
    );
    assert!(field(&spec, "output").required);
    assert_eq!(field(&spec, "output").flag, "--output");
    assert_eq!(field(&spec, "output").help, "output path");
}

#[test]
fn test_click_plain_argument_is_required() {
    let spec = read_cli(
        "import click\n@click.command()\n@click.argument('name')\ndef m(name): pass\n",
    )
    .unwrap();
    assert!(spec.fields[0].required);
}

#[test]
fn test_click_group_degrades_as_subcommands() {
    let spec = read_cli(concat!(
        "import click\n@click.group()\ndef cli(): pass\n",
        "@click.command()\n@click.option('--x')\ndef sub(x): pass\n",
    ))
    .unwrap();
    assert!(!spec.ok);
    assert_eq!(spec.reason, Some(DegradationReason::Subcommands));
}

#[test]
fn test_click_count_option_degrades_field() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n@click.option('-v', '--verbose', count=True)\n",
        "def m(verbose): pass\n",
    ))
    .unwrap();
    assert!(spec.fields[0].degraded);
}

// --------------------------------------------------------------------------
// typer
// --------------------------------------------------------------------------

#[test]
fn test_typer_signature_order_and_kinds() {
    let spec = read_cli(TYPER_SCRIPT).unwrap();
    assert!(spec.ok);
    assert_eq!(names(&spec), ["inputs", "output", "gap", "fast", "label"]);
    assert!(field(&spec, "inputs").flag.is_empty()); // Argument -> positional
    assert!(field(&spec, "inputs").required); // Ellipsis default
    assert_eq!(field(&spec, "output").flag, "--output");
    assert!(field(&spec, "output").required);
    assert_eq!(field(&spec, "output").help, "output path");
    assert_eq!(field(&spec, "gap").parameter_type, ParameterType::Int);
    assert_eq!(
        field(&spec, "gap").default,
        Some(ParameterValue::Integer(0))
    );
    assert_eq!(field(&spec, "gap").flag, "--gap"); // derived from the parameter name
    assert_eq!(field(&spec, "fast").parameter_type, ParameterType::Bool);
    assert_eq!(field(&spec, "fast").action, "store_true");
    assert_eq!(
        field(&spec, "label").default,
        Some(ParameterValue::String("x".to_owned()))
    ); // plain literal default becomes an option
    assert_eq!(field(&spec, "label").flag, "--label");
}

#[test]
fn test_typer_run_pattern_reads_the_function() {
    let spec = read_cli("import typer\n\ndef main(n: int = 3):\n    pass\n\ntyper.run(main)\n")
        .unwrap();
    assert_eq!(spec.fields[0].name, "n");
    assert_eq!(spec.fields[0].parameter_type, ParameterType::Int);
    assert_eq!(spec.fields[0].default, Some(ParameterValue::Integer(3)));
}

#[test]
fn test_typer_bool_default_true_degrades_not_guesses() {
    let spec = read_cli(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    let field = &spec.fields[0];
    assert!(field.degraded); // --color/--no-color pairing can't be assembled faithfully
    assert!(field.action.is_empty());
}

#[test]
fn test_typer_underscored_param_gets_kebab_flag() {
    let spec = read_cli(
        "import typer\n\ndef main(max_size: int = 1):\n    pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    assert_eq!(spec.fields[0].flag, "--max-size");
}

#[test]
fn test_typer_two_commands_degrade_as_subcommands() {
    let spec = read_cli(concat!(
        "import typer\napp = typer.Typer()\n",
        "@app.command()\ndef a(x: int = 1): pass\n",
        "@app.command()\ndef b(y: int = 2): pass\n",
    ))
    .unwrap();
    assert!(!spec.ok);
    assert_eq!(spec.reason, Some(DegradationReason::Subcommands));
}

#[test]
fn test_argparse_still_wins_when_present() {
    let source = concat!(
        "import argparse\nimport click\n",
        "ap = argparse.ArgumentParser()\nap.add_argument('--x')\nap.parse_args()\n",
    );
    let spec = read_cli(source).unwrap();
    assert_eq!(spec.framework, "argparse");
    assert_eq!(names(&spec), ["x"]);
}

#[test]
fn test_read_cli_none_for_plain_scripts() {
    assert!(read_cli("print('hi')\n").is_none());
    assert!(read_cli("def broken(:\n").is_none());
}

// --------------------------------------------------------------------------
// mutation hardening: exact contracts for the click/typer readers
// --------------------------------------------------------------------------

#[test]
fn test_click_field_orders_increment_by_one() {
    let spec = read_cli(CLICK_SCRIPT).unwrap();
    // Order is list position now: five decorators -> five fields, in decorator-runtime order.
    assert_eq!((0..spec.fields.len()).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
}

#[test]
fn test_click_from_import_form_is_recognized() {
    let spec = read_cli(concat!(
        "from click import command, option\n@command()\n",
        "@option('--x', type=int)\ndef m(x): pass\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["x"]);
}

#[test]
fn test_click_dotted_import_is_recognized() {
    let spec = read_cli(concat!(
        "import click.decorators\nimport click\n",
        "@click.command()\n@click.option('--x')\ndef m(x): pass\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["x"]);
}

#[test]
fn test_click_secret_name_precheck_and_flag_default() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('--api-key')\n@click.option('--fast', is_flag=True)\n",
        "def m(api_key, fast): pass\n",
    ))
    .unwrap();
    assert!(field(&spec, "api_key").secret);
    assert!(!field(&spec, "fast").secret);
    assert_eq!(
        field(&spec, "fast").default,
        Some(ParameterValue::Bool(false))
    ); // an is_flag option starts unchecked
}

#[test]
fn test_click_uppercase_type_constants() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('--n', type=click.INT)\n",
        "@click.option('--r', type=click.FLOAT)\n",
        "@click.option('--s', type=click.STRING)\n",
        "def m(n, r, s): pass\n",
    ))
    .unwrap();
    // Bottom-up decorator order: STRING, FLOAT, INT.
    assert_eq!(
        spec.fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [ParameterType::Str, ParameterType::Float, ParameterType::Int]
    );
    assert!(spec.fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_click_non_choice_call_type_degrades_even_with_list_arg() {
    // A random callable taking a list is NOT a Choice — it must degrade, never be
    // mistaken for one (kills the and->or mutant in the Choice detection).
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('--c', type=Wrapper(['a', 'b']))\ndef m(c): pass\n",
    ))
    .unwrap();
    assert!(spec.fields[0].degraded);
    assert!(spec.fields[0].choices.is_empty());
}

#[test]
fn test_click_non_literal_default_degrades() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('--bg', default=(255, 255, 255))\ndef m(bg): pass\n",
    ))
    .unwrap();
    assert!(spec.fields[0].degraded);
}

#[test]
fn test_click_multiple_option_flag() {
    let spec = read_cli(
        "import click\n@click.command()\n@click.option('--tag', multiple=True)\ndef m(tag): pass\n",
    )
    .unwrap();
    assert!(spec.fields[0].multiple);
    // click's multiple=True consumes ONE value per occurrence, so assembly must REPEAT the flag
    // (`--tag a --tag b`); `--tag a b` is an exit-2 usage error to click. repeat records that.
    assert!(spec.fields[0].repeat);
}

#[test]
fn test_click_short_flag_only_and_help() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('-v', help='verbosity')\ndef m(v): pass\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].flag, "-v");
    assert_eq!(spec.fields[0].name, "v");
    assert_eq!(spec.fields[0].help, "verbosity");
}

#[test]
fn test_typer_from_import_form_is_recognized() {
    let spec = read_cli(concat!(
        "from typer import Typer\napp = Typer()\n",
        "@app.command()\ndef m(x: int = 1): pass\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["x"]);
}

#[test]
fn test_typer_orders_match_signature_positions() {
    let spec = read_cli(TYPER_SCRIPT).unwrap();
    // Order is list position now: five parameters -> five fields, in signature order.
    assert_eq!((0..spec.fields.len()).collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
}

#[test]
fn test_typer_bare_positional_no_default() {
    let spec = read_cli("import typer\n\ndef main(n: int): pass\n\ntyper.run(main)\n").unwrap();
    let field = &spec.fields[0];
    assert!(field.flag.is_empty()); // positional
    assert!(field.required);
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert!(!field.degraded);
}

#[test]
fn test_typer_unannotated_param_is_plain_text_not_degraded() {
    let spec = read_cli("import typer\n\ndef main(x=1): pass\n\ntyper.run(main)\n").unwrap();
    let field = &spec.fields[0];
    assert_eq!(field.parameter_type, ParameterType::Str);
    assert!(!field.degraded);
    assert_eq!(field.default, Some(ParameterValue::Integer(1)));
}

#[test]
fn test_typer_unmodelable_annotation_degrades() {
    let spec = read_cli("import typer\n\ndef main(xs: list = None): pass\n\ntyper.run(main)\n")
        .unwrap();
    assert!(spec.fields[0].degraded);
}

#[test]
fn test_typer_option_none_default_is_clean() {
    let spec = read_cli(concat!(
        "import typer\n\ndef main(x: str = typer.Option(None, '--x')): pass\n",
        "\ntyper.run(main)\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert!(!field.degraded);
    assert_eq!(field.default, None);
    assert!(!field.required);
}

#[test]
fn test_typer_secret_param_name_precheck() {
    let spec = read_cli(
        "import typer\n\ndef main(api_token: str = ''): pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    assert!(spec.fields[0].secret);
}

#[test]
fn test_decorator_name_unnameable_callable_is_empty() {
    // Python checks `_decorator_name((f())()) == ""`: the callee is itself a Call.
    // The public Rust consequence is that this decorator does not make a Click command.
    assert!(
        read_cli("import click\n@(command())()\ndef main(): pass\n").is_none()
    );
}

#[test]
fn test_click_is_flag_defaulting_on_degrades_not_guesses() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('--color', is_flag=True, default=True)\ndef m(color): pass\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert!(field.degraded); // --color/--no-color pairing can't be assembled faithfully
    assert!(field.action.is_empty());
}

// --------------------------------------------------------------------------
// mutation tail: exact contracts for import guards, orders, dests, defaults
// --------------------------------------------------------------------------

#[test]
fn test_click_dotted_only_import_counts() {
    // `import click.testing` (no plain `import click`) binds the click name at runtime;
    // the import guard must dot-split module paths on BOTH import forms.
    let spec = read_cli(
        "import click.testing\n@click.command()\n@click.option('--x')\ndef m(x): pass\n",
    )
    .unwrap();
    assert_eq!(names(&spec), ["x"]);
}

#[test]
fn test_click_from_dotted_module_counts() {
    let spec = read_cli(concat!(
        "from click.decorators import command, option\n@command()\n",
        "@option('--x')\ndef m(x): pass\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["x"]);
}

#[test]
fn test_typer_dotted_only_import_counts() {
    let spec = read_cli(
        "import typer.main\n\ndef main(n: int = 1):\n    pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    assert_eq!(names(&spec), ["n"]);
}

#[test]
fn test_typer_from_dotted_module_counts() {
    let spec = read_cli(concat!(
        "from typer.main import Typer\napp = Typer()\n",
        "@app.command()\ndef m(x: int = 1): pass\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["x"]);
}

#[test]
fn test_click_two_commands_without_group_degrade() {
    let spec = read_cli(concat!(
        "import click\n",
        "@click.command()\n@click.option('--x')\ndef a(x): pass\n",
        "@click.command()\n@click.option('--y')\ndef b(y): pass\n",
    ))
    .unwrap();
    assert!(!spec.ok);
    assert_eq!(spec.reason, Some(DegradationReason::Subcommands));
}

#[test]
fn test_click_foreign_decorators_between_options_are_skipped_not_fatal() {
    // A bare decorator AND a non-click call decorator sit between two options: the
    // reader must skip them and keep walking (a `break` would silently drop fields).
    let spec = read_cli(concat!(
        "import click\nimport functools\n",
        "@click.command()\n",
        "@click.option('--first')\n",
        "@functools.cache\n",
        "@other.thing()\n",
        "@click.option('--second')\n",
        "def m(first, second): pass\n",
    ))
    .unwrap();
    let mut names = names(&spec);
    names.sort_unstable();
    assert_eq!(names, ["first", "second"]);
}

#[test]
fn test_click_non_literal_name_skips_that_call_only() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option(FLAG_CONST)\n@click.option('--real')\ndef m(real): pass\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["real"]);
}

#[test]
fn test_click_partly_non_literal_names_skip_that_call_only() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('-x', EXTRA)\n@click.option('--real')\ndef m(x, real): pass\n",
    ))
    .unwrap();
    assert_eq!(names(&spec), ["real"]);
}

#[test]
fn test_click_short_first_declaration_still_prefers_long_flag() {
    let spec = read_cli(
        "import click\n@click.command()\n@click.option('-o', '--output')\ndef m(output): pass\n",
    )
    .unwrap();
    assert_eq!(spec.fields[0].flag, "--output");
    assert_eq!(spec.fields[0].name, "output");
}

#[test]
fn test_click_dest_strips_dashes_not_letters() {
    let spec = read_cli(
        "import click\n@click.command()\n@click.option('--Xray')\ndef m(xray): pass\n",
    )
    .unwrap();
    assert_eq!(spec.fields[0].name, "Xray");
}

#[test]
fn test_click_default_none_is_clean() {
    let spec = read_cli(
        "import click\n@click.command()\n@click.option('--x', default=None)\ndef m(x): pass\n",
    )
    .unwrap();
    assert!(!spec.fields[0].degraded);
    assert_eq!(spec.fields[0].default, None);
}

#[test]
fn test_click_bare_float_and_str_types() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('--r', type=float)\n@click.option('--s', type=str)\n",
        "def m(r, s): pass\n",
    ))
    .unwrap();
    // bottom-up: s first, then r
    assert_eq!(
        spec.fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [ParameterType::Str, ParameterType::Float]
    );
    assert!(spec.fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_click_unknown_name_type_degrades() {
    let spec = read_cli(
        "import click\n@click.command()\n@click.option('--p', type=parse_color)\ndef m(p): pass\n",
    )
    .unwrap();
    assert!(spec.fields[0].degraded); // an arbitrary callable we won't execute
}

#[test]
fn test_click_path_and_file_types() {
    // click.Path(...)/click.File(...) calls and a bare pathlib Path callable are all
    // path signals; keyword refinements (exists=, dir_okay=, ...) are not modelled.
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('--src', type=click.Path(exists=True))\n",
        "@click.option('--out', type=click.File('w'))\n",
        "@click.option('--raw', type=Path)\n",
        "def m(src, out, raw): pass\n",
    ))
    .unwrap();
    assert_eq!(
        spec.fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [ParameterType::Path, ParameterType::Path, ParameterType::Path]
    );
    assert!(spec.fields.iter().all(|field| !field.degraded));
}

#[test]
fn test_typer_option_extra_decl_positions() {
    // The long flag may not be the FIRST declaration after the default.
    let spec = read_cli(concat!(
        "import typer\n\ndef main(out: str = typer.Option('x', '-o', '--renamed')):\n",
        "    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].flag, "--renamed");
    assert_eq!(
        spec.fields[0].default,
        Some(ParameterValue::String("x".to_owned()))
    );
}

#[test]
fn test_typer_path_annotation_is_path() {
    // A Path annotation is a path signal, in both the legacy-default and the
    // Annotated spellings.
    let spec = read_cli(concat!(
        "import typer\nfrom pathlib import Path\nfrom typing import Annotated\n\n",
        "def main(src: Path, out: Annotated[Path, typer.Option()] = Path('o.txt')):\n",
        "    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert_eq!(
        spec.fields
            .iter()
            .map(|field| field.parameter_type)
            .collect::<Vec<_>>(),
        [ParameterType::Path, ParameterType::Path]
    );
}

#[test]
fn test_typer_non_constant_decl_is_ignored_not_fatal() {
    let spec = read_cli(concat!(
        "import typer\n\ndef main(out: str = typer.Option('x', SOME_DECL)):\n",
        "    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].flag, "--out"); // falls back to the derived flag
    assert_eq!(
        spec.fields[0].default,
        Some(ParameterValue::String("x".to_owned()))
    );
}

#[test]
fn test_typer_computed_plain_default_degrades() {
    let spec = read_cli(
        "import typer\n\ndef main(x: int = make_default()):\n    pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    assert!(spec.fields[0].degraded);
}

#[test]
fn test_typer_option_computed_first_arg_degrades() {
    let spec = read_cli(
        "import typer\n\ndef main(x: str = typer.Option(CONST_REF)):\n    pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    assert!(spec.fields[0].degraded);
}

#[test]
fn test_typer_bool_true_degrade_renders_as_text() {
    let spec = read_cli(
        "import typer\n\ndef main(color: bool = True):\n    pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    let field = &spec.fields[0];
    assert!(field.degraded);
    assert_eq!(field.parameter_type, ParameterType::Str); // the degrade path pins the free-text kind exactly
}

#[test]
fn test_typer_bool_false_flag_contract_exact() {
    let spec = read_cli(
        "import typer\n\ndef main(fast: bool = False):\n    pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    let field = &spec.fields[0];
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.action, "store_true");
    assert_eq!(field.default, Some(ParameterValue::Bool(false)));
    assert!(!field.degraded);
}

#[test]
fn test_click_non_literal_choice_list_degrades() {
    let spec = read_cli(concat!(
        "import click\n@click.command()\n",
        "@click.option('--mode', type=click.Choice(MODES))\ndef m(mode): pass\n",
    ))
    .unwrap();
    assert!(spec.fields[0].degraded);
    assert_ne!(spec.fields[0].parameter_type, ParameterType::Choice);
}

#[test]
fn test_typer_unmodelable_annotation_degrades_despite_literal_default() {
    // The annotation-driven degrade must hold on its own — a clean literal default
    // (which does NOT degrade) must not mask it.
    let spec = read_cli(
        "import typer\n\ndef main(mode: dict = 'x'):\n    pass\n\ntyper.run(main)\n",
    )
    .unwrap();
    assert!(spec.fields[0].degraded);
}

#[test]
fn test_typer_option_single_extra_decl_is_read() {
    // The declaration list starts right AFTER the default (args[1:], not args[2:]).
    let spec = read_cli(concat!(
        "import typer\n\ndef main(out: str = typer.Option('x', '--renamed')):\n",
        "    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].flag, "--renamed");
}

// --------------------------------------------------------------------------
// typer Annotated[...] (A6) — the modern style AI-written typer scripts use
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
    let spec = read_cli(ANNOTATED_SCRIPT).unwrap();
    assert!(spec.ok);
    // Argument -> required positional, type from Annotated's first arg
    assert!(field(&spec, "name").flag.is_empty());
    assert!(field(&spec, "name").required);
    assert_eq!(field(&spec, "name").parameter_type, ParameterType::Str);
    assert_eq!(field(&spec, "name").help, "who");
    // Option with a literal `= default`, type int
    assert_eq!(field(&spec, "count").parameter_type, ParameterType::Int);
    assert_eq!(
        field(&spec, "count").default,
        Some(ParameterValue::Integer(3))
    );
    assert_eq!(field(&spec, "count").flag, "--count");
    assert_eq!(field(&spec, "count").help, "how many");
    assert!(!field(&spec, "count").degraded);
    // Explicit flag declarations inside the Annotated Option
    assert_eq!(field(&spec, "mode").flag, "--mode");
    assert_eq!(
        field(&spec, "mode").default,
        Some(ParameterValue::String("fast".to_owned()))
    );
    // bool option defaulting False -> checkbox
    assert_eq!(field(&spec, "fast").parameter_type, ParameterType::Bool);
    assert_eq!(field(&spec, "fast").action, "store_true");
}

#[test]
fn test_annotated_option_without_default_is_required() {
    let spec = read_cli(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[int, typer.Option()]):\n    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert_eq!(field.flag, "--x"); // still an option, not a positional
    assert!(field.required);
    assert_eq!(field.parameter_type, ParameterType::Int);
}

#[test]
fn test_annotated_argument_with_default_is_optional_positional() {
    let spec = read_cli(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(name: Annotated[str, typer.Argument()] = 'anon'):\n    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert!(field.flag.is_empty());
    assert!(!field.required);
    assert_eq!(
        field.default,
        Some(ParameterValue::String("anon".to_owned()))
    );
}

#[test]
fn test_annotated_unmodelable_inner_type_degrades() {
    let spec = read_cli(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(tags: Annotated[list, typer.Option()] = None):\n    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert!(spec.fields[0].degraded);
}

#[test]
fn test_annotated_bool_default_true_degrades() {
    let spec = read_cli(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(color: Annotated[bool, typer.Option()] = True):\n    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert!(field.degraded);
    assert!(field.action.is_empty()); // not a store_true — the --no-color pairing can't be assembled
}

#[test]
fn test_annotated_choice_via_typing_qualified_name() {
    // `typing.Annotated` (attribute form) must be recognized too, not just a bare name.
    let spec = read_cli(concat!(
        "import typer\nimport typing\n",
        "def main(gap: typing.Annotated[int, typer.Option()] = 0):\n    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].parameter_type, ParameterType::Int);
    assert_eq!(spec.fields[0].default, Some(ParameterValue::Integer(0)));
}

#[test]
fn test_annotated_help_kwarg_survives_on_degraded_field() {
    let spec = read_cli(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[MyType, typer.Option(help='hint')] = None):\n",
        "    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert!(spec.fields[0].degraded);
    assert_eq!(spec.fields[0].help, "hint"); // the user's hint still reaches the form
}

#[test]
fn test_legacy_typer_style_still_works_after_annotated_refactor() {
    // Regression: the non-Annotated `x: int = typer.Option(5, '--renamed')` path must be
    // untouched by the Annotated addition.
    let spec = read_cli(TYPER_SCRIPT).unwrap();
    assert!(field(&spec, "output").required);
    assert_eq!(
        field(&spec, "gap").default,
        Some(ParameterValue::Integer(0))
    );
    assert_eq!(field(&spec, "fast").action, "store_true");
}

#[test]
fn test_annotated_only_recognizes_the_real_annotated_name() {
    // A qualified subscript that is NOT typing.Annotated must not be unwrapped as one.
    let spec = read_cli(concat!(
        "import typer\nimport mod\n",
        "def main(x: mod.Wrapper[int, typer.Option()] = 1):\n    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert!(spec.fields[0].degraded); // unknown subscript type -> free text, not int
}

#[test]
fn test_annotated_without_typer_metadata_reads_as_plain_type() {
    // Annotated with only a doc string (no typer.Option/Argument): the field is still a
    // plain int with its signature default — the meta search must return None, not raise.
    let spec = read_cli(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[int, 'a note'] = 5):\n    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert_eq!(field.default, Some(ParameterValue::Integer(5)));
    assert!(!field.degraded);
}

#[test]
fn test_annotated_picks_the_typer_call_among_several() {
    // Two calls in the metadata: the typer one must be chosen, not the first call.
    let spec = read_cli(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[int, Validator(), typer.Option(help='H')] = 5):\n",
        "    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    assert_eq!(spec.fields[0].help, "H");
}

#[test]
fn test_annotated_option_positional_decl_is_a_flag_not_a_default() {
    // In the Annotated style, Option's positional strings are flag DECLARATIONS, never a
    // value default (that lives in the `= value`). A required Annotated Option with two
    // long decls keeps the first as the flag and stays default-less.
    let spec = read_cli(concat!(
        "import typer\nfrom typing import Annotated\n",
        "def main(x: Annotated[str, typer.Option('--primary', '--secondary')]):\n",
        "    pass\n\ntyper.run(main)\n",
    ))
    .unwrap();
    let field = &spec.fields[0];
    assert_eq!(field.flag, "--primary");
    assert_eq!(field.default, None); // not '--primary' (the True mutant would read it as a default)
    assert!(field.required);
}
