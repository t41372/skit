use std::collections::BTreeMap;

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    CliSurface, DegradationReason, LanguageError, ParseOutcome, parse_document,
    write_managed_params,
};

fn parsed(source: &str) -> skit_language::ParsedDocument {
    match parse_document("python", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected a parsed document, got {other:?}"),
    }
}

fn field<'a>(fields: &'a [skit_language::SemanticField], name: &str) -> &'a ParamDecl {
    &fields
        .iter()
        .find(|field| field.declaration.name == name)
        .unwrap_or_else(|| panic!("missing field {name}"))
        .declaration
}

#[test]
fn parse_outcome_and_cli_surface_keep_absent_empty_and_dynamic_distinct() {
    assert!(matches!(
        parse_document("python", "def broken(:\n"),
        ParseOutcome::SyntaxError(_)
    ));

    assert!(matches!(
        parsed("print('plain')\n").cli_surface(),
        CliSurface::Absent
    ));

    let empty = parsed("p.add_argument('--help', action='help')\n");
    let CliSurface::Static(surface) = empty.cli_surface() else {
        panic!("a readable zero-field surface must stay static");
    };
    assert_eq!(surface.framework, "argparse");
    assert!(surface.fields.is_empty());

    let subcommands =
        parsed("p.add_argument('--name')\nsub = p.add_subparsers()\nsub.add_parser('show')\n");
    let CliSurface::Dynamic(surface) = subcommands.cli_surface() else {
        panic!("subparsers must publish an explicit dynamic surface");
    };
    assert_eq!(surface.framework, "argparse");
    assert_eq!(surface.reason, DegradationReason::Subcommands);

    let loop_generated = parsed("for flag in FLAGS:\n    p.add_argument(flag)\n");
    let CliSurface::Dynamic(surface) = loop_generated.cli_surface() else {
        panic!("loop-generated declarations must publish a dynamic surface");
    };
    assert_eq!(surface.reason, DegradationReason::DynamicDeclaration);
}

#[test]
fn python_analysis_uses_exact_main_guard_scope_demotions_and_stable_spans() {
    let source = concat!(
        "TOP = 1\n",
        "COUNT = 0\n",
        "COUNT += 1\n",
        "def nested():\n    LOCAL = 2\n",
        "if __name__ != '__main__':\n    WRONG = 3\n",
        "if '__main__' == __name__:\n    TOP = 9\n    GUARD = 4\n",
        "for item in range(2):\n    LOOP = 0\n    LOOP = item\n",
        "first = input('First: ')\n",
        "def ask():\n    return input('Second: ')\n",
    );
    let document = parsed(source);
    let analysis = document.analysis();
    let names = analysis
        .candidates
        .iter()
        .map(|candidate| candidate.declaration.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["TOP", "COUNT", "GUARD", "input-1", "input-2"]);
    assert_eq!(
        analysis.candidates[0].declaration.default,
        Some(ParameterValue::Integer(1)),
        "module scope wins over a same-name main-guard override"
    );
    assert_eq!(
        analysis.candidates[1].demotion,
        Some(DegradationReason::Accumulator)
    );
    for candidate in &analysis.candidates {
        assert!(candidate.span.start < candidate.span.end);
        assert!(candidate.span.start_line >= 1);
    }
    assert_eq!(analysis.candidates[3].declaration.prompt, "First: ");
    assert_eq!(analysis.candidates[4].declaration.prompt, "Second: ");
}

#[test]
fn prompt_identity_drives_reconcile_and_injection_before_order_fallback() {
    let original = parsed("name = input('Name: ')\n");
    let managed = original.analysis().candidates[0].declaration.clone();

    let edited = parsed("age = input('Age: ')\nname = input('Name: ')\n");
    let report = edited.reconcile(std::slice::from_ref(&managed));
    assert_eq!(report.ok.len(), 1);
    assert!(report.rebound.is_empty());
    assert!(report.missing.is_empty());

    let plan = edited
        .plan_injection(
            std::slice::from_ref(&managed),
            &BTreeMap::from([(managed.name.clone(), "Ada".to_owned())]),
        )
        .unwrap();
    assert!(matches!(
        plan.apply("name = input('changed')\n"),
        Err(LanguageError::SourceChanged)
    ));
    let rewritten = plan.apply(edited.source()).unwrap();
    assert!(rewritten.contains("age = input('Age: ')"));
    assert!(rewritten.contains("name = _skit_i[1]('Name: ')"));
    assert!(rewritten.contains("'Ada'"));
}

#[test]
fn injection_rejects_typed_conversion_instead_of_quoting_it() {
    let document = parsed("COUNT = 1\n");
    let declaration = document.analysis().candidates[0].declaration.clone();
    let error = document
        .plan_injection(
            &[declaration],
            &BTreeMap::from([("COUNT".to_owned(), "many".to_owned())]),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        LanguageError::InvalidValue {
            name,
            parameter_type: ParameterType::Int,
            ..
        } if name == "COUNT"
    ));
}

#[test]
fn argparse_reflection_restores_dest_nargs_paths_defaults_actions_and_degradation() {
    let source = r#"
import argparse
DEFAULT_COUNT = 3
p = argparse.ArgumentParser()
p.add_argument("input", nargs="?", type=pathlib.Path, help="Input")
p.add_argument("items", nargs="+")
p.add_argument("-n", "--number", dest="count", type=int, default=DEFAULT_COUNT)
p.add_argument("--mode", choices=["fast", 2, True], default="fast")
p.add_argument("--verbose", action="store_true")
p.add_argument("--quiet", action="store_false")
p.add_argument("--append", action="append")
p.add_argument("--help", action="help")
"#;
    let CliSurface::Static(surface) = parsed(source).cli_surface() else {
        panic!("argparse must be static");
    };
    assert_eq!(surface.framework, "argparse");
    assert_eq!(surface.fields.len(), 7);
    let input = field(&surface.fields, "input");
    assert_eq!(input.parameter_type, ParameterType::Path);
    assert!(!input.required);
    assert!(input.flag.is_empty());
    let items = field(&surface.fields, "items");
    assert!(items.required && items.multiple);
    let count = field(&surface.fields, "count");
    assert_eq!(count.flag, "--number");
    assert_eq!(count.default, Some(ParameterValue::Integer(3)));
    let mode = field(&surface.fields, "mode");
    assert_eq!(mode.parameter_type, ParameterType::Choice);
    assert_eq!(mode.choices, ["fast", "2", "True"]);
    assert_eq!(
        field(&surface.fields, "verbose").default,
        Some(ParameterValue::Bool(false))
    );
    assert_eq!(
        field(&surface.fields, "quiet").default,
        Some(ParameterValue::Bool(true))
    );
    assert!(field(&surface.fields, "append").degraded);
}

#[test]
fn click_reflection_restores_arguments_runtime_order_types_and_group_degradation() {
    let source = r#"
import click
@click.command()
@click.option("-n", "--number", type=click.INT, default=2)
@click.option("--mode", type=click.Choice(["fast", "safe"]))
@click.argument("paths", nargs=-1, type=click.Path())
def main(number, mode, paths): pass
"#;
    let CliSurface::Static(surface) = parsed(source).cli_surface() else {
        panic!("click must be static");
    };
    assert_eq!(surface.framework, "click");
    assert_eq!(
        surface
            .fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["paths", "mode", "number"]
    );
    let paths = field(&surface.fields, "paths");
    assert!(paths.flag.is_empty() && paths.multiple && !paths.required);
    assert_eq!(paths.parameter_type, ParameterType::Path);
    assert_eq!(
        field(&surface.fields, "mode").parameter_type,
        ParameterType::Choice
    );

    let grouped = parsed("import click\n@click.group()\ndef root(): pass\n");
    assert!(matches!(
        grouped.cli_surface(),
        CliSurface::Dynamic(surface)
            if surface.framework == "click"
                && surface.reason == DegradationReason::Subcommands
    ));
}

#[test]
fn modern_typer_reflection_restores_annotated_plain_path_and_multicommand_contracts() {
    let source = r#"
import pathlib
import typer
from typing import Annotated
app = typer.Typer()
@app.command()
def run(
    name: Annotated[str, typer.Option("--name", help="Name")],
    path: pathlib.Path = typer.Argument(...),
    count: int = 3,
    verbose: bool = False,
    plain = "text",
): pass
"#;
    let CliSurface::Static(surface) = parsed(source).cli_surface() else {
        panic!("Typer must be static");
    };
    assert_eq!(surface.framework, "typer");
    let name = field(&surface.fields, "name");
    assert_eq!(name.flag, "--name");
    assert!(name.required);
    assert_eq!(name.help, "Name");
    let path = field(&surface.fields, "path");
    assert_eq!(path.parameter_type, ParameterType::Path);
    assert!(path.flag.is_empty() && path.required);
    assert_eq!(
        field(&surface.fields, "count").default,
        Some(ParameterValue::Integer(3))
    );
    let verbose = field(&surface.fields, "verbose");
    assert_eq!(verbose.action, "store_true");
    assert_eq!(verbose.default, Some(ParameterValue::Bool(false)));
    assert_eq!(
        field(&surface.fields, "plain").default,
        Some(ParameterValue::String("text".to_owned()))
    );

    let multi = parsed(
        "import typer\napp=typer.Typer()\n@app.command()\ndef one(): pass\n@app.command()\ndef two(): pass\n",
    );
    assert!(matches!(
        multi.cli_surface(),
        CliSurface::Dynamic(surface)
            if surface.framework == "typer"
                && surface.reason == DegradationReason::Subcommands
    ));
}

#[test]
fn the_first_python_cli_framework_wins() {
    let source = r#"
import argparse, click, typer
p = argparse.ArgumentParser()
p.add_argument("--from-argparse")
@click.command()
@click.option("--from-click")
def click_main(from_click): pass
def typer_main(from_typer: str = typer.Option("x")): pass
typer.run(typer_main)
"#;
    let CliSurface::Static(surface) = parsed(source).cli_surface() else {
        panic!("argparse must win");
    };
    assert_eq!(surface.framework, "argparse");
    assert_eq!(surface.fields.len(), 1);
    assert_eq!(surface.fields[0].declaration.name, "from_argparse");
}

#[test]
fn python_metadata_insertion_keeps_shebang_coding_line_crlf_and_source_bytes() {
    let source = "#!/usr/bin/env python3\r\n# -*- coding: latin-1 -*-\r\nVALUE = 'ok'\r\n";
    let mut declaration = ParamDecl::new("VALUE");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    let written = write_managed_params("python", source, &[declaration]).unwrap();
    let mut lines = written.split_inclusive("\r\n");
    assert_eq!(lines.next(), Some("#!/usr/bin/env python3\r\n"));
    assert_eq!(lines.next(), Some("# -*- coding: latin-1 -*-\r\n"));
    assert!(written.contains("# /// script\r\n"));
    assert!(written.ends_with("VALUE = 'ok'\r\n"));
    assert!(!written.replace("\r\n", "").contains('\n'));

    let second_line_cookie = "# generated source\r\n# coding: utf-8\r\nVALUE = 'ok'\r\n";
    let written = write_managed_params(
        "python",
        second_line_cookie,
        &[ParamDecl {
            binding: ParameterBinding::Const,
            delivery: ParameterDelivery::Inject,
            ..ParamDecl::new("VALUE")
        }],
    )
    .unwrap();
    assert!(written.starts_with("# generated source\r\n# coding: utf-8\r\n# /// script\r\n"));
    assert!(written.ends_with("VALUE = 'ok'\r\n"));

    let header_only = write_managed_params(
        "python",
        "#!/usr/bin/env python3",
        &[ParamDecl {
            binding: ParameterBinding::Const,
            delivery: ParameterDelivery::Inject,
            ..ParamDecl::new("VALUE")
        }],
    )
    .unwrap();
    assert!(header_only.starts_with("#!/usr/bin/env python3\n# /// script\n"));

    let with_bom = write_managed_params(
        "python",
        "\u{feff}VALUE = 'ok'\n",
        &[ParamDecl {
            binding: ParameterBinding::Const,
            delivery: ParameterDelivery::Inject,
            ..ParamDecl::new("VALUE")
        }],
    )
    .unwrap();
    assert!(with_bom.starts_with("\u{feff}# /// script\n"));
}

#[test]
fn origin_main_scope_oracle_keeps_unrelated_builtin_input_calls() {
    let source = concat!(
        "top = input('Top: ')\n",
        "def shadowed(input):\n    return input('Own: ')\n",
        "def unshadowed():\n    return input('Nested: ')\n",
    );
    let analysis = parsed(source).analysis();
    let prompts = analysis
        .candidates
        .iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .map(|candidate| candidate.declaration.prompt.as_str())
        .collect::<Vec<_>>();
    assert_eq!(prompts, ["Top: ", "Nested: "]);
}

#[test]
fn origin_main_nargs_and_click_repeat_oracle_preserves_argv_grammar() {
    let argparse = parsed("p.add_argument('--point', nargs=2, type=float)\n");
    let CliSurface::Static(surface) = argparse.cli_surface() else {
        panic!("argparse must be static");
    };
    assert!(field(&surface.fields, "point").multiple);

    let click = parsed(concat!(
        "import click\n",
        "@click.command()\n",
        "@click.option('--pair', nargs=2)\n",
        "@click.option('--tag', multiple=True)\n",
        "@click.option('--unsupported', nargs=2, multiple=True)\n",
        "def main(pair, tag, unsupported): pass\n",
    ));
    let CliSurface::Static(surface) = click.cli_surface() else {
        panic!("Click must be static");
    };
    let pair = field(&surface.fields, "pair");
    assert!(pair.multiple && !pair.repeat);
    let tag = field(&surface.fields, "tag");
    assert!(tag.multiple && tag.repeat);
    assert!(
        surface
            .fields
            .iter()
            .all(|field| field.declaration.name != "unsupported")
    );
}
