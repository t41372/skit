//! Completeness guard for Python `tests/test_draft_and_reader_tui.py` at `main@206f9ef`.
//!
//! All sixteen contracts have executable Rust equivalents. Review and keyboard behavior lives in
//! the Ratatui test target; the real FileStore and PTY Settings boundaries live in skit-cli tests.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGETS: &[&str] = &[
    "crates/skit-tui/tests/port_test_draft_and_reader_tui.rs",
    "crates/skit-cli/tests/port_test_draft_and_reader_tui_host.rs",
];

const EXECUTABLE: &[&str] = &[
    "test_resume_bash_shebang_draft_lands_as_shell",
    "test_review_versioned_shebang_shows_and_stores_pin",
    "test_review_pin_follows_a_shebang_edit_on_rescan",
    "test_review_explicit_python_is_not_overwritten_by_the_shebang",
    "test_review_dynamic_optstring_keeps_ticks_and_space_chip",
    "test_review_modeled_getopts_suppresses_ticks_and_space_chip",
    "test_settings_dynamic_optstring_offers_tick_checkboxes",
    "test_settings_modeled_getopts_hides_tick_checkboxes",
    "test_review_one_field_getopts_says_singular",
    "test_review_multi_field_getopts_says_plural",
    "test_ctrl_d_deletes_the_highlighted_draft_after_confirm",
    "test_ctrl_d_confirm_esc_keeps_the_draft",
    "test_ctrl_d_while_editing_a_field_is_the_inputs_delete_right",
    "test_delete_draft_action_is_a_noop_when_no_drafts",
    "test_delete_draft_action_is_a_noop_when_nothing_highlighted",
    "test_delete_draft_chip_only_renders_when_drafts_exist",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn parity_test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                let name = function.sig.ident.to_string();
                (!name.starts_with("rust_additive_")).then_some(name)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn every_draft_and_reader_tui_contract_has_exactly_one_executable_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 16);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let actual = TARGETS
        .iter()
        .flat_map(|target| {
            let source = fs::read_to_string(repo.join(target)).unwrap();
            parity_test_names(&source)
        })
        .collect::<Vec<_>>();
    let unique = actual.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        actual.len(),
        unique.len(),
        "a draft/reader TUI contract has more than one exact-name Rust oracle: {actual:?}"
    );
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(unique, expected, "draft/reader TUI executable mapping drifted");
}
