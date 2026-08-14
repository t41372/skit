//! Exact frontend-neutral settings ports from Python v0.4 `tests/test_js_deps.py`.
//!
//! Python exercised these through Textual widgets. Rust deliberately exposes the same screen state
//! and save payload as frontend-neutral data; these tests keep the field visibility and typed save
//! contracts rather than reducing them to a string helper.

use skit_ui::{
    DEPENDENCIES_KEY, PYTHON_KEY, DependencyFlavor, FieldValue, SettingsInputs, SettingsSectionId,
    SettingsView, TypedValue,
};

fn npm_inputs(reference_mode: bool) -> SettingsInputs {
    SettingsInputs {
        selector: "t".to_owned(),
        kind: "js".to_owned(),
        name: "t".to_owned(),
        source: "/work/t.mjs".to_owned(),
        reference_mode,
        supports_modes: true,
        has_original_file: reference_mode,
        has_stored_name: !reference_mode,
        dependency_flavor: Some(DependencyFlavor::Npm),
        effective_dependencies: Vec::new(),
        ..SettingsInputs::default()
    }
}

#[test]
fn test_settings_js_copy_offers_deps_without_python_constraint() {
    let mut view = SettingsView::from_inputs(&npm_inputs(false));
    assert!(view.has_section(SettingsSectionId::Dependencies));
    assert!(view.field(DEPENDENCIES_KEY).is_some());
    assert!(view.field(PYTHON_KEY).is_none());

    assert!(view.set_value(
        DEPENDENCIES_KEY,
        FieldValue::text("chalk@^5, zod"),
    ));
    assert_eq!(
        view.dependencies_edit(),
        Some(vec!["chalk@^5".to_owned(), "zod".to_owned()])
    );
    assert_eq!(
        view.submitted_values().get(DEPENDENCIES_KEY),
        Some(&FieldValue::Explicit(TypedValue::Arguments(vec![
            "chalk@^5".to_owned(),
            "zod".to_owned(),
        ])))
    );
}

#[test]
fn test_settings_js_reference_hides_the_deps_section() {
    let view = SettingsView::from_inputs(&npm_inputs(true));
    assert!(!view.has_section(SettingsSectionId::Dependencies));
    assert!(view.field(DEPENDENCIES_KEY).is_none());
    assert!(view.field(PYTHON_KEY).is_none());
    assert!(view.submitted_values().is_empty());
}

#[test]
fn test_split_requirements_keeps_scoped_packages_apart() {
    for (typed, expected) in [
        (
            "chalk, @aws-sdk/client-s3",
            vec!["chalk", "@aws-sdk/client-s3"],
        ),
        (
            " zod@^3 ,, @trpc/server@10 , ",
            vec!["zod@^3", "@trpc/server@10"],
        ),
        ("", Vec::<&str>::new()),
    ] {
        let mut view = SettingsView::from_inputs(&npm_inputs(false));
        assert!(view.set_value(DEPENDENCIES_KEY, FieldValue::text(typed)));
        assert_eq!(
            view.dependencies_edit(),
            Some(expected.into_iter().map(str::to_owned).collect()),
            "typed={typed:?}"
        );
    }
}

#[test]
fn test_settings_save_keeps_scoped_packages_apart() {
    let mut view = SettingsView::from_inputs(&npm_inputs(false));
    assert!(view.set_value(
        DEPENDENCIES_KEY,
        FieldValue::text("chalk, @aws-sdk/client-s3"),
    ));
    let submitted = view.submitted_values();
    assert_eq!(
        submitted.get(DEPENDENCIES_KEY),
        Some(&FieldValue::Explicit(TypedValue::Arguments(vec![
            "chalk".to_owned(),
            "@aws-sdk/client-s3".to_owned(),
        ])))
    );
}