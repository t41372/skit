//! Completeness guard for Python `tests/test_packaging.py` at `main@206f9ef`.
//!
//! Three contracts have executable Rust distribution equivalents. Four inspect Python-only mutmut,
//! `importlib.metadata`, or module-`__getattr__` behavior and are architecture-closed rather than
//! represented by unrelated Cargo or binary behavior.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGET: &str = "crates/skit-cli/tests/port_test_packaging.rs";

const EXECUTABLE: &[&str] = &[
    "test_no_dead_optional_dependencies",
    "test_wheel_excludes_catalog_sources",
    "test_version_is_single_sourced_from_the_distribution",
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_mutmut_refreshes_all_runtime_package_data_in_a_reused_worktree",
        "Python mutmut reuses a copied Python package tree and needs also_copy to refresh runtime data. Rust uses cargo-mutants over crate sources and has no reused Python runtime-package worktree.",
    ),
    (
        "test_version_falls_back_when_no_distribution_is_installed",
        "Python resolves skit.__version__ through importlib.metadata and has a PackageNotFoundError fallback. The compiled Rust binary always receives CARGO_PKG_VERSION at build time and exposes no importable Python module hook.",
    ),
    (
        "test_version_is_resolved_once_and_then_memoized",
        "Python measures lazy module-level importlib.metadata lookup and memoization. Rust performs no runtime distribution-metadata lookup and has no equivalent module __getattr__ cache.",
    ),
    (
        "test_module_getattr_refuses_anything_but_the_version",
        "The contract is specifically Python module-level __getattr__ behavior. The Rust binary is not an importable Python module and cannot expose that attribute-resolution seam.",
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
fn every_executable_packaging_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 3);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 4);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 7);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let source = fs::read_to_string(repo.join(TARGET)).unwrap();
    let actual = test_names(&source);
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "packaging executable mapping drifted");
}

#[test]
fn python_only_packaging_contracts_are_not_impersonated() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
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
