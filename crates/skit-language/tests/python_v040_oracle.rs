use std::{collections::BTreeMap, fs};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    CliSurface, DegradationReason, LanguageError, ParseOutcome, ParsedDocument,
    external_dependencies_at, parse_document, source_is_valid,
};
use tempfile::TempDir;

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid Python, got {other:?}"),
    }
}

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

fn inputs(source: &str) -> Vec<ParamDecl> {
    parsed(source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .filter(|declaration| declaration.binding == ParameterBinding::Input)
        .collect()
}

#[test]
fn origin_main_analyzer_literal_duplicate_main_guard_and_signal_oracle() {
    let source = concat!(
        "import typer, click\n",
        "import sys\n",
        "X = 1\nY = 5\nX = -2\n",
        "FLAG: bool = False\n",
        "(WRAPPED) = 8\n",
        "if __name__ != '__main__':\n    WRONG = 1\n",
        "if '__main__' == __name__:\n    X = 99\n    Z = +2.5\n",
        "if ((__name__) == ('__main__')):\n    PAREN = 7\n",
        "open(1, 'notes.txt')\n",
        "print((sys).argv, __file__)\n",
    );
    let analysis = parsed(source).analysis();
    let declarations = analysis
        .candidates
        .iter()
        .map(|candidate| &candidate.declaration)
        .collect::<Vec<_>>();
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["X", "Y", "FLAG", "WRAPPED", "Z", "PAREN"]
    );
    assert_eq!(declarations[0].default, Some(ParameterValue::Integer(-2)));
    assert_eq!(declarations[2].parameter_type, ParameterType::Bool);
    assert_eq!(declarations[4].default, Some(ParameterValue::Float(2.5)));
    assert_eq!(analysis.frameworks, ["typer", "click"]);
    assert!(analysis.uses_argv && analysis.uses_self_location);
    assert_eq!(analysis.filename_literals, ["notes.txt"]);

    assert!(
        parsed("from .click import command\n")
            .analysis()
            .frameworks
            .is_empty()
    );
}

#[test]
fn origin_main_builtin_input_scope_oracle_covers_all_binding_families() {
    for shadowed in [
        "def input(prompt=''): return 'x'\nvalue=input('Q: ')\n",
        "input = str\nvalue=input('Q: ')\n",
        "from provider import input\nvalue=input('Q: ')\n",
        "import input.sub\nvalue=input('Q: ')\n",
        "from provider import *\nvalue=input('Q: ')\n",
        "for input in values: pass\nvalue=input('Q: ')\n",
    ] {
        assert!(inputs(shadowed).is_empty(), "source={shadowed}");
    }

    let independent = concat!(
        "import input.sub as other\n",
        "xs = [input for input in range(3)]\n",
        "identity = lambda input: input\n",
        "def a(input): return input('Own: ')\n",
        "def b(): return input('B: ')\n",
        "wrapped = (input)('Wrapped: ')\n",
        "top = input('Top: ')\n",
    );
    assert_eq!(
        inputs(independent)
            .iter()
            .map(|declaration| declaration.prompt.as_str())
            .collect::<Vec<_>>(),
        ["B: ", "Wrapped: ", "Top: "]
    );

    let local_exception = concat!(
        "def local():\n",
        "    try: pass\n",
        "    except ValueError as input: pass\n",
        "    return input('Own: ')\n",
        "top = input('Top: ')\n",
    );
    assert_eq!(
        inputs(local_exception)
            .iter()
            .map(|declaration| declaration.prompt.as_str())
            .collect::<Vec<_>>(),
        ["Top: "]
    );
}

#[test]
fn origin_main_argparse_oracle_covers_static_skip_and_degrade_rules() {
    let fields = static_fields(concat!(
        "import argparse, pathlib\n",
        "DEFAULT = 3\n",
        "p = argparse.ArgumentParser()\n",
        "p.add_argument('files', nargs='*', type=pathlib.Path)\n",
        "p.add_argument('-o', '--output', dest='target', required=True, type=argparse.FileType('w'))\n",
        "p.add_argument('--mode', choices=['fast', 2, True], type=Path, default='fast')\n",
        "p.add_argument('--count', type=int, default=DEFAULT)\n",
        "p.add_argument('--append', action='append', help='keep me')\n",
        "p.add_argument('--opaque', choices=MODES)\n",
        "p.add_argument('--wrapped', choices=(['a', 'b']), type=(int))\n",
        "p.add_argument(FLAG_NAME)\n",
        "p.add_argument('-x', EXTRA)\n",
        "p.add_argument('--help', action='help')\n",
        "p.add_argument('--version', action='version')\n",
    ));
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        [
            "files", "target", "mode", "count", "append", "opaque", "wrapped"
        ]
    );
    assert!(fields[0].multiple && !fields[0].required);
    assert_eq!(fields[0].parameter_type, ParameterType::Path);
    assert_eq!(fields[1].flag, "--output");
    assert!(fields[1].required);
    assert_eq!(fields[2].parameter_type, ParameterType::Choice);
    assert_eq!(fields[2].choices, ["fast", "2", "True"]);
    assert_eq!(fields[3].default, Some(ParameterValue::Integer(3)));
    assert!(fields[4].degraded && fields[5].degraded);
    assert_eq!(fields[4].help, "keep me");
    assert_eq!(fields[6].parameter_type, ParameterType::Choice);
    assert!(!fields[6].degraded);
}

#[test]
fn origin_main_constant_environment_refuses_secret_and_rebound_defaults() {
    let fields = static_fields(concat!(
        "SAFE = 3\n",
        "API_KEY = 'secret'\n",
        "REBOUND = 1\n",
        "def helper(REBOUND): pass\n",
        "p.add_argument('--safe', default=SAFE)\n",
        "p.add_argument('--secret', default=API_KEY)\n",
        "p.add_argument('--rebound', default=REBOUND)\n",
    ));
    assert_eq!(fields[0].default, Some(ParameterValue::Integer(3)));
    assert!(!fields[0].degraded);
    assert!(fields[1].default.is_none() && fields[1].degraded);
    assert!(fields[2].default.is_none() && fields[2].degraded);
}

#[test]
fn origin_main_click_oracle_covers_imports_order_types_and_lossless_arity() {
    let fields = static_fields(concat!(
        "from click.decorators import command, option\n",
        "@command()\n",
        "@option('-o', '--output', type=click.Path(exists=True), required=True)\n",
        "@foreign.cache\n",
        "@option('--mode', type=click.Choice(['a', 'b']), default='a')\n",
        "@option('--tag', multiple=True)\n",
        "@option('--pair', nargs=2)\n",
        "@option('--drop', nargs=2, multiple=True)\n",
        "@option(FLAG_NAME)\n",
        "def main(output, mode, tag, pair, drop): pass\n",
    ));
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["pair", "tag", "mode", "output"]
    );
    assert!(fields[0].multiple && !fields[0].repeat);
    assert!(fields[1].multiple && fields[1].repeat);
    assert_eq!(fields[2].parameter_type, ParameterType::Choice);
    assert_eq!(fields[3].parameter_type, ParameterType::Path);
    assert!(fields[3].required);
}

#[test]
fn origin_main_click_boolean_and_dynamic_type_oracle_degrades_without_guessing() {
    let fields = static_fields(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--on', is_flag=True)\n",
        "@click.option('--off', is_flag=True, default=True)\n",
        "@click.option('--count', count=True)\n",
        "@click.option('--custom', type=parse_color)\n",
        "def main(on, off, count, custom): pass\n",
    ));
    let by_name = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(by_name["on"].parameter_type, ParameterType::Bool);
    assert_eq!(by_name["on"].action, "store_true");
    assert_eq!(by_name["on"].default, Some(ParameterValue::Bool(false)));
    assert!(by_name["off"].degraded && by_name["off"].action.is_empty());
    assert!(by_name["count"].degraded && by_name["custom"].degraded);

    assert!(matches!(
        parsed("import click\n@(command())()\ndef main(): pass\n").cli_surface(),
        CliSurface::Absent
    ));
}

#[test]
fn origin_main_typer_legacy_and_modern_oracle_preserves_signature_semantics() {
    let fields = static_fields(concat!(
        "import typer, typing\n",
        "from pathlib import Path\n",
        "def main(\n",
        "  src: Path,\n",
        "  output: str = typer.Option(..., '-o', '--output', help='Out'),\n",
        "  count: int = 3,\n",
        "  fast: bool = False,\n",
        "  color: bool = True,\n",
        "  note: typing.Annotated[MyType, Validator(), typer.Option(help='Hint')] = None,\n",
        "): pass\n",
        "typer.run(main)\n",
    ));
    let by_name = fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect::<BTreeMap<_, _>>();
    assert!(by_name["src"].flag.is_empty() && by_name["src"].required);
    assert_eq!(by_name["src"].parameter_type, ParameterType::Path);
    assert_eq!(by_name["output"].flag, "--output");
    assert!(by_name["output"].required);
    assert_eq!(by_name["output"].help, "Out");
    assert_eq!(by_name["count"].default, Some(ParameterValue::Integer(3)));
    assert_eq!(by_name["fast"].action, "store_true");
    assert!(by_name["color"].degraded && by_name["color"].action.is_empty());
    assert_eq!(by_name["color"].parameter_type, ParameterType::Str);
    assert!(by_name["note"].degraded);
    assert_eq!(by_name["note"].help, "Hint");
}

#[test]
fn origin_main_reconcile_duplicate_prompt_oracle_is_one_to_one() {
    let original = parsed("a=input('Go? ')\nb=input('Go? ')\n");
    let managed = original
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    let one_left = parsed("b=input('Go? ')\n");
    let report = one_left.reconcile(&managed);
    assert_eq!(report.ok.len(), 1);
    assert_eq!(report.missing.len(), 1);
    assert!(report.rebound.is_empty());

    let renamed = parsed("a=input('Go? ')\nb=input('Different: ')\n");
    let report = renamed.reconcile(&managed);
    assert_eq!(report.ok.len(), 1);
    assert_eq!(report.rebound.len(), 1);
    assert!(report.missing.is_empty());
}

#[test]
fn origin_main_reconcile_never_publishes_a_secret_source_default() {
    let document = parsed("API_TOKEN = 'source-secret'\n");
    let stored = document.analysis().candidates[0].declaration.clone();
    assert!(stored.secret);
    let report = document.reconcile(&[stored]);
    assert_eq!(report.ok.len(), 1);
    assert!(report.current_defaults.is_empty());
}

#[test]
fn origin_main_injection_oracle_rewrites_only_semantic_targets() {
    let source = concat!(
        "\"\"\"Module doc.\"\"\"\n",
        "from __future__ import annotations\n",
        "CITY = 'a'\n",
        "if __name__ == '__main__':\n    CITY = 'b'\n",
        "name = input('Name: ')\n",
    );
    let document = parsed(source);
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    let output = document
        .plan_injection(
            &declarations,
            &BTreeMap::from([
                ("CITY".to_owned(), "Zürich\u{2028}".to_owned()),
                ("input-1".to_owned(), "A\0da".to_owned()),
            ]),
        )
        .unwrap()
        .apply(source)
        .unwrap();
    assert_eq!(output.matches("CITY = 'Zürich\\u2028'").count(), 2);
    assert!(output.contains("name = _skit_i[0]('Name: ')"));
    assert!(output.contains("'A\\x00da'"));
    assert!(
        output
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("from __future__")
    );
    assert!(output.lines().nth(2).unwrap().ends_with("# skit:shim"));
    assert!(source_is_valid("python", &output));
}

#[test]
fn origin_main_typed_injection_refuses_invalid_int_float_and_bool() {
    for (source, parameter_type, raw) in [
        ("VALUE = 1\n", ParameterType::Int, "many"),
        ("VALUE = 1.0\n", ParameterType::Float, "nan"),
        ("VALUE = True\n", ParameterType::Bool, "maybe"),
    ] {
        let document = parsed(source);
        let mut declaration = document.analysis().candidates[0].declaration.clone();
        declaration.parameter_type = parameter_type;
        let error = document
            .plan_injection(
                &[declaration],
                &BTreeMap::from([("VALUE".to_owned(), raw.to_owned())]),
            )
            .unwrap_err();
        assert!(matches!(error, LanguageError::InvalidValue { .. }));
    }
}

#[test]
fn every_reflected_field_keeps_none_binding_and_flag_delivery() {
    for source in [
        "p.add_argument('--x')\n",
        "import click\n@click.command()\n@click.option('--x')\ndef main(x): pass\n",
        "import typer\ndef main(x: int = 1): pass\ntyper.run(main)\n",
    ] {
        let fields = static_fields(source);
        assert_eq!(fields[0].binding, ParameterBinding::None);
        assert_eq!(fields[0].delivery, ParameterDelivery::Flag);
    }
}

#[test]
fn dynamic_surfaces_keep_typed_reasons() {
    assert!(matches!(
        parsed("p.add_argument('--x')\np.add_subparsers()\n").cli_surface(),
        CliSurface::Dynamic(surface)
            if surface.reason == DegradationReason::Subcommands
    ));
    assert!(matches!(
        parsed("for name in NAMES:\n    p.add_argument(name)\n").cli_surface(),
        CliSurface::Dynamic(surface)
            if surface.reason == DegradationReason::DynamicDeclaration
    ));
}

#[test]
fn origin_main_import_oracle_uses_syntax_nodes_across_line_continuations() {
    let document = parsed(concat!(
        "import click, \\",
        "\n    typer\n",
        "from provider import \\",
        "\n    input\n",
        "value = input('not builtin')\n",
    ));
    assert_eq!(document.analysis().frameworks, ["click", "typer"]);
    assert!(
        document
            .analysis()
            .candidates
            .iter()
            .all(|candidate| candidate.declaration.binding != ParameterBinding::Input)
    );
}

#[test]
fn origin_main_filename_hint_oracle_counts_the_stem_not_the_extension() {
    let longest_stem = "a".repeat(120);
    let too_long_stem = "b".repeat(121);
    let source = format!("open('{longest_stem}.txt')\nopen('{too_long_stem}.txt')\nopen('.txt')\n");
    assert_eq!(
        parsed(&source).analysis().filename_literals,
        [format!("{longest_stem}.txt")]
    );
}

#[test]
fn origin_main_local_dependency_oracle_distinguishes_python_from_data_directories() {
    let directory = TempDir::new().unwrap();
    fs::create_dir(directory.path().join("helpers")).unwrap();
    fs::write(
        directory.path().join("helpers").join("module.py"),
        "VALUE = 1\n",
    )
    .unwrap();
    fs::create_dir(directory.path().join("rich")).unwrap();
    fs::write(directory.path().join("rich").join("notes.txt"), "data\n").unwrap();

    assert_eq!(
        external_dependencies_at(
            "python",
            "import helpers\nimport rich\n",
            Some(directory.path()),
        ),
        ["rich"]
    );
}
