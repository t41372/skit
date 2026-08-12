//! Exact completeness guard for Python `tests/test_rename.py` at `main@206f9ef`.

use syn::{Attribute, Item};

const STORE_SOURCE: &str = include_str!("../../skit-store/tests/port_test_rename.rs");
const TUI_SOURCE: &str = include_str!("port_test_rename_tui.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_rename_changes_name_and_keeps_slug_dir_and_state",
    "test_rename_updates_resolution_and_listing",
    "test_rename_conflict_is_a_clean_error",
    "test_rename_to_own_name_is_a_no_op",
    "test_rename_empty_name_rejected",
    "test_rename_survives_doctor_rebuild",
    "test_settings_screen_renames_on_save",
    "test_settings_screen_rename_conflict_stays_open",
    "test_settings_hides_manage_checkboxes_for_argparse_script",
    "test_settings_save_keeps_argparse_source",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("rename parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn every_python_rename_contract_has_one_same_named_executable_rust_oracle_in_order() {
    let actual = tests(STORE_SOURCE)
        .into_iter()
        .chain(tests(TUI_SOURCE))
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(PYTHON_TESTS.len(), 10);
    assert_eq!(actual, expected);
}
