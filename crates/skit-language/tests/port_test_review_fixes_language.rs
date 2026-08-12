//! Public language-layer ports from Python `tests/test_review_fixes.py` at `main@206f9ef`.
//!
//! These assertions exercise the actual parser/rewrite boundary. They do not recreate the Python
//! helpers inside the test: malformed scalar input must be rejected by `inject_values`, and metadata
//! round trips must survive the public writer/reader pair.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{
    inject_values, managed_params, read_uv_metadata, write_managed_params, write_uv_metadata,
};

fn source_param(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut parameter = ParamDecl::new(name);
    parameter.binding = ParameterBinding::Const;
    parameter.delivery = ParameterDelivery::Inject;
    parameter.parameter_type = parameter_type;
    parameter
}

#[test]
fn test_inject_rejects_non_finite_float() {
    let text = "RATE = 1.5\nprint(RATE)\n";
    let declarations = [source_param("RATE", ParameterType::Float)];

    for bad in ["inf", "-inf", "nan", "Infinity"] {
        let values = BTreeMap::from([("RATE".to_owned(), bad.to_owned())]);
        assert!(
            inject_values("python", text, &declarations, &values).is_err(),
            "non-finite float {bad:?} reached Python source"
        );
    }
}

#[test]
fn test_inject_accepts_normal_float() {
    let text = "RATE = 1.5\nprint(RATE)\n";
    let declarations = [source_param("RATE", ParameterType::Float)];
    let values = BTreeMap::from([("RATE".to_owned(), "2.75".to_owned())]);

    let output = inject_values("python", text, &declarations, &values).unwrap();

    assert!(output.contains("RATE = 2.75"), "{output}");
    assert!(
        output.contains("print(RATE)"),
        "unrelated source disappeared: {output}"
    );
}

#[test]
fn test_write_params_prompt_with_newline_roundtrips() {
    let text = "CITY = \"Taipei\"\nprint(CITY)\n";
    let mut city = source_param("CITY", ParameterType::Str);
    city.prompt = "City:\nwith newline\t!".to_owned();

    let output = write_managed_params("python", text, &[city]).unwrap();
    let back = managed_params("python", &output);

    assert_eq!(back.len(), 1, "{output}");
    assert_eq!(back[0].prompt, "City:\nwith newline\t!");
}

#[test]
fn test_set_dependencies_multiline_array_with_comment() {
    let text = concat!(
        "# /// script\n",
        "# dependencies = [  # my deps\n",
        "#     \"requests\",\n",
        "# ]\n",
        "# ///\n",
        "print(1)\n",
    );

    let output = write_uv_metadata(text, &["httpx".to_owned()], "").unwrap();
    let metadata = read_uv_metadata(&output).expect("rewritten block must remain parseable");

    assert_eq!(metadata.dependencies, ["httpx"]);
    assert!(
        !output.contains("requests"),
        "orphaned old array row survived: {output}"
    );
    assert!(
        output.contains("print(1)"),
        "source body disappeared: {output}"
    );
}

#[test]
fn test_write_params_no_block_no_params() {
    let text = "print(1)\n";
    assert_eq!(write_managed_params("python", text, &[]).unwrap(), text);
}

#[test]
fn test_parse_block_corrupt_body_returns_none() {
    let bad = "# /// script\n# not: valid: toml: [\n# ///\nprint(1)\n";
    assert_eq!(read_uv_metadata(bad), None);
}

#[test]
fn test_inject_annotated_assignment() {
    let source = "CITY: str = 'Taipei'\nprint(CITY)\n";
    let declaration = source_param("CITY", ParameterType::Str);
    let values = BTreeMap::from([("CITY".to_owned(), "Kaohsiung".to_owned())]);

    let output = inject_values("python", source, &[declaration], &values).unwrap();

    assert!(output.contains("CITY: str = 'Kaohsiung'"), "{output}");
    assert!(output.contains("print(CITY)"), "{output}");
}
