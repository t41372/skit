//! Exact drift-aggregation ports from Python v0.4 `tests/test_shell_inject_mut.py`.
//!
//! These stay separate from the broader shell injection file so both aggregation failures execute
//! independently. Python requires every missing target in declaration order; checking only the first
//! name would weaken the diagnostic contract.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::inject_values;

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn input_decl() -> ParamDecl {
    let mut declaration = ParamDecl::new("input-1");
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.order = 0;
    declaration.prompt = "P: ".to_owned();
    declaration
}

fn const_decl(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

#[test]
fn test_drift_error_lists_every_missing_name_joined() {
    let declarations = [input_decl(), const_decl("GONE")];
    let error = inject_values(
        "shell",
        "#!/usr/bin/env bash\nX=1\n",
        &declarations,
        &values(&[("input-1", "v"), ("GONE", "x")]),
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "input-1, GONE");
}

#[test]
fn test_drift_error_keeps_scanning_past_a_missing_const() {
    let declarations = [const_decl("GONE1"), const_decl("GONE2")];
    let error = inject_values(
        "shell",
        "#!/usr/bin/env bash\nX=1\n",
        &declarations,
        &values(&[("GONE1", "a"), ("GONE2", "b")]),
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "GONE1, GONE2");
}
