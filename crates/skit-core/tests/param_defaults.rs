use skit_core::{ParamDecl, synthesized_placeholder};

#[test]
fn universal_param_default_uses_absent_order_sentinel() {
    let decl = ParamDecl::default();
    assert_eq!(decl.order, -1);
    assert!(!decl.to_meta_table().contains_key("order"));
}

#[test]
fn synthesized_placeholder_does_not_invent_input_order() {
    let decl = synthesized_placeholder("target");
    assert_eq!(decl.order, -1);
    assert!(!decl.to_meta_table().contains_key("order"));
}
