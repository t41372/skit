//! Exact stage-boundary contracts from Python v0.4 `tests/test_flows.py`.
//!
//! These intentionally target the lower public delivery layer where Python's frozen assemble
//! contract assumes pre-submit validation has already happened. Using `assemble_run_inputs` here
//! would incorrectly add a second type-check and weaken the compatibility boundary.

use std::collections::BTreeMap;

use skit_application::delivery::{PreparedValue, assemble};
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};

#[test]
fn test_assemble_does_not_retypecheck_plain_values() {
    let mut gap = ParamDecl::new("gap");
    gap.delivery = ParameterDelivery::Flag;
    gap.parameter_type = ParameterType::Int;
    gap.flag = "--gap".to_owned();

    let assembly = assemble(
        &[gap],
        &BTreeMap::from([(
            "gap".to_owned(),
            PreparedValue::Scalar("abc".to_owned()),
        )]),
        &[],
    )
    .unwrap();

    assert_eq!(assembly.args, ["--gap", "abc"]);
}
