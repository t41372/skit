//! Public-API ports of Python v0.4 managed-metadata fidelity contracts.
//!
//! Parameter management is a comment-only source edit. Existing PEP 723 dependency fields, user
//! code bytes around the block, shebang placement, and unknown metadata must survive independently
//! of the managed `[tool.skit]` projection.

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterType, ParameterValue};
use skit_language::{
    has_uv_metadata_block, managed_params, read_uv_metadata, write_managed_params,
    write_uv_metadata,
};

fn params() -> Vec<ParamDecl> {
    let mut key = ParamDecl::new("API_KEY");
    key.binding = ParameterBinding::Const;
    key.parameter_type = ParameterType::Str;
    key.default = Some(ParameterValue::String("abc".to_owned()));
    key.secret = true;

    let mut retries = ParamDecl::new("RETRIES");
    retries.binding = ParameterBinding::Const;
    retries.parameter_type = ParameterType::Int;
    retries.default = Some(ParameterValue::Integer(3));

    let mut input = ParamDecl::new("input-1");
    input.binding = ParameterBinding::Input;
    input.parameter_type = ParameterType::Str;
    input.prompt = "City: ".to_owned();
    input.order = 0;

    vec![key, retries, input]
}

#[test]
fn test_write_managed_params_creates_a_block_without_touching_user_code() {
    let source = "print('hi')\n";
    let output = write_managed_params("python", source, &params()).unwrap();

    assert!(has_uv_metadata_block(&output));
    assert!(output.ends_with("# ///\nprint('hi')\n"), "{output}");
    assert_eq!(
        managed_params("python", &output)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY", "RETRIES", "input-1"]
    );
}

#[test]
fn test_write_managed_params_adds_no_non_comment_separator_outside_a_new_block() {
    let source = "CITY = 'Taipei'\nprint(CITY)\n";
    let output = write_managed_params("python", source, &params()[..1]).unwrap();

    assert!(
        output.ends_with("# ///\nCITY = 'Taipei'\nprint(CITY)\n"),
        "{output}"
    );
    assert!(!output.contains("# ///\n\nCITY = 'Taipei'"), "{output}");
}

#[test]
fn test_write_managed_params_preserves_shebang_first_and_leading_blank_body() {
    let shebang = "#!/usr/bin/env python3\nCITY = 'Taipei'\nprint(CITY)\n";
    let output = write_managed_params("python", shebang, &params()[..1]).unwrap();
    assert!(
        output.starts_with("#!/usr/bin/env python3\n# /// script\n"),
        "{output}"
    );
    assert!(output.ends_with("# ///\nCITY = 'Taipei'\nprint(CITY)\n"));

    let leading_blank = "\nprint(1)\n";
    let output = write_managed_params("python", leading_blank, &params()[..1]).unwrap();
    assert!(output.ends_with("# ///\n\nprint(1)\n"), "{output}");
    assert!(!output.ends_with("# ///\n\n\nprint(1)\n"));
}

#[test]
fn test_managed_param_roundtrip_preserves_types_defaults_secret_prompt_and_order() {
    let output = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let decoded = managed_params("python", &output)
        .into_iter()
        .map(|parameter| (parameter.name.clone(), parameter))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert!(decoded["API_KEY"].secret);
    assert_eq!(
        decoded["API_KEY"].default,
        Some(ParameterValue::String("abc".to_owned()))
    );
    assert_eq!(decoded["RETRIES"].parameter_type, ParameterType::Int);
    assert_eq!(decoded["RETRIES"].default, Some(ParameterValue::Integer(3)));
    assert_eq!(decoded["input-1"].order, 0);
    assert_eq!(decoded["input-1"].prompt, "City: ");
}

#[test]
fn test_write_managed_params_preserves_existing_dependencies_and_python_constraint() {
    let source = concat!(
        "# /// script\n",
        "# requires-python = \">=3.11\"\n",
        "# dependencies = [\n",
        "#     \"requests\",\n",
        "# ]\n",
        "# ///\n",
        "import requests\n",
    );
    let output = write_managed_params("python", source, &params()).unwrap();
    let uv = read_uv_metadata(&output).unwrap();

    assert_eq!(uv.dependencies, ["requests"]);
    assert_eq!(uv.requires_python, ">=3.11");
    assert_eq!(managed_params("python", &output).len(), 3);
}

#[test]
fn test_rewrite_replaces_managed_section_instead_of_duplicating_it() {
    let first = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let second = write_managed_params("python", &first, &params()[..1]).unwrap();

    assert_eq!(
        managed_params("python", &second)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY"]
    );
    assert_eq!(second.matches("[tool.skit]").count(), 1);
}

#[test]
fn test_empty_managed_params_remove_only_skit_section_but_leave_metadata_block() {
    let with_params = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let cleared = write_managed_params("python", &with_params, &[]).unwrap();

    assert!(managed_params("python", &cleared).is_empty());
    assert!(!cleared.contains("[tool.skit]"));
    assert!(has_uv_metadata_block(&cleared));
}

#[test]
fn test_managed_string_default_roundtrips_quotes_and_backslashes() {
    let mut message = ParamDecl::new("MSG");
    message.binding = ParameterBinding::Const;
    message.default = Some(ParameterValue::String("say \"hi\" \\ bye".to_owned()));
    let output = write_managed_params("python", "x = 1\n", &[message]).unwrap();
    let decoded = managed_params("python", &output);

    assert_eq!(
        decoded[0].default,
        Some(ParameterValue::String("say \"hi\" \\ bye".to_owned()))
    );
}

#[test]
fn test_write_uv_metadata_preserves_managed_parameter_rows() {
    let with_params = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let updated = write_uv_metadata(
        &with_params,
        &["requests".to_owned(), "rich".to_owned()],
        ">=3.12",
    )
    .unwrap();
    let uv = read_uv_metadata(&updated).unwrap();
    assert_eq!(uv.dependencies, ["requests", "rich"]);
    assert_eq!(uv.requires_python, ">=3.12");
    assert_eq!(managed_params("python", &updated).len(), 3);

    let cleared = write_uv_metadata(&updated, &[], "").unwrap();
    assert_eq!(
        read_uv_metadata(&cleared).unwrap().dependencies,
        Vec::<String>::new()
    );
    assert_eq!(managed_params("python", &cleared).len(), 3);
}

#[test]
fn test_write_uv_metadata_without_a_block_injects_one() {
    let output = write_uv_metadata("print('x')\n", &["httpx".to_owned()], "").unwrap();
    let uv = read_uv_metadata(&output).unwrap();
    assert_eq!(uv.dependencies, ["httpx"]);
}

#[test]
fn test_managed_rewrite_preserves_blank_lines_after_existing_block() {
    let source = "# /// script\n# dependencies = []\n# ///\n\n\nimport requests\n";
    let output = write_managed_params("python", source, &params()[..1]).unwrap();
    let suffix = output.split_once("# ///\n").unwrap().1;
    assert_eq!(suffix, "\n\nimport requests\n");
}

#[test]
fn test_managed_params_reader_is_total_for_malformed_but_valid_container_shapes() {
    for body in [
        "# dependencies = []\n# tool = 5\n",
        "# dependencies = []\n# [tool]\n# skit = 5\n",
        "# dependencies = []\n# [tool.skit]\n# params = 5\n",
    ] {
        let source = format!("# /// script\n{body}# ///\n");
        assert!(managed_params("python", &source).is_empty(), "{source}");
    }
}

#[test]
fn test_managed_params_reader_tolerates_non_numeric_order() {
    let source = concat!(
        "# /// script\n",
        "# dependencies = []\n",
        "#\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"X\"\n",
        "# order = \"abc\"\n",
        "# ///\n",
    );
    let decoded = managed_params("python", source);
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].name, "X");
    assert_eq!(decoded[0].order, -1);
}
