//! Completeness guard for Python `tests/test_agent_install.py` at `main@206f9ef`.
//!
//! Twenty-one Python contracts have executable Rust behavior tests. The one Python runtime case
//! that monkeypatches the packaged resource out from under the process is architecture-closed in
//! Rust: the production composition root uses `include_bytes!`, so a missing resource fails the
//! build and cannot become a runtime `FileNotFoundError`. This guard requires that compile-time
//! embedding and forbids a fake same-named runtime test.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const APPLICATION: &str = include_str!("../../skit-application/tests/port_test_agent_install.rs");
const STORE: &str = include_str!("../../skit-store/tests/port_test_agent_install.rs");
const CLI: &str = include_str!("port_test_agent_install.rs");
const PRODUCTION_CLI: &str = include_str!("../src/cli.rs");

const ARCHITECTURE_CLOSED: &str = "test_cli_install_broken_package_fails_loudly";
const EXECUTABLE: &[&str] = &[
    "test_skill_text_is_the_bundled_skill",
    "test_detect_targets_reports_only_existing_marker_dirs",
    "test_detect_targets_empty_when_nothing_exists",
    "test_named_target_user_and_project_scopes",
    "test_named_target_agents_is_always_project_scoped",
    "test_named_target_unknown_is_none",
    "test_install_into_writes_and_upgrades",
    "test_cli_install_to_explicit_dir",
    "test_cli_install_to_a_file_fails_cleanly",
    "test_cli_install_to_with_project_is_a_conflict",
    "test_cli_install_to_expands_tilde",
    "test_cli_install_named_target_user_scope",
    "test_cli_install_named_target_project_scope",
    "test_cli_install_unknown_target_exits_2",
    "test_cli_install_target_and_to_conflict_exits_2",
    "test_cli_bare_non_interactive_refuses",
    "test_cli_bare_interactive_no_candidates_exits_1",
    "test_cli_bare_interactive_picks_and_confirms",
    "test_cli_bare_interactive_backing_out_writes_nothing",
    "test_agent_pick_target_renders_the_menu_exactly",
    "test_agent_pick_target_backing_out_returns_none",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("Agent installer parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn actual_tests() -> Vec<String> {
    tests(APPLICATION)
        .into_iter()
        .chain(tests(STORE))
        .chain(tests(CLI))
        .collect()
}

#[test]
fn all_twenty_one_executable_python_agent_install_contracts_are_present_once() {
    let actual = actual_tests();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(EXECUTABLE.len(), 21);
    assert_eq!(actual.len(), 21, "unexpected extra or missing Agent installer tests");
    assert_eq!(actual_set.len(), actual.len(), "duplicate test names hide a missing contract");
    assert_eq!(actual_set, expected);
    assert!(!actual.iter().any(|name| name == ARCHITECTURE_CLOSED));
}

#[test]
fn the_python_missing_packaged_resource_runtime_case_is_compile_time_closed_in_rust() {
    assert!(
        PRODUCTION_CLI.contains(
            "include_bytes!(\"../../../skills/skit/SKILL.md\")"
        ),
        "the Rust installer stopped making the bundled skill a compile-time resource"
    );

    const SHIPPED: &[u8] = include_bytes!("../../../skills/skit/SKILL.md");
    assert!(SHIPPED.starts_with(b"---\nname: skit\n"));
    assert!(!actual_tests().iter().any(|name| name == ARCHITECTURE_CLOSED));
}
