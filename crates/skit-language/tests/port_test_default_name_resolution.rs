//! Mechanical port of the Python oracle module `tests/test_default_name_resolution.py`
//! (`origin/main@206f9ef`): a `default=` naming a top-level literal constant resolves
//! through the analyzer's own constant harvest, exactly as if the literal were inline —
//! but only when the name is bound EXACTLY ONCE file-wide and is not secret-looking.
//! Each `#[test]` keeps its Python `def test_*` name so it traces back to its origin, and
//! each Python "WHY" comment is preserved above it.
//!
//! Concept mapping used throughout:
//! - Python `argspec.read_argparse(src)` / `argspec.read_cli(src)` (argparse, then click,
//!   then typer) -> `parsed("python", src).cli_surface()`.
//! - Python `cli_reader.read_cli(src, lang="js"|"ts")` -> `parsed("js"|"ts", src).cli_surface()`.
//! - Python `spec is not None` (every case here builds a readable CLI surface) ->
//!   `CliSurface::Static(surface)` (the `static_fields` helper panics on any other shape).
//! - Python `spec.fields[i]` -> `surface.fields[i].declaration` (a `ParamDecl`).
//! - Python `f.default` -> `ParamDecl.default: Option<ParameterValue>` (`ParameterValue::String`,
//!   `Integer`, `Bool`); Python `f.default is None` -> `None`.
//! - Python `f.degraded` -> `ParamDecl.degraded: bool`.
//! - Python `f.type == "int"` -> `ParamDecl.parameter_type == ParameterType::Int`.
//! - Python `"<secret>" not in repr(f)` -> `!format!("{declaration:?}").contains("<secret>")`.
//!
//! Buckets: every test is API-EXISTS (a real asserting `#[test]`). The full constant-folding
//! surface (`argparse_field` / `click_field` / `typer_field` / `javascript::cli_surface` and
//! their shared `constant_environment` / `bound_name_counts` / `declared_names`) lives in
//! `skit-language` and is reachable through the public `cli_surface()` API. No cross-crate
//! stub, no absent gap.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_language::{CliSurface, ParseOutcome, ParsedDocument, parse_document};

fn parsed(kind: &str, source: &str) -> ParsedDocument {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid {kind} source, got {other:?}"),
    }
}

/// Python `spec.fields` for an `.ok` spec: the declarations of a static CLI surface.
fn static_fields(kind: &str, source: &str) -> Vec<ParamDecl> {
    let CliSurface::Static(surface) = parsed(kind, source).cli_surface() else {
        panic!("expected a static CLI surface for {kind} source");
    };
    surface
        .fields
        .into_iter()
        .map(|field| field.declaration)
        .collect()
}

/// Python `{f.name: f for f in spec.fields}`.
fn by_name(fields: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

// --------------------------------------------------------------------------
// argparse: default=CONST resolving through the top-level constant environment
// --------------------------------------------------------------------------

#[test]
fn test_argparse_string_constant_default_resolves() {
    // A bare Name default referring to a top-level literal const resolves as if the string
    // literal were inline — clean field, not a degraded free-text box.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert_eq!(
        f.default,
        Some(ParameterValue::String("example.com".to_owned()))
    );
    assert!(!f.degraded);
}

#[test]
fn test_argparse_int_and_bool_constant_defaults_resolve() {
    // int and bool constant values resolve to their literal value, both clean.
    let fields = static_fields(
        "python",
        "import argparse\nPORT = 8080\nDEBUG = True\nap = argparse.ArgumentParser()\nap.add_argument('--port', type=int, default=PORT)\nap.add_argument('--debug', default=DEBUG)\n",
    );
    let by = by_name(&fields);
    assert_eq!(by["port"].default, Some(ParameterValue::Integer(8080)));
    assert!(!by["port"].degraded);
    assert_eq!(by["debug"].default, Some(ParameterValue::Bool(true)));
    assert!(!by["debug"].degraded);
}

#[test]
fn test_argparse_augmented_assigned_name_does_not_resolve() {
    // A name augmented-assigned anywhere is a working variable, not a knowable constant:
    // its value at parse time isn't a single literal, so the field degrades.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'a'\nHOST += 'b'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_loop_reassigned_name_does_not_resolve() {
    // Reassigned inside a loop body -> mutated -> excluded from the constant environment.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'a'\nfor i in range(3):\n    HOST = str(i)\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_non_store_value_binding_does_not_resolve() {
    // Definitions/imports/handlers/patterns/delete all invalidate the outer literal.
    // The `case HOST:` capture-pattern member is pinned separately with its qualified reverse.
    for binding in [
        "def HOST():\n    pass",
        "async def HOST():\n    pass",
        "class HOST:\n    pass",
        "import pathlib as HOST",
        "from pathlib import Path as HOST",
        "try:\n    pass\nexcept Exception as HOST:\n    pass",
        "match value:\n    case [*HOST]:\n        pass",
        "match value:\n    case {\"x\": _, **HOST}:\n        pass",
        "del HOST",
    ] {
        let source = format!(
            "import argparse\nHOST = 'outer'\n{binding}\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
        );
        let fields = static_fields("python", &source);
        let f = &fields[0];
        assert!(f.degraded, "{binding:?}");
        assert_eq!(f.default, None, "{binding:?}");
    }
}

#[test]
fn test_argparse_non_store_value_binding_does_not_resolve_case_capture() {
    // The `case HOST:` member of the oracle parametrize set: a capture pattern binds HOST a
    // second time, so the outer literal must not resolve.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'outer'\nmatch value:\n    case HOST:\n        pass\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);

    // A qualified value pattern reads both segments. It does not bind the trailing name.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'outer'\nmatch value:\n    case Color.HOST:\n        pass\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(!f.degraded);
    assert_eq!(f.default, Some(ParameterValue::String("outer".to_owned())));
}

#[test]
fn test_argparse_star_import_makes_every_constant_default_opaque() {
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'outer'\nfrom defaults import *\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert!(fields[0].degraded);
}

#[test]
fn test_argparse_unknown_name_default_degrades() {
    // A default naming something that isn't a top-level literal const at all: no resolution,
    // so the field degrades (shown, omitted when left empty so the script's own default applies).
    let fields = static_fields(
        "python",
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=MISSING)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_call_default_still_degrades() {
    // Unchanged behavior: a computed default (a call) is never read as a value — it degrades.
    let fields = static_fields(
        "python",
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=str(f()))\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_constant_used_twice_resolves_in_both_fields() {
    // The rule counts BINDINGS, not USES: one assignment read by two add_argument calls is
    // still provably one literal, so both fields resolve. (Were Load-context names counted
    // too, this constant would look "bound" three times and both fields would degrade —
    // over-tightening the fix into uselessness, since a constant exists to be referenced.)
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\nap.add_argument('--mirror', default=HOST)\n",
    );
    let by = by_name(&fields);
    assert_eq!(
        by["host"].default,
        Some(ParameterValue::String("example.com".to_owned()))
    );
    assert!(!by["host"].degraded);
    assert_eq!(
        by["mirror"].default,
        Some(ParameterValue::String("example.com".to_owned()))
    );
    assert!(!by["mirror"].degraded);
}

#[test]
fn test_argparse_conditional_rebinding_does_not_resolve() {
    // Rebound in an `if` body: the top-level harvest sees only `HOST = 'localhost'`, but the
    // script's own PROD branch may have replaced it by the time the parser is built.
    // Resolving would make skit pass `--host localhost` on EVERY run and silently defeat
    // that branch — so the name is bound twice module-wide and never resolves.
    let fields = static_fields(
        "python",
        "import argparse, os\nHOST = 'localhost'\nif os.getenv('PROD'):\n    HOST = 'prod.example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_try_except_rebinding_does_not_resolve() {
    // The same rule inside a try/except: neither rebinding is a top-level statement, so the
    // harvest can't see them — the module-wide binding count is what catches this.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'localhost'\ntry:\n    import prodcfg\n    HOST = 'prod.example.com'\nexcept ImportError:\n    HOST = 'fallback.example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_with_block_rebinding_does_not_resolve() {
    // And inside a `with` body — one more non-top-level statement position.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'localhost'\nwith open('cfg') as fh:\n    HOST = 'from-config.example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_function_local_assignment_blocks_resolution() {
    // A function-local assignment to the same name is a second binding module-wide. skit
    // refuses to reason about which one the parser call sees, so the field degrades.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'localhost'\ndef setup():\n    HOST = 'inner.example.com'\n    return HOST\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_function_parameter_shadow_blocks_resolution() {
    // A PARAMETER of the same name binds it too (_bound_names counts ast.arg), so the
    // top-level literal is no longer provably the only binding — degrade, don't guess.
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'localhost'\ndef connect(HOST):\n    return HOST\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_argparse_secret_constant_never_resolves() {
    // C3: a hardcoded API key resolved into a field default would be prefilled into the run
    // form, printed by `show --json` and written into preset TOML on disk — the literal
    // leaving the script's own text for the first time. The field degrades instead, and the
    // secret appears NOWHERE in the resulting declaration.
    let fields = static_fields(
        "python",
        "import argparse\nAPI_KEY = 'sk-live-abc123'\nap = argparse.ArgumentParser()\nap.add_argument('--auth', default=API_KEY)\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
    assert!(!format!("{f:?}").contains("sk-live-abc123"));
}

#[test]
fn test_argparse_password_and_token_constants_never_resolve() {
    // The same C3 rule for the other secret-looking spellings the harvest flags.
    let fields = static_fields(
        "python",
        "import argparse\nPASSWORD = 'hunter2'\nGH_TOKEN = 'ghp_xyz789'\nap = argparse.ArgumentParser()\nap.add_argument('--auth', default=PASSWORD)\nap.add_argument('--creds', default=GH_TOKEN)\n",
    );
    let by = by_name(&fields);
    assert!(by["auth"].degraded);
    assert_eq!(by["auth"].default, None);
    assert!(by["creds"].degraded);
    assert_eq!(by["creds"].default, None);
    assert!(!format!("{fields:?}").contains("hunter2"));
    assert!(!format!("{fields:?}").contains("ghp_xyz789"));
}

#[test]
fn test_argparse_constant_bound_twice_does_not_resolve() {
    // Two top-level assignments to the same name: which one is in force at the
    // add_argument call depends on where that call sits between them, and skit refuses to
    // guess (A4/C4). The field degrades to free-text, so an untouched field is omitted and
    // the script's own default applies — the honest answer, not a coin flip.
    let fields = static_fields(
        "python",
        "import argparse\nC = 1\nC = 2\nap = argparse.ArgumentParser()\nap.add_argument('--x', type=int, default=C)\n",
    );
    let f = &fields[0];
    assert_eq!(f.default, None);
    assert!(f.degraded);
}

// --------------------------------------------------------------------------
// click: @click.option(default=CONST)
// --------------------------------------------------------------------------

#[test]
fn test_click_constant_default_resolves() {
    let fields = static_fields(
        "python",
        "import click\nCONST = 'prod'\n@click.command()\n@click.option('--n', default=CONST)\ndef m(n): pass\n",
    );
    let f = &fields[0];
    assert_eq!(f.default, Some(ParameterValue::String("prod".to_owned())));
    assert!(!f.degraded);
}

#[test]
fn test_click_constant_also_read_inside_the_body_still_resolves() {
    // The positive, guarded against over-tightening: the const is READ again inside the
    // command body. Reads are not bindings, so it stays single-bound and resolves.
    let fields = static_fields(
        "python",
        "import click\nCONST = 'prod'\n@click.command()\n@click.option('--n', default=CONST)\ndef m(n):\n    print(CONST, n)\n",
    );
    let f = &fields[0];
    assert_eq!(f.default, Some(ParameterValue::String("prod".to_owned())));
    assert!(!f.degraded);
}

#[test]
fn test_click_secret_constant_default_degrades() {
    // C3 holds on click's surface too: the key never leaves the script's own text.
    let fields = static_fields(
        "python",
        "import click\nAPI_KEY = 'sk-live-abc123'\n@click.command()\n@click.option('--auth', default=API_KEY)\ndef m(auth): pass\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
    assert!(!format!("{f:?}").contains("sk-live-abc123"));
}

#[test]
fn test_click_unknown_name_default_degrades() {
    let fields = static_fields(
        "python",
        "import click\n@click.command()\n@click.option('--n', default=MISSING)\ndef m(n): pass\n",
    );
    assert!(fields[0].degraded);
}

// --------------------------------------------------------------------------
// typer: legacy Option positional default, Annotated signature default, bare signature default
// --------------------------------------------------------------------------

#[test]
fn test_typer_legacy_option_constant_default_resolves() {
    // `x: str = typer.Option(CONST)` — the first positional of the Option call is the value
    // default, and a Name there resolves through the constant environment.
    let fields = static_fields(
        "python",
        "import typer\nCONST = 'prod'\ndef main(x: str = typer.Option(CONST)):\n    pass\ntyper.run(main)\n",
    );
    let f = &fields[0];
    assert_eq!(f.default, Some(ParameterValue::String("prod".to_owned())));
    assert!(!f.degraded);
}

#[test]
fn test_typer_annotated_signature_constant_default_resolves() {
    // `x: Annotated[str, typer.Option()] = CONST` — the `= value` default is a Name and resolves.
    let fields = static_fields(
        "python",
        "import typer\nfrom typing import Annotated\nCONST = 'prod'\ndef main(x: Annotated[str, typer.Option()] = CONST):\n    pass\ntyper.run(main)\n",
    );
    let f = &fields[0];
    assert_eq!(f.default, Some(ParameterValue::String("prod".to_owned())));
    assert!(!f.degraded);
}

#[test]
fn test_typer_bare_signature_constant_default_resolves() {
    // `x: int = CONST` — a plain signature default naming an int const resolves to its value.
    let fields = static_fields(
        "python",
        "import typer\nCONST = 42\ndef main(x: int = CONST):\n    pass\ntyper.run(main)\n",
    );
    let f = &fields[0];
    assert_eq!(f.parameter_type, ParameterType::Int);
    assert_eq!(f.default, Some(ParameterValue::Integer(42)));
    assert!(!f.degraded);
}

#[test]
fn test_typer_unknown_signature_default_degrades() {
    // A signature default naming an unknown constant degrades (unchanged non-resolvable behavior).
    let fields = static_fields(
        "python",
        "import typer\ndef main(x: int = MISSING):\n    pass\ntyper.run(main)\n",
    );
    assert!(fields[0].degraded);
}

// --------------------------------------------------------------------------
// JS/TS parseArgs: default naming a top-level const
// --------------------------------------------------------------------------

#[test]
fn test_js_constant_default_resolves() {
    // `default: DEFAULT_HOST` naming a top-level `const` resolves as if the literal were inline.
    let fields = static_fields(
        "js",
        "const DEFAULT_HOST = \"example.com\";\nparseArgs({options:{host:{type:\"string\", default: DEFAULT_HOST}}});\n",
    );
    let f = &fields[0];
    assert_eq!(
        f.default,
        Some(ParameterValue::String("example.com".to_owned()))
    );
    assert!(!f.degraded);
}

#[test]
fn test_js_let_binding_default_does_not_resolve() {
    // A `let` binding is demoted (reassignable), so it is excluded from the constant
    // environment and the field degrades.
    let fields = static_fields(
        "js",
        "let HOST = \"example.com\";\nparseArgs({options:{host:{type:\"string\", default: HOST}}});\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_js_reassigned_const_default_does_not_resolve() {
    // A const that is nonetheless reassigned is a working variable -> mutated -> excluded.
    let fields = static_fields(
        "js",
        "const HOST = \"a\";\nHOST = \"b\";\nparseArgs({options:{host:{type:\"string\", default: HOST}}});\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_js_unknown_identifier_default_degrades() {
    // An identifier that names nothing top-level and literal degrades the field.
    let fields = static_fields(
        "js",
        "parseArgs({options:{host:{type:\"string\", default: UNKNOWN}}});\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_js_function_local_const_shadow_does_not_resolve() {
    // `_const_candidates` only sees the TOP-LEVEL `const HOST`, so without the file-wide
    // declaration count the field would resolve to the outer "localhost" — overriding the
    // inner value the script would actually have used, on every run. Two declarations of
    // the name means the harvested literal isn't provably the one in scope: degrade.
    let fields = static_fields(
        "js",
        "const HOST = \"localhost\";\nfunction main() {\n  const HOST = process.env.HOST ?? \"prod.internal\";\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain();\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_js_function_parameter_shadow_does_not_resolve() {
    // A formal PARAMETER of the same name shadows the top-level const just as a local
    // declaration does — `_declared_names` counts parameters, so this degrades too.
    let fields = static_fields(
        "js",
        "const HOST = \"localhost\";\nfunction main(HOST) {\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain(\"prod.internal\");\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_js_constant_read_as_a_parameter_default_still_resolves() {
    // `function main(a = HOST)` BINDS `a` and merely READS HOST. Counting that read as a
    // declaration would make the constant look bound twice and refuse to fold a value
    // that is provably still the one literal — a false negative that would quietly
    // degrade fields as soon as a script used its own constant as a parameter default.
    let fields = static_fields(
        "js",
        "const HOST = \"localhost\";\nfunction main(a = HOST) { return a; }\nparseArgs({options:{host:{type:\"string\", default: HOST}}});\n",
    );
    let f = &fields[0];
    assert_eq!(
        f.default,
        Some(ParameterValue::String("localhost".to_owned()))
    );
    assert!(!f.degraded);
}

#[test]
fn test_ts_typed_parameter_default_reads_the_constant_without_declaring_it() {
    // TypeScript's shape differs from JS's: `pattern`, `type` and `value` all hang off one
    // required_parameter, so `a: string = HOST` reaches HOST through a SIBLING of the
    // binding rather than through an assignment_pattern. Only `pattern` binds a name —
    // counting the sibling read would refuse to fold a constant that is still one literal.
    let fields = static_fields(
        "ts",
        "const HOST = \"localhost\";\nfunction main(a: string = HOST) { return a; }\nparseArgs({options:{host:{type:\"string\", default: HOST}}});\n",
    );
    let f = &fields[0];
    assert_eq!(
        f.default,
        Some(ParameterValue::String("localhost".to_owned()))
    );
    assert!(!f.degraded);
}

#[test]
fn test_ts_destructured_parameter_default_is_also_only_a_read() {
    // Same rule through a destructuring pattern: `{x}: Opts = DEF` binds x, reads DEF.
    let fields = static_fields(
        "ts",
        "const DEF = \"d\";\nfunction main({x}: Opts = DEF) { return x; }\nparseArgs({options:{host:{type:\"string\", default: DEF}}});\n",
    );
    let f = &fields[0];
    assert_eq!(f.default, Some(ParameterValue::String("d".to_owned())));
    assert!(!f.degraded);
}

#[test]
fn test_ts_typed_parameter_with_a_default_still_shadows_by_its_bound_name() {
    // The other half: the typed parameter's own name still shadows the top-level const,
    // default or no default — `pattern` is exactly what gets counted.
    let fields = static_fields(
        "ts",
        "const HOST = \"localhost\";\nfunction main(HOST: string = \"inner.example.com\") {\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain();\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_js_parameter_with_a_default_still_shadows_by_its_bound_name() {
    // The other half of the same branch: in `function main(HOST = "x")` the LEFT of the
    // default IS the bound name, so it shadows the top-level const and blocks folding.
    let fields = static_fields(
        "js",
        "const HOST = \"localhost\";\nfunction main(HOST = \"inner.example.com\") {\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain();\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_ts_typed_function_parameter_shadow_does_not_resolve() {
    // The TypeScript grammar wraps a parameter in a required_parameter pattern rather than
    // exposing a bare identifier, so the parameter walk has to reach INSIDE it — the typed
    // spelling is a distinct shape and gets its own pin.
    let fields = static_fields(
        "ts",
        "const HOST: string = \"localhost\";\nfunction main(HOST: string) {\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain(\"prod.internal\");\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
}

#[test]
fn test_js_secret_constant_never_resolves() {
    // C3 on the JS surface: a hardcoded key must not escape the script's own text through a
    // resolved field default (prefill, `show --json`, preset TOML on disk).
    let fields = static_fields(
        "js",
        "const API_KEY = \"sk-live-abc123\";\nparseArgs({options:{auth:{type:\"string\", default: API_KEY}}});\n",
    );
    let f = &fields[0];
    assert!(f.degraded);
    assert_eq!(f.default, None);
    assert!(!format!("{f:?}").contains("sk-live-abc123"));
}

#[test]
fn test_js_non_parameter_value_binding_does_not_resolve() {
    for (prefix, suffix) in [
        ("function main() { class HOST {}\n", "\n}"),
        ("function main() { function HOST() {}\n", "\n}"),
        ("function main() { const {HOST} = source;\n", "\n}"),
        ("function main() { const {x: HOST} = source;\n", "\n}"),
        ("function main() { try {} catch (HOST) {\n", "\n}\n}"),
        ("(function HOST() {\n", "\n})();"),
    ] {
        let source = format!(
            "const HOST = \"outer\";\n{prefix}parseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});{suffix}"
        );
        let fields = static_fields("js", &source);
        let f = &fields[0];
        assert!(f.degraded, "{prefix:?}{suffix:?}");
        assert_eq!(f.default, None, "{prefix:?}{suffix:?}");
    }
}

#[test]
fn test_js_import_binding_does_not_resolve() {
    // Tree-sitter reports syntax, not the later module-linking duplicate-binding error;
    // the static reader must still never fold through a value import binding.
    for import_clause in ["HOST", "* as HOST", "{source as HOST}", "{HOST}"] {
        let source = format!(
            "import {import_clause} from \"defaults\";\nconst HOST = \"outer\";\nparseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});\n"
        );
        let fields = static_fields("js", &source);
        assert!(fields[0].degraded, "{import_clause:?}");
    }
}

#[test]
fn test_js_nonbinding_shapes_leave_constant_resolution_intact() {
    for preamble in [
        "import \"defaults\";",
        "import {} from \"defaults\";",
        "const Anonymous = class {};",
        "try {} catch {}",
    ] {
        let source = format!(
            "const HOST = \"outer\";\n{preamble}\nparseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});\n"
        );
        let fields = static_fields("js", &source);
        assert_eq!(
            fields[0].default,
            Some(ParameterValue::String("outer".to_owned())),
            "{preamble:?}"
        );
        assert!(!fields[0].degraded, "{preamble:?}");
    }
}

#[test]
fn test_ts_constant_default_resolves() {
    // The same resolution holds under the TypeScript grammar (annotated const declaration).
    let fields = static_fields(
        "ts",
        "const DEFAULT_HOST: string = \"example.com\";\nparseArgs({options:{host:{type:\"string\", default: DEFAULT_HOST}}});\n",
    );
    let f = &fields[0];
    assert_eq!(
        f.default,
        Some(ParameterValue::String("example.com".to_owned()))
    );
    assert!(!f.degraded);
}
