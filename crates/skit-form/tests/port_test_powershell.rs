//! Public-surface ports from Python `tests/test_powershell.py` at `main@206f9ef`.
//!
//! Python v0.4 reflected PowerShell through a `pwsh` subprocess and a JSON envelope. Rust owns a
//! tree-sitter PowerShell document instead. These tests keep every semantic/form contract that still
//! has a public equivalent; subprocess/envelope/discovery-only contracts are accounted separately by
//! the completeness manifest and are not recreated as fake test helpers.

use std::collections::BTreeMap;

use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::{CliFormProjection, FormSource, cli_form_projection, form_plan, onboarding_plan};

fn fields(source: &str) -> Vec<ParamDecl> {
    match onboarding_plan("powershell", source).cli_surface {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, "param");
            fields
        }
        other => panic!("expected static PowerShell param surface: {other:?}"),
    }
}

fn by_name(fields: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

#[test]
fn test_string_param_with_default_and_help() {
    let actual = fields(
        "<#\n.PARAMETER Name\nwho\n#>\nparam([string]$Name = 'world')\nWrite-Host $Name\n",
    );
    let field = &by_name(&actual)["Name"];
    assert_eq!(field.parameter_type, ParameterType::Str);
    assert_eq!(
        field.default,
        Some(ParameterValue::String("world".to_owned()))
    );
    assert_eq!(field.flag, "-Name");
    assert_eq!(field.help, "who");
    assert_eq!(field.binding, ParameterBinding::None);
    assert_eq!(field.delivery, ParameterDelivery::Flag);
    assert!(!field.degraded);
}

#[test]
fn test_help_is_stripped_of_surrounding_whitespace() {
    let actual = fields(
        "<#\n.PARAMETER Name\nThe city to deploy to.\n\n#>\nparam([string]$Name)\nWrite-Host $Name\n",
    );
    assert_eq!(by_name(&actual)["Name"].help, "The city to deploy to.");
}

#[test]
fn test_int_and_long_map_to_int() {
    let actual = fields("param([int]$A = 5, [long]$B = 9)\nWrite-Host $A $B\n");
    let named = by_name(&actual);
    assert_eq!(named["A"].parameter_type, ParameterType::Int);
    assert_eq!(named["A"].default, Some(ParameterValue::Integer(5)));
    assert_eq!(named["B"].parameter_type, ParameterType::Int);
    assert_eq!(named["B"].default, Some(ParameterValue::Integer(9)));
}

#[test]
fn test_double_and_single_map_to_float() {
    let actual = fields("param([double]$R = 2.5, [single]$S = 1.5)\nWrite-Host $R $S\n");
    let named = by_name(&actual);
    assert_eq!(named["R"].parameter_type, ParameterType::Float);
    assert_eq!(named["R"].default, Some(ParameterValue::Float(2.5)));
    assert_eq!(named["S"].parameter_type, ParameterType::Float);
    assert_eq!(named["S"].default, Some(ParameterValue::Float(1.5)));
}

#[test]
fn test_switch_is_a_store_true_flag() {
    let actual = fields("param([switch]$Verbose)\nWrite-Host $Verbose\n");
    let field = &by_name(&actual)["Verbose"];
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.action, "store_true");
    assert_eq!(field.default, Some(ParameterValue::Bool(false)));
    assert_eq!(field.flag, "-Verbose");
}

#[test]
fn test_validate_set_becomes_choice() {
    let actual = fields(
        "param([ValidateSet('dev','stage','prod')][string]$Mode = 'dev')\nWrite-Host $Mode\n",
    );
    let field = &by_name(&actual)["Mode"];
    assert_eq!(field.parameter_type, ParameterType::Choice);
    assert_eq!(field.choices, ["dev", "stage", "prod"]);
    assert_eq!(
        field.default,
        Some(ParameterValue::String("dev".to_owned()))
    );
}

#[test]
fn test_unknown_static_type_degrades() {
    let actual = fields("param([System.Collections.Hashtable]$Obj)\nWrite-Host $Obj\n");
    let field = &by_name(&actual)["Obj"];
    assert!(field.degraded);
    assert_eq!(field.parameter_type, ParameterType::Str);
}

#[test]
fn test_mandatory_is_required() {
    let actual = fields("param([Parameter(Mandatory)][string]$Target)\nWrite-Host $Target\n");
    assert!(by_name(&actual)["Target"].required);
}

#[test]
fn test_non_constant_default_degrades_field() {
    let actual = fields("param([string]$When = (Get-Date))\nWrite-Host $When\n");
    let field = &by_name(&actual)["When"];
    assert!(field.degraded);
    assert_eq!(field.default, None);
}

#[test]
fn test_non_scalar_default_is_left_unset() {
    let actual = fields("param([string]$Items = @(1, 2))\nWrite-Host $Items\n");
    let field = &by_name(&actual)["Items"];
    assert_eq!(field.default, None);
    assert!(
        !field.degraded,
        "a readable non-scalar default must be omitted without degrading the known scalar type"
    );
}

#[test]
fn test_bool_default_is_carried() {
    let actual = fields("param([System.Boolean]$On = $true)\nWrite-Host $On\n");
    let field = &by_name(&actual)["On"];
    assert_eq!(field.default, Some(ParameterValue::Bool(true)));
    assert!(
        field.degraded,
        "Python v0.4 treats System.Boolean as an unmapped static type even though its scalar default survives"
    );
}

#[test]
fn test_secret_name_flagged() {
    let actual = fields("param([string]$ApiToken)\nWrite-Host $ApiToken\n");
    assert!(by_name(&actual)["ApiToken"].secret);
}

#[test]
fn test_declaration_order_is_preserved() {
    let actual = fields("param([string]$First, [string]$Second)\nWrite-Host $First $Second\n");
    assert_eq!(
        actual
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second"]
    );
}

#[test]
fn test_empty_param_block_is_a_zero_field_surface() {
    let actual = fields("param()\nWrite-Host hi\n");
    assert!(actual.is_empty());
}

#[test]
fn test_no_param_block_returns_none() {
    assert!(matches!(
        cli_form_projection("powershell", "Write-Host hi\n"),
        CliFormProjection::Absent
    ));
}

#[test]
fn test_parse_error_returns_none() {
    assert!(matches!(
        cli_form_projection("powershell", "param(\n"),
        CliFormProjection::Absent
    ));
}

#[test]
fn test_plan_reads_powershell_param_block() {
    let plan = form_plan(
        "powershell",
        "param([string]$City = 'Taipei')\nWrite-Host $City\n",
        &EntrySettings::default(),
    );
    assert_eq!(plan.source, FormSource::Reader);
    assert_eq!(plan.source.as_str(), "argparse");
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["City"]
    );
    assert_eq!(plan.fields[0].declaration.flag, "-City");
}

#[test]
fn test_plan_none_when_reader_finds_no_surface() {
    let plan = form_plan(
        "powershell",
        "Write-Host hi\n",
        &EntrySettings::default(),
    );
    assert_eq!(plan.source, FormSource::None);
    assert_eq!(plan.source.as_str(), "none");
    assert!(plan.fields.is_empty());
}

#[test]
fn test_integration_reads_a_real_param_block() {
    let actual = fields(
        "<#\n.PARAMETER City\nThe city to deploy to.\n#>\n\
         param(\n\
           [Parameter(Mandatory)][string]$City,\n\
           [Parameter(Mandatory=$true)][string]$Region,\n\
           [ValidateSet('dev','prod')][string]$Env = 'dev',\n\
           [int]$Retries = 3,\n\
           [switch]$DryRun\n\
         )\n\
         Write-Host $City\n",
    );
    let named = by_name(&actual);
    assert!(named["City"].required);
    assert!(named["Region"].required);
    assert_eq!(named["City"].help, "The city to deploy to.");
    assert_eq!(named["Env"].parameter_type, ParameterType::Choice);
    assert_eq!(named["Env"].choices, ["dev", "prod"]);
    assert_eq!(
        named["Env"].default,
        Some(ParameterValue::String("dev".to_owned()))
    );
    assert_eq!(named["Retries"].parameter_type, ParameterType::Int);
    assert_eq!(named["Retries"].default, Some(ParameterValue::Integer(3)));
    assert_eq!(named["DryRun"].parameter_type, ParameterType::Bool);
    assert_eq!(named["DryRun"].action, "store_true");
}
