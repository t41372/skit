//! Executable completeness guard for Python `tests/test_uv_metadata_unpinning.py` at
//! `main@206f9ef`.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

struct Mapping {
    python: &'static str,
    path: &'static str,
    rust: &'static str,
}

const CLI: &str = "crates/skit-cli/tests/port_test_uv_metadata_unpinning.rs";
const SETTINGS: &str = "crates/skit-cli/tests/port_test_effective_uv_metadata_settings.rs";

const MAPPINGS: &[Mapping] = &[
    Mapping {
        python: "test_pin_unpin_repin_block_line_tracks_the_constraint_end_to_end",
        path: CLI,
        rust: "test_pin_unpin_repin_block_line_tracks_the_constraint_end_to_end",
    },
    Mapping {
        python: "test_deps_only_edit_preserves_a_pin_that_lives_only_in_the_block",
        path: CLI,
        rust: "test_deps_only_edit_preserves_a_pin_that_lives_only_in_the_block",
    },
    Mapping {
        python: "test_deps_only_edit_preserves_a_pin_that_lives_in_meta",
        path: CLI,
        rust: "test_deps_only_edit_preserves_a_pin_that_lives_in_meta",
    },
    Mapping {
        python: "test_settings_clearing_python_unpins_the_block",
        path: SETTINGS,
        rust: "test_settings_clearing_python_unpins_the_block",
    },
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_uv_unpinning_python_test_has_a_real_rust_test() {
    assert_eq!(
        MAPPINGS.len(),
        4,
        "frozen Python UV-unpinning oracle count changed"
    );
    assert_eq!(
        MAPPINGS
            .iter()
            .map(|mapping| mapping.python)
            .collect::<BTreeSet<_>>()
            .len(),
        4,
        "duplicate Python mappings make the completeness count dishonest"
    );
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let mut failures = Vec::new();
    for mapping in MAPPINGS {
        let path = repo.join(mapping.path);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        let file = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("{} is not valid Rust: {error}", path.display()));
        let matched = file.items.iter().find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == mapping.rust => {
                Some(has_test_attribute(&function.attrs))
            }
            _ => None,
        });
        match matched {
            Some(true) => {}
            Some(false) => failures.push(format!(
                "{} -> {}::{} exists but is not #[test]",
                mapping.python, mapping.path, mapping.rust
            )),
            None => failures.push(format!(
                "{} -> {}::{} is missing",
                mapping.python, mapping.path, mapping.rust
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "UV-unpinning parity manifest contains fake/non-executable mappings:\n{}",
        failures.join("\n")
    );
}
