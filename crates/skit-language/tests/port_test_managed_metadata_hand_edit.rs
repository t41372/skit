//! Public-API ports of Python v0.4 hand-edited PEP 723 regressions.
//!
//! These intentionally do not assume skit's preferred formatter. A user may close a dependency
//! array on the last item, add comments containing brackets, or use requirement strings containing
//! brackets. Updating dependency fields must still preserve the managed `[tool.skit]` section.

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterValue};
use skit_language::{managed_params, read_uv_metadata, source_is_valid, write_managed_params, write_uv_metadata};

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
            "print('ok')\n",
        ),
        deps_block
    )
}

fn assert_dependency_update_preserves_params(source: &str, dependency: &str) {
    assert_eq!(
        managed_params("python", source)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY"]
    );

    let updated = write_uv_metadata(source, &[dependency.to_owned()], "").unwrap();
    assert_eq!(
        read_uv_metadata(&updated).unwrap().dependencies,
        [dependency.to_owned()]
    );
    assert_eq!(
        managed_params("python", &updated)
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["API_KEY"]
    );
    assert!(source_is_valid("python", &updated), "{updated}");
}

#[test]
fn test_dependency_update_survives_hand_edited_array_closers_comments_and_brackets() {
    for deps_block in [
        "# dependencies = [\n#     \"requests\"]\n",
        "# dependencies = [\n#     \"requests\",\n# ]  # pin\n",
        "# dependencies = [\n#     \"a\",\n#     \"b\"]\n",
        "# dependencies = [\n#     \"pkg[extra]\",\n# ]\n",
        "# dependencies = [\n#     \"requests\",  # pin later [\n#     \"httpx\",\n# ]\n",
        "# dependencies = [\n#     \"foo]bar\",\n# ]\n",
    ] {
        let source = hand_edited(deps_block);
        assert_dependency_update_preserves_params(&source, "httpx");
    }
}

#[test]
fn test_dependency_update_ignores_unbalanced_bracket_inside_inline_comment() {
    let source = hand_edited(concat!(
        "# dependencies = [\n",
        "#     \"requests\",  # pin later [\n",
        "#     \"httpx\",\n",
        "# ]\n",
    ));
    assert_dependency_update_preserves_params(&source, "rich");
}

#[test]
fn test_dependency_update_preserves_blank_lines_after_the_metadata_block() {
    let source = "# /// script\n# dependencies = []\n# ///\n\n\nimport requests\n";
    let updated = write_uv_metadata(source, &["httpx".to_owned()], "").unwrap();
    let suffix = updated.split_once("# ///\n").unwrap().1;
    assert_eq!(suffix, "\n\nimport requests\n");
}

#[test]
fn test_unicode_line_separators_in_prompt_roundtrip_without_splitting_comment_lines() {
    for separator in ['\u{0085}', '\u{2028}', '\u{2029}'] {
        let mut declaration = ParamDecl::new("CITY");
        declaration.binding = ParameterBinding::Const;
        declaration.default = Some(ParameterValue::String("x".to_owned()));
        declaration.prompt = format!("a{separator}b");

        let output = write_managed_params("python", "CITY = 'x'\n", &[declaration]).unwrap();
        let decoded = managed_params("python", &output);
        assert_eq!(decoded.len(), 1, "separator {separator:?}: {output}");
        assert_eq!(decoded[0].prompt, format!("a{separator}b"));
        assert!(source_is_valid("python", &output));
    }
}

#[test]
fn test_dependency_update_preserves_unknown_tool_skit_fields_while_replacing_dependencies() {
    let source = concat!(
        "# /// script\n",
        "# dependencies = [\"requests\"]\n",
        "#\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "# future = \"keep-me\"\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"API_KEY\"\n",
        "# custom = 7\n",
        "# ///\n",
        "print('ok')\n",
    );

    let updated = write_uv_metadata(source, &["rich".to_owned()], ">=3.12").unwrap();
    assert_eq!(read_uv_metadata(&updated).unwrap().dependencies, ["rich"]);
    assert!(updated.contains("future = \"keep-me\""), "{updated}");
    assert!(updated.contains("custom = 7"), "{updated}");
    assert_eq!(managed_params("python", &updated)[0].name, "API_KEY");
}
