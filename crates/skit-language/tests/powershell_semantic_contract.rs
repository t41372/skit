use skit_domain::parameters::{ParameterType, ParameterValue};
use skit_language::{CliSurface, ParseOutcome, parse_document};

#[test]
fn powershell_param_blocks_are_read_from_the_language_ast() {
    let source = concat!(
        "param(\n",
        "  [Parameter(Mandatory=$true)][ValidateSet('fast','safe')][string]$Mode = 'fast',\n",
        "  [double]$Ratio = 1.5,\n",
        "  [switch]$Force\n",
        ")\n",
    );
    let ParseOutcome::Parsed(document) = parse_document("powershell", source) else {
        panic!("valid PowerShell must parse");
    };
    let CliSurface::Static(surface) = document.cli_surface() else {
        panic!("param block must be a static CLI surface");
    };
    assert_eq!(surface.framework, "param");
    assert_eq!(surface.fields.len(), 3);
    assert!(surface.fields[0].declaration.required);
    assert_eq!(
        surface.fields[0].declaration.parameter_type,
        ParameterType::Choice
    );
    assert_eq!(surface.fields[0].declaration.choices, ["fast", "safe"]);
    assert_eq!(
        surface.fields[1].declaration.default,
        Some(ParameterValue::Float(1.5))
    );
    assert_eq!(
        surface.fields[2].declaration.parameter_type,
        ParameterType::Bool
    );
}
