//! Granular public-API port of all 24 Python `tests/test_metawriter.py` contracts from
//! `main@206f9ef`.
//!
//! The Python module deliberately mixes end-to-end metadata rewrites with two private scanner
//! probes and two `ParamDecl.from_block_dict` coercion probes. Rust's architectural twins are the
//! public `write_uv_metadata` / `managed_params` boundary and `ParamDecl::from_block_map`. No test
//! here is weakened to match the current Rust implementation; a mismatch is a parity finding.

use std::collections::BTreeMap;

use serde_json::{Value as JsonValue, json};
use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterType, ParameterValue};
use skit_language::{
    has_uv_metadata_block, managed_params, read_uv_metadata, source_is_valid, write_managed_params,
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

fn added_lines<'a>(before: &'a str, after: &'a str) -> Vec<&'a str> {
    let original = before.split_inclusive(['\r', '\n']).collect::<Vec<_>>();
    after
        .split_inclusive(['\r', '\n'])
        .filter(|line| !original.contains(line))
        .collect()
}

fn hand_edited(deps_block: &str) -> String {
    format!(
        concat!(
            "# /// script\n",
            "{}",
            "#\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"API_KEY\"\n",
            "# ///\n",
        ),
        deps_block
    )
}

fn assert_update_keeps_api_key(source: &str, dependency: &str) {
    assert_eq!(
        managed_params("python", source)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY"],
        "fixture managed row did not parse: {source}"
    );
    let updated = write_uv_metadata(source, &[dependency.to_owned()], "").unwrap();
    assert_eq!(
        read_uv_metadata(&updated).unwrap().dependencies,
        [dependency]
    );
    assert_eq!(
        managed_params("python", &updated)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY"],
        "dependency rewrite swallowed the managed section: {updated}"
    );
    assert!(source_is_valid("python", &updated), "{updated}");
}

#[test]
fn test_write_creates_block_when_missing() {
    let source = "print('hi')\n";
    let output = write_managed_params("python", source, &params()).unwrap();
    assert!(has_uv_metadata_block(&output));
    assert!(output.contains("print('hi')"));
    assert_eq!(
        managed_params("python", &output)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY", "RETRIES", "input-1"]
    );
}

#[test]
fn test_write_creates_block_adds_no_line_outside_the_block() {
    let source = "CITY = 'Taipei'\nprint(CITY)\n";
    let output = write_managed_params("python", source, &params()[..1]).unwrap();
    let added = added_lines(source, &output);
    assert!(
        added.iter().all(|line| line.trim_start().starts_with('#')),
        "managed metadata added a non-comment line: {added:?}"
    );
    assert!(output.ends_with("# ///\nCITY = 'Taipei'\nprint(CITY)\n"));
}

#[test]
fn test_write_creates_block_after_shebang_adds_no_line_outside_the_block() {
    let source = "#!/usr/bin/env python3\nCITY = 'Taipei'\nprint(CITY)\n";
    let output = write_managed_params("python", source, &params()[..1]).unwrap();
    let added = added_lines(source, &output);
    assert!(
        added.iter().all(|line| line.trim_start().starts_with('#')),
        "managed metadata added a non-comment line: {added:?}"
    );
    assert!(output.starts_with("#!/usr/bin/env python3\n"));
    assert!(output.ends_with("# ///\nCITY = 'Taipei'\nprint(CITY)\n"));
}

#[test]
fn test_write_creates_block_preserves_a_pre_existing_leading_blank_line() {
    let source = "\nprint(1)\n";
    let output = write_managed_params("python", source, &params()[..1]).unwrap();
    assert_eq!(
        managed_params("python", &output)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY"]
    );
    assert!(output.ends_with("# ///\n\nprint(1)\n"));
    assert!(!output.contains("# ///\n\n\nprint(1)\n"));
    let added = added_lines(source, &output);
    assert!(
        added.iter().all(|line| line.trim_start().starts_with('#')),
        "managed metadata changed user body lines: {added:?}"
    );
}

#[test]
fn test_roundtrip_types_and_fields() {
    let output = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let got = managed_params("python", &output)
        .into_iter()
        .map(|parameter| (parameter.name.clone(), parameter))
        .collect::<BTreeMap<_, _>>();
    assert!(got["API_KEY"].secret);
    assert_eq!(
        got["API_KEY"].default,
        Some(ParameterValue::String("abc".to_owned()))
    );
    assert_eq!(got["RETRIES"].default, Some(ParameterValue::Integer(3)));
    assert_eq!(got["RETRIES"].parameter_type, ParameterType::Int);
    assert_eq!(got["input-1"].order, 0);
    assert_eq!(got["input-1"].prompt, "City: ");
}

#[test]
fn test_preserves_existing_dependencies() {
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
    let metadata = read_uv_metadata(&output).unwrap();
    assert_eq!(metadata.dependencies, ["requests"]);
    assert_eq!(metadata.requires_python, ">=3.11");
    assert_eq!(managed_params("python", &output).len(), 3);
}

#[test]
fn test_rewrite_replaces_not_duplicates() {
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
fn test_empty_params_removes_section() {
    let first = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let second = write_managed_params("python", &first, &[]).unwrap();
    assert!(managed_params("python", &second).is_empty());
    assert!(!second.contains("[tool.skit]"));
    assert!(has_uv_metadata_block(&second));
}

#[test]
fn test_string_escaping() {
    let mut message = ParamDecl::new("MSG");
    message.binding = ParameterBinding::Const;
    message.default = Some(ParameterValue::String("say \"hi\" \\ bye".to_owned()));
    let output = write_managed_params("python", "x = 1\n", &[message]).unwrap();
    let got = managed_params("python", &output);
    assert_eq!(
        got[0].default,
        Some(ParameterValue::String("say \"hi\" \\ bye".to_owned()))
    );
}

#[test]
fn test_shebang_preserved_first() {
    let source = "#!/usr/bin/env python3\nprint('x')\n";
    let output = write_managed_params("python", source, &params()[..1]).unwrap();
    assert!(output.starts_with("#!/usr/bin/env python3\n"));
}

#[test]
fn test_script_still_valid_python() {
    let source = "CITY = 'Taipei'\nprint(CITY)\n";
    let output = write_managed_params("python", source, &params()).unwrap();
    assert!(
        source_is_valid("python", &output),
        "managed metadata made valid Python invalid: {output}"
    );
}

#[test]
fn test_set_dependencies_preserves_tool_skit() {
    let with_params = write_managed_params("python", "x = 1\n", &params()).unwrap();
    let updated = write_uv_metadata(
        &with_params,
        &["requests".to_owned(), "rich".to_owned()],
        ">=3.12",
    )
    .unwrap();
    let metadata = read_uv_metadata(&updated).unwrap();
    assert_eq!(metadata.dependencies, ["requests", "rich"]);
    assert_eq!(metadata.requires_python, ">=3.12");
    assert_eq!(
        managed_params("python", &updated)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY", "RETRIES", "input-1"]
    );

    let cleared = write_uv_metadata(&updated, &[], "").unwrap();
    assert_eq!(
        read_uv_metadata(&cleared).unwrap().dependencies,
        Vec::<String>::new()
    );
    assert_eq!(managed_params("python", &cleared).len(), 3);
    assert!(source_is_valid("python", &cleared), "{cleared}");
}

#[test]
fn test_set_dependencies_without_block_injects() {
    let output = write_uv_metadata("print('x')\n", &["httpx".to_owned()], "").unwrap();
    let metadata = read_uv_metadata(&output).expect("dependency edit must inject a PEP 723 block");
    assert_eq!(metadata.dependencies, ["httpx"]);
}

#[test]
fn test_set_dependencies_survives_hand_edited_deps_closer() {
    for deps_block in [
        "# dependencies = [\n#     \"requests\"]\n",
        "# dependencies = [\n#     \"requests\",\n# ]  # pin\n",
        "# dependencies = [\n#     \"a\",\n#     \"b\"]\n",
        "# dependencies = [\n#     \"pkg[extra]\",\n# ]\n",
        "# dependencies = [\n#     \"requests\",  # pin later [\n#     \"httpx\",\n# ]\n",
        "# dependencies = [\n#     \"foo]bar\",\n# ]\n",
    ] {
        assert_update_keeps_api_key(&hand_edited(deps_block), "httpx");
    }
}

#[test]
fn test_set_dependencies_handles_unbalanced_bracket_in_inline_comment() {
    let source = hand_edited(concat!(
        "# dependencies = [\n",
        "#     \"requests\",  # pin later [\n",
        "#     \"httpx\",\n",
        "# ]\n",
    ));
    assert_update_keeps_api_key(&source, "rich");
}

#[test]
fn test_structural_bracket_delta_escaped_quote_in_basic_string() {
    // Python probes the private scanner with `\"a\\\"]b\"`. Put that exact structural hazard in a
    // valid TOML dependency array, followed by a managed section. If an escaped quote incorrectly
    // ends the string, the `]` inside the string desynchronizes the array scanner and the subsequent
    // managed section is the observable casualty.
    let source = hand_edited(concat!(
        "# dependencies = [\n",
        "#     \"a\\\"]b\",\n",
        "# ]\n",
    ));
    assert_update_keeps_api_key(&source, "rich");
}

#[test]
fn test_structural_bracket_delta_literal_string_has_no_escapes() {
    // TOML literal strings do not treat backslash as an escape. The bracket remains inside the
    // string and therefore cannot close the dependency array scanner.
    let source = hand_edited(concat!("# dependencies = [\n", "#     'a\\]b',\n", "# ]\n",));
    assert_update_keeps_api_key(&source, "rich");
}

#[test]
fn test_write_params_preserves_blank_lines_after_block() {
    let source = "# /// script\n# dependencies = []\n# ///\n\n\nimport requests\n";
    let output = write_managed_params("python", source, &params()[..1]).unwrap();
    assert_eq!(
        output.split_once("# ///\n").unwrap().1,
        "\n\nimport requests\n"
    );
}

#[test]
fn test_set_dependencies_preserves_blank_lines_after_block() {
    let source = "# /// script\n# dependencies = []\n# ///\n\n\nimport requests\n";
    let output = write_uv_metadata(source, &["httpx".to_owned()], "").unwrap();
    assert_eq!(
        output.split_once("# ///\n").unwrap().1,
        "\n\nimport requests\n"
    );
}

#[test]
fn test_read_params_tolerates_malformed_container_shapes() {
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
fn test_read_params_tolerates_non_numeric_order() {
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
    let got = managed_params("python", source);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "X");
    assert_eq!(got[0].order, -1);
}

#[test]
fn test_from_dict_coerces_non_numeric_order() {
    for order in [JsonValue::String("abc".to_owned()), JsonValue::Null] {
        let input = BTreeMap::from([
            ("name".to_owned(), JsonValue::String("X".to_owned())),
            ("order".to_owned(), order),
        ]);
        assert_eq!(ParamDecl::from_block_map(&input).order, -1);
    }
}

#[test]
fn test_from_dict_still_coerces_numeric_string_and_float_order() {
    let numeric_string = BTreeMap::from([
        ("name".to_owned(), json!("X")),
        ("order".to_owned(), json!("3")),
    ]);
    assert_eq!(ParamDecl::from_block_map(&numeric_string).order, 3);

    let float = BTreeMap::from([
        ("name".to_owned(), json!("X")),
        ("order".to_owned(), json!(1.9)),
    ]);
    // Python's `int(1.9)` contract is deliberate backward compatibility. Current Rust's integer
    // reader rejects JSON floats and returns -1; keep this red until implementation parity is fixed.
    assert_eq!(ParamDecl::from_block_map(&float).order, 1);
}

#[test]
fn test_write_params_survives_unicode_line_separators_in_prompt() {
    for separator in ['\u{0085}', '\u{2028}', '\u{2029}'] {
        let mut declaration = ParamDecl::new("CITY");
        declaration.binding = ParameterBinding::Const;
        declaration.default = Some(ParameterValue::String("x".to_owned()));
        declaration.prompt = format!("a{separator}b");

        let output = write_managed_params("python", "CITY = 'x'\n", &[declaration]).unwrap();
        let back = managed_params("python", &output);
        assert_eq!(
            back.len(),
            1,
            "block lost for separator {separator:?}: {output}"
        );
        assert_eq!(back[0].prompt, format!("a{separator}b"));
    }
}
