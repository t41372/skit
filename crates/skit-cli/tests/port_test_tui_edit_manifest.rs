//! Exact executable completeness guard for Python `tests/test_tui_edit.py` at `main@206f9ef`.

use std::{fs, path::Path};

use syn::{Attribute, Item};

const EXPECTED: &[&str] = &[
    "test_editable_source_copy_mode_points_at_the_stored_copy",
    "test_editable_source_reference_mode_points_at_the_original",
    "test_editable_source_command_entry_has_none",
    "test_edit_opens_editor_and_reports",
    "test_edit_command_entry_reports_no_source",
    "test_edit_invalidates_the_drift_cache",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn tui_edit_has_exactly_the_6_frozen_python_oracles() {
    assert_eq!(EXPECTED.len(), 6);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let path = repo.join("crates/skit-cli/tests/port_test_tui_edit.rs");
    let source = fs::read_to_string(&path).unwrap();
    let file = syn::parse_file(&source).expect("TUI-edit parity target must parse as Rust");
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
        "TUI-edit parity target must be exactly the frozen Python test sequence"
    );
}
