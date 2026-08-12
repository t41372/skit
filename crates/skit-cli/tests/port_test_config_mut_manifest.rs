//! Completeness guard for Python v0.4 `tests/test_config_mut.py`.
//!
//! Five contracts are executable through FileConfigStore's public nested-scalar transaction API.
//! Two contracts assert exact frontend stderr warnings after malformed-config recovery; the store
//! exposes only the typed recovery fact, so those wording contracts stay explicitly blocked rather
//! than being replaced by weaker backup-file assertions.

use std::{fs, path::Path};

const EXECUTABLE: &[&str] = &[
    "test_save_bash_path_clear_tolerates_missing_bash_path_key",
    "test_save_bash_path_clear_tolerates_missing_shell_section",
    "test_save_js_runner_clear_tolerates_missing_runner_key",
    "test_save_js_runner_clear_tolerates_missing_js_section",
    "test_save_js_runner_preserves_sibling_js_keys",
];

const BLOCKED: &[&str] = &[
    "test_corrupt_config_backup_warning_is_verbatim",
    "test_corrupt_config_no_backup_warning_is_verbatim",
];

fn test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function)
                if function
                    .attrs
                    .iter()
                    .any(|attribute| attribute.path().is_ident("test")) =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn config_mutation_coverage_is_five_executable_two_blocked() {
    assert_eq!(EXECUTABLE.len() + BLOCKED.len(), 7);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source =
        fs::read_to_string(repo.join("crates/skit-store/tests/port_test_config_mut_exact.rs"))
            .unwrap();
    assert_eq!(
        test_names(&source),
        EXECUTABLE
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );
    for blocked in BLOCKED {
        assert!(
            !source.contains(&format!("fn {blocked}(")),
            "blocked exact-warning contract {blocked} must not be faked as executable coverage"
        );
    }
}
