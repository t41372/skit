//! Exact completeness guard for Python `tests/test_add_review_contracts.py` at `main@206f9ef`.

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("../../skit-tui/tests/port_test_add_review_contracts.rs");
const PYTHON_TESTS: &[&str] = &[
    "test_high_unmodeled_self_parser_writes_ticked_candidate",
    "test_high_modeled_form_collects_nothing_without_crashing",
    "test_prompt_draft_with_shebang_body_resumes_into_prompt_review",
    "test_reference_note_modeled_keeps_wrap_and_short_line",
    "test_reference_note_unmodeled_folds_and_keeps_old_line",
    "test_kind_pick_modal_label_switches_on_shebang",
    "test_review_names_extra_arguments_field_once",
    "test_rv_python_typed_constraint_lands_in_stored_copy",
    "test_rv_python_empty_means_automatic",
    "test_rv_python_typed_value_survives_an_edit_rescan",
    "test_resumed_draft_has_no_storage_section",
    "test_short_terminal_scrolls_focused_candidate_into_view",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_python_add_review_contract_has_one_same_named_executable_rust_oracle_in_order() {
    let actual = syn::parse_file(SOURCE)
        .expect("add-review parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(PYTHON_TESTS.len(), 12);
    assert_eq!(actual, expected);
}
