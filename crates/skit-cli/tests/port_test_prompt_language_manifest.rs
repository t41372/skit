//! Exact-name/order guard for the public language slice of Python v0.4 `tests/test_prompt_kind.py`.
//!
//! This is intentionally **not** a full-module manifest: runner argv, argv limits, registry/store,
//! launch, CLI, and TUI contracts live at other public boundaries. The 14 names below are only the
//! prompt grammar/render/kind-inference contracts executable through `skit-language`.

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("../../skit-language/tests/port_test_prompt.rs");

const PYTHON_LANGUAGE_TESTS: &[&str] = &[
    "test_placeholder_names_dedupes_in_body_order",
    "test_placeholder_names_single_braces_are_never_candidates",
    "test_placeholder_names_brace_adjacent_is_not_a_candidate",
    "test_placeholder_names_reserved_name_excluded",
    "test_placeholder_names_accept_unicode_identifiers_and_reject_non_names",
    "test_placeholder_names_high_cardinality_stays_ordered_and_complete",
    "test_prompt_grammar_is_independent_of_command_templates",
    "test_corpus_basic_detection_and_render_byte_identity",
    "test_corpus_crlf_preserved_verbatim",
    "test_corpus_cjk_emoji_no_trailing_newline",
    "test_corpus_reserved_prompt_stays_verbatim",
    "test_render_body_substitutes_raw_never_quotes",
    "test_render_body_empty_value_substitutes_empty",
    "test_infer_kind_compound_suffix",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn prompt_language_slice_has_all_14_python_oracles_in_exact_order() {
    let file = syn::parse_file(SOURCE).expect("prompt parity source must parse as Rust");
    let actual = file
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function)
                if has_test_attribute(&function.attrs)
                    && !function.sig.ident.to_string().starts_with("rust_additive_") =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = PYTHON_LANGUAGE_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
    assert_eq!(PYTHON_LANGUAGE_TESTS.len(), 14);
}
