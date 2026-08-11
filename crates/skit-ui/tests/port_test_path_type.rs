//! Exact frontend-neutral ports of Python v0.4 `tests/test_path_type.py` form contracts.
//!
//! The test uses the public typed run-form model rather than inferring control shape from rendered
//! text. A degraded path must become ordinary free text just as Python's `FormField.kind == "str"`.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_ui::{FormControl, FormInputKind, RunFormView};

fn view(declaration: ParamDecl) -> RunFormView {
    RunFormView::from_declarations(
        "entry",
        "entry",
        &[declaration],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
}

fn path_control_kind(declaration: ParamDecl) -> FormInputKind {
    let view = view(declaration);
    let parameter = view
        .fields()
        .iter()
        .find(|field| field.key == "value:src")
        .expect("path parameter field");
    let FormControl::Text(control) = &parameter.control else {
        panic!("path must be represented by a text control: {parameter:?}");
    };
    control.kind
}

#[test]
fn test_formfield_carries_path_for_every_delivery() {
    let cases = [
        (ParameterDelivery::Inject, ParameterBinding::Const),
        (ParameterDelivery::Flag, ParameterBinding::None),
        (ParameterDelivery::Env, ParameterBinding::EnvDefault),
        (ParameterDelivery::Placeholder, ParameterBinding::None),
    ];
    for (delivery, binding) in cases {
        let mut declaration = ParamDecl::new("src");
        declaration.parameter_type = ParameterType::Path;
        declaration.delivery = delivery;
        declaration.binding = binding;
        let form = view(declaration);
        let parameter = form
            .fields()
            .iter()
            .find(|field| field.key == "value:src")
            .unwrap();
        assert_eq!(
            parameter.parameter_type,
            ParameterType::Path,
            "{delivery:?}"
        );
        let FormControl::Text(control) = &parameter.control else {
            panic!("path must stay a text-shaped control for {delivery:?}: {parameter:?}");
        };
        assert_eq!(control.kind, FormInputKind::Path, "{delivery:?}");
    }
}

#[test]
fn test_degraded_flag_field_still_renders_free_text() {
    let mut declaration = ParamDecl::new("src");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Path;
    declaration.degraded = true;
    assert_eq!(
        path_control_kind(declaration),
        FormInputKind::Text,
        "a degraded path is unmodeled free text; keeping path-specific UI invents structure"
    );
}

#[test]
fn test_type_label_path() {
    let mut declaration = ParamDecl::new("src");
    declaration.parameter_type = ParameterType::Path;
    let form = view(declaration);
    let parameter = form
        .fields()
        .iter()
        .find(|field| field.key == "value:src")
        .unwrap();
    assert_eq!(parameter.parameter_type.as_str(), "path");
    assert_eq!(
        path_control_kind(ParamDecl {
            parameter_type: ParameterType::Path,
            ..ParamDecl::new("src")
        }),
        FormInputKind::Path
    );
}
