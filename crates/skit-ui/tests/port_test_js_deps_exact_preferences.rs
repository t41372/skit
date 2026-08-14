//! Exact npm Preferences port from Python v0.4 `tests/test_js_deps.py`.

use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, MirrorConfiguration,
    PreferencesDraft, PreferencesField, PreferencesSnapshot,
};
use skit_ui::{
    PreferencesAction, PreferencesControlId, PreferencesControlKind, PreferencesEffect,
    PreferencesView,
};

fn view() -> PreferencesView {
    PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: None,
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names: Vec::new(),
        mirror: MirrorConfiguration::default(),
    }))
}

#[test]
fn test_prefs_custom_mirror_saves_the_npm_registry() {
    let mut view = view();
    assert_eq!(
        view.update(PreferencesAction::ChooseMirror {
            field: PreferencesField::NpmMirror,
            choice: MirrorChoice::Custom,
        }),
        PreferencesEffect::None
    );
    assert!(view.draft().custom_npm_visible());
    assert!(view.controls().iter().any(|control| {
        control.id == PreferencesControlId::NpmUrl
            && matches!(control.kind, PreferencesControlKind::Text(_))
    }));

    assert_eq!(
        view.update(PreferencesAction::SetMirrorUrl {
            field: PreferencesField::NpmMirror,
            value: "https://npm.example".to_owned(),
        }),
        PreferencesEffect::None
    );
    let PreferencesEffect::Save(changes) = view.update(PreferencesAction::Save) else {
        panic!("saving a valid custom npm mirror must emit one atomic Preferences save");
    };
    assert_eq!(
        changes.settings.get("mirror.npm").map(String::as_str),
        Some("https://npm.example")
    );
    assert_eq!(
        changes.settings.get("mirror.pypi").map(String::as_str),
        Some("")
    );
    assert_eq!(
        changes.settings.get("mirror.enabled").map(String::as_str),
        Some("true")
    );
}