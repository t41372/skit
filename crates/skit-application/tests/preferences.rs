use std::collections::BTreeMap;

use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, MirrorConfiguration,
    PreferencesChangeSet, PreferencesDraft, PreferencesError, PreferencesField,
    PreferencesSnapshot,
};
use skit_i18n::{Locale, Localize as _};

fn snapshot(mirror: MirrorConfiguration) -> PreferencesSnapshot {
    PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: Some("vim".to_owned()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names: vec!["claude".to_owned(), "codex".to_owned()],
        mirror,
    }
}

#[test]
fn fresh_preferences_expose_every_default_and_each_mirror_axis() {
    let draft = PreferencesDraft::from_snapshot(snapshot(MirrorConfiguration::default()));

    assert_eq!(draft.language, "auto");
    assert_eq!(draft.language_options, ["auto", "en", "zh-CN", "zh-TW"]);
    assert_eq!(draft.effective_language, "en");
    assert_eq!(draft.editor_fallback.as_deref(), Some("vim"));
    assert_eq!(draft.form, InteractiveFormChoice::Tui);
    assert_eq!(draft.after_run, AfterRunChoice::Exit);
    assert_eq!(draft.javascript, JavascriptChoice::Automatic);
    assert_eq!(draft.runner_names, ["claude", "codex"]);
    assert!(draft.mirror_master);
    assert_eq!(draft.pypi, MirrorChoice::Off);
    assert_eq!(draft.github, MirrorChoice::Off);
    assert_eq!(draft.npm, MirrorChoice::Off);
    assert!(!draft.custom_pypi_visible());
    assert!(!draft.custom_github_visible());
    assert!(!draft.custom_npm_visible());
    assert!(!draft.dirty());
}

#[test]
fn one_atomic_submission_resolves_presets_custom_urls_and_core_preferences() {
    let mut draft = PreferencesDraft::from_snapshot(snapshot(MirrorConfiguration::default()));
    draft.language = "zh-TW".to_owned();
    draft.editor = " code --wait ".to_owned();
    draft.form = InteractiveFormChoice::Plain;
    draft.after_run = AfterRunChoice::Stay;
    draft.javascript = JavascriptChoice::Bun;
    draft.pypi = MirrorChoice::Preset("tsinghua".to_owned());
    draft.github = MirrorChoice::Custom;
    draft.github_url = "https://mirror.example/gh/".to_owned();
    draft.npm = MirrorChoice::Preset("npmmirror".to_owned());

    let change = draft.resolve(|_| false).unwrap();

    assert_eq!(
        change.settings,
        BTreeMap::from([
            ("after_run".to_owned(), "stay".to_owned()),
            ("editor".to_owned(), "code --wait".to_owned()),
            ("form".to_owned(), "plain".to_owned()),
            ("js.runner".to_owned(), "bun".to_owned()),
            ("lang".to_owned(), "zh-TW".to_owned()),
            ("mirror".to_owned(), "on".to_owned()),
            (
                "mirror.github".to_owned(),
                "https://mirror.example/gh".to_owned(),
            ),
            ("mirror.npm".to_owned(), "npmmirror".to_owned()),
            ("mirror.pypi".to_owned(), "tsinghua".to_owned()),
        ])
    );
}

#[test]
fn custom_axes_validate_before_the_submission_can_write_any_section() {
    let mut draft = PreferencesDraft::from_snapshot(snapshot(MirrorConfiguration::default()));
    draft.editor = "micro".to_owned();
    draft.npm = MirrorChoice::Custom;
    draft.npm_url = "npm mirror".to_owned();

    let error = draft.resolve(|_| false).unwrap_err();
    assert_eq!(error.field(), PreferencesField::NpmMirror);
    assert_eq!(
        error.message().localize(Locale::En),
        "A custom choice needs a URL."
    );

    draft.npm_url = "https://npm.example".to_owned();
    draft.github = MirrorChoice::Custom;
    draft.github_url = "http://mirror.example/gh".to_owned();
    let error = draft.resolve(|_| false).unwrap_err();
    assert_eq!(error.field(), PreferencesField::GithubMirror);
    assert_eq!(
        error.message().localize(Locale::En),
        concat!(
            "The uv binary is downloaded and executed, so the github-release base URL must ",
            "use https:// (got: http://mirror.example/gh)."
        )
    );
}

#[test]
fn an_underivable_hand_edited_github_pair_survives_an_unrelated_save() {
    let original = MirrorConfiguration {
        enabled: true,
        pypi: String::new(),
        python_install: "https://one.example/python/".to_owned(),
        uv_binary: "https://two.example/uv".to_owned(),
        npm: String::new(),
    };
    let mut draft = PreferencesDraft::from_snapshot(snapshot(original));
    assert_eq!(draft.github, MirrorChoice::Custom);
    assert!(draft.github_url.is_empty());
    draft.language = "en".to_owned();

    let change = draft.resolve(|_| false).unwrap();

    assert_eq!(change.settings["lang"], "en");
    assert!(!change.settings.contains_key("mirror"));
    assert!(!change.settings.contains_key("mirror.github"));
}

#[test]
fn paused_mirror_urls_remain_visible_and_the_master_stays_off() {
    let original = MirrorConfiguration {
        enabled: false,
        pypi: "https://corp.example/simple".to_owned(),
        python_install: String::new(),
        uv_binary: String::new(),
        npm: String::new(),
    };
    let draft = PreferencesDraft::from_snapshot(snapshot(original));

    assert!(!draft.mirror_master);
    assert_eq!(draft.pypi, MirrorChoice::Custom);
    assert_eq!(draft.pypi_url, "https://corp.example/simple");
    let change = draft.resolve(|_| false).unwrap();
    assert_eq!(change.settings["mirror"], "off");
    assert_eq!(
        change.settings["mirror.pypi"],
        "https://corp.example/simple"
    );
}

#[test]
fn windows_bash_path_uses_the_same_preflight_and_never_half_submits() {
    let mut source = snapshot(MirrorConfiguration::default());
    source.bash_path = Some(String::new());
    let mut draft = PreferencesDraft::from_snapshot(source);
    draft.editor = "micro".to_owned();
    draft.bash_path = Some("C:/missing/bash.exe".to_owned());

    let error = draft
        .resolve(|path| path == std::path::Path::new("C:/valid/bash.exe"))
        .unwrap_err();
    assert_eq!(error.field(), PreferencesField::BashPath);
    assert_eq!(
        error.message().localize(Locale::En),
        "No such file: C:/missing/bash.exe"
    );
    assert!(matches!(error, PreferencesError::BashPathMissing { .. }));
}

#[test]
fn a_host_can_repeat_file_validation_after_the_reducer_preflight() {
    let change = PreferencesChangeSet {
        settings: BTreeMap::from([
            ("editor".to_owned(), "micro".to_owned()),
            (
                "shell.bash_path".to_owned(),
                "~/missing/bash.exe".to_owned(),
            ),
        ]),
    };

    assert_eq!(
        change.validate_files(|path| path == std::path::Path::new("/valid/bash.exe")),
        Err(PreferencesError::BashPathMissing {
            path: "~/missing/bash.exe".to_owned(),
        })
    );
    assert!(change.validate_files(|_| true).is_ok());
}

#[test]
fn preference_refusals_are_complete_in_both_chinese_locales() {
    let custom = PreferencesError::CustomUrlRequired {
        field: PreferencesField::NpmMirror,
    };
    assert_eq!(
        custom.message().localize(Locale::ZhCn),
        "自定义选项需要 URL。"
    );
    assert_eq!(
        custom.message().localize(Locale::ZhTw),
        "自訂選項需要 URL。"
    );

    let https = PreferencesError::GithubHttpsRequired {
        url: "http://mirror.example".to_owned(),
    };
    assert!(
        https
            .message()
            .localize(Locale::ZhCn)
            .contains("http://mirror.example")
    );
    assert!(https.message().localize(Locale::ZhTw).contains("https://"));
}
