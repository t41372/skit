use skit_domain::parameters::{ParameterBinding, ParameterType, ParameterValue};
use skit_language::{CliSurface, DegradationReason, ParseOutcome, ParsedDocument, parse_document};

fn parsed(kind: &str, source: &str) -> ParsedDocument {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document,
        outcome => panic!("{kind} must parse, got {outcome:?}"),
    }
}

#[test]
fn shell_ast_restores_constants_defaults_reads_demotions_and_hints() {
    let source = concat!(
        "A=plain\nB=42\nC='raw text'\nD=\"double q\"\n",
        "EMPTY=\nARR=(1 2)\nDYNAMIC=$OTHER\nreadonly LOCKED=1\n",
        "COUNT=0\nCOUNT+=1\n",
        ": \"${PORT:-8080}\"\nMODE=prod\necho \"${MODE:-dev}\"\n",
        "read -p \"Name: \" NAME\n",
        "builtin read -s PASSWORD\n",
        "read -n 3 CODE\nIFS=: read LEFT RIGHT\n",
        "printf x | read PIPE\n",
        "echo \"$@ $1 $BASH_SOURCE\"\n",
    );
    let analysis = parsed("shell", source).analysis();
    let names = analysis
        .candidates
        .iter()
        .map(|candidate| candidate.declaration.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "A", "B", "C", "D", "COUNT", "MODE", "PORT", "input-1", "input-2"
        ]
    );
    let count = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "COUNT")
        .unwrap();
    assert_eq!(count.demotion, Some(DegradationReason::Accumulator));
    let port = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "PORT")
        .unwrap();
    assert_eq!(port.declaration.binding, ParameterBinding::EnvDefault);
    assert_eq!(
        port.declaration.default,
        Some(ParameterValue::Integer(8080))
    );
    assert!(port.empty_uses_default);
    assert!(analysis.uses_argv);
    assert!(analysis.uses_self_location);
}

#[test]
fn shell_getopts_keeps_absent_static_zero_and_dynamic_distinct() {
    assert!(matches!(
        parsed("shell", "echo ok\n").cli_surface(),
        CliSurface::Absent
    ));
    let CliSurface::Static(surface) = parsed("shell", "getopts ':a:b!' opt\n").cli_surface() else {
        panic!("literal getopts must be static");
    };
    assert_eq!(
        surface
            .fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        surface.fields[0].declaration.parameter_type,
        ParameterType::Str
    );
    assert_eq!(
        surface.fields[1].declaration.parameter_type,
        ParameterType::Bool
    );
    assert!(matches!(
        parsed("shell", "getopts \"$OPTIONS\" opt\n").cli_surface(),
        CliSurface::Dynamic(_)
    ));
}

#[test]
fn javascript_ast_restores_demotions_signals_and_parseargs_honesty() {
    let document = parsed(
        "js",
        concat!(
            "const HOST = 'localhost';\nlet count = 1;\nvar total = 2;\n",
            "const HEX = 0xFF;\nconst EXPONENT = 1e3;\nconst BIG = 100n;\n",
            "count += 1;\n",
            "const OPTIONS = 'shadowed';\n",
            "function f(OPTIONS) {}\n",
            "const SECRET_TOKEN = 'hidden';\n",
            "const { values } = parseArgs({ options: {\n",
            "  host: { type: 'string', default: HOST },\n",
            "  fallback: { type: 'string', default: OPTIONS },\n",
            "  token: { type: 'string', default: SECRET_TOKEN },\n",
            "  tag: { type: 'string', multiple: true },\n",
            "  verbose: { type: 'boolean' },\n",
            "  broken: { type: dynamicType },\n",
            "}});\nconsole.log(process.argv);\n",
        ),
    );
    let analysis = document.analysis();
    assert!(analysis.uses_argv);
    assert_eq!(analysis.frameworks, ["parseArgs"]);
    for name in ["HEX", "EXPONENT", "BIG"] {
        let declaration = &analysis
            .candidates
            .iter()
            .find(|candidate| candidate.declaration.name == name)
            .unwrap()
            .declaration;
        assert_eq!(declaration.parameter_type, ParameterType::Float);
        assert!(matches!(
            declaration.default,
            Some(ParameterValue::String(_))
        ));
    }
    assert_eq!(
        analysis
            .candidates
            .iter()
            .find(|candidate| candidate.declaration.name == "count")
            .unwrap()
            .demotion,
        Some(DegradationReason::Accumulator)
    );
    let CliSurface::Static(surface) = document.cli_surface() else {
        panic!("inline options must be static");
    };
    let host = surface
        .fields
        .iter()
        .find(|field| field.declaration.name == "host")
        .unwrap();
    assert_eq!(
        host.declaration.default,
        Some(ParameterValue::String("localhost".to_owned()))
    );
    for name in ["fallback", "token"] {
        assert!(
            surface
                .fields
                .iter()
                .find(|field| field.declaration.name == name)
                .unwrap()
                .declaration
                .degraded
        );
    }
    let tag = surface
        .fields
        .iter()
        .find(|field| field.declaration.name == "tag")
        .unwrap();
    assert!(tag.declaration.multiple && tag.declaration.repeat);
    assert!(matches!(
        parsed("js", "parseArgs({ options: shared })\n").cli_surface(),
        CliSurface::Dynamic(_)
    ));
    assert!(matches!(
        parsed("js", "parseArgs({ options: { ...shared, x: {} } })\n").cli_surface(),
        CliSurface::Dynamic(_)
    ));
}

#[test]
fn fish_ast_restores_env_idiom_hints_and_full_argparse_spec_grammar() {
    let document = parsed(
        "fish",
        concat!(
            "set -q PORT; or set PORT 8080\n",
            "set -q MODE; or set MODE safe\nset MODE forced\n",
            "echo $argv\nstatus dirname\n",
            "argparse -n tool 'h/help' 'o/output=?' 't/tag=+' 'm#max' 'dry-run' -- $argv\n",
        ),
    );
    let analysis = document.analysis();
    assert_eq!(analysis.candidates.len(), 1);
    assert_eq!(analysis.candidates[0].declaration.name, "PORT");
    assert!(analysis.uses_argv);
    assert!(analysis.uses_self_location);
    let CliSurface::Static(surface) = document.cli_surface() else {
        panic!("literal argparse must be static");
    };
    assert_eq!(surface.fields.len(), 5);
    let tag = &surface.fields[2].declaration;
    assert!(tag.multiple && tag.repeat);
    assert_eq!(
        surface.fields[3].degradation,
        Some(DegradationReason::UnsupportedAction)
    );
    assert!(matches!(
        parsed("fish", "argparse $specs -- $argv\n").cli_surface(),
        CliSurface::Dynamic(_)
    ));
}

#[test]
fn powershell_parser_restores_zero_fields_choices_defaults_and_degradation() {
    let CliSurface::Static(empty) = parsed("powershell", "param()\n").cli_surface() else {
        panic!("empty param block must be static");
    };
    assert!(empty.fields.is_empty());

    let source = concat!(
        "param(\n",
        " [Parameter(Mandatory=$false)][string]$Optional,\n",
        " [Parameter(Mandatory)][ValidateSet('fast','safe')][string]$Mode = 'fast',\n",
        " [object]$Dynamic = (Get-Date),\n",
        " [switch]$Force\n",
        ")\n",
    );
    let CliSurface::Static(surface) = parsed("powershell", source).cli_surface() else {
        panic!("param block must be static");
    };
    assert!(!surface.fields[0].declaration.required);
    assert!(surface.fields[1].declaration.required);
    assert_eq!(surface.fields[1].declaration.choices, ["fast", "safe"]);
    assert!(surface.fields[2].declaration.degraded);
    assert_eq!(
        surface.fields[3].declaration.parameter_type,
        ParameterType::Bool
    );
    assert!(matches!(
        parse_document("powershell", "param([string]$Name = )\n"),
        ParseOutcome::SyntaxError(_)
    ));
}
