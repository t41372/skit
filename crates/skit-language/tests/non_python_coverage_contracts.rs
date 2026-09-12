use skit_domain::parameters::{ParameterType, ParameterValue};
use skit_language::{CliSurface, ParseOutcome, ParsedDocument, parse_document};

fn parsed(kind: &str, source: &str) -> ParsedDocument {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document,
        outcome => panic!("{kind} source must parse: {outcome:?}"),
    }
}

#[test]
fn fish_analysis_rejects_incomplete_guards_and_decodes_static_escaped_words() {
    assert!(
        parsed("fish", "set -- -q NAME\nset -q; or set VALUE fallback\n")
            .analysis()
            .candidates
            .is_empty()
    );
    let analysis = parsed(
        "fish",
        concat!(
            "set -q CITY; or set CITY Taipei\n",
            "argparse 'x/long\\\\name' -- $argv\n",
        ),
    )
    .analysis();

    assert_eq!(analysis.candidates.len(), 1);
    assert_eq!(analysis.candidates[0].declaration.name, "CITY");
    assert_eq!(
        analysis.candidates[0].declaration.default,
        Some(ParameterValue::String("Taipei".to_owned()))
    );
    let CliSurface::Static(surface) =
        parsed("fish", "argparse 'x/long-name' 'q/quoted' -- $argv\n").cli_surface()
    else {
        panic!("literal fish specs must stay static");
    };
    assert_eq!(surface.fields.len(), 2);

    for source in [
        "argparse x/'long-name' -- $argv\n",
        "argparse y/\"wide-name\" -- $argv\n",
    ] {
        assert!(matches!(
            parsed("fish", source).cli_surface(),
            CliSurface::Static(_)
        ));
    }
}

#[test]
fn javascript_analysis_keeps_huge_numbers_and_public_empty_surfaces_total() {
    let document = parsed(
        "js",
        concat!(
            "const HUGE = 999999999999999999999999999999;\n",
            "const { values } = parseArgs({ options: {\n",
            "  ['computed']: { type: 'string' },\n",
            "  shorthand,\n",
            "  plain: 42,\n",
            "} });\n",
        ),
    );
    let analysis = document.analysis();
    let huge = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "HUGE")
        .expect("huge literal candidate");
    assert_eq!(huge.declaration.parameter_type, ParameterType::Float);
    assert_eq!(
        huge.declaration.default,
        Some(ParameterValue::String(
            "999999999999999999999999999999".to_owned()
        ))
    );
    let CliSurface::Static(surface) = document.cli_surface() else {
        panic!("object options must stay static");
    };
    assert_eq!(surface.fields.len(), 1);
    assert_eq!(surface.fields[0].declaration.name, "plain");
    assert!(surface.fields[0].declaration.degraded);

    assert!(matches!(
        parsed("js", "parseArgs()\n").cli_surface(),
        CliSurface::Absent
    ));
}

#[test]
fn powershell_analysis_covers_untyped_and_scalar_default_forms() {
    let source = concat!(
        "<#\n.PARAMETER Name\nDisplay name\n#>\n",
        "param(\n",
        "  $Untyped,\n",
        "  [string]$Name = \"a`\"b``c\",\n",
        "  [int]$Negative = -5,\n",
        "  [int]$Positive = +6,\n",
        "  [int]$Hex = 0x10,\n",
        "  [string]$ReadableHash = @{ a = 1; b = @(2, $true) },\n",
        "  [object]$DynamicHash = @{ a = $outside },\n",
        "  [string]$Interpolated = \"$outside\",\n",
        "  [Alias('nick')][string]$Aliased = 'ok'\n",
        ")\n",
    );
    let document = parsed("powershell", source);
    let analysis = document.analysis();
    assert_eq!(analysis.frameworks, ["param"]);
    let CliSurface::Static(surface) = document.cli_surface() else {
        panic!("param block must stay static");
    };
    let field = |name: &str| {
        &surface
            .fields
            .iter()
            .find(|field| field.declaration.name == name)
            .expect("parameter field")
            .declaration
    };
    assert!(field("Untyped").degraded);
    assert_eq!(
        field("Name").default,
        Some(ParameterValue::String("a\"b`c".to_owned()))
    );
    assert_eq!(field("Negative").default, Some(ParameterValue::Integer(-5)));
    assert_eq!(field("Positive").default, Some(ParameterValue::Integer(6)));
    assert_eq!(field("Hex").default, Some(ParameterValue::Integer(16)));
    assert!(!field("ReadableHash").degraded);
    assert!(field("DynamicHash").degraded);
    assert_eq!(field("Interpolated").default, None);
    assert!(!field("Interpolated").degraded);
}

#[test]
fn powershell_recovery_accepts_only_scalar_bare_defaults() {
    let recovered = parsed("powershell", "param([string]$Name = bare)\n");
    let CliSurface::Static(surface) = recovered.cli_surface() else {
        panic!("recoverable bare defaults must keep a static surface");
    };
    assert_eq!(
        surface.fields[0].declaration.default,
        Some(ParameterValue::String("bare".to_owned()))
    );
    assert!(matches!(
        parse_document("powershell", "param([string]$Name = foo::bar)\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

#[test]
fn shell_analysis_ignores_incomplete_bindings_without_losing_valid_neighbors() {
    let analysis = parsed(
        "shell",
        concat!(
            "EMPTY=\n",
            "VALUE=ok\n",
            ": \"${VALID:-kept}\"\n",
            "VALUE+=tail\n",
        ),
    )
    .analysis();
    let names = analysis
        .candidates
        .iter()
        .map(|candidate| candidate.declaration.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["VALUE", "VALID"]);
}
