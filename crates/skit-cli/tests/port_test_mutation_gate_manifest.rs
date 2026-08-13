//! Architecture accounting for Python `tests/test_mutation_gate.py` at `main@206f9ef`.
//!
//! Version 0.4 unit-tests `scripts/check_mutation_stats.py`, a mutmut JSON post-processor. The Rust
//! rewrite has no equivalent stats parser: `.github/workflows/mutation.yml` runs cargo-mutants as
//! the gate itself. Recreating `failure_detail` only inside a test would test invented code, not the
//! Rust product. These tests therefore pin the real replacement gate and classify all four Python
//! helper contracts precisely instead of impersonating them.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_clean_mutation_stats_pass",
        "Python calls failure_detail() on a mutmut stats dictionary. Rust has no repository stats parser; cargo-mutants is the process whose exit status gates the workflow directly.",
    ),
    (
        "test_every_unsuccessful_mutation_state_fails",
        "The parametrized rows enumerate mutmut-specific result keys such as survived, no_tests, suspicious, segfault, and interrupted. Rust does not translate cargo-mutants output into that mutmut schema, so copying the state names into a test-only parser would invent an implementation.",
    ),
    (
        "test_empty_or_unaccounted_mutation_stats_fail",
        "The no-mutants/unaccounted arithmetic belongs to the absent Python check_mutation_stats.py post-processor. The Rust workflow consumes cargo-mutants' own process result and does not read that JSON shape.",
    ),
    (
        "test_main_returns_failure_and_emits_ci_annotation",
        "Python tests its wrapper script's return code and ::error:: annotation. The Rust mutation workflow has no wrapper main() and no repository-generated annotation; cargo-mutants runs as the failing step itself.",
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

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli")
}

#[test]
fn rust_mutation_gate_is_direct_and_cannot_suppress_cargo_mutants_failure() {
    let workflow = fs::read_to_string(repo_root().join(".github/workflows/mutation.yml")).unwrap();

    assert!(
        workflow.contains("cargo mutants --workspace --all-features --cargo-arg=--locked"),
        "the Rust mutation workflow stopped running cargo-mutants as its gate"
    );
    assert!(
        workflow.contains("--minimum-test-timeout 20"),
        "the mutation timeout floor disappeared"
    );
    assert!(
        workflow.contains("--timeout-multiplier 3.0"),
        "the mutation timeout multiplier disappeared"
    );
    assert!(
        !workflow.contains("continue-on-error"),
        "the cargo-mutants step may not suppress its failure"
    );
    assert!(
        !workflow.contains("|| true"),
        "the cargo-mutants command may not erase its exit status"
    );
}

#[test]
fn mutmut_stats_helper_contracts_are_not_impersonated_by_rust_tests() {
    assert_eq!(ARCHITECTURE_CLOSED.len(), 4);
    let this = fs::read_to_string(
        repo_root().join("crates/skit-cli/tests/port_test_mutation_gate_manifest.rs"),
    )
    .unwrap();
    let actual = test_names(&this);

    for (name, reason) in ARCHITECTURE_CLOSED {
        assert!(
            !actual.contains(*name),
            "{name} is architecture-closed ({reason}); do not add a same-named test-only reimplementation"
        );
        assert!(!reason.trim().is_empty());
    }
}
