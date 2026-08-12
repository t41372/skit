//! Executable completeness guard for Python `tests/test_pep723_split.py` at `main@206f9ef`.
//!
//! The Python module has 24 tests. Private-helper assertions are mapped to the strongest public Rust
//! behavior that carries their contract; every mapping below must still name a real `#[test]`.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

struct Mapping {
    python: &'static str,
    path: &'static str,
    rust: &'static str,
}

const LANGUAGE: &str = "crates/skit-language/tests/port_test_pep723_split.rs";
const UI: &str = "crates/skit-ui/tests/port_test_pep723_split.rs";
const CLI: &str = "crates/skit-cli/tests/port_test_pep723_split_cli.rs";

const MAPPINGS: &[Mapping] = &[
    Mapping { python: "test_block_re_hash_pattern_is_byte_identical_to_the_frozen_literal", path: LANGUAGE, rust: "test_block_re_hash_pattern_is_byte_identical_to_the_frozen_literal" },
    Mapping { python: "test_block_re_double_slash_pattern_mirrors_the_hash_form", path: LANGUAGE, rust: "test_block_re_double_slash_pattern_mirrors_the_hash_form" },
    Mapping { python: "test_slash_block_round_trips_with_shebang_skip", path: LANGUAGE, rust: "test_slash_block_round_trips_with_shebang_skip" },
    Mapping { python: "test_simple_list_splits", path: UI, rust: "test_simple_list_splits" },
    Mapping { python: "test_single_item_no_commas", path: UI, rust: "test_single_item_no_commas" },
    Mapping { python: "test_specifier_commas_stay_joined", path: UI, rust: "test_specifier_commas_stay_joined" },
    Mapping { python: "test_specifier_lists_split_only_between_requirements", path: UI, rust: "test_specifier_lists_split_only_between_requirements" },
    Mapping { python: "test_spaces_around_specifier_commas", path: UI, rust: "test_spaces_around_specifier_commas" },
    Mapping { python: "test_extras_bracket_commas_stay_joined", path: UI, rust: "test_extras_bracket_commas_stay_joined" },
    Mapping { python: "test_parenthesized_specifier_commas_stay_joined", path: UI, rust: "test_parenthesized_specifier_commas_stay_joined" },
    Mapping { python: "test_double_quoted_marker_comma_stays_joined", path: UI, rust: "test_double_quoted_marker_comma_stays_joined" },
    Mapping { python: "test_single_quoted_marker_comma_stays_joined", path: UI, rust: "test_single_quoted_marker_comma_stays_joined" },
    Mapping { python: "test_name_starting_with_digit_splits", path: UI, rust: "test_name_starting_with_digit_splits" },
    Mapping { python: "test_trailing_comma_dropped", path: UI, rust: "test_trailing_comma_dropped" },
    Mapping { python: "test_empty_and_blank_input", path: UI, rust: "test_empty_and_blank_input" },
    Mapping { python: "test_uppercase_x_in_name_is_ordinary_text", path: UI, rust: "test_uppercase_x_in_name_is_ordinary_text" },
    Mapping { python: "test_nested_brackets_tracked_by_depth_not_flag", path: UI, rust: "test_nested_brackets_tracked_by_depth_not_flag" },
    Mapping { python: "test_next_nonspace_end_of_text_is_empty_string", path: UI, rust: "test_next_nonspace_end_of_text_is_empty_string" },
    Mapping { python: "test_add_dep_flags_carry_specifier_commas", path: CLI, rust: "test_add_dep_flags_carry_specifier_commas" },
    Mapping { python: "test_interactive_deps_answer_keeps_specifier_commas", path: UI, rust: "test_interactive_deps_answer_keeps_specifier_commas" },
    Mapping { python: "test_deps_dep_flags_carry_specifier_commas", path: CLI, rust: "test_deps_dep_flags_carry_specifier_commas" },
    Mapping { python: "test_build_block_escapes_double_quoted_marker", path: LANGUAGE, rust: "test_build_block_escapes_double_quoted_marker" },
    Mapping { python: "test_set_dependencies_escapes_double_quoted_marker", path: LANGUAGE, rust: "test_set_dependencies_escapes_double_quoted_marker" },
    Mapping { python: "test_build_block_escapes_backslash_in_dependency", path: LANGUAGE, rust: "test_build_block_escapes_backslash_in_dependency" },
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_pep723_split_python_test_has_a_real_rust_test() {
    assert_eq!(MAPPINGS.len(), 24, "frozen Python splitter oracle count changed");
    assert_eq!(
        MAPPINGS.iter().map(|mapping| mapping.python).collect::<BTreeSet<_>>().len(),
        24,
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
        "PEP 723 parity manifest contains fake/non-executable mappings:\n{}",
        failures.join("\n")
    );
}
