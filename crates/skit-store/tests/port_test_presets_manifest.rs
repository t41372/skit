//! Mechanical completeness guard for Python v0.4 `tests/test_presets.py`.
//!
//! This is not behavioral coverage. `port_test_presets.rs` owns the behavioral assertions; this
//! guard only prevents a frozen Python oracle from silently disappearing or being renamed.

const SOURCE: &str = include_str!("port_test_presets.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_preset_roundtrip",
    "test_resolution_order_preset_over_last_over_default",
    "test_c3_secret_never_touches_disk",
    "test_preset_preserved_across_save_last",
    "test_purge_secret_removes_from_values_and_every_preset",
    "test_purge_secret_drops_a_preset_left_empty_but_keeps_others",
    "test_purge_secret_empty_names_is_noop",
    "test_purge_secret_reports_only_names_actually_stored",
    "test_save_last_drops_stale_value_once_param_becomes_secret",
    "test_save_last_values_are_a_snapshot_not_a_merge",
    "test_save_last_none_values_still_scrubs_stale_secret",
    "test_save_last_regression_non_secret_values_persist_normally",
];

#[test]
fn every_python_preset_test_has_the_same_named_executable_rust_oracle_in_order() {
    assert_eq!(PYTHON_TESTS.len(), 12);
    assert_eq!(
        SOURCE.matches("#[test]").count(),
        PYTHON_TESTS.len(),
        "the behavioral file must contain exactly the frozen Python test count"
    );

    let mut cursor = 0;
    for name in PYTHON_TESTS {
        let needle = format!("fn {name}()");
        let offset = SOURCE[cursor..]
            .find(&needle)
            .unwrap_or_else(|| panic!("missing executable Rust oracle {name}"));
        cursor += offset + needle.len();
    }
}
