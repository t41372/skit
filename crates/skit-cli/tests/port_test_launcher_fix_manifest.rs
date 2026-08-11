//! Completeness guard for Python v0.4 `tests/test_launcher_fix.py`.
//!
//! Eleven contracts map to public Rust launch/render/execution seams. The final Python test drives a
//! private platform-branch quoting helper directly; Rust exposes no equivalent public helper, so it
//! is explicitly blocked rather than replaced with a weaker fake test. This manifest is not
//! behavioral coverage.

use std::collections::BTreeMap;

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("../../skit-runtime/tests/port_test_launcher_fix.rs");

const EXECUTABLE: &[&str] = &[
    "test_placeholder_value_with_double_braces_round_trips",
    "test_placeholder_value_with_double_braces_inside_quoted_template_slot",
    "test_template_escape_still_unescaped_alongside_a_corrupting_value",
    "test_run_entry_executes_correctly_with_double_brace_value",
    "test_normalize_exit_code_maps_negative_returncode_to_128_plus_n",
    "test_run_entry_normalizes_signal_killed_child_to_shell_convention",
    "test_build_python_missing_script_raises_before_calling_ensure_uv",
    "test_build_python_healthy_script_still_calls_ensure_uv",
    "test_placeholder_value_with_space_is_quoted_as_one_word",
    "test_placeholder_value_with_shell_metacharacters_cannot_inject",
    "test_run_entry_placeholder_value_with_space_reaches_child_intact",
];

const BLOCKED_PRIVATE: &str = "test_quote_for_shell_uses_list2cmdline_on_windows";

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names() -> Vec<String> {
    syn::parse_file(SOURCE)
        .expect("launcher-fix parity source must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn launcher_fix_has_exactly_the_11_public_python_oracles() {
    let mut counts = BTreeMap::<String, usize>::new();
    for name in names() {
        *counts.entry(name).or_default() += 1;
    }
    for expected in EXECUTABLE {
        assert_eq!(
            counts.get(*expected).copied().unwrap_or_default(),
            1,
            "Python launcher-fix contract {expected} must map to exactly one Rust test"
        );
    }
    assert_eq!(EXECUTABLE.len(), 11);
}

#[test]
fn launcher_fix_private_windows_quote_helper_is_not_faked_as_coverage() {
    assert!(
        !names().iter().any(|name| name == BLOCKED_PRIVATE),
        "{BLOCKED_PRIVATE} needs an equivalent public Rust quote seam; do not fake it with a weaker test"
    );
}
