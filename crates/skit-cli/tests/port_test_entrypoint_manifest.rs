//! Completeness guard for Python `tests/test_entrypoint.py` at `main@206f9ef`.
//!
//! Five contracts have executable Rust public-surface equivalents. Five are Python-runtime-only
//! seams and are deliberately architecture-closed rather than replaced by weaker stand-ins.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGET: &str = "crates/skit-cli/tests/port_test_entrypoint.rs";

const EXECUTABLE: &[&str] = &[
    "test_version_is_plain_text_not_rich_markup",
    "test_a_real_command_still_reaches_the_cli",
    "test_no_arguments_reaches_the_cli",
    "test_the_console_script_points_at_the_dispatcher",
    "test_a_bad_invocation_still_fails_through_the_dispatcher",
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_version_flag_is_answered_without_building_the_cli",
        "Python proves the fast path by inspecting freshly imported runtime modules; a statically linked Rust binary has no equivalent import graph to observe.",
    ),
    (
        "test_version_flag_answers_in_process_too",
        "Python monkeypatches sys.argv and skit.cli.app inside one interpreter; Rust entry() consumes the process argv and exposes no injectable argv/CLI callback seam.",
    ),
    (
        "test_both_version_paths_print_the_identical_line",
        "Python forces the exact same --version argv through two distinct internal paths (the fast dispatcher and the Typer callback) and compares their output. Rust has one Clap version path; changing argv to manufacture a second path would test different behavior.",
    ),
    (
        "test_the_flag_is_claimed_only_as_the_whole_command_line",
        "Python observes dispatcher-vs-Typer routing by monkeypatching the Typer app; Rust has one Clap parser and no separate observable dispatcher callback. The malformed trailing-argv behavior is still tested end to end.",
    ),
    (
        "test_python_dash_m_skit_is_the_same_entry",
        "The Rust distribution has no Python package module and therefore no `python -m skit` entry point.",
    ),
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn test_names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn every_executable_entrypoint_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 5);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 5);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 10);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source = fs::read_to_string(repo.join(TARGET)).unwrap();
    let actual = test_names(&source);
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "entrypoint executable mapping drifted");
}

#[test]
fn python_runtime_only_entrypoint_contracts_are_not_impersonated() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source = fs::read_to_string(repo.join(TARGET)).unwrap();
    let actual = test_names(&source);

    for (name, reason) in ARCHITECTURE_CLOSED {
        assert!(
            !actual.contains(*name),
            "{name} is architecture-closed ({reason}); do not add a same-named weaker stand-in"
        );
        assert!(!reason.trim().is_empty());
    }
}
