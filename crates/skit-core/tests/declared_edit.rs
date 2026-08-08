use std::collections::{BTreeMap, BTreeSet};

use skit_core::{DeclaredEdits, Delivery, ParamDecl, ParamDefault, ParamType, edit_declared};

#[test]
fn remove_then_add_then_tweak_has_stable_order_and_warnings() {
    let initial = vec![
        ParamDecl {
            name: "old".to_owned(),
            delivery: Delivery::Flag,
            flag: "--old".to_owned(),
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "keep".to_owned(),
            delivery: Delivery::Env,
            ..ParamDecl::default()
        },
    ];
    let mut edits = DeclaredEdits {
        add: vec!["new".to_owned(), "keep".to_owned()],
        remove: vec!["old".to_owned(), "missing".to_owned()],
        ..DeclaredEdits::default()
    };
    edits.types.insert("new".to_owned(), ParamType::Integer);
    edits.flags.insert("new".to_owned(), "--new".to_owned());
    edits.required.insert("new".to_owned());

    let result = edit_declared(&initial, &edits);
    assert_eq!(
        result
            .decls
            .iter()
            .map(|decl| decl.name.as_str())
            .collect::<Vec<_>>(),
        ["keep", "new"]
    );
    let new = &result.decls[1];
    assert_eq!(new.param_type, ParamType::Integer);
    assert_eq!(new.delivery, Delivery::Flag);
    assert_eq!(new.flag, "--new");
    assert!(new.required);
    assert_eq!(
        result.warnings,
        ["not-declared:missing", "already-declared:keep"]
    );
}

#[test]
fn placeholder_add_is_required_and_uses_placeholder_delivery() {
    let edits = DeclaredEdits {
        add: vec!["target".to_owned()],
        placeholder_names: BTreeSet::from(["target".to_owned()]),
        ..DeclaredEdits::default()
    };

    let result = edit_declared(&[], &edits);
    assert_eq!(result.decls.len(), 1);
    assert_eq!(result.decls[0].delivery, Delivery::Placeholder);
    assert!(result.decls[0].required);
}

#[test]
fn choice_type_and_choices_can_be_applied_in_the_same_operation() {
    let initial = vec![ParamDecl {
        name: "format".to_owned(),
        ..ParamDecl::default()
    }];
    let mut edits = DeclaredEdits::default();
    edits.types.insert("format".to_owned(), ParamType::Choice);
    edits.choices.insert(
        "format".to_owned(),
        vec!["png".to_owned(), "jpg".to_owned()],
    );
    edits.defaults.insert("format".to_owned(), "jpg".to_owned());

    let result = edit_declared(&initial, &edits);
    assert!(result.warnings.is_empty());
    let decl = &result.decls[0];
    assert_eq!(decl.param_type, ParamType::Choice);
    assert_eq!(decl.choices, ["png", "jpg"]);
    assert_eq!(decl.default, Some(ParamDefault::String("jpg".to_owned())));
}

#[test]
fn choice_without_choices_reverts_the_whole_tweak_for_that_name() {
    let initial = vec![ParamDecl {
        name: "format".to_owned(),
        help: "keep".to_owned(),
        ..ParamDecl::default()
    }];
    let mut edits = DeclaredEdits::default();
    edits.types.insert("format".to_owned(), ParamType::Choice);
    edits.help.insert("format".to_owned(), "changed".to_owned());

    let result = edit_declared(&initial, &edits);
    assert_eq!(result.decls, initial);
    assert_eq!(result.warnings, ["choice-without-choices:format"]);
}

#[test]
fn typed_defaults_coerce_and_invalid_values_only_skip_that_default() {
    let initial = vec![
        ParamDecl {
            name: "count".to_owned(),
            param_type: ParamType::Integer,
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "ratio".to_owned(),
            param_type: ParamType::Float,
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "enabled".to_owned(),
            param_type: ParamType::Boolean,
            ..ParamDecl::default()
        },
    ];
    let mut edits = DeclaredEdits::default();
    edits.defaults.insert("count".to_owned(), "4".to_owned());
    edits.defaults.insert("ratio".to_owned(), "NaN".to_owned());
    edits
        .defaults
        .insert("enabled".to_owned(), "yes".to_owned());

    let result = edit_declared(&initial, &edits);
    assert_eq!(result.decls[0].default, Some(ParamDefault::Integer(4)));
    assert!(result.decls[1].default.is_none());
    assert_eq!(result.decls[2].default, Some(ParamDefault::Boolean(true)));
    assert_eq!(result.warnings, ["invalid-default:ratio"]);
}

#[test]
fn bool_flag_gets_store_true_but_on_by_default_is_refused() {
    let initial = vec![
        ParamDecl {
            name: "force".to_owned(),
            delivery: Delivery::Flag,
            flag: "--force".to_owned(),
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "verbose".to_owned(),
            delivery: Delivery::Flag,
            flag: "--verbose".to_owned(),
            default: Some(ParamDefault::Boolean(true)),
            ..ParamDecl::default()
        },
    ];
    let mut edits = DeclaredEdits::default();
    edits.types.insert("force".to_owned(), ParamType::Boolean);
    edits.types.insert("verbose".to_owned(), ParamType::Boolean);

    let result = edit_declared(&initial, &edits);
    assert_eq!(result.decls[0].param_type, ParamType::Boolean);
    assert_eq!(result.decls[0].action, "store_true");
    assert_eq!(result.decls[1], initial[1]);
    assert_eq!(result.warnings, ["bool-flag-on-by-default:verbose"]);
}

#[test]
fn secret_transitions_own_env_source_hygiene() {
    let initial = vec![ParamDecl {
        name: "token".to_owned(),
        delivery: Delivery::Env,
        ..ParamDecl::default()
    }];
    let mut nonsecret_env = DeclaredEdits::default();
    nonsecret_env
        .env_sources
        .insert("token".to_owned(), "API_TOKEN".to_owned());
    let result = edit_declared(&initial, &nonsecret_env);
    assert_eq!(result.decls[0], initial[0]);
    assert_eq!(result.warnings, ["env-source-not-secret:token"]);

    let mut make_secret = DeclaredEdits::default();
    make_secret.secret.insert("token".to_owned());
    make_secret
        .env_sources
        .insert("token".to_owned(), " API_TOKEN ".to_owned());
    let result = edit_declared(&initial, &make_secret);
    assert!(result.decls[0].secret);
    assert_eq!(result.decls[0].env_source, "API_TOKEN");

    let mut clear_secret = DeclaredEdits::default();
    clear_secret.no_secret.insert("token".to_owned());
    let result = edit_declared(&result.decls, &clear_secret);
    assert!(!result.decls[0].secret);
    assert!(result.decls[0].env_source.is_empty());
}

#[test]
fn delivery_changes_are_limited_to_the_allowed_set() {
    let initial = vec![ParamDecl {
        name: "x".to_owned(),
        ..ParamDecl::default()
    }];
    let mut edits = DeclaredEdits {
        allowed_deliveries: vec![Delivery::Flag, Delivery::Env],
        ..DeclaredEdits::default()
    };
    edits
        .deliveries
        .insert("x".to_owned(), Delivery::Placeholder);

    let result = edit_declared(&initial, &edits);
    assert_eq!(result.decls, initial);
    assert_eq!(result.warnings, ["invalid-delivery:x"]);
}

#[test]
fn placeholder_delivery_requires_a_real_template_placeholder() {
    let initial = vec![ParamDecl {
        name: "other".to_owned(),
        delivery: Delivery::Env,
        ..ParamDecl::default()
    }];
    let mut edits = DeclaredEdits {
        allowed_deliveries: vec![Delivery::Placeholder, Delivery::Env],
        placeholder_names: BTreeSet::from(["size".to_owned()]),
        ..DeclaredEdits::default()
    };
    edits
        .deliveries
        .insert("other".to_owned(), Delivery::Placeholder);

    let result = edit_declared(&initial, &edits);
    assert_eq!(result.decls, initial);
    assert_eq!(result.warnings, ["not-a-placeholder:other"]);
}

#[test]
fn flag_edits_trim_but_keep_empty_positional_form() {
    let initial = vec![ParamDecl {
        name: "out".to_owned(),
        delivery: Delivery::Flag,
        ..ParamDecl::default()
    }];
    let mut edits = DeclaredEdits::default();
    edits
        .flags
        .insert("out".to_owned(), "  --output  ".to_owned());
    let result = edit_declared(&initial, &edits);
    assert_eq!(result.decls[0].flag, "--output");

    let mut clear = DeclaredEdits::default();
    clear.flags.insert("out".to_owned(), "   ".to_owned());
    let result = edit_declared(&result.decls, &clear);
    assert!(result.decls[0].flag.is_empty());
}

#[test]
fn optional_and_required_flags_are_name_keyed() {
    let initial = vec![ParamDecl {
        name: "x".to_owned(),
        required: true,
        ..ParamDecl::default()
    }];
    let edits = DeclaredEdits {
        optional: BTreeSet::from(["x".to_owned()]),
        required: BTreeSet::from(["missing".to_owned()]),
        prompts: BTreeMap::from([("x".to_owned(), "Value".to_owned())]),
        ..DeclaredEdits::default()
    };

    let result = edit_declared(&initial, &edits);
    assert!(!result.decls[0].required);
    assert_eq!(result.decls[0].prompt, "Value");
    assert_eq!(result.warnings, ["not-declared:missing"]);
}
