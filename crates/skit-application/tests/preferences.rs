use std::collections::BTreeMap;

use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, MirrorConfiguration,
    PreferencesChangeSet, PreferencesDraft, PreferencesError, PreferencesField,
    PreferencesSnapshot, RunnerChange, RunnerDraftError, RunnerDraftMarker, RunnerDraftRow,
    RunnerDraftState, ThemeChoice, github_preset_names, npm_preset_names, pypi_preset_names,
    runner_row_taken_by_its_key,
};
use skit_application::runner_management::{
    RunnerRow, RunnerRowIdentity, RunnerSaveRequest, RunnerSaveTarget,
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
        theme: ThemeChoice::Terminal,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runners: vec![runner_row(0, "claude", &["claude", "-p", "{{prompt}}"], 2)],
        mirror,
    }
}

#[test]
fn any_single_mirror_url_counts_as_configured() {
    // The master switch reads "no URL anywhere". Each field alone must therefore answer that
    // question on its own, so a draft that carries one URL and is switched off is not a fresh one.
    for (name, mirror) in [
        (
            "pypi",
            MirrorConfiguration {
                pypi: "https://pypi.example/simple".to_owned(),
                ..MirrorConfiguration::default()
            },
        ),
        (
            "python_install",
            MirrorConfiguration {
                python_install: "https://python.example/".to_owned(),
                ..MirrorConfiguration::default()
            },
        ),
        (
            "uv_binary",
            MirrorConfiguration {
                uv_binary: "https://uv.example/".to_owned(),
                ..MirrorConfiguration::default()
            },
        ),
        (
            "npm",
            MirrorConfiguration {
                npm: "https://npm.example/".to_owned(),
                ..MirrorConfiguration::default()
            },
        ),
    ] {
        let draft = PreferencesDraft::from_snapshot(snapshot(mirror));
        assert!(!draft.mirror_master, "{name} alone must count as a URL");
    }

    // With nothing set anywhere the switch is on, which is the fresh state.
    let empty = PreferencesDraft::from_snapshot(snapshot(MirrorConfiguration::default()));
    assert!(empty.mirror_master);
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
    assert_eq!(draft.runner_rows().len(), 1);
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
        runners: Vec::new(),
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
        runners: Vec::new(),
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

fn identity(index: Option<usize>, token: &str) -> RunnerRowIdentity {
    RunnerRowIdentity {
        index,
        snapshot_token: token.to_owned(),
    }
}

fn runner_row(index: usize, name: &str, argv: &[&str], pinned_count: usize) -> RunnerRow {
    let identity = identity(Some(index), &format!("token-{index}"));
    RunnerRow {
        key_identities: vec![identity.clone()],
        identity,
        name: Some(name.to_owned()),
        argv: Some(argv.iter().map(|value| (*value).to_owned()).collect()),
        reason: None,
        descriptor: format!("prompt.runners[{index}]"),
        pinned_count,
    }
}

fn malformed_row(index: Option<usize>, argv: Option<&[&str]>) -> RunnerRow {
    let identity = identity(index, &format!("token-malformed-{index:?}"));
    RunnerRow {
        identity,
        name: None,
        argv: argv.map(|argv| argv.iter().map(|value| (*value).to_owned()).collect()),
        reason: Some("name".to_owned()),
        descriptor: "prompt.runners[9]".to_owned(),
        key_identities: Vec::new(),
        pinned_count: 0,
    }
}

fn draft_with(runners: Vec<RunnerRow>) -> PreferencesDraft {
    PreferencesDraft::from_snapshot(PreferencesSnapshot {
        runners,
        ..snapshot(MirrorConfiguration::default())
    })
}

fn save(name: &str, argv: &[&str], target: RunnerSaveTarget) -> RunnerSaveRequest {
    RunnerSaveRequest {
        name: name.to_owned(),
        argv: argv.iter().map(|value| (*value).to_owned()).collect(),
        target,
    }
}

#[test]
fn a_fresh_agent_list_is_clean_and_reports_every_stored_row() {
    let draft = draft_with(vec![
        runner_row(0, "claude", &["claude", "-p", "{{prompt}}"], 2),
        runner_row(1, "codex", &["codex", "exec", "{{prompt}}"], 0),
    ]);

    assert!(!draft.dirty());
    assert_eq!(draft.runner_rows().len(), 2);
    assert_eq!(draft.runner_rows()[0].name(), Some("claude"));
    assert_eq!(
        draft.runner_rows()[1].argv(),
        Some(
            [
                "codex".to_owned(),
                "exec".to_owned(),
                "{{prompt}}".to_owned()
            ]
            .as_slice()
        )
    );
    assert_eq!(draft.runner_rows()[0].marker(), None);
    assert!(!draft.runner_rows()[0].is_removed());
    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        Vec::<RunnerChange>::new()
    );
}

#[test]
fn staging_an_add_an_edit_and_a_removal_marks_the_draft_dirty() {
    let mut draft = draft_with(vec![
        runner_row(0, "claude", &["claude", "-p", "{{prompt}}"], 2),
        runner_row(1, "codex", &["codex", "exec", "{{prompt}}"], 0),
    ]);

    draft
        .stage_runner(
            save(
                "codex",
                &["codex", "exec", "--full", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "codex".to_owned(),
                    expected: vec![identity(Some(1), "token-1")],
                },
            ),
            None,
        )
        .unwrap();
    draft
        .stage_runner(
            save(
                "my-agent",
                &["my-agent", "{{prompt}}"],
                RunnerSaveTarget::New,
            ),
            None,
        )
        .unwrap();
    draft.toggle_runner_removal(0).unwrap();

    assert!(draft.dirty());
    assert_eq!(
        draft
            .runner_rows()
            .iter()
            .map(RunnerDraftRow::marker)
            .collect::<Vec<_>>(),
        [
            Some(RunnerDraftMarker::Removed),
            Some(RunnerDraftMarker::Edited),
            Some(RunnerDraftMarker::Added),
        ]
    );
    assert!(draft.runner_rows()[0].is_removed());
    assert_eq!(draft.runner_rows()[2].name(), Some("my-agent"));
}

#[test]
fn resolve_emits_every_removal_before_repairs_edits_and_adds() {
    let mut draft = draft_with(vec![
        runner_row(0, "claude", &["claude", "-p", "{{prompt}}"], 2),
        runner_row(1, "codex", &["codex", "exec", "{{prompt}}"], 0),
        malformed_row(Some(2), Some(&["broken", "{{prompt}}"])),
        malformed_row(None, None),
    ]);

    draft
        .stage_runner(
            save(
                "new-agent",
                &["new-agent", "{{prompt}}"],
                RunnerSaveTarget::New,
            ),
            None,
        )
        .unwrap();
    draft
        .stage_runner(
            save(
                "codex",
                &["codex", "exec", "--full", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "codex".to_owned(),
                    expected: vec![identity(Some(1), "token-1")],
                },
            ),
            None,
        )
        .unwrap();
    draft
        .stage_runner(
            save(
                "repaired",
                &["repaired", "{{prompt}}"],
                RunnerSaveTarget::RawRow {
                    expected: identity(Some(2), "token-malformed-Some(2)"),
                },
            ),
            None,
        )
        .unwrap();
    draft.toggle_runner_removal(0).unwrap();
    draft.toggle_runner_removal(3).unwrap();

    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        [
            RunnerChange::RemoveNamed {
                name: "claude".to_owned(),
                expected: vec![identity(Some(0), "token-0")],
                expected_pinned_count: 2,
            },
            RunnerChange::RemoveRow {
                expected: identity(None, "token-malformed-None"),
            },
            RunnerChange::ReplaceNamed {
                name: "codex".to_owned(),
                argv: vec![
                    "codex".to_owned(),
                    "exec".to_owned(),
                    "--full".to_owned(),
                    "{{prompt}}".to_owned(),
                ],
                expected: vec![identity(Some(1), "token-1")],
            },
            RunnerChange::RepairRow {
                name: "repaired".to_owned(),
                argv: vec!["repaired".to_owned(), "{{prompt}}".to_owned()],
                expected: identity(Some(2), "token-malformed-Some(2)"),
            },
            RunnerChange::Add {
                name: "new-agent".to_owned(),
                argv: vec!["new-agent".to_owned(), "{{prompt}}".to_owned()],
            },
        ]
    );
}

#[test]
fn an_added_row_is_dropped_by_removal_and_an_edited_row_restores_its_edit() {
    let mut draft = draft_with(vec![runner_row(0, "claude", &["claude", "{{prompt}}"], 1)]);
    draft
        .stage_runner(
            save("extra", &["extra", "{{prompt}}"], RunnerSaveTarget::New),
            None,
        )
        .unwrap();
    draft.toggle_runner_removal(1).unwrap();
    assert_eq!(draft.runner_rows().len(), 1);

    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--fast", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(0), "token-0")],
                },
            ),
            None,
        )
        .unwrap();
    draft.toggle_runner_removal(0).unwrap();
    assert_eq!(
        draft.runner_rows()[0].marker(),
        Some(RunnerDraftMarker::Removed)
    );
    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        [RunnerChange::RemoveNamed {
            name: "claude".to_owned(),
            expected: vec![identity(Some(0), "token-0")],
            expected_pinned_count: 1,
        }]
    );

    draft.toggle_runner_removal(0).unwrap();
    assert_eq!(
        draft.runner_rows()[0].marker(),
        Some(RunnerDraftMarker::Edited)
    );
    assert_eq!(
        draft.runner_rows()[0].argv(),
        Some(
            [
                "claude".to_owned(),
                "--fast".to_owned(),
                "{{prompt}}".to_owned()
            ]
            .as_slice()
        )
    );
    assert!(matches!(
        &draft.runner_rows()[0],
        RunnerDraftRow::Existing {
            state: RunnerDraftState::Edited(edit),
            ..
        } if edit.name == "claude"
    ));

    draft.toggle_runner_removal(0).unwrap();
    draft.toggle_runner_removal(0).unwrap();
    draft.toggle_runner_removal(9).unwrap();
    assert_eq!(
        draft.runner_rows()[0].marker(),
        Some(RunnerDraftMarker::Edited)
    );
}

#[test]
fn an_unchanged_row_restores_to_unchanged_and_a_clean_draft_stays_clean() {
    let mut draft = draft_with(vec![runner_row(0, "claude", &["claude", "{{prompt}}"], 0)]);
    draft.toggle_runner_removal(0).unwrap();
    assert!(draft.dirty());

    draft.toggle_runner_removal(0).unwrap();

    assert_eq!(draft.runner_rows()[0].marker(), None);
    assert!(!draft.dirty());
    assert!(matches!(
        &draft.runner_rows()[0],
        RunnerDraftRow::Existing {
            state: RunnerDraftState::Unchanged,
            ..
        }
    ));
}

#[test]
fn a_name_staged_for_removal_can_be_reused_while_a_live_name_is_refused() {
    let mut draft = draft_with(vec![
        runner_row(0, "claude", &["claude", "{{prompt}}"], 0),
        runner_row(1, "codex", &["codex", "{{prompt}}"], 0),
    ]);

    assert_eq!(
        draft.stage_runner(
            save("codex", &["codex", "{{prompt}}"], RunnerSaveTarget::New),
            None,
        ),
        Err(RunnerDraftError::DuplicateName)
    );
    assert_eq!(draft.runner_rows().len(), 2);

    draft.toggle_runner_removal(1).unwrap();
    draft
        .stage_runner(
            save(
                "codex",
                &["codex", "--new", "{{prompt}}"],
                RunnerSaveTarget::New,
            ),
            None,
        )
        .unwrap();

    assert_eq!(draft.runner_rows().len(), 3);
    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        [
            RunnerChange::RemoveNamed {
                name: "codex".to_owned(),
                expected: vec![identity(Some(1), "token-1")],
                expected_pinned_count: 0,
            },
            RunnerChange::Add {
                name: "codex".to_owned(),
                argv: vec![
                    "codex".to_owned(),
                    "--new".to_owned(),
                    "{{prompt}}".to_owned(),
                ],
            },
        ]
    );
}

#[test]
fn an_edit_may_keep_its_own_name_but_not_take_another_live_name() {
    let mut draft = draft_with(vec![
        runner_row(0, "claude", &["claude", "{{prompt}}"], 0),
        malformed_row(Some(1), Some(&["broken", "{{prompt}}"])),
    ]);

    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--fast", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(0), "token-0")],
                },
            ),
            None,
        )
        .unwrap();
    assert_eq!(
        draft.runner_rows()[0].marker(),
        Some(RunnerDraftMarker::Edited)
    );

    assert_eq!(
        draft.stage_runner(
            save(
                "claude",
                &["claude", "{{prompt}}"],
                RunnerSaveTarget::RawRow {
                    expected: identity(Some(1), "token-malformed-Some(1)"),
                },
            ),
            None,
        ),
        Err(RunnerDraftError::DuplicateName)
    );
    assert_eq!(draft.runner_rows()[1].marker(), None);
}

#[test]
fn a_new_request_rewrites_only_the_added_row_the_editor_opened() {
    let mut draft = draft_with(vec![runner_row(0, "claude", &["claude", "{{prompt}}"], 0)]);
    draft
        .stage_runner(
            save("draft", &["draft", "{{prompt}}"], RunnerSaveTarget::New),
            None,
        )
        .unwrap();

    draft
        .stage_runner(
            save("renamed", &["renamed", "{{prompt}}"], RunnerSaveTarget::New),
            Some(1),
        )
        .unwrap();
    assert_eq!(draft.runner_rows().len(), 2);
    assert_eq!(draft.runner_rows()[1].name(), Some("renamed"));

    draft
        .stage_runner(
            save("second", &["second", "{{prompt}}"], RunnerSaveTarget::New),
            Some(0),
        )
        .unwrap();

    assert_eq!(draft.runner_rows().len(), 3);
    assert_eq!(draft.runner_rows()[0].name(), Some("claude"));
    assert_eq!(draft.runner_rows()[2].name(), Some("second"));
}

#[test]
fn a_save_request_for_a_row_the_draft_no_longer_has_changes_nothing() {
    let mut draft = draft_with(vec![runner_row(0, "claude", &["claude", "{{prompt}}"], 0)]);

    assert_eq!(
        draft.stage_runner(
            save(
                "ghost",
                &["ghost", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "ghost".to_owned(),
                    expected: vec![identity(Some(7), "token-7")],
                },
            ),
            None,
        ),
        Ok(())
    );
    assert_eq!(
        draft.stage_runner(
            save(
                "ghost",
                &["ghost", "{{prompt}}"],
                RunnerSaveTarget::RawRow {
                    expected: identity(Some(7), "token-7"),
                },
            ),
            None,
        ),
        Ok(())
    );
    assert_eq!(
        draft.stage_runner(
            save("ghost", &["ghost", "{{prompt}}"], RunnerSaveTarget::New),
            Some(9),
        ),
        Ok(())
    );

    assert_eq!(draft.runner_rows().len(), 2);
    assert_eq!(draft.runner_rows()[0].marker(), None);
    assert_eq!(draft.runner_rows()[1].name(), Some("ghost"));
}

#[test]
fn a_staged_agent_list_reaches_the_change_set_through_the_unchanged_mirror_shortcut() {
    let mut draft = PreferencesDraft::from_snapshot(PreferencesSnapshot {
        runners: vec![runner_row(0, "claude", &["claude", "{{prompt}}"], 0)],
        ..snapshot(MirrorConfiguration {
            enabled: true,
            python_install: "https://mirror.example/python/".to_owned(),
            uv_binary: "https://mirror.example/uv".to_owned(),
            ..MirrorConfiguration::default()
        })
    });
    draft.toggle_runner_removal(0).unwrap();

    let change = draft.resolve(|_| true).unwrap();

    assert!(!change.settings.contains_key("mirror.pypi"));
    assert_eq!(change.runners.len(), 1);
}

#[test]
fn the_agent_save_refusals_localize_and_name_the_agent_list() {
    for error in [
        PreferencesError::RunnersChanged,
        PreferencesError::RunnerNameTaken,
        PreferencesError::RunnerPinsChanged {
            name: "claude".to_owned(),
            actual: 3,
        },
    ] {
        assert_eq!(error.field(), PreferencesField::Runners);
        assert!(!error.message().localize(Locale::ZhTw).is_empty());
    }
    assert_eq!(
        PreferencesError::RunnersChanged
            .message()
            .localize(Locale::En),
        "The agent list changed on disk. Reopen Preferences and try again."
    );
    assert_eq!(
        PreferencesError::RunnerNameTaken
            .message()
            .localize(Locale::En),
        "Another row already uses this runner name."
    );
}

#[test]
fn an_unresolved_malformed_row_can_only_be_removed_by_its_raw_identity() {
    let mut draft = draft_with(vec![malformed_row(None, None)]);
    draft.toggle_runner_removal(0).unwrap();

    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        [RunnerChange::RemoveRow {
            expected: identity(None, "token-malformed-None"),
        }]
    );
    assert_eq!(draft.runner_rows()[0].name(), None);
    assert_eq!(draft.runner_rows()[0].argv(), None);
}

#[test]
fn only_a_stored_row_resolves_back_into_a_management_row() {
    let mut draft = draft_with(vec![runner_row(0, "claude", &["claude", "{{prompt}}"], 4)]);
    let stored = draft.runner_rows()[0].resolved_row().unwrap();
    assert_eq!(stored.name.as_deref(), Some("claude"));
    assert_eq!(stored.pinned_count, 4);

    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--fast", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(0), "token-0")],
                },
            ),
            None,
        )
        .unwrap();
    let edited = draft.runner_rows()[0].resolved_row().unwrap();
    assert_eq!(edited.name.as_deref(), Some("claude"));
    assert_eq!(
        edited.argv.as_deref(),
        Some(
            [
                "claude".to_owned(),
                "--fast".to_owned(),
                "{{prompt}}".to_owned()
            ]
            .as_slice()
        )
    );

    draft
        .stage_runner(
            save("extra", &["extra", "{{prompt}}"], RunnerSaveTarget::New),
            None,
        )
        .unwrap();
    assert_eq!(draft.runner_rows()[1].resolved_row(), None);
}

/// A cancelled removal brings the name back, so it obeys the rule every staged name obeys.
#[test]
fn a_restore_is_refused_while_another_live_row_uses_the_same_name() {
    let mut draft = draft_with(vec![runner_row(0, "claude", &["claude", "{{prompt}}"], 0)]);

    draft.toggle_runner_removal(0).unwrap();
    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--new", "{{prompt}}"],
                RunnerSaveTarget::New,
            ),
            None,
        )
        .unwrap();

    assert_eq!(
        draft.toggle_runner_removal(0),
        Err(RunnerDraftError::DuplicateName)
    );
    assert!(draft.runner_rows()[0].is_removed());
    assert_eq!(draft.runner_rows().len(), 2);

    // Dropping the appended row frees the name again.
    draft.toggle_runner_removal(1).unwrap();
    draft.toggle_runner_removal(0).unwrap();
    assert!(!draft.runner_rows()[0].is_removed());
    assert!(!draft.dirty());
}

/// One stable name can be stored on two raw rows. Its own rows never refuse its own save.
#[test]
fn a_name_stored_on_two_rows_is_still_editable() {
    let mut duplicate = runner_row(1, "claude", &["claude", "{{prompt}}"], 0);
    duplicate.reason = Some("duplicate".to_owned());
    let mut draft = draft_with(vec![
        runner_row(0, "claude", &["claude", "{{prompt}}"], 0),
        duplicate,
    ]);

    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--fast", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(0), "token-0")],
                },
            ),
            None,
        )
        .unwrap();

    // The save coalesces the key, so the marker lands on the first row of that name.
    assert_eq!(
        draft.runner_rows()[0].marker(),
        Some(RunnerDraftMarker::Edited)
    );
    assert_eq!(draft.runner_rows()[1].marker(), None);
    assert_eq!(
        draft.runner_rows()[0].argv(),
        Some(
            [
                "claude".to_owned(),
                "--fast".to_owned(),
                "{{prompt}}".to_owned()
            ]
            .as_slice()
        )
    );

    // A different name still collides with the untouched second row.
    let mut other = draft_with(vec![
        runner_row(0, "claude", &["claude", "{{prompt}}"], 0),
        runner_row(1, "codex", &["codex", "{{prompt}}"], 0),
    ]);
    assert_eq!(
        other.stage_runner(
            save(
                "codex",
                &["codex", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(0), "token-0")],
                },
            ),
            None,
        ),
        Err(RunnerDraftError::DuplicateName)
    );
}

/// Two raw rows of one name are one stored key, so a restore never collides with its own twin.
#[test]
fn a_restore_ignores_the_other_raw_row_of_its_own_name() {
    let mut duplicate = runner_row(1, "claude", &["claude", "{{prompt}}"], 0);
    duplicate.reason = Some("duplicate".to_owned());
    let mut draft = draft_with(vec![
        runner_row(0, "claude", &["claude", "{{prompt}}"], 0),
        duplicate,
    ]);

    draft.toggle_runner_removal(0).unwrap();
    assert!(draft.runner_rows()[0].is_removed());

    assert_eq!(draft.toggle_runner_removal(0), Ok(()));
    assert!(!draft.runner_rows()[0].is_removed());
    assert!(!draft.dirty());
}

/// A duplicate raw row of one key goes with the key, and it says so instead of staging itself.
#[test]
fn a_key_change_takes_the_duplicate_rows_of_that_key() {
    let key = vec![identity(Some(0), "token-0"), identity(Some(1), "token-1")];
    let mut primary = runner_row(0, "claude", &["claude", "{{prompt}}"], 0);
    primary.key_identities = key.clone();
    let mut duplicate = runner_row(1, "claude", &["claude", "{{prompt}}"], 0);
    duplicate.reason = Some("duplicate".to_owned());
    duplicate.key_identities = key;
    let rows = vec![
        primary,
        duplicate,
        runner_row(2, "codex", &["codex", "{{prompt}}"], 0),
    ];

    // Nothing is staged, so every row still speaks for itself.
    let mut draft = draft_with(rows.clone());
    assert!(!runner_row_taken_by_its_key(draft.runner_rows(), 1));

    // A key removal takes every raw row of the key, and only of that key.
    draft.toggle_runner_removal(0).unwrap();
    assert!(runner_row_taken_by_its_key(draft.runner_rows(), 1));
    assert!(!runner_row_taken_by_its_key(draft.runner_rows(), 2));
    assert!(!runner_row_taken_by_its_key(draft.runner_rows(), 0));
    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        [RunnerChange::RemoveNamed {
            name: "claude".to_owned(),
            expected: vec![identity(Some(0), "token-0"), identity(Some(1), "token-1")],
            expected_pinned_count: 0,
        }]
    );

    // An edit of the key folds its duplicates into the first row, so they go with it too.
    let mut draft = draft_with(rows);
    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--fast", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(0), "token-0")],
                },
            ),
            None,
        )
        .unwrap();
    assert!(runner_row_taken_by_its_key(draft.runner_rows(), 1));
    assert_eq!(draft.resolve(|_| true).unwrap().runners.len(), 1);
}

/// One raw row of a key leaves its twins, and a removal staged on both must not repeat itself.
#[test]
fn one_removed_raw_row_leaves_its_twins_and_never_repeats_a_key_removal() {
    let key = vec![
        identity(Some(0), "token-0"),
        identity(Some(1), "token-1"),
        identity(Some(2), "token-2"),
    ];
    let mut rows = Vec::new();
    for index in 0..3 {
        let mut row = runner_row(index, "claude", &["claude", "{{prompt}}"], 0);
        row.key_identities = key.clone();
        if index > 0 {
            row.reason = Some("duplicate".to_owned());
        }
        rows.push(row);
    }
    let mut draft = draft_with(rows);

    // Removing one duplicate leaves the other duplicate to speak for itself.
    draft.toggle_runner_removal(1).unwrap();
    assert!(!runner_row_taken_by_its_key(draft.runner_rows(), 2));

    // The key removal then covers both duplicates, and the save asks for it once.
    draft.toggle_runner_removal(0).unwrap();
    assert!(runner_row_taken_by_its_key(draft.runner_rows(), 1));
    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        [RunnerChange::RemoveNamed {
            name: "claude".to_owned(),
            expected: key,
            expected_pinned_count: 0,
        }]
    );
}

/// A removal staged on a duplicate row and an edit of the key that row repeats are one change.
///
/// The edit folds every duplicate of the key into the first row, so a second removal would find
/// nothing and refuse the complete save. The change set asks for the edit alone, and it names
/// every raw row the read reported for the key.
#[test]
fn an_edit_of_a_key_absorbs_a_removal_staged_on_its_duplicate_row() {
    let key = vec![identity(Some(0), "token-0"), identity(Some(1), "token-1")];
    let mut primary = runner_row(0, "claude", &["claude", "{{prompt}}"], 0);
    primary.key_identities = key.clone();
    let mut duplicate = runner_row(1, "claude", &["claude", "{{prompt}}"], 0);
    duplicate.reason = Some("duplicate".to_owned());
    duplicate.key_identities = key;
    let mut draft = draft_with(vec![primary, duplicate]);

    draft.toggle_runner_removal(1).unwrap();
    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--fast", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(0), "token-0")],
                },
            ),
            None,
        )
        .unwrap();

    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        [RunnerChange::ReplaceNamed {
            name: "claude".to_owned(),
            argv: vec![
                "claude".to_owned(),
                "--fast".to_owned(),
                "{{prompt}}".to_owned(),
            ],
            expected: vec![identity(Some(0), "token-0"), identity(Some(1), "token-1")],
        }]
    );
}

/// A key staged for removal frees its name, duplicate rows and all.
///
/// The save removes before it adds, so a new agent may take the name of a key the same save
/// deletes. A duplicate row of that key must not refuse the name it no longer keeps.
#[test]
fn a_key_staged_for_removal_frees_its_name_for_a_new_agent() {
    let key = vec![identity(Some(0), "token-0"), identity(Some(1), "token-1")];
    let mut primary = runner_row(0, "claude", &["claude", "{{prompt}}"], 0);
    primary.key_identities = key.clone();
    let mut duplicate = runner_row(1, "claude", &["claude", "{{prompt}}"], 0);
    duplicate.reason = Some("duplicate".to_owned());
    duplicate.key_identities = key;
    let mut draft = draft_with(vec![primary, duplicate]);

    draft.toggle_runner_removal(0).unwrap();
    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--new", "{{prompt}}"],
                RunnerSaveTarget::New,
            ),
            None,
        )
        .unwrap();

    assert_eq!(draft.runner_rows().len(), 3);
    assert_eq!(draft.runner_rows()[2].name(), Some("claude"));
    // The restore then finds the name again, so it stays refused while the new row holds it.
    assert_eq!(
        draft.toggle_runner_removal(0),
        Err(RunnerDraftError::DuplicateName)
    );
}

/// A change set never asks for an edit of a key the same save removes.
///
/// The draft still holds the edit: the frontend opens no editor on a row the key removal takes,
/// and the change set leaves the edit out instead of asking the store for a row that is gone.
#[test]
fn a_key_removal_drops_an_edit_staged_on_a_duplicate_of_that_key() {
    let key = vec![identity(Some(0), "token-0"), identity(Some(1), "token-1")];
    let mut primary = runner_row(0, "claude", &["claude", "{{prompt}}"], 0);
    primary.key_identities = key.clone();
    let mut duplicate = runner_row(1, "claude", &["claude", "{{prompt}}"], 0);
    duplicate.reason = Some("duplicate".to_owned());
    duplicate.key_identities = key.clone();
    let mut draft = draft_with(vec![primary, duplicate]);

    draft.toggle_runner_removal(0).unwrap();
    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--fast", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(1), "token-1")],
                },
            ),
            None,
        )
        .unwrap();

    // The save skips the removed row and marks the duplicate, which the removal takes as well.
    assert_eq!(
        draft.runner_rows()[1].marker(),
        Some(RunnerDraftMarker::Edited)
    );
    assert!(runner_row_taken_by_its_key(draft.runner_rows(), 1));
    assert_eq!(
        draft.resolve(|_| true).unwrap().runners,
        [RunnerChange::RemoveNamed {
            name: "claude".to_owned(),
            expected: key,
            expected_pinned_count: 0,
        }]
    );
}

/// A named save rewrites the first row of its key that the save keeps.
///
/// A removal staged on an earlier raw row of the same name stays staged, and the edit marks the
/// row that survives it, so neither change covers the other.
#[test]
fn a_named_save_marks_the_first_row_of_its_key_that_the_save_keeps() {
    let key = vec![identity(Some(0), "token-0"), identity(Some(1), "token-1")];
    let mut broken = malformed_row(Some(0), Some(&["claude", "{{prompt}}"]));
    broken.name = Some("claude".to_owned());
    broken.key_identities = key.clone();
    let mut valid = runner_row(1, "claude", &["claude", "{{prompt}}"], 0);
    valid.key_identities = key;
    let mut draft = draft_with(vec![broken, valid]);

    draft.toggle_runner_removal(0).unwrap();
    draft
        .stage_runner(
            save(
                "claude",
                &["claude", "--fast", "{{prompt}}"],
                RunnerSaveTarget::Named {
                    name: "claude".to_owned(),
                    expected: vec![identity(Some(1), "token-1")],
                },
            ),
            None,
        )
        .unwrap();

    assert!(draft.runner_rows()[0].is_removed());
    assert_eq!(
        draft.runner_rows()[1].marker(),
        Some(RunnerDraftMarker::Edited)
    );
}

#[test]
fn a_repaired_raw_row_keeps_its_raw_identity_and_takes_the_typed_name() {
    let mut draft = draft_with(vec![malformed_row(
        Some(4),
        Some(&["broken", "{{prompt}}"]),
    )]);
    draft
        .stage_runner(
            save(
                "repaired",
                &["repaired", "{{prompt}}"],
                RunnerSaveTarget::RawRow {
                    expected: identity(Some(4), "token-malformed-Some(4)"),
                },
            ),
            None,
        )
        .unwrap();

    let resolved = draft.runner_rows()[0].resolved_row().unwrap();
    // The repair the user typed travels with the row, so the editor reopens on it.
    assert_eq!(resolved.name.as_deref(), Some("repaired"));
    assert_eq!(
        resolved.identity,
        identity(Some(4), "token-malformed-Some(4)")
    );
    assert_eq!(
        resolved.argv.as_deref(),
        Some(["repaired".to_owned(), "{{prompt}}".to_owned()].as_slice())
    );
    assert_eq!(draft.runner_rows()[0].name(), Some("repaired"));
}
