//! Exact-name ports of frozen `tests/test_params_edit.py` at `206f9ef`.
//!
//! The declared-schema editor is one pure domain operation. Frontends collect input and render
//! its typed warnings; the engine owns operation order, partial success, and row rollback.

use skit_domain::parameters::{
    DeclaredEditContext, DeclaredEditRequest, DeclaredEditWarning, NamedEdit, ParamDecl,
    ParameterDelivery, ParameterType, ParameterValue, as_param_type, coerce_default, edit_declared,
};

fn named(values: &[(&str, &str)]) -> Vec<NamedEdit<String>> {
    values
        .iter()
        .map(|(name, value)| NamedEdit::new(*name, *value))
        .collect()
}

fn binary_context() -> DeclaredEditContext {
    DeclaredEditContext::new(
        ParameterDelivery::Flag,
        [ParameterDelivery::Env],
        Vec::<String>::new(),
    )
}

fn template_context(
    default_delivery: ParameterDelivery,
    placeholders: &[&str],
) -> DeclaredEditContext {
    DeclaredEditContext::new(
        default_delivery,
        [ParameterDelivery::Env, ParameterDelivery::Placeholder],
        placeholders.iter().map(|name| (*name).to_owned()),
    )
}

fn run(
    declarations: &[ParamDecl],
    request: DeclaredEditRequest,
) -> skit_domain::parameters::DeclaredEditResult {
    edit_declared(declarations, &request, &binary_context())
}

fn warning(name: &str, build: impl FnOnce(String) -> DeclaredEditWarning) -> DeclaredEditWarning {
    build(name.to_owned())
}

// add / remove / order

#[test]
fn test_add_defaults_to_first_allowed_delivery_for_a_binary() {
    let result = run(
        &[],
        DeclaredEditRequest {
            add: vec!["width".to_owned()],
            ..DeclaredEditRequest::default()
        },
    );
    assert!(result.warnings.is_empty());
    let declaration = &result.declarations[0];
    assert_eq!(declaration.name, "width");
    assert_eq!(declaration.delivery, ParameterDelivery::Flag);
    assert_eq!(declaration.parameter_type, ParameterType::Str);
    assert!(!declaration.required);
}

#[test]
fn test_add_on_a_template_placeholder_name_becomes_a_required_placeholder() {
    let result = edit_declared(
        &[],
        &DeclaredEditRequest {
            add: vec!["size".to_owned()],
            ..DeclaredEditRequest::default()
        },
        &template_context(ParameterDelivery::Placeholder, &["size"]),
    );
    assert_eq!(
        result.declarations[0].delivery,
        ParameterDelivery::Placeholder
    );
    assert!(result.declarations[0].required);
}

#[test]
fn test_add_non_placeholder_name_on_a_template_uses_first_allowed_delivery() {
    let result = edit_declared(
        &[],
        &DeclaredEditRequest {
            add: vec!["RETRIES".to_owned()],
            ..DeclaredEditRequest::default()
        },
        &template_context(ParameterDelivery::Placeholder, &["size"]),
    );
    assert_eq!(
        result.declarations[0].delivery,
        ParameterDelivery::Placeholder
    );
    assert!(!result.declarations[0].required);
}

#[test]
fn test_add_existing_name_warns_already_declared() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            add: vec!["a".to_owned()],
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [warning("a", |name| DeclaredEditWarning::AlreadyDeclared {
            name
        })]
    );
    assert_eq!(result.declarations.len(), 1);
}

#[test]
fn test_rm_drops_the_row() {
    let result = run(
        &[ParamDecl::new("a"), ParamDecl::new("b")],
        DeclaredEditRequest {
            remove: vec!["a".to_owned()],
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(result.declarations, [ParamDecl::new("b")]);
}

#[test]
fn test_rm_unknown_name_warns_not_declared() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            remove: vec!["ghost".to_owned()],
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [warning("ghost", |name| DeclaredEditWarning::NotDeclared {
            name
        })]
    );
    assert_eq!(result.declarations, [ParamDecl::new("a")]);
}

#[test]
fn test_apply_order_is_rm_then_add_then_tweak() {
    let original = ParamDecl {
        parameter_type: ParameterType::Int,
        ..ParamDecl::new("a")
    };
    let result = run(
        &[original],
        DeclaredEditRequest {
            remove: vec!["a".to_owned()],
            add: vec!["a".to_owned()],
            parameter_types: named(&[("a", "float")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(result.declarations[0].parameter_type, ParameterType::Float);
}

#[test]
fn test_inputs_are_never_mutated() {
    let original = ParamDecl {
        prompt: "orig".to_owned(),
        ..ParamDecl::new("a")
    };
    let _ = run(
        std::slice::from_ref(&original),
        DeclaredEditRequest {
            prompts: named(&[("a", "changed")]),
            secret: vec!["a".to_owned()],
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(original.prompt, "orig");
    assert!(!original.secret);
}

// tweaks

#[test]
fn test_delivery_tweak_within_allowed_set() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            deliveries: named(&[("a", "env")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(result.declarations[0].delivery, ParameterDelivery::Env);
}

#[test]
fn test_delivery_outside_allowed_set_warns_bad_delivery() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            deliveries: named(&[("a", "placeholder")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [warning("a", |name| DeclaredEditWarning::BadDelivery {
            name
        })]
    );
    assert_eq!(result.declarations[0].delivery, ParameterDelivery::Flag);
}

#[test]
fn test_placeholder_delivery_on_a_non_placeholder_name_warns() {
    let declaration = ParamDecl {
        delivery: ParameterDelivery::Env,
        ..ParamDecl::new("a")
    };
    let result = edit_declared(
        &[declaration],
        &DeclaredEditRequest {
            deliveries: named(&[("a", "placeholder")]),
            ..DeclaredEditRequest::default()
        },
        &template_context(ParameterDelivery::Env, &["other"]),
    );
    assert_eq!(
        result.warnings,
        [warning("a", |name| DeclaredEditWarning::NotAPlaceholder {
            name
        })]
    );
    assert_eq!(result.declarations[0].delivery, ParameterDelivery::Env);
}

#[test]
fn test_placeholder_delivery_on_a_matching_placeholder_name_is_allowed() {
    let declaration = ParamDecl {
        delivery: ParameterDelivery::Env,
        ..ParamDecl::new("size")
    };
    let result = edit_declared(
        &[declaration],
        &DeclaredEditRequest {
            deliveries: named(&[("size", "placeholder")]),
            ..DeclaredEditRequest::default()
        },
        &template_context(ParameterDelivery::Env, &["size"]),
    );
    assert_eq!(
        result.declarations[0].delivery,
        ParameterDelivery::Placeholder
    );
}

#[test]
fn test_type_tweak_valid() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            parameter_types: named(&[("a", "int")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(result.declarations[0].parameter_type, ParameterType::Int);
}

#[test]
fn test_type_tweak_invalid_warns_bad_type() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            parameter_types: named(&[("a", "integer")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [warning("a", |name| DeclaredEditWarning::BadType { name })]
    );
    assert_eq!(result.declarations[0].parameter_type, ParameterType::Str);
}

#[test]
fn test_choices_tweak_sets_the_tuple() {
    let declaration = ParamDecl {
        parameter_type: ParameterType::Choice,
        ..ParamDecl::new("a")
    };
    let result = run(
        &[declaration],
        DeclaredEditRequest {
            choices: vec![NamedEdit::new("a", vec!["x".to_owned(), "y".to_owned()])],
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(result.declarations[0].choices, ["x", "y"]);
}

#[test]
fn test_default_coerced_to_the_declared_type() {
    let declaration = ParamDecl {
        parameter_type: ParameterType::Int,
        ..ParamDecl::new("a")
    };
    let result = run(
        &[declaration],
        DeclaredEditRequest {
            defaults: named(&[("a", "42")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.declarations[0].default,
        Some(ParameterValue::Integer(42))
    );
}

#[test]
fn test_default_type_set_in_same_call_applies_before_coercion() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            parameter_types: named(&[("a", "float")]),
            defaults: named(&[("a", "1.5")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.declarations[0].default,
        Some(ParameterValue::Float(1.5))
    );
}

#[test]
fn test_default_bad_value_warns_bad_default_and_keeps_old() {
    let declaration = ParamDecl {
        parameter_type: ParameterType::Int,
        default: Some(ParameterValue::Integer(3)),
        ..ParamDecl::new("a")
    };
    let result = run(
        &[declaration],
        DeclaredEditRequest {
            defaults: named(&[("a", "notanint")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [warning("a", |name| DeclaredEditWarning::BadDefault {
            name
        })]
    );
    assert_eq!(
        result.declarations[0].default,
        Some(ParameterValue::Integer(3))
    );
}

#[test]
fn test_flag_tweak_strips_and_sets_empty_for_positional() {
    let first = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            flags: named(&[("a", "  --out ")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(first.declarations[0].flag, "--out");
    let second = run(
        &[ParamDecl {
            flag: "--out".to_owned(),
            ..ParamDecl::new("a")
        }],
        DeclaredEditRequest {
            flags: named(&[("a", "")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert!(second.declarations[0].flag.is_empty());
}

#[test]
fn test_required_and_optional_tweaks() {
    let required = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            required: vec!["a".to_owned()],
            ..DeclaredEditRequest::default()
        },
    );
    assert!(required.declarations[0].required);
    let optional = run(
        &[ParamDecl {
            required: true,
            ..ParamDecl::new("a")
        }],
        DeclaredEditRequest {
            optional: vec!["a".to_owned()],
            ..DeclaredEditRequest::default()
        },
    );
    assert!(!optional.declarations[0].required);
}

#[test]
fn test_help_text_and_prompt_tweaks() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            help: named(&[("a", "what it does")]),
            prompts: named(&[("a", "A?")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(result.declarations[0].help, "what it does");
    assert_eq!(result.declarations[0].prompt, "A?");
}

#[test]
fn test_secret_and_env_source_together() {
    let result = run(
        &[ParamDecl::new("tok")],
        DeclaredEditRequest {
            secret: vec!["tok".to_owned()],
            env_sources: named(&[("tok", " API_TOKEN ")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert!(result.declarations[0].secret);
    assert_eq!(result.declarations[0].env_source, "API_TOKEN");
}

#[test]
fn test_env_source_on_a_non_secret_param_warns_and_leaves_it_unset() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            env_sources: named(&[("a", "VAR")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert!(result.declarations[0].env_source.is_empty());
    assert_eq!(
        result.warnings,
        [warning("a", |name| {
            DeclaredEditWarning::EnvSourceNotSecret { name }
        })]
    );
}

#[test]
fn test_no_secret_clears_the_env_source() {
    let result = run(
        &[ParamDecl {
            secret: true,
            env_source: "API_TOKEN".to_owned(),
            ..ParamDecl::new("tok")
        }],
        DeclaredEditRequest {
            no_secret: vec!["tok".to_owned()],
            ..DeclaredEditRequest::default()
        },
    );
    assert!(!result.declarations[0].secret);
    assert!(result.declarations[0].env_source.is_empty());
}

#[test]
fn test_tweak_on_unknown_name_warns_not_declared() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            parameter_types: named(&[("ghost", "int")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [warning("ghost", |name| DeclaredEditWarning::NotDeclared {
            name
        })]
    );
}

#[test]
fn test_a_name_touched_by_two_ops_is_listed_once_and_both_apply() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            parameter_types: named(&[("a", "int")]),
            defaults: named(&[("a", "5")]),
            secret: vec!["a".to_owned()],
            prompts: named(&[("a", "A?")]),
            ..DeclaredEditRequest::default()
        },
    );
    let declaration = &result.declarations[0];
    assert_eq!(declaration.parameter_type, ParameterType::Int);
    assert_eq!(declaration.default, Some(ParameterValue::Integer(5)));
    assert!(declaration.secret);
    assert_eq!(declaration.prompt, "A?");
}

// rollback on invalid

#[test]
fn test_choice_type_without_choices_reverts_and_warns() {
    let original = ParamDecl {
        help: "keep me".to_owned(),
        ..ParamDecl::new("a")
    };
    let result = run(
        std::slice::from_ref(&original),
        DeclaredEditRequest {
            parameter_types: named(&[("a", "choice")]),
            help: named(&[("a", "changed")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [warning("a", |name| {
            DeclaredEditWarning::ChoiceWithoutChoices { name }
        })]
    );
    assert_eq!(result.declarations, [original]);
}

#[test]
fn test_choice_type_with_choices_in_the_same_call_is_valid() {
    let result = run(
        &[ParamDecl::new("a")],
        DeclaredEditRequest {
            parameter_types: named(&[("a", "choice")]),
            choices: vec![NamedEdit::new("a", vec!["r".to_owned(), "g".to_owned()])],
            ..DeclaredEditRequest::default()
        },
    );
    assert!(result.warnings.is_empty());
    assert_eq!(result.declarations[0].parameter_type, ParameterType::Choice);
    assert_eq!(result.declarations[0].choices, ["r", "g"]);
}

// bool flag action hygiene

#[test]
fn test_type_tweak_to_bool_on_a_flag_sets_store_true() {
    let result = run(
        &[ParamDecl {
            flag: "--v".to_owned(),
            ..ParamDecl::new("v")
        }],
        DeclaredEditRequest {
            parameter_types: named(&[("v", "bool")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(result.declarations[0].action, "store_true");
}

#[test]
fn test_type_tweak_to_bool_on_a_positional_keeps_empty_action() {
    let result = run(
        &[ParamDecl::new("b")],
        DeclaredEditRequest {
            parameter_types: named(&[("b", "bool")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert!(result.declarations[0].action.is_empty());
}

#[test]
fn test_type_tweak_to_bool_on_env_delivery_keeps_empty_action() {
    let result = run(
        &[ParamDecl {
            delivery: ParameterDelivery::Env,
            flag: "--v".to_owned(),
            ..ParamDecl::new("v")
        }],
        DeclaredEditRequest {
            parameter_types: named(&[("v", "bool")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert!(result.declarations[0].action.is_empty());
}

#[test]
fn test_type_tweak_off_bool_sheds_stale_action() {
    let result = run(
        &[ParamDecl {
            parameter_type: ParameterType::Bool,
            flag: "--v".to_owned(),
            action: "store_true".to_owned(),
            ..ParamDecl::new("v")
        }],
        DeclaredEditRequest {
            parameter_types: named(&[("v", "str")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert!(result.declarations[0].action.is_empty());
}

#[test]
fn test_non_type_tweak_on_a_bool_leaves_its_action_alone() {
    let result = run(
        &[ParamDecl {
            parameter_type: ParameterType::Bool,
            flag: "--c".to_owned(),
            action: "store_false".to_owned(),
            ..ParamDecl::new("c")
        }],
        DeclaredEditRequest {
            defaults: named(&[("c", "true")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(result.declarations[0].action, "store_false");
}

#[test]
fn test_non_type_tweak_on_a_str_with_stale_action_clears_it() {
    let result = run(
        &[ParamDecl {
            flag: "--a".to_owned(),
            action: "store_true".to_owned(),
            ..ParamDecl::new("a")
        }],
        DeclaredEditRequest {
            help: named(&[("a", "x")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert!(result.declarations[0].action.is_empty());
}

// coerce_default

#[test]
fn test_coerce_default_success() {
    let cases = [
        ("42", ParameterType::Int, ParameterValue::Integer(42)),
        ("3.5", ParameterType::Float, ParameterValue::Float(3.5)),
        ("true", ParameterType::Bool, ParameterValue::Bool(true)),
        ("YES", ParameterType::Bool, ParameterValue::Bool(true)),
        ("on", ParameterType::Bool, ParameterValue::Bool(true)),
        ("false", ParameterType::Bool, ParameterValue::Bool(false)),
        ("0", ParameterType::Bool, ParameterValue::Bool(false)),
        ("off", ParameterType::Bool, ParameterValue::Bool(false)),
        (
            "anything",
            ParameterType::Str,
            ParameterValue::String("anything".to_owned()),
        ),
        (
            "anything",
            ParameterType::Choice,
            ParameterValue::String("anything".to_owned()),
        ),
    ];
    for (value, parameter_type, expected) in cases {
        assert_eq!(coerce_default(value, parameter_type), Ok(expected));
    }
}

#[test]
fn test_coerce_default_rejects_bad_values() {
    for (value, parameter_type) in [
        ("x", ParameterType::Int),
        ("x", ParameterType::Float),
        ("maybe", ParameterType::Bool),
        ("inf", ParameterType::Float),
        ("nan", ParameterType::Float),
    ] {
        assert!(coerce_default(value, parameter_type).is_err());
    }
}

#[test]
fn test_coerce_default_rejects_infinity_specifically() {
    assert!("1e999".parse::<f64>().unwrap().is_infinite());
    assert!(coerce_default("1e999", ParameterType::Float).is_err());
}

// as_param_type

#[test]
fn test_as_param_type_accepts_the_five() {
    for (value, expected) in [
        ("str", ParameterType::Str),
        ("int", ParameterType::Int),
        ("float", ParameterType::Float),
        ("bool", ParameterType::Bool),
        ("choice", ParameterType::Choice),
        ("path", ParameterType::Path),
    ] {
        assert_eq!(as_param_type(value), Some(expected));
    }
}

#[test]
fn test_as_param_type_rejects_others() {
    for value in ["integer", "", "STR", "number"] {
        assert_eq!(as_param_type(value), None);
    }
}

#[test]
fn test_bool_flag_that_is_on_by_default_is_refused_not_stamped() {
    let original = ParamDecl {
        flag: "--verbose".to_owned(),
        ..ParamDecl::new("verbose")
    };
    let result = run(
        std::slice::from_ref(&original),
        DeclaredEditRequest {
            parameter_types: named(&[("verbose", "bool")]),
            defaults: named(&[("verbose", "true")]),
            ..DeclaredEditRequest::default()
        },
    );
    assert_eq!(
        result.warnings,
        [warning("verbose", |name| {
            DeclaredEditWarning::BoolFlagOnByDefault { name }
        })]
    );
    assert_eq!(result.declarations, [original]);
}

#[test]
fn test_bool_flag_that_is_off_by_default_still_gets_store_true() {
    let result = run(
        &[ParamDecl {
            flag: "--verbose".to_owned(),
            ..ParamDecl::new("verbose")
        }],
        DeclaredEditRequest {
            parameter_types: named(&[("verbose", "bool")]),
            defaults: named(&[("verbose", "false")]),
            ..DeclaredEditRequest::default()
        },
    );
    let declaration = &result.declarations[0];
    assert!(result.warnings.is_empty());
    assert_eq!(declaration.parameter_type, ParameterType::Bool);
    assert_eq!(declaration.default, Some(ParameterValue::Bool(false)));
    assert_eq!(declaration.action, "store_true");
}
