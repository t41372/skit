//! Mechanical port of the Python oracle module `tests/test_powershell.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name so it traces back to
//! its origin, and the Python "WHY" comment is preserved. Same behavioral claim, same expected
//! output.
//!
//! ## Architecture divergence (read this first)
//! The Python PowerShell reader (`skit.langs.powershell.cli_reader`) spawns a real `pwsh`
//! subprocess that extracts the `param()` block as JSON, then maps that JSON onto `ParamDecl`
//! fields. Every Python "type matrix" / "envelope" test monkeypatches `subprocess.run` to inject a
//! canned JSON row (`_row(...)`); the script text passed to `read_cli` is irrelevant (the real
//! extractor never runs).
//!
//! The Rust reader (`crates/skit-language/src/semantic/powershell.rs`) is tree-sitter-backed: it
//! parses the `param()` block straight from the source AST — NO subprocess, NO JSON envelope, NO
//! `pwsh` on PATH. It is the same behavioral contract (str/int/float/bool/choice/switch/mandatory/
//! ValidateSet/secret/degradation) re-implemented over the parse tree.
//!
//! ## Concept mapping
//! - Python `cli_reader.read_cli(src)` (subprocess+JSON) -> `parse_document("powershell", src)
//!   .cli_surface()`. A JSON `_row(...)` is re-expressed as the PowerShell SOURCE that models it:
//!   `staticType` -> the `[type]` cast (`System.Int32` -> `[int]`, `System.Int64` -> `[long]`,
//!   `System.Double` -> `[double]`, `System.Single` -> `[single]`, `System.String` -> `[string]`,
//!   `SwitchParameter` -> `[switch]`, an unmapped type -> `[hashtable]`); `mandatory` ->
//!   `[Parameter(Mandatory=$true)]`; `validateSet` -> `[ValidateSet(...)]`; `defaultConst` -> the
//!   `= <literal>` default; a non-constant default (`defaultReadable=False`) -> `= (Get-Date)`; a
//!   non-scalar default -> `= @(1, 2)`; `helpText` -> a `<# .PARAMETER #>` comment-help block.
//! - Python `spec is not None` with fields -> `CliSurface::Static`; `spec.fields == []` -> a static
//!   surface with an empty field list; `read_cli(..) is None` (no param block) -> `CliSurface::
//!   Absent`; an unparseable param block -> `ParseOutcome::SyntaxError`.
//! - Each Python `spec.fields[i]` (`f.type/default/flag/help/binding/delivery/required/choices/
//!   action/secret/degraded`) -> a `SemanticField.declaration` field. Python `f.degraded` ->
//!   `declaration.degraded` (the bool the reader sets alongside `SemanticField.degradation`).
//!
//! ## Buckets
//! - **Bucket 1 (pure surface byte-logic):** the JSON->ParamDecl type matrix, the empty/absent/
//!   unparseable surface envelope, declaration order, secret detection, and the (skip-gated) real
//!   `param()` block, all re-expressed as PowerShell source and asserted on `.cli_surface()`. A
//!   bucket-1 test that fails on the tree-sitter reader's actual behavior stays FAILING — that is
//!   the highest-value signal (a candidate PowerShell reader gap).
//! - **Bucket 2 (execution claim the injected BYTES establish):** NONE. PowerShell has no injector
//!   (`plan_injection` falls to the `UnsupportedKind` arm, just like fish), and `test_powershell.py`
//!   ships no injection test.
//! - **Bucket 3 (`#[ignore]`):** the subprocess plumbing (timeout/exit-code/unparseable-JSON/
//!   OSError), the JSON-envelope robustness (non-dict payload/missing status/params-not-a-list/
//!   non-dict-row/nameless-row), the `_find_powershell` executable-discovery matrix, and the
//!   `flows`/`store` plan+assemble tests. None of these has a tree-sitter analogue: the Rust reader
//!   spawns no process and consumes no JSON, and `flows`/`store` live above `skit-language`.

use std::collections::BTreeMap;

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{CliSurface, ParseOutcome, ParsedDocument, parse_document};

// ---------------------------------------------------------------- helpers

/// Python's implicit `parse` of the script the reader was handed. PowerShell is parser-backed, so a
/// well-formed `param()` block must yield a `Parsed` document.
fn parsed(source: &str) -> ParsedDocument {
    match parse_document("powershell", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid PowerShell, got {other:?}"),
    }
}

/// The static `param()` surface's field declarations, or a panic if the surface is not static
/// (Python `read(src)` first asserts `spec is not None`, i.e. a readable static surface).
fn surface_fields(source: &str) -> Vec<ParamDecl> {
    match parsed(source).cli_surface() {
        CliSurface::Static(surface) => surface
            .fields
            .into_iter()
            .map(|field| field.declaration)
            .collect(),
        other => panic!("expected a static param() surface, got {other:?}"),
    }
}

/// Python `_read(...)` returns `{f.name: f for f in spec.fields}`.
fn fields_by_name(source: &str) -> BTreeMap<String, ParamDecl> {
    surface_fields(source)
        .into_iter()
        .map(|declaration| (declaration.name.clone(), declaration))
        .collect()
}

/// Python `[f.name for f in spec.fields]`: field names in declaration order.
fn field_names(source: &str) -> Vec<String> {
    surface_fields(source)
        .into_iter()
        .map(|declaration| declaration.name)
        .collect()
}

// ---------------------------------------------------------------- the type matrix

#[test]
fn test_string_param_with_default_and_help() {
    // JSON `_row("Name", hasDefault, defaultReadable, defaultConst="world", helpText="who")` models
    // a `[string]$Name = 'world'` with a `.PARAMETER Name` help block.
    let fields = fields_by_name(concat!(
        "<#\n",
        ".PARAMETER Name\n",
        "who\n",
        "#>\n",
        "param([string]$Name = 'world')\n",
    ));
    let f = &fields["Name"];
    // Python: assert (f.type, f.default, f.flag, f.help) == ("str", "world", "-Name", "who")
    assert_eq!(
        (
            f.parameter_type,
            &f.default,
            f.flag.as_str(),
            f.help.as_str()
        ),
        (
            ParameterType::Str,
            &Some(ParameterValue::String("world".to_owned())),
            "-Name",
            "who"
        )
    );
    // Python: assert (f.binding, f.delivery) == ("none", "flag")
    assert_eq!(
        (f.binding, f.delivery),
        (ParameterBinding::None, ParameterDelivery::Flag)
    );
    // Python: assert not f.degraded
    assert!(!f.degraded);
}

#[test]
fn test_help_is_stripped_of_surrounding_whitespace() {
    // pwsh's GetHelpContent trails a `.PARAMETER` block with newlines on some versions and not
    // on others; the reader normalizes so the field text is identical whatever version ran.
    // JSON `helpText="The city to deploy to.\n\n"` models a help block with trailing blank lines.
    let fields = fields_by_name(concat!(
        "<#\n",
        ".PARAMETER Name\n",
        "The city to deploy to.\n",
        "\n",
        "#>\n",
        "param([string]$Name)\n",
    ));
    assert_eq!(fields["Name"].help, "The city to deploy to.");
}

#[test]
fn test_int_and_long_map_to_int() {
    // System.Int32 -> [int], System.Int64 -> [long]; both map to int.
    let fields = fields_by_name("param([int]$A = 5, [long]$B = 9)\n");
    assert_eq!(
        (fields["A"].parameter_type, &fields["A"].default),
        (ParameterType::Int, &Some(ParameterValue::Integer(5)))
    );
    assert_eq!(
        (fields["B"].parameter_type, &fields["B"].default),
        (ParameterType::Int, &Some(ParameterValue::Integer(9)))
    );
}

#[test]
fn test_double_and_single_map_to_float() {
    // System.Double -> [double], System.Single -> [single]; both map to float.
    let fields = fields_by_name("param([double]$R = 2.5, [single]$S = 1.5)\n");
    assert_eq!(
        (fields["R"].parameter_type, &fields["R"].default),
        (ParameterType::Float, &Some(ParameterValue::Float(2.5)))
    );
    assert_eq!(
        (fields["S"].parameter_type, &fields["S"].default),
        (ParameterType::Float, &Some(ParameterValue::Float(1.5)))
    );
}

#[test]
fn test_switch_is_a_store_true_flag() {
    // SwitchParameter -> [switch].
    let fields = fields_by_name("param([switch]$Verbose)\n");
    let f = &fields["Verbose"];
    assert_eq!(
        (
            f.parameter_type,
            f.action.as_str(),
            &f.default,
            f.flag.as_str()
        ),
        (
            ParameterType::Bool,
            "store_true",
            &Some(ParameterValue::Bool(false)),
            "-Verbose"
        )
    );
}

#[test]
fn test_validate_set_becomes_choice() {
    let fields =
        fields_by_name("param([ValidateSet('dev','stage','prod')][string]$Mode = 'dev')\n");
    let f = &fields["Mode"];
    // Python: assert (f.type, f.choices, f.default) == ("choice", ("dev","stage","prod"), "dev")
    assert_eq!(f.parameter_type, ParameterType::Choice);
    assert_eq!(f.choices, ["dev", "stage", "prod"]);
    assert_eq!(f.default, Some(ParameterValue::String("dev".to_owned())));
}

#[test]
fn test_unknown_static_type_degrades() {
    // System.Collections.Hashtable is not in the scalar type map.
    let fields = fields_by_name("param([hashtable]$Obj)\n");
    assert!(fields["Obj"].degraded);
    assert_eq!(fields["Obj"].parameter_type, ParameterType::Str);
}

#[test]
fn test_mandatory_is_required() {
    let fields = fields_by_name("param([Parameter(Mandatory=$true)][string]$Target)\n");
    assert!(fields["Target"].required);
}

#[test]
fn test_non_constant_default_degrades_field() {
    // `[string]$When = (Get-Date)` — SafeGetValue throws PS-side, so defaultReadable is false.
    let fields = fields_by_name("param([string]$When = (Get-Date))\n");
    let f = &fields["When"];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_non_scalar_default_is_left_unset() {
    // `$Items = @(1, 2)` — a readable but non-scalar default; the type is known (str), so the
    // field is not degraded, but the array default is not carried onto the scalar model.
    let fields = fields_by_name("param([string]$Items = @(1, 2))\n");
    assert_eq!(fields["Items"].default, None);
    assert!(!fields["Items"].degraded);
}

#[test]
fn test_bool_default_is_carried() {
    // The v0.4 type map does not include System.Boolean. It degrades the field to free text, but
    // `_apply_default` still carries the independently read Boolean scalar through the domain.
    let fields = fields_by_name("param([System.Boolean]$On = $true)\n");
    assert_eq!(fields["On"].default, Some(ParameterValue::Bool(true)));
    assert!(fields["On"].degraded);
}

#[test]
fn rust_additive_unknown_static_types_keep_readable_scalar_default_types() {
    let fields = fields_by_name(concat!(
        "param(\n",
        "  [object]$Count = 5,\n",
        "  [object]$Ratio = 2.5,\n",
        "  [object]$Off = $false,\n",
        "  [object]$Label = 'five',\n",
        "  [object]$Nothing = $null,\n",
        "  [object]$Items = @(1, 2),\n",
        "  [object]$Variable = $outside,\n",
        "  [object]$When = (Get-Date)\n",
        ")\n",
    ));
    assert_eq!(fields["Count"].default, Some(ParameterValue::Integer(5)));
    assert_eq!(fields["Ratio"].default, Some(ParameterValue::Float(2.5)));
    assert_eq!(fields["Off"].default, Some(ParameterValue::Bool(false)));
    assert_eq!(
        fields["Label"].default,
        Some(ParameterValue::String("five".to_owned()))
    );
    assert_eq!(fields["Nothing"].default, None);
    assert_eq!(fields["Items"].default, None);
    assert_eq!(fields["Variable"].default, None);
    assert_eq!(fields["When"].default, None);
    assert!(fields.values().all(|field| field.degraded));
}

#[test]
fn rust_additive_readable_scalar_defaults_use_runtime_value_types() {
    let fields = fields_by_name("param([int]$Quoted = '5', [string]$Number = 5)\n");
    assert_eq!(fields["Quoted"].parameter_type, ParameterType::Int);
    assert_eq!(
        fields["Quoted"].default,
        Some(ParameterValue::String("5".to_owned()))
    );
    assert_eq!(fields["Number"].parameter_type, ParameterType::Str);
    assert_eq!(fields["Number"].default, Some(ParameterValue::Integer(5)));
    assert!(!fields["Quoted"].degraded);
    assert!(!fields["Number"].degraded);
}

#[test]
fn test_secret_name_flagged() {
    let fields = fields_by_name("param([string]$ApiToken)\n");
    assert!(fields["ApiToken"].secret);
}

#[test]
fn test_declaration_order_is_preserved() {
    assert_eq!(
        field_names("param([string]$First, [string]$Second)\n"),
        ["First", "Second"]
    );
}

// ---------------------------------------------------------------- envelope / degrade paths

#[test]
fn test_empty_param_block_is_a_zero_field_surface() {
    // Python: spec is not None, spec.fields == [], spec.ok -> a readable zero-field static surface.
    let CliSurface::Static(surface) = parsed("param()\n").cli_surface() else {
        panic!("empty param() must be a static zero-field surface");
    };
    assert!(surface.fields.is_empty());
}

#[test]
fn test_no_param_block_returns_none() {
    // Python `read_cli("Write-Host hi\n") is None` -> no detected CLI surface at all.
    assert!(matches!(
        parsed("Write-Host hi\n").cli_surface(),
        CliSurface::Absent
    ));
}

#[test]
fn test_parse_error_returns_none() {
    // Python stubs `status=parse-error` for the unparseable `param(\n`. The faithful tree-sitter
    // mapping: an unclosed param block is invalid PowerShell, so it yields no readable surface.
    assert!(matches!(
        parse_document("powershell", "param(\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

#[test]
#[ignore = "UNMAPPED: JSON-envelope robustness — `read_cli` on a JSON `null` payload returns None. The tree-sitter reader consumes no subprocess JSON envelope; there is no `null payload` analogue to a source parse -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_non_dict_payload_returns_none() {}

#[test]
#[ignore = "UNMAPPED: JSON-envelope robustness — a payload with no `status` key returns None. No JSON envelope in the tree-sitter reader -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_missing_status_returns_none() {}

#[test]
#[ignore = "UNMAPPED: JSON-envelope robustness — `params` not a list yields zero fields. No JSON envelope in the tree-sitter reader; the zero-field case is covered publicly by test_empty_param_block_is_a_zero_field_surface -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_params_not_a_list_yields_zero_fields() {}

#[test]
#[ignore = "UNMAPPED: JSON-envelope robustness — a non-dict `params` row (an int) is skipped. No JSON row layer in the tree-sitter reader (a source param is always a `script_parameter` node) -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_non_dict_row_is_skipped() {}

#[test]
#[ignore = "UNMAPPED: JSON-envelope robustness — a row with an empty `name` is dropped. There is no source construct for a nameless param; the reader's own empty-name guard is exercised only through the JSON layer -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_nameless_row_is_dropped() {}

// ---------------------------------------------------------------- subprocess plumbing

#[test]
#[ignore = "UNMAPPED: `read_cli` returns None when no pwsh is on PATH and never spawns a subprocess. The tree-sitter reader never spawns pwsh at all (it parses source directly), so there is no executable-presence gate to observe -> Tier 3 white-box (Python subprocess plumbing). MUST-VERIFY (resolved in the SAFE direction): PowerShell reading in the rewrite is fully inert — no process is ever spawned."]
fn test_no_powershell_at_all_returns_none() {}

#[test]
#[ignore = "UNMAPPED: `read_cli` returns None on a non-zero pwsh exit. No subprocess in the tree-sitter reader -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_nonzero_exit_returns_none() {}

#[test]
#[ignore = "UNMAPPED: `read_cli` returns None when pwsh stdout is not JSON. No subprocess/JSON in the tree-sitter reader -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_unparseable_json_returns_none() {}

#[test]
#[ignore = "UNMAPPED: `read_cli` returns None on `subprocess.TimeoutExpired`. No subprocess in the tree-sitter reader -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_timeout_returns_none() {}

#[test]
#[ignore = "UNMAPPED: pins that `subprocess.run(..., timeout=_TIMEOUT)` is wired through. The tree-sitter reader spawns no process, so there is no timeout to wire -> Tier 3 white-box (Python subprocess plumbing). MUST-VERIFY (resolved in the SAFE direction): with no pwsh subprocess there is no unbounded-run DoS surface to guard."]
fn test_extract_passes_the_configured_timeout() {}

#[test]
#[ignore = "UNMAPPED: `read_cli` returns None on an OSError from `subprocess.run`. No subprocess in the tree-sitter reader -> Tier 3 white-box (Python subprocess plumbing)"]
fn test_oserror_returns_none() {}

// ---------------------------------------------------------------- executable discovery

#[test]
#[ignore = "UNMAPPED: `_find_powershell` prefers `pwsh` on PATH. The tree-sitter reader needs no interpreter on PATH -> Tier 3 white-box (Python executable discovery)"]
fn test_find_prefers_pwsh() {}

#[test]
#[ignore = "UNMAPPED: `_find_powershell` returns None on non-Windows without pwsh. No executable discovery in the tree-sitter reader -> Tier 3 white-box (Python executable discovery)"]
fn test_find_none_on_non_windows() {}

#[test]
#[ignore = "UNMAPPED: `_find_powershell` falls back to `powershell.exe` on Windows. No executable discovery in the tree-sitter reader -> Tier 3 white-box (Python executable discovery)"]
fn test_find_falls_back_to_powershell_exe_on_windows() {}

#[test]
#[ignore = "UNMAPPED: `_find_powershell` returns None on Windows with neither shell. No executable discovery in the tree-sitter reader -> Tier 3 white-box (Python executable discovery)"]
fn test_find_none_on_windows_without_powershell() {}

// ---------------------------------------------------------------- flag assembly + plan

#[test]
#[ignore = "UNMAPPED: `flows.FormPlan`/`flows.assemble` build `-Name value` argv and fire a bare `[switch]`. Argv assembly from a form plan lives in skit-application/flows, above skit-language -> Tier 4. The reader half (single-dash PascalCase `flag = -Name`) is covered by test_string_param_with_default_and_help and test_switch_is_a_store_true_flag."]
fn test_single_dash_flags_assemble() {}

#[test]
#[ignore = "UNMAPPED: `store.add_script` + `flows.plan_for_entry` assert plan.source=='argparse' and plan.fields[0].flag=='-City' -> Tier 4 (skit-cli/flows/store). The reader half (a `param([string]$City = 'Taipei')` block yields a static City field with flag `-City`) is covered by the type-matrix tests."]
fn test_plan_reads_powershell_param_block() {}

#[test]
#[ignore = "UNMAPPED: `store.add_script` + `flows.plan_for_entry` assert plan.source=='none' when the reader finds no surface -> Tier 4 (skit-cli/flows/store). The reader half (no param block -> Absent surface) is covered by test_no_param_block_returns_none."]
fn test_plan_none_when_reader_finds_no_surface() {}

// ---------------------------------------------------------------- real pwsh (skip-gated in Python)

#[test]
fn test_integration_reads_a_real_param_block() {
    // Python gates this on `shutil.which("pwsh")`; the tree-sitter reader parses the same source
    // directly, so it always runs. It exercises both Mandatory spellings, ValidateSet, [int], and
    // [switch] over one real block.
    let fields = fields_by_name(concat!(
        "<#\n.PARAMETER City\nThe city to deploy to.\n#>\n",
        "param(\n",
        "  [Parameter(Mandatory)][string]$City,\n", // bare Mandatory (ExpressionOmitted)
        "  [Parameter(Mandatory=$true)][string]$Region,\n", // explicit Mandatory=$true
        "  [ValidateSet('dev','prod')][string]$Env = 'dev',\n",
        "  [int]$Retries = 3,\n",
        "  [switch]$DryRun\n",
        ")\n",
        "Write-Host $City\n",
    ));
    assert!(fields["City"].required); // bare Mandatory spelling
    assert!(fields["Region"].required); // explicit Mandatory=$true spelling
    assert_eq!(fields["City"].help, "The city to deploy to."); // normalized across pwsh versions
    assert_eq!(fields["Env"].parameter_type, ParameterType::Choice);
    assert_eq!(fields["Env"].choices, ["dev", "prod"]);
    assert_eq!(
        fields["Env"].default,
        Some(ParameterValue::String("dev".to_owned()))
    );
    assert_eq!(
        (fields["Retries"].parameter_type, &fields["Retries"].default),
        (ParameterType::Int, &Some(ParameterValue::Integer(3)))
    );
    assert_eq!(fields["DryRun"].parameter_type, ParameterType::Bool);
    assert_eq!(fields["DryRun"].action, "store_true");
}
