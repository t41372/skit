//! Exact application-layer port of the Python v0.4 path free-text validation contract.
//!
//! Frozen oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.

use skit_application::value_preparation::validate_form_value;
use skit_domain::parameters::{ParamDecl, ParameterType};

#[test]
fn test_validate_value_path_is_free_text() {
    let mut declaration = ParamDecl::new("src");
    declaration.parameter_type = ParameterType::Path;
    assert_eq!(
        validate_form_value(&declaration, "./definitely/not/created/yet.csv"),
        Ok(())
    );
}
