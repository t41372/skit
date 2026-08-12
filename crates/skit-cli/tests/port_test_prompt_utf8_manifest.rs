//! Exact completeness guard for Python `tests/test_prompt_utf8.py` at `main@206f9ef`.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const STORE: &str = include_str!("../../skit-store/tests/port_test_prompt_utf8.rs");
const CLI: &str = include_str!("port_test_prompt_utf8.rs");
const TUI: &str = include_str!("../../skit-tui/tests/port_test_prompt_utf8.rs");
const SETTINGS: &str = include_str!("port_test_prompt_utf8_settings.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_store_rejects_invalid_utf8_prompt_and_reports_byte_offset",
    "test_store_accepts_valid_utf8_prompt_byte_exact",
    "test_store_add_prompt_copies_the_validated_snapshot_not_a_second_read",
    "test_store_add_prompt_reference_hash_tracks_the_validated_snapshot",
    "test_prompt_snapshot_read_error_is_not_ambiguous",
    "test_prompt_copy_keeps_the_snapshot_permissions",
    "test_generic_store_api_also_refuses_invalid_prompt_utf8",
    "test_stdin_prompt_cli_rejects_invalid_utf8_with_real_byte_offset",
    "test_stdin_prompt_inprocess_rejects_invalid_utf8_with_real_byte_offset",
    "test_runtime_prompt_payload_seam_refuses_invalid_bytes",
    "test_changed_prompt_is_rejected_by_launch_and_health_with_byte_offset",
    "test_cli_edit_invalid_prompt_refuses_then_reedit_recovers_without_replacement",
    "test_library_edit_preserves_valid_prompt_utf8_and_refreshes_placeholders",
    "test_cli_add_params_run_doctor_share_the_strict_prompt_contract",
    "test_tui_initial_add_review_rejects_invalid_prompt_without_replacement_character",
    "test_tui_review_rescan_and_settings_reject_invalid_prompt_without_replacement_character",
];

const SETTINGS_COMPANION: &str =
    "prompt_utf8_settings_surface_rejects_invalid_bytes_without_replacement_character";

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("prompt UTF-8 parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn all_sixteen_python_prompt_utf8_contracts_are_executable_once() {
    let actual = tests(STORE)
        .into_iter()
        .chain(tests(CLI))
        .chain(tests(TUI))
        .collect::<Vec<_>>();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(PYTHON_TESTS.len(), 16);
    assert_eq!(
        actual.len(),
        16,
        "unexpected extra or missing prompt UTF-8 tests"
    );
    assert_eq!(
        actual_set.len(),
        actual.len(),
        "duplicate names hide a missing contract"
    );
    assert_eq!(actual_set, expected);
}

#[test]
fn the_combined_review_rescan_and_settings_contract_keeps_its_real_settings_half() {
    let settings_tests = tests(SETTINGS);
    assert_eq!(settings_tests, [SETTINGS_COMPANION.to_owned()]);
}
