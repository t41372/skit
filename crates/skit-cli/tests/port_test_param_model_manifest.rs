//! Completeness guard for Python v0.4 `tests/test_params_model.py`.
//!
//! Eighteen contracts have equivalent public Rust domain/semantic behavior. Python's
//! `field_replace` is a dataclass convenience helper with no Rust product API; testing `Clone` plus
//! direct field assignment would test the language rather than skit, so that one name is explicitly
//! blocked. This manifest is not behavioral coverage.

use std::collections::BTreeMap;

use syn::{Attribute, Item};

const DOMAIN_SOURCE: &str = include_str!("../../skit-domain/tests/port_test_param_model.rs");
const CANDIDATE_SOURCE: &str =
    include_str!("../../skit-language/tests/port_test_param_model_candidate.rs");

const EXECUTABLE: &[&str] = &[
    "test_block_dict_const_shape_is_frozen",
    "test_block_dict_input_shape_is_frozen",
    "test_block_roundtrip_derives_delivery_from_binding",
    "test_from_block_dict_is_total_on_garbage",
    "test_from_candidate_maps_fields_and_derives_delivery",
    "test_meta_roundtrip_full_model",
    "test_meta_dict_omits_defaults",
    "test_meta_dict_omits_repeat_when_false",
    "test_meta_dict_repeat_emitted_and_roundtrips_only_when_set",
    "test_from_meta_dict_repeat_defaults_false_when_absent",
    "test_from_meta_dict_repeat_coerces_truthy_to_bool",
    "test_meta_dict_includes_binding_and_order_when_set",
    "test_meta_roundtrip_env_delivery_and_target",
    "test_from_meta_dict_is_total_on_garbage",
    "test_env_var_defaults_to_name",
    "test_invariants_binding_implies_delivery",
    "test_invariants_choice_needs_choices",
    "test_normalize_repairs_delivery_from_binding",
];

const BLOCKED_HELPER: &str = "test_field_replace_returns_modified_copy";

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("ParamDecl parity source must parse as Rust")
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
fn param_model_has_exactly_18_executable_python_oracles() {
    let mut counts = BTreeMap::<String, usize>::new();
    for source in [DOMAIN_SOURCE, CANDIDATE_SOURCE] {
        for name in names(source) {
            *counts.entry(name).or_default() += 1;
        }
    }
    for expected in EXECUTABLE {
        assert_eq!(
            counts.get(*expected).copied().unwrap_or_default(),
            1,
            "Python ParamDecl contract {expected} must map exactly once"
        );
    }
    assert_eq!(EXECUTABLE.len(), 18);
}

#[test]
fn param_model_field_replace_helper_is_not_faked_as_coverage() {
    let mut all = names(DOMAIN_SOURCE);
    all.extend(names(CANDIDATE_SOURCE));
    assert!(
        !all.iter().any(|name| name == BLOCKED_HELPER),
        "{BLOCKED_HELPER} has no equivalent Rust product API; do not fake it with Clone/direct-field mutation"
    );
}
