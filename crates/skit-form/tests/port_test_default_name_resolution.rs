//! Exact public-surface ports of Python v0.4 `tests/test_default_name_resolution.py`.
//!
//! Python oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//! Every Python parametrized row is retained inside its corresponding Rust test. Secret cases assert
//! that the literal never enters the field projection; degraded cases require both `degraded=true`
//! and `default=None` rather than accepting a generic parse success.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};
use skit_form::{CliFormProjection, cli_form_projection};

fn static_fields(kind: &str, source: &str) -> Vec<ParamDecl> {
    match cli_form_projection(kind, source) {
        CliFormProjection::Static { fields, .. } => fields,
        other => panic!("expected a static {kind} CLI surface, got {other:?}\nsource:\n{source}"),
    }
}

fn one(kind: &str, source: &str) -> ParamDecl {
    let fields = static_fields(kind, source);
    let [field] = fields.as_slice() else {
        panic!("expected exactly one field, got {fields:?}\nsource:\n{source}");
    };
    field.clone()
}

fn by_name(fields: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

fn assert_degraded_without_default(field: &ParamDecl) {
    assert!(field.degraded, "field was not degraded: {field:?}");
    assert_eq!(field.default, None, "degraded field leaked a default: {field:?}");
}

fn assert_string_default(field: &ParamDecl, expected: &str) {
    assert_eq!(
        field.default,
        Some(ParameterValue::String(expected.to_owned())),
        "{field:?}"
    );
    assert!(!field.degraded, "{field:?}");
}

#[test]
fn test_argparse_string_constant_default_resolves() {
    let field = one(
        "python",
        "import argparse\nHOST = 'example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_string_default(&field, "example.com");
}

#[test]
fn test_argparse_int_and_bool_constant_defaults_resolve() {
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
    let field = one(
        "python",
        "import argparse\nHOST = 'a'\nHOST += 'b'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_loop_reassigned_name_does_not_resolve() {
    let field = one(
        "python",
        "import argparse\nHOST = 'a'\nfor i in range(3):\n    HOST = str(i)\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_non_store_value_binding_does_not_resolve() {
    let bindings = [
        "def HOST():\n    pass",
        "async def HOST():\n    pass",
        "class HOST:\n    pass",
        "import pathlib as HOST",
        "from pathlib import Path as HOST",
        "try:\n    pass\nexcept Exception as HOST:\n    pass",
        "match value:\n    case HOST:\n        pass",
        "match value:\n    case [*HOST]:\n        pass",
        "match value:\n    case {\"x\": _, **HOST}:\n        pass",
        "del HOST",
    ];
    assert_eq!(bindings.len(), 10);
    for binding in bindings {
        let source = format!(
            "import argparse\nHOST = 'outer'\n{binding}\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
        );
        let field = one("python", &source);
        assert_degraded_without_default(&field);
    }
}

#[test]
fn test_argparse_star_import_makes_every_constant_default_opaque() {
    let field = one(
        "python",
        "import argparse\nHOST = 'outer'\nfrom defaults import *\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_unknown_name_default_degrades() {
    let field = one(
        "python",
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=MISSING)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_call_default_still_degrades() {
    let field = one(
        "python",
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=str(f()))\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_constant_used_twice_resolves_in_both_fields() {
    let fields = static_fields(
        "python",
        "import argparse\nHOST = 'example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\nap.add_argument('--mirror', default=HOST)\n",
    );
    let by = by_name(&fields);
    assert_string_default(by["host"], "example.com");
    assert_string_default(by["mirror"], "example.com");
}

#[test]
fn test_argparse_conditional_rebinding_does_not_resolve() {
    let field = one(
        "python",
        "import argparse, os\nHOST = 'localhost'\nif os.getenv('PROD'):\n    HOST = 'prod.example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_try_except_rebinding_does_not_resolve() {
    let field = one(
        "python",
        "import argparse\nHOST = 'localhost'\ntry:\n    import prodcfg\n    HOST = 'prod.example.com'\nexcept ImportError:\n    HOST = 'fallback.example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_with_block_rebinding_does_not_resolve() {
    let field = one(
        "python",
        "import argparse\nHOST = 'localhost'\nwith open('cfg') as fh:\n    HOST = 'from-config.example.com'\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_function_local_assignment_blocks_resolution() {
    let field = one(
        "python",
        "import argparse\nHOST = 'localhost'\ndef setup():\n    HOST = 'inner.example.com'\n    return HOST\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_function_parameter_shadow_blocks_resolution() {
    let field = one(
        "python",
        "import argparse\nHOST = 'localhost'\ndef connect(HOST):\n    return HOST\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_argparse_secret_constant_never_resolves() {
    let field = one(
        "python",
        "import argparse\nAPI_KEY = 'sk-live-abc123'\nap = argparse.ArgumentParser()\nap.add_argument('--auth', default=API_KEY)\n",
    );
    assert_degraded_without_default(&field);
    assert!(!format!("{field:?}").contains("sk-live-abc123"), "{field:?}");
}

#[test]
fn test_argparse_password_and_token_constants_never_resolve() {
    let fields = static_fields(
        "python",
        "import argparse\nPASSWORD = 'hunter2'\nGH_TOKEN = 'ghp_xyz789'\nap = argparse.ArgumentParser()\nap.add_argument('--auth', default=PASSWORD)\nap.add_argument('--creds', default=GH_TOKEN)\n",
    );
    let by = by_name(&fields);
    assert_degraded_without_default(by["auth"]);
    assert_degraded_without_default(by["creds"]);
    let debug = format!("{fields:?}");
    assert!(!debug.contains("hunter2"), "{debug}");
    assert!(!debug.contains("ghp_xyz789"), "{debug}");
}

#[test]
fn test_argparse_constant_bound_twice_does_not_resolve() {
    let field = one(
        "python",
        "import argparse\nC = 1\nC = 2\nap = argparse.ArgumentParser()\nap.add_argument('--x', type=int, default=C)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_click_constant_default_resolves() {
    let field = one(
        "python",
        "import click\nCONST = 'prod'\n@click.command()\n@click.option('--n', default=CONST)\ndef m(n): pass\n",
    );
    assert_string_default(&field, "prod");
}

#[test]
fn test_click_constant_also_read_inside_the_body_still_resolves() {
    let field = one(
        "python",
        "import click\nCONST = 'prod'\n@click.command()\n@click.option('--n', default=CONST)\ndef m(n):\n    print(CONST, n)\n",
    );
    assert_string_default(&field, "prod");
}

#[test]
fn test_click_secret_constant_default_degrades() {
    let field = one(
        "python",
        "import click\nAPI_KEY = 'sk-live-abc123'\n@click.command()\n@click.option('--auth', default=API_KEY)\ndef m(auth): pass\n",
    );
    assert_degraded_without_default(&field);
    assert!(!format!("{field:?}").contains("sk-live-abc123"), "{field:?}");
}

#[test]
fn test_click_unknown_name_default_degrades() {
    let field = one(
        "python",
        "import click\n@click.command()\n@click.option('--n', default=MISSING)\ndef m(n): pass\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_typer_legacy_option_constant_default_resolves() {
    let field = one(
        "python",
        "import typer\nCONST = 'prod'\ndef main(x: str = typer.Option(CONST)):\n    pass\ntyper.run(main)\n",
    );
    assert_string_default(&field, "prod");
}

#[test]
fn test_typer_annotated_signature_constant_default_resolves() {
    let field = one(
        "python",
        "import typer\nfrom typing import Annotated\nCONST = 'prod'\ndef main(x: Annotated[str, typer.Option()] = CONST):\n    pass\ntyper.run(main)\n",
    );
    assert_string_default(&field, "prod");
}

#[test]
fn test_typer_bare_signature_constant_default_resolves() {
    let field = one(
        "python",
        "import typer\nCONST = 42\ndef main(x: int = CONST):\n    pass\ntyper.run(main)\n",
    );
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert_eq!(field.default, Some(ParameterValue::Integer(42)));
    assert!(!field.degraded);
}

#[test]
fn test_typer_unknown_signature_default_degrades() {
    let field = one(
        "python",
        "import typer\ndef main(x: int = MISSING):\n    pass\ntyper.run(main)\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_js_constant_default_resolves() {
    let field = one(
        "js",
        "const DEFAULT_HOST = \"example.com\";\nparseArgs({options:{host:{type:\"string\", default: DEFAULT_HOST}}});\n",
    );
    assert_string_default(&field, "example.com");
}

#[test]
fn test_js_let_binding_default_does_not_resolve() {
    let field = one(
        "js",
        "let HOST = \"example.com\";\nparseArgs({options:{host:{type:\"string\", default: HOST}}});\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_js_reassigned_const_default_does_not_resolve() {
    let field = one(
        "js",
        "const HOST = \"a\";\nHOST = \"b\";\nparseArgs({options:{host:{type:\"string\", default: HOST}}});\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_js_unknown_identifier_default_degrades() {
    let field = one(
        "js",
        "parseArgs({options:{host:{type:\"string\", default: UNKNOWN}}});\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_js_function_local_const_shadow_does_not_resolve() {
    let field = one(
        "js",
        "const HOST = \"localhost\";\nfunction main() {\n  const HOST = process.env.HOST ?? \"prod.internal\";\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain();\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_js_function_parameter_shadow_does_not_resolve() {
    let field = one(
        "js",
        "const HOST = \"localhost\";\nfunction main(HOST) {\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain(\"prod.internal\");\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_js_constant_read_as_a_parameter_default_still_resolves() {
    let field = one(
        "js",
        "const HOST = \"localhost\";\nfunction main(a = HOST) { return a; }\nparseArgs({options:{host:{type:\"string\", default: HOST}}});\n",
    );
    assert_string_default(&field, "localhost");
}

#[test]
fn test_ts_typed_parameter_default_reads_the_constant_without_declaring_it() {
    let field = one(
        "ts",
        "const HOST = \"localhost\";\nfunction main(a: string = HOST) { return a; }\nparseArgs({options:{host:{type:\"string\", default: HOST}}});\n",
    );
    assert_string_default(&field, "localhost");
}

#[test]
fn test_ts_destructured_parameter_default_is_also_only_a_read() {
    let field = one(
        "ts",
        "const DEF = \"d\";\nfunction main({x}: Opts = DEF) { return x; }\nparseArgs({options:{host:{type:\"string\", default: DEF}}});\n",
    );
    assert_string_default(&field, "d");
}

#[test]
fn test_ts_typed_parameter_with_a_default_still_shadows_by_its_bound_name() {
    let field = one(
        "ts",
        "const HOST = \"localhost\";\nfunction main(HOST: string = \"inner.example.com\") {\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain();\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_js_parameter_with_a_default_still_shadows_by_its_bound_name() {
    let field = one(
        "js",
        "const HOST = \"localhost\";\nfunction main(HOST = \"inner.example.com\") {\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain();\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_ts_typed_function_parameter_shadow_does_not_resolve() {
    let field = one(
        "ts",
        "const HOST: string = \"localhost\";\nfunction main(HOST: string) {\n  parseArgs({options:{host:{type:\"string\", default: HOST}}});\n}\nmain(\"prod.internal\");\n",
    );
    assert_degraded_without_default(&field);
}

#[test]
fn test_js_secret_constant_never_resolves() {
    let field = one(
        "js",
        "const API_KEY = \"sk-live-abc123\";\nparseArgs({options:{auth:{type:\"string\", default: API_KEY}}});\n",
    );
    assert_degraded_without_default(&field);
    assert!(!format!("{field:?}").contains("sk-live-abc123"), "{field:?}");
}

#[test]
fn test_js_non_parameter_value_binding_does_not_resolve() {
    let cases = [
        ("function main() { class HOST {}\n", "\n}"),
        ("function main() { function HOST() {}\n", "\n}"),
        ("function main() { const {HOST} = source;\n", "\n}"),
        ("function main() { const {x: HOST} = source;\n", "\n}"),
        ("function main() { try {} catch (HOST) {\n", "\n}\n}"),
        ("(function HOST() {\n", "\n})();"),
    ];
    assert_eq!(cases.len(), 6);
    for (prefix, suffix) in cases {
        let source = format!(
            "const HOST = \"outer\";\n{prefix}parseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});{suffix}"
        );
        let field = one("js", &source);
        assert_degraded_without_default(&field);
    }
}

#[test]
fn test_js_import_binding_does_not_resolve() {
    let clauses = ["HOST", "* as HOST", "{source as HOST}", "{HOST}"];
    assert_eq!(clauses.len(), 4);
    for clause in clauses {
        let source = format!(
            "import {clause} from \"defaults\";\nconst HOST = \"outer\";\nparseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});\n"
        );
        let field = one("js", &source);
        assert_degraded_without_default(&field);
    }
}

#[test]
fn test_js_nonbinding_shapes_leave_constant_resolution_intact() {
    let preambles = [
        "import \"defaults\";",
        "import {} from \"defaults\";",
        "const Anonymous = class {};",
        "try {} catch {}",
    ];
    assert_eq!(preambles.len(), 4);
    for preamble in preambles {
        let source = format!(
            "const HOST = \"outer\";\n{preamble}\nparseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});\n"
        );
        let field = one("js", &source);
        assert_string_default(&field, "outer");
    }
}

#[test]
fn test_ts_constant_default_resolves() {
    let field = one(
        "ts",
        "const DEFAULT_HOST: string = \"example.com\";\nparseArgs({options:{host:{type:\"string\", default: DEFAULT_HOST}}});\n",
    );
    assert_string_default(&field, "example.com");
}
