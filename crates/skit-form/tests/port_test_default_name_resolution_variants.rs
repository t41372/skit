//! Rust-additive expansion of every parametrized row in Python v0.4
//! `tests/test_default_name_resolution.py`.
//!
//! The exact 42 Python test functions remain in `port_test_default_name_resolution.rs`. These
//! additive tests do not count toward that parity number; they ensure a failure in one Python
//! parametrized row cannot prevent later rows from executing in Rust.

use skit_domain::parameters::{ParamDecl, ParameterValue};
use skit_form::{CliFormProjection, cli_form_projection};

fn one(kind: &str, source: &str) -> ParamDecl {
    match cli_form_projection(kind, source) {
        CliFormProjection::Static { fields, .. } => {
            let [field] = fields.as_slice() else {
                panic!("expected one field, got {fields:?}\nsource:\n{source}");
            };
            field.clone()
        }
        other => panic!("expected static {kind} CLI surface, got {other:?}\nsource:\n{source}"),
    }
}

fn assert_degraded(kind: &str, source: &str) {
    let field = one(kind, source);
    assert!(field.degraded, "field was not degraded: {field:?}\nsource:\n{source}");
    assert_eq!(
        field.default, None,
        "degraded field leaked a default: {field:?}\nsource:\n{source}"
    );
}

fn assert_string_default(kind: &str, source: &str, expected: &str) {
    let field = one(kind, source);
    assert_eq!(
        field.default,
        Some(ParameterValue::String(expected.to_owned())),
        "{field:?}\nsource:\n{source}"
    );
    assert!(!field.degraded, "{field:?}\nsource:\n{source}");
}

fn argparse_binding(binding: &str) -> String {
    format!(
        "import argparse\nHOST = 'outer'\n{binding}\nap = argparse.ArgumentParser()\nap.add_argument('--host', default=HOST)\n"
    )
}

#[test]
fn rust_additive_argparse_function_definition_binding_invalidates_constant() {
    assert_degraded("python", &argparse_binding("def HOST():\n    pass"));
}

#[test]
fn rust_additive_argparse_async_function_definition_binding_invalidates_constant() {
    assert_degraded("python", &argparse_binding("async def HOST():\n    pass"));
}

#[test]
fn rust_additive_argparse_class_definition_binding_invalidates_constant() {
    assert_degraded("python", &argparse_binding("class HOST:\n    pass"));
}

#[test]
fn rust_additive_argparse_import_alias_binding_invalidates_constant() {
    assert_degraded("python", &argparse_binding("import pathlib as HOST"));
}

#[test]
fn rust_additive_argparse_from_import_alias_binding_invalidates_constant() {
    assert_degraded(
        "python",
        &argparse_binding("from pathlib import Path as HOST"),
    );
}

#[test]
fn rust_additive_argparse_except_alias_binding_invalidates_constant() {
    assert_degraded(
        "python",
        &argparse_binding("try:\n    pass\nexcept Exception as HOST:\n    pass"),
    );
}

#[test]
fn rust_additive_argparse_match_capture_binding_invalidates_constant() {
    assert_degraded(
        "python",
        &argparse_binding("match value:\n    case HOST:\n        pass"),
    );
}

#[test]
fn rust_additive_argparse_match_splat_binding_invalidates_constant() {
    assert_degraded(
        "python",
        &argparse_binding("match value:\n    case [*HOST]:\n        pass"),
    );
}

#[test]
fn rust_additive_argparse_match_dictionary_splat_binding_invalidates_constant() {
    assert_degraded(
        "python",
        &argparse_binding("match value:\n    case {\"x\": _, **HOST}:\n        pass"),
    );
}

#[test]
fn rust_additive_argparse_delete_binding_invalidates_constant() {
    assert_degraded("python", &argparse_binding("del HOST"));
}

fn js_binding(prefix: &str, suffix: &str) -> String {
    format!(
        "const HOST = \"outer\";\n{prefix}parseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});{suffix}"
    )
}

#[test]
fn rust_additive_js_class_binding_invalidates_constant() {
    assert_degraded(
        "js",
        &js_binding("function main() { class HOST {}\n", "\n}"),
    );
}

#[test]
fn rust_additive_js_function_binding_invalidates_constant() {
    assert_degraded(
        "js",
        &js_binding("function main() { function HOST() {}\n", "\n}"),
    );
}

#[test]
fn rust_additive_js_destructured_binding_invalidates_constant() {
    assert_degraded(
        "js",
        &js_binding("function main() { const {HOST} = source;\n", "\n}"),
    );
}

#[test]
fn rust_additive_js_renamed_destructured_binding_invalidates_constant() {
    assert_degraded(
        "js",
        &js_binding("function main() { const {x: HOST} = source;\n", "\n}"),
    );
}

#[test]
fn rust_additive_js_catch_binding_invalidates_constant() {
    assert_degraded(
        "js",
        &js_binding("function main() { try {} catch (HOST) {\n", "\n}\n}"),
    );
}

#[test]
fn rust_additive_js_named_function_expression_binding_invalidates_constant() {
    assert_degraded("js", &js_binding("(function HOST() {\n", "\n})();"));
}

fn js_import(clause: &str) -> String {
    format!(
        "import {clause} from \"defaults\";\nconst HOST = \"outer\";\nparseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});\n"
    )
}

#[test]
fn rust_additive_js_default_import_binding_invalidates_constant() {
    assert_degraded("js", &js_import("HOST"));
}

#[test]
fn rust_additive_js_namespace_import_binding_invalidates_constant() {
    assert_degraded("js", &js_import("* as HOST"));
}

#[test]
fn rust_additive_js_renamed_named_import_binding_invalidates_constant() {
    assert_degraded("js", &js_import("{source as HOST}"));
}

#[test]
fn rust_additive_js_named_import_binding_invalidates_constant() {
    assert_degraded("js", &js_import("{HOST}"));
}

fn js_nonbinding(preamble: &str) -> String {
    format!(
        "const HOST = \"outer\";\n{preamble}\nparseArgs({{options:{{host:{{type:\"string\", default: HOST}}}}}});\n"
    )
}

#[test]
fn rust_additive_js_side_effect_import_does_not_invalidate_constant() {
    assert_string_default("js", &js_nonbinding("import \"defaults\";"), "outer");
}

#[test]
fn rust_additive_js_empty_named_import_does_not_invalidate_constant() {
    assert_string_default(
        "js",
        &js_nonbinding("import {} from \"defaults\";"),
        "outer",
    );
}

#[test]
fn rust_additive_js_anonymous_class_expression_does_not_invalidate_constant() {
    assert_string_default(
        "js",
        &js_nonbinding("const Anonymous = class {};"),
        "outer",
    );
}

#[test]
fn rust_additive_js_catch_without_binding_does_not_invalidate_constant() {
    assert_string_default("js", &js_nonbinding("try {} catch {}"), "outer");
}
