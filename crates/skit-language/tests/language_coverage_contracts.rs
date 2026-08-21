use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterValue};
use skit_language::{
    UvMetadata, external_dependencies, plan_uv_metadata_edit, suggest_description,
    write_managed_params_bytes, write_uv_metadata,
};

#[test]
fn a_first_line_python_coding_cookie_stays_before_inserted_metadata() {
    let source = "# coding: utf-8\nprint('ok')\n";

    let written = write_uv_metadata(source, &["requests".to_owned()], "").unwrap();

    assert!(written.starts_with("# coding: utf-8\n# /// script\n"));
    assert!(written.ends_with("print('ok')\n"));
}

#[test]
fn non_dependency_calls_and_empty_prompts_have_no_invented_projection() {
    assert!(external_dependencies("js", "other('package')\n").is_empty());
    assert_eq!(suggest_description("prompt", b" \n\t\n"), "");
}

#[test]
fn the_legacy_dash_clears_only_the_python_constraint_axis() {
    let stored = UvMetadata {
        dependencies: vec!["requests".to_owned()],
        requires_python: ">=3.11".to_owned(),
    };

    let plan = plan_uv_metadata_edit(None, &stored, None, Some(" - ".to_owned())).unwrap();

    assert_eq!(plan.effective.dependencies, ["requests"]);
    assert!(plan.effective.requires_python.is_empty());
    assert_eq!(plan.stored, plan.effective);
    assert_eq!(plan.rewritten_source, None);
}

#[test]
fn managed_byte_edits_reserve_string_defaults_in_the_lossless_marker_protocol() {
    let mut declaration = ParamDecl::new("VALUE");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.default = Some(ParameterValue::String("future default".to_owned()));

    let written = write_managed_params_bytes("python", b"VALUE = 'old'\n", &[declaration]).unwrap();

    assert!(
        String::from_utf8(written)
            .unwrap()
            .contains("default = \"future default\"")
    );
}
