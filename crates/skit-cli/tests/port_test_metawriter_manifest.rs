//! Exact executable completeness guard for Python `tests/test_metawriter.py` at `main@206f9ef`.

use std::{fs, path::Path};

use syn::{Attribute, Item};

const EXPECTED: &[&str] = &[
    "test_write_creates_block_when_missing",
    "test_write_creates_block_adds_no_line_outside_the_block",
    "test_write_creates_block_after_shebang_adds_no_line_outside_the_block",
    "test_write_creates_block_preserves_a_pre_existing_leading_blank_line",
    "test_roundtrip_types_and_fields",
    "test_preserves_existing_dependencies",
    "test_rewrite_replaces_not_duplicates",
    "test_empty_params_removes_section",
    "test_string_escaping",
    "test_shebang_preserved_first",
    "test_script_still_valid_python",
    "test_set_dependencies_preserves_tool_skit",
    "test_set_dependencies_without_block_injects",
    "test_set_dependencies_survives_hand_edited_deps_closer",
    "test_set_dependencies_handles_unbalanced_bracket_in_inline_comment",
    "test_structural_bracket_delta_escaped_quote_in_basic_string",
    "test_structural_bracket_delta_literal_string_has_no_escapes",
    "test_write_params_preserves_blank_lines_after_block",
    "test_set_dependencies_preserves_blank_lines_after_block",
    "test_read_params_tolerates_malformed_container_shapes",
    "test_read_params_tolerates_non_numeric_order",
    "test_from_dict_coerces_non_numeric_order",
    "test_from_dict_still_coerces_numeric_string_and_float_order",
    "test_write_params_survives_unicode_line_separators_in_prompt",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn metawriter_has_exactly_the_24_frozen_python_oracles() {
    assert_eq!(EXPECTED.len(), 24);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let path = repo.join("crates/skit-language/tests/port_test_metawriter_exact.rs");
    let source = fs::read_to_string(&path).unwrap();
    let file = syn::parse_file(&source).expect("metawriter parity target must parse as Rust");
    let actual = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        EXPECTED.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>(),
        "metawriter parity target must be exactly the frozen Python test sequence"
    );
}
