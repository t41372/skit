use std::collections::BTreeMap;

use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, MirrorConfiguration,
    PreferencesChangeSet, PreferencesDraft, PreferencesError, PreferencesField,
    PreferencesSnapshot, github_preset_names, npm_preset_names, pypi_preset_names,
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
    assert_eq!(pypi_preset_names(), ["tsinghua", "aliyun", "ustc"]);
    assert_eq!(github_preset_names(), ["nju"]);
    assert_eq!(npm_preset_names(), ["npmmirror"]);
}

#[test]
fn stored_language_and_every_javascript_choice_round_trip_to_settings() {
    let mut source = snapshot(MirrorConfiguration::default());
    source.language = "fr".to_owned();
    source.available_languages = vec!["en".to_owned(), "en".to_owned()];
    let draft = PreferencesDraft::from_snapshot(source);
    assert_eq!(draft.language, "fr");
    assert_eq!(draft.language_options, ["auto", "en", "fr"]);

    for (choice, expected) in [
        (JavascriptChoice::Automatic, ""),
        (JavascriptChoice::Deno, "deno"),
        (JavascriptChoice::Bun, "bun"),
        (JavascriptChoice::Node, "node"),
    ] {
        let mut draft = draft.clone();
        draft.javascript = choice;
        assert_eq!(
            draft.resolve(|_| false).unwrap().settings["js.runner"],
            expected
        );
    }
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
fn github_presets_resolve_and_unknown_presets_are_refused_by_their_axis() {
    let mut draft = PreferencesDraft::from_snapshot(snapshot(MirrorConfiguration::default()));
    draft.github = MirrorChoice::Preset("nju".to_owned());
    let change = draft.resolve(|_| false).unwrap();
    assert_eq!(change.settings["mirror.github"], "nju");
    assert_eq!(change.settings["mirror"], "on");

    let mut draft = PreferencesDraft::from_snapshot(snapshot(MirrorConfiguration::default()));
    draft.pypi = MirrorChoice::Preset("missing".to_owned());
    assert_eq!(
        draft.resolve(|_| false),
        Err(PreferencesError::CustomUrlRequired {
            field: PreferencesField::PypiMirror,
        })
    );

    let mut draft = PreferencesDraft::from_snapshot(snapshot(MirrorConfiguration::default()));
    draft.github = MirrorChoice::Preset("missing".to_owned());
    assert_eq!(
        draft.resolve(|_| false),
        Err(PreferencesError::CustomUrlRequired {
            field: PreferencesField::GithubMirror,
        })
    );

    let mut draft = PreferencesDraft::from_snapshot(snapshot(MirrorConfiguration::default()));
    draft.npm = MirrorChoice::Preset("missing".to_owned());
    assert_eq!(
        draft.resolve(|_| false),
        Err(PreferencesError::CustomUrlRequired {
            field: PreferencesField::NpmMirror,
        })
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

    draft.github_url = "https://mirror.example/with space".to_owned();
    assert_eq!(
        draft.resolve(|_| false),
        Err(PreferencesError::CustomUrlRequired {
            field: PreferencesField::GithubMirror,
        })
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
fn an_uv_only_legacy_pair_passes_through_and_a_mismatched_derived_pair_has_no_base() {
    let uv_only = MirrorConfiguration {
        enabled: true,
        pypi: String::new(),
        python_install: String::new(),
        uv_binary: "https://legacy.example/uv".to_owned(),
        npm: String::new(),
    };
    let mut draft = PreferencesDraft::from_snapshot(snapshot(uv_only));
    assert_eq!(draft.github, MirrorChoice::Custom);
    assert!(draft.github_url.is_empty());
    draft.editor = "micro".to_owned();
    let change = draft.resolve(|_| false).unwrap();
    assert!(!change.settings.contains_key("mirror.github"));

    let mismatched = MirrorConfiguration {
        enabled: true,
        pypi: String::new(),
        python_install: "https://other.example/python/".to_owned(),
        uv_binary: "https://mirror.example/astral-sh/uv".to_owned(),
        npm: String::new(),
    };
    let draft = PreferencesDraft::from_snapshot(snapshot(mismatched));
    assert_eq!(draft.github, MirrorChoice::Custom);
    assert!(draft.github_url.is_empty());

    let matching = MirrorConfiguration {
        enabled: true,
        pypi: String::new(),
        python_install:
            "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/".to_owned(),
        uv_binary: "https://mirror.nju.edu.cn/github-release/astral-sh/uv".to_owned(),
        npm: String::new(),
    };
    let draft = PreferencesDraft::from_snapshot(snapshot(matching));
    assert_eq!(draft.github, MirrorChoice::Preset("nju".to_owned()));
    assert_eq!(draft.github_url, "https://mirror.nju.edu.cn/github-release");
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
    for invalid in ["C:/missing/bash.exe", "C:/directory"] {
        let mut source = snapshot(MirrorConfiguration::default());
        source.bash_path = Some(String::new());
        let mut draft = PreferencesDraft::from_snapshot(source);
        draft.editor = "micro".to_owned();
        draft.bash_path = Some(invalid.to_owned());

        let error = draft
            .resolve(|path| path == std::path::Path::new("C:/valid/bash.exe"))
            .unwrap_err();
        assert_eq!(error.field(), PreferencesField::BashPath);
        assert_eq!(
            error.message().localize(Locale::En),
            format!("No such file: {invalid}")
        );
        assert_eq!(
            error,
            PreferencesError::BashPathMissing {
                path: invalid.to_owned()
            }
        );
    }
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

    let clear = PreferencesChangeSet {
        settings: BTreeMap::from([
            ("editor".to_owned(), "micro".to_owned()),
            ("shell.bash_path".to_owned(), " \t ".to_owned()),
        ]),
    };
    assert!(
        clear
            .validate_files(|_| panic!("empty clear must not query the filesystem"))
            .is_ok()
    );
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
