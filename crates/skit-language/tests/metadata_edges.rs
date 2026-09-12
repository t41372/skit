//! Malformed or unusual stored metadata must refuse or degrade without losing user bytes.

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    LanguageError, external_dependencies, managed_params, placeholder_params, python_version_pin,
    render_prompt_body, write_managed_params,
};

fn declaration(name: &str) -> ParamDecl {
    ParamDecl {
        name: name.to_owned(),
        binding: ParameterBinding::Const,
        delivery: ParameterDelivery::Inject,
        parameter_type: ParameterType::Str,
        default: None,
        required: false,
        multiple: false,
        repeat: false,
        choices: Vec::new(),
        prompt: String::new(),
        help: String::new(),
        secret: false,
        env_source: String::new(),
        flag: String::new(),
        action: String::new(),
        order: -1,
        env_target: String::new(),
        degraded: false,
    }
}

fn python_block(body: &str) -> String {
    format!("# /// script\n{body}# ///\nprint(1)\n")
}

#[test]
fn a_versioned_python_program_needs_digits_in_every_part() {
    assert_eq!(
        python_version_pin("python3.12"),
        Some(">=3.12,<3.13".to_owned())
    );
    assert_eq!(
        python_version_pin("python3.12.4"),
        Some(">=3.12.4,<3.13".to_owned())
    );
    // A trailing separator or a non-digit micro part is not a version.
    assert_eq!(python_version_pin("python3.12."), None);
    assert_eq!(python_version_pin("python3.12.x"), None);
    assert_eq!(python_version_pin("python3."), None);
    assert_eq!(python_version_pin("python3.x"), None);
    assert_eq!(python_version_pin("python3"), None);
}

#[test]
fn a_metadata_block_that_is_not_valid_toml_refuses_the_write() {
    let source = python_block("# name = \n");

    let error = write_managed_params("python", &source, &[declaration("target")]).unwrap_err();

    assert!(matches!(error, LanguageError::InvalidMetadata { .. }));
    assert!(error.to_string().contains("not valid TOML"));
}

#[test]
fn a_tool_table_with_the_wrong_shape_refuses_the_write() {
    for (body, detail) in [
        ("# tool = \"text\"\n", "tool is not a table"),
        ("# [tool]\n# skit = \"text\"\n", "tool.skit is not a table"),
    ] {
        let source = python_block(body);
        let error = write_managed_params("python", &source, &[declaration("target")]).unwrap_err();
        assert!(error.to_string().contains(detail), "{}", error);
    }
}

#[test]
fn an_unrelated_tool_section_keeps_its_rows_and_gains_a_skit_section() {
    let source = python_block("# [tool.ruff]\n# line-length = 100\n");

    let updated = write_managed_params("python", &source, &[declaration("target")]).unwrap();

    assert!(updated.contains("line-length = 100"));
    assert!(updated.contains("[tool.skit]"));
    assert_eq!(managed_params("python", &updated).len(), 1);
}

#[test]
fn existing_parameter_rows_without_a_name_are_kept_as_written() {
    // A hand-written block can hold rows skit cannot address. Those bytes must survive.
    let source = python_block(
        "# [tool.skit]\n# schema = 1\n# params = [ \"free text\", { note = \"anonymous\" } ]\n",
    );

    let updated = write_managed_params("python", &source, &[declaration("target")]).unwrap();

    assert!(updated.contains("free text"));
    assert!(updated.contains("anonymous"));
    assert_eq!(managed_params("python", &updated).len(), 1);
}

#[test]
fn a_float_default_round_trips_through_the_declaration_block() {
    let mut parameter = declaration("ratio");
    parameter.parameter_type = ParameterType::Float;
    parameter.default = Some(ParameterValue::Float(0.25));

    let updated = write_managed_params("python", &python_block(""), &[parameter]).unwrap();

    assert!(updated.contains("0.25"));
    assert_eq!(
        managed_params("python", &updated)[0].default,
        Some(ParameterValue::Float(0.25))
    );
}

#[test]
fn an_unterminated_placeholder_ends_the_scan_without_a_panic() {
    let text = "Review {{subject}} and {{unterminated";

    let names = placeholder_params("prompt", text)
        .into_iter()
        .map(|item| item.name)
        .collect::<Vec<_>>();

    assert_eq!(names, ["subject"]);
    assert_eq!(
        render_prompt_body(text, &std::collections::BTreeMap::new(), true),
        "Review {{subject}} and {{unterminated"
    );
}

#[test]
fn a_source_that_the_parser_refuses_reports_no_dependencies() {
    // Broken syntax must not guess an import list.
    assert!(external_dependencies("python", "def (:\n").is_empty());
    assert!(external_dependencies("js", "import from;\nfunction (").is_empty());
    assert!(external_dependencies("ts", "class {{{").is_empty());
}

#[test]
fn python_aliased_imports_report_their_real_package() {
    let names = external_dependencies("python", "import numpy as np\nimport os.path as p\n");

    assert_eq!(names, ["numpy"]);
}

#[test]
fn javascript_dynamic_calls_need_exactly_one_string_argument() {
    let text = concat!(
        "const a = require('chalk');\n",
        "const b = require();\n",
        "const c = require('zod', 'extra');\n",
        "const d = require(name);\n",
        "const e = import(`template`);\n",
    );

    assert_eq!(external_dependencies("js", text), ["chalk"]);
}

#[test]
fn a_scoped_specifier_needs_both_a_scope_and_a_name() {
    let text = concat!(
        "import a from '@scope/tool';\n",
        "import b from '@/alias';\n",
        "import c from '@';\n",
        "import d from '@scope/';\n",
        "import e from '@scope';\n",
    );

    assert_eq!(external_dependencies("js", text), ["@scope/tool"]);
}

#[test]
fn an_injected_boolean_uses_the_spelling_of_the_target_language() {
    use skit_language::inject_values;
    use std::collections::BTreeMap;

    let mut flag = declaration("FLAG");
    flag.parameter_type = ParameterType::Bool;
    flag.binding = ParameterBinding::Const;

    // The stored constant is an int literal, so its spelling cannot pick the Boolean form.
    let python = "FLAG = 1\nprint(FLAG)\n";
    for (value, expected) in [("false", "FLAG = False"), ("true", "FLAG = True")] {
        let values = BTreeMap::from([("FLAG".to_owned(), value.to_owned())]);
        let rewritten = inject_values("python", python, &[flag.clone()], &values).unwrap();
        assert!(rewritten.starts_with(expected), "{rewritten}");
        assert!(
            skit_language::source_is_valid("python", &rewritten),
            "{rewritten}"
        );
    }

    let javascript = "const FLAG = 1;\nconsole.log(FLAG);\n";
    for (value, expected) in [
        ("false", "const FLAG = false;"),
        ("true", "const FLAG = true;"),
    ] {
        let values = BTreeMap::from([("FLAG".to_owned(), value.to_owned())]);
        let rewritten = inject_values("js", javascript, &[flag.clone()], &values).unwrap();
        assert!(rewritten.starts_with(expected), "{rewritten}");
        assert!(
            skit_language::source_is_valid("js", &rewritten),
            "{rewritten}"
        );
    }
}

#[test]
fn an_unknown_tool_skit_sub_table_survives_a_source_operation() {
    // v0.4 data can carry a `[tool.skit.<name>]` table skit does not own.
    let source = python_block("# [tool.skit.custom]\n# keep = \"me\"\n");

    let updated = write_managed_params("python", &source, &[declaration("target")]).unwrap();

    assert!(updated.contains("keep = \"me\""), "{updated}");
    assert_eq!(managed_params("python", &updated).len(), 1);
    // The block must still be readable, which a duplicated table header would prevent.
    let again = write_managed_params("python", &updated, &[declaration("target")]).unwrap();
    assert_eq!(managed_params("python", &again).len(), 1);
}
