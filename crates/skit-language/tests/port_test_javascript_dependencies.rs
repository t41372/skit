//! Public-API ports of Python v0.4 JavaScript/TypeScript dependency-scanner contracts.

use skit_language::external_dependencies;

#[test]
fn test_external_imports_covers_all_import_forms() {
    let text = concat!(
        "import chalk from \"chalk\";\n",
        "import { z } from \"zod\";\n",
        "export { x } from \"commander\";\n",
        "const dyn = await import(\"execa\");\n",
        "const cjs = require(\"rimraf\");\n",
        "import chalk2 from \"chalk\";\n",
    );
    assert_eq!(
        external_dependencies("js", text),
        ["chalk", "zod", "commander", "execa", "rimraf"]
    );
}

#[test]
fn test_external_imports_excludes_non_packages() {
    let text = concat!(
        "import fs from \"node:fs\";\n",
        "import path from \"path\";\n",
        "import local from \"./util.mjs\";\n",
        "import abs from \"/opt/x.js\";\n",
        "import n from \"npm:chalk@5\";\n",
        "import j from \"jsr:@std/fs\";\n",
        "import remote from \"https://esm.sh/preact\";\n",
        "import d from \"data:text/javascript,export default 1\";\n",
        "import log from \"#internal/log\";\n",
        "import cfg from \"#config\";\n",
    );
    assert!(external_dependencies("js", text).is_empty());
}

#[test]
fn test_external_imports_rejects_malformed_scoped_specifiers() {
    for specifier in ["@scope/", "@scope//pkg", "@/pkg", "@only-a-scope"] {
        let text = format!("import x from \"{specifier}\";\n");
        assert!(external_dependencies("js", &text).is_empty(), "{specifier}");
    }
}

#[test]
fn test_external_imports_maps_deep_imports_to_the_package_root() {
    let text = concat!(
        "import fp from \"lodash/fp\";\n",
        "import cmd from \"@aws-sdk/client-s3/commands\";\n",
        "import a from \"@a/b\";\n",
    );
    assert_eq!(
        external_dependencies("js", text),
        ["lodash", "@aws-sdk/client-s3", "@a/b"]
    );
}

#[test]
fn test_external_imports_skips_unreadable_specifiers() {
    let text = concat!(
        "const a = require(name);\n",
        "const b = require(\"a\", \"b\");\n",
        "const c = notrequire(\"pkg\");\n",
        "const d = require();\n",
        "const e = require(`tpl`);\n",
    );
    assert!(external_dependencies("js", text).is_empty());
}

#[test]
fn test_external_imports_reads_typescript_under_the_ts_grammar() {
    let text = concat!(
        "import type { X } from \"type-fest\";\n",
        "import { t } from \"@trpc/server\";\n",
    );
    assert_eq!(
        external_dependencies("ts", text),
        ["type-fest", "@trpc/server"]
    );
}

#[test]
fn test_external_imports_degrades_to_empty_on_a_parse_error() {
    assert!(external_dependencies("js", "import broken from ;").is_empty());
}

#[test]
fn test_external_imports_ignores_an_import_statement_without_a_string_source() {
    assert!(external_dependencies("js", "import x from 1;\n").is_empty());
}

#[test]
fn rust_additive_sourceless_export_never_becomes_a_dependency() {
    assert!(external_dependencies("js", "export const X = 5;\n").is_empty());
    assert_eq!(
        external_dependencies("js", "import chalk from 'chalk';\nexport const X = 5;\n",),
        ["chalk"]
    );
}