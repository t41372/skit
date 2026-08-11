//! Exact port of Python v0.4
//! `tests/test_declared_params.py::test_param_decl_meta_round_trip_and_block_round_trip`.
//!
//! Python requires both serialization homes to preserve the same declared semantics in one
//! contract. Existing Rust tests split those homes for extra diagnostics; this exact-name oracle
//! keeps the frozen Python behavioral unit intact.

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};

#[test]
fn test_param_decl_meta_round_trip_and_block_round_trip() {
    let mut declaration = ParamDecl::new("x");
    declaration.binding = ParameterBinding::None;
    declaration.delivery = ParameterDelivery::Env;
    declaration.parameter_type = ParameterType::Int;
    declaration.default = Some(ParameterValue::Integer(3));
    declaration.required = true;
    declaration.multiple = true;
    declaration.repeat = true;
    declaration.prompt = "X".to_owned();
    declaration.help = "help".to_owned();
    declaration.secret = true;
    declaration.env_source = "X_SRC".to_owned();
    declaration.env_target = "X_DST".to_owned();
    declaration.flag = "--x".to_owned();
    declaration.action = "store_true".to_owned();
    declaration.order = 4;
    declaration.degraded = true;

    let from_meta = ParamDecl::from_meta_map(&declaration.to_meta_map());
    assert_eq!(from_meta, declaration);

    // Block declarations carry source-owned semantics. Use a source-bound declaration, exactly as
    // Python's block roundtrip does, and require its implied delivery/binding/default to survive.
    let mut source_bound = ParamDecl::new("CITY");
    source_bound.binding = ParameterBinding::Const;
    source_bound.delivery = ParameterDelivery::Inject;
    source_bound.parameter_type = ParameterType::Str;
    source_bound.default = Some(ParameterValue::String("Taipei".to_owned()));
    source_bound.required = true;
    source_bound.secret = true;
    source_bound.env_source = "CITY_SECRET".to_owned();
    source_bound.prompt = "City".to_owned();
    source_bound.order = 2;

    let from_block = ParamDecl::from_block_map(&source_bound.to_block_map());
    assert_eq!(from_block, source_bound);
}
