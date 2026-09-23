//! Runtime delivery port from Python `tests/test_powershell.py` at `main@206f9ef`.

use std::collections::BTreeMap;

use skit_application::delivery::{PreparedValue, assemble};
use skit_domain::parameters::{ParamDecl, ParameterType, ParameterValue};

#[test]
fn test_single_dash_flags_assemble() {
    let mut name = ParamDecl::new("Name");
    name.flag = "-Name".to_owned();
    name.parameter_type = ParameterType::Str;

    let mut verbose = ParamDecl::new("Verbose");
    verbose.flag = "-Verbose".to_owned();
    verbose.parameter_type = ParameterType::Bool;
    verbose.action = "store_true".to_owned();
    verbose.default = Some(ParameterValue::Bool(false));

    let values = BTreeMap::from([
        ("Name".to_owned(), PreparedValue::Scalar("Ada".to_owned())),
        (
            "Verbose".to_owned(),
            PreparedValue::Scalar("true".to_owned()),
        ),
    ]);
    let assembly = assemble(&[name, verbose], &values, &[]).unwrap();
    assert_eq!(assembly.args, ["-Name", "Ada", "-Verbose"]);
}
