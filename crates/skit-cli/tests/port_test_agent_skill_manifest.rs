//! Exact completeness guard for Python `tests/test_agent_skill.py` at `main@206f9ef`.

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("port_test_agent_skill.rs");
const PYTHON_TESTS: &[&str] = &[
    "test_root_and_packaged_copies_are_identical",
    "test_skill_ships_inside_the_package",
    "test_frontmatter_satisfies_the_agent_skills_spec",
    "test_skill_stays_within_the_progressive_disclosure_budget",
    "test_every_command_the_skill_teaches_exists",
    "test_the_skill_never_mentions_json_free_surfaces_wrongly",
    "test_skill_describes_placeholder_delivery_for_both_real_entry_kinds",
    "test_skill_teaches_executable_empty_value_spellings_for_clearing_pins",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_python_agent_skill_contract_has_the_same_named_executable_rust_oracle_in_order() {
    let actual = syn::parse_file(SOURCE)
        .expect("Agent Skill parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(PYTHON_TESTS.len(), 8);
    assert_eq!(actual, expected);
}
