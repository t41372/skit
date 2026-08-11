//! Public semantic placement of Python v0.4
//! `tests/test_params_model.py::test_from_candidate_maps_fields_and_derives_delivery`.
//!
//! Rust does not expose a separate `ParamDecl::from_candidate` conversion: the parser-owned
//! semantic candidate already carries its frontend-neutral declaration. This test therefore uses
//! real source analysis rather than constructing a candidate whose declaration is pre-filled by the
//! test itself.

use skit_domain::parameters::{
    ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{ParseOutcome, parse_document};

#[test]
fn test_from_candidate_maps_fields_and_derives_delivery() {
    let source = "API_KEY = \"Taipei\"\nname = input(\"Name: \")\nprint(API_KEY, name)\n";
    let ParseOutcome::Parsed(document) = parse_document("python", source) else {
        panic!("expected Python source to parse");
    };
    let candidates = document.analysis().candidates;
    assert_eq!(candidates.len(), 2, "{candidates:?}");

    let constant = &candidates[0].declaration;
    assert_eq!(constant.name, "API_KEY");
    assert_eq!(constant.binding, ParameterBinding::Const);
    assert_eq!(constant.delivery, ParameterDelivery::Inject);
    assert_eq!(constant.parameter_type, ParameterType::Str);
    assert_eq!(
        constant.default,
        Some(ParameterValue::String("Taipei".to_owned()))
    );
    assert!(constant.secret);
    assert_eq!(constant.order, -1);
    assert_eq!(constant.prompt, "");

    let input = &candidates[1].declaration;
    assert_eq!(input.binding, ParameterBinding::Input);
    assert_eq!(input.delivery, ParameterDelivery::Inject);
    assert_eq!(input.name, "input-1");
    assert_eq!(input.prompt, "Name: ");
    assert_eq!(input.order, 0);
}
