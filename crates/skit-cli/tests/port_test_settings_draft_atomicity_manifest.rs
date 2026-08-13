//! Completeness guard for Python `tests/test_settings_and_draft_review_atomicity.py` at
//! `main@206f9ef`.
//!
//! Twelve contracts have executable Rust public/TUI/FileStore equivalents. Four inject failures at
//! exact Python store/dependency helper call sites that the Rust composition root does not expose as
//! replaceable ports. Those four stay architecture-closed rather than being represented by a
//! different filesystem failure or another mutation phase.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGETS: &[&str] = &[
    "crates/skit-tui/tests/port_test_settings_draft_atomicity_review.rs",
    "crates/skit-cli/tests/port_test_settings_draft_atomicity_host.rs",
];

const EXECUTABLE: &[&str] = &[
    "test_settings_bad_dep_refuses_the_whole_save_including_the_rename",
    "test_settings_bad_python_refuses_the_whole_save_including_the_rename",
    "test_settings_dash_python_saves_as_automatic",
    "test_settings_valid_deps_and_python_save_normally",
    "test_settings_npm_deps_are_not_pep508_validated",
    "test_settings_name_conflict_is_refused_before_npm_clear",
    "test_add_panel_on_a_kept_draft_hides_storage_and_copies",
    "test_prompt_panel_on_a_kept_draft_hides_storage_and_copies",
    "test_add_panel_on_a_nondraft_still_shows_storage",
    "test_resumed_draft_through_the_tui_add_lane_is_consumed",
    "test_add_panel_prefill_drops_a_pep508_illegal_import",
    "test_add_panel_prefill_drops_a_sibling_local_module",
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_settings_failed_npm_clear_commits_no_other_form_edits",
        "Python monkeypatches skit.langs.javascript.deps.clear to fail at the exact pre-metadata npm-cleanup call after simultaneous name, description, workdir, interpreter, parameter, and dependency edits. Rust calls clear_javascript_dependencies inside the private CLI composition root and exposes no injected cleaner port. A malformed real filesystem would fail with a different reason and would not prove the same injected call-site contract.",
    ),
    (
        "test_settings_name_precheck_store_failure_is_reported_without_writes",
        "Python replaces store.resolve only during the rename-name precheck and proves that this exact repository read failure is surfaced before any write. Rust FileStore is concrete at the private Settings composition root and has no injected repository/read port at that precheck. Corrupting the real store would change more than this one call and would be a different contract.",
    ),
    (
        "test_settings_rename_race_failure_stops_later_writes",
        "Python replaces store.rename after the precheck and forces a failure in the exact race window before later description/settings writes. Rust exposes no hook that can insert a competitor or failure precisely between its private precheck and rename step without changing production code.",
    ),
    (
        "test_settings_late_dependency_store_failure_is_reported_and_stays_open",
        "Python replaces store.update_dependencies at the late dependency-write phase and proves that this exact host failure stays on Settings with no dependency metadata committed. Rust does not expose an injectable late dependency repository operation at the private Settings save boundary; causing another FileStore failure at another phase would not preserve the contract.",
    ),
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

fn actual_names(repo: &Path) -> Vec<String> {
    TARGETS
        .iter()
        .flat_map(|target| {
            let source = fs::read_to_string(repo.join(target)).unwrap();
            parity_test_names(&source)
        })
        .collect()
}

#[test]
fn every_executable_settings_draft_atomicity_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 12);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 4);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 16);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let actual = actual_names(repo);
    let unique = actual.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        actual.len(),
        unique.len(),
        "a Settings/draft atomicity contract has more than one exact-name Rust oracle: {actual:?}"
    );
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        unique, expected,
        "Settings/draft atomicity executable mapping drifted"
    );
}

#[test]
fn private_settings_fault_contracts_are_not_impersonated() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let actual = actual_names(repo).into_iter().collect::<BTreeSet<_>>();

    for (name, reason) in ARCHITECTURE_CLOSED {
        assert!(
            !actual.contains(*name),
            "{name} is architecture-closed ({reason}); do not add a weaker same-named stand-in"
        );
        assert!(!reason.trim().is_empty());
    }
}
