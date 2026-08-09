use std::{collections::BTreeMap, fs, io, path::Path};

use clap::{CommandFactory as _, Parser as _};
use skit_application::{ExitClass, LibraryService, RepositoryError};
use skit_domain::{
    Entry, EntryKind, EntryMeta, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_store::FileStore;
use skit_ui::{FormControl, FormField, FormPurpose, FormView, Screen};
use tempfile::TempDir;

use super::*;

fn write_meta(root: &TempDir, slug: &str, name: &str, description: &str) {
    let directory = root.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        format!(
            "name = {name:?}\nkind = \"command\"\nmode = \"copy\"\ndescription = {description:?}\n"
        ),
    )
    .unwrap();
    FileStore::new(root.path()).rebuild_registry().unwrap();
}

fn add_command(service: &LibraryService<FileStore>, name: &str, template: &str) -> Entry {
    add(
        service,
        AddOptions {
            source: None,
            kind: Some("command".to_owned()),
            name: Some(name.to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: Some(template.to_owned()),
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    service.show(name).unwrap()
}

fn form_values(screen: Screen) -> BTreeMap<String, String> {
    let Screen::Form(form) = screen else {
        panic!("expected a form");
    };
    form.fields
        .into_iter()
        .map(|field| (field.key, field.value))
        .collect()
}

#[test]
fn completion_candidates_include_each_entry_slug_and_display_name() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "alpha", "Alpha tool", "Human description");
    let candidates = entry_candidates_from(&FileStore::new(root.path()));
    let values = candidates
        .iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(values, ["alpha", "Alpha tool"]);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.get_help().is_some())
    );
}

#[test]
fn completion_candidates_include_prompt_runners_and_saved_presets() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        concat!(
            "[[prompt.runners]]\n",
            "name = \"codex\"\n",
            "argv = [\"codex\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let runners = runner_candidates_from(&skit_store::FileConfigStore::new(root.path()))
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(runners.contains(&"codex".to_owned()));
    assert_eq!(
        runners
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        runners.len()
    );

    let values = root.path().join("state/values");
    fs::create_dir_all(&values).unwrap();
    fs::write(
        values.join("alpha.toml"),
        "[presets.fast]\ncount = \"1\"\n[presets.safe]\ncount = \"2\"\n",
    )
    .unwrap();
    fs::write(
        values.join("beta.toml"),
        "[presets.fast]\ncount = \"3\"\n[presets.thorough]\ncount = \"4\"\n",
    )
    .unwrap();
    let presets = preset_candidates_from(&root.path().join("state"))
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(presets, ["fast", "safe", "thorough"]);
}

#[test]
fn completion_adapters_follow_each_latest_main_degradation_contract() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("scripts"), "not a directory").unwrap();
    assert!(entry_candidates_from(&FileStore::new(root.path())).is_empty());

    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("config.toml"), "[").unwrap();
    let candidates = runner_candidates_from(&FileConfigStore::new(&config))
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(candidates.contains(&"claude".to_owned()));
    assert!(candidates.contains(&"codex".to_owned()));
    assert_eq!(fs::read_to_string(config.join("config.toml")).unwrap(), "[");

    let state = root.path().join("state");
    assert!(preset_candidates_from(&state).is_empty());
    let values = state.join("values");
    fs::create_dir_all(&values).unwrap();
    fs::write(values.join("ignore.txt"), "[presets]\n").unwrap();
    fs::write(values.join("binary.toml"), [0xff]).unwrap();
    fs::write(values.join("invalid.toml"), "[").unwrap();
    fs::write(values.join("empty.toml"), "future = true\n").unwrap();
    fs::write(
        values.join("mixed.toml"),
        "[presets]\nnot_a_table = 1\n[presets.valid]\nvalue = \"x\"\n",
    )
    .unwrap();
    fs::create_dir(values.join("directory.toml")).unwrap();
    let candidates = preset_candidates_from(&state)
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(candidates, ["valid"]);
}

#[test]
fn destructive_and_create_prompts_have_explicit_automation_paths() {
    assert!(user_confirmed("y", false));
    assert!(user_confirmed("YES", false));
    assert!(user_confirmed("", true));
    assert!(!user_confirmed("", false));
    assert!(!user_confirmed("no", true));

    let remove = Cli::try_parse_from(["skit", "remove", "alpha", "-y", "--no-input"]).unwrap();
    assert!(matches!(
        remove.command,
        Some(Command::Remove {
            yes: true,
            no_input: true,
            ..
        })
    ));
    let edit = Cli::try_parse_from(["skit", "edit", "new-tool", "--no-input"]).unwrap();
    assert!(matches!(
        edit.command,
        Some(Command::Edit { no_input: true, .. })
    ));
}

#[test]
fn every_clap_help_message_has_complete_chinese_catalog_rows() {
    fn collect(command: &clap::Command, output: &mut Vec<String>) {
        if let Some(about) = command.get_about() {
            output.push(about.to_string());
        }
        if let Some(about) = command.get_long_about() {
            output.push(about.to_string());
        }
        for argument in command.get_arguments() {
            if let Some(help) = argument.get_help() {
                output.push(help.to_string());
            }
            if let Some(help) = argument.get_long_help() {
                output.push(help.to_string());
            }
        }
        for child in command.get_subcommands() {
            collect(child, output);
        }
    }

    let mut messages = Vec::new();
    collect(&Cli::command(), &mut messages);
    messages.sort();
    messages.dedup();
    let missing = messages
        .into_iter()
        .filter(|message| {
            skit_i18n::text(skit_i18n::Locale::ZhCn, message).as_ref() == message
                || skit_i18n::text(skit_i18n::Locale::ZhTw, message).as_ref() == message
        })
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "missing help translations:\n{missing:#?}"
    );
}

#[test]
fn source_helpers_preserve_bytes_names_and_storage_conventions() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("archive.custom");
    fs::write(&source, b"alpha\r\nbeta\r\n").unwrap();

    let snapshot = read_source(&source, false).unwrap();

    assert_eq!(snapshot.bytes, b"alpha\r\nbeta\r\n");
    assert_eq!(source_default_name(&source), "archive");
    assert_eq!(source_default_name(Path::new("")), "script");
    assert_eq!(
        [
            ("python", "script.py"),
            ("shell", "script.sh"),
            ("js", "script.js"),
            ("ts", "script.ts"),
            ("fish", "script.fish"),
            ("powershell", "script.ps1"),
            ("ruby", "script.rb"),
            ("perl", "script.pl"),
            ("lua", "script.lua"),
            ("r", "script.r"),
            ("future-kind", "payload"),
        ]
        .map(|(kind, _)| { payload_stored_name(&EntryKind::parse(kind).unwrap(), &source) }),
        [
            "script.py",
            "script.sh",
            "script.js",
            "script.ts",
            "script.fish",
            "script.ps1",
            "script.rb",
            "script.pl",
            "script.lua",
            "script.r",
            "payload",
        ]
        .map(str::to_owned)
    );
    #[cfg(unix)]
    assert!(snapshot.permissions.unix_mode.is_some());
    #[cfg(not(unix))]
    assert!(snapshot.permissions.unix_mode.is_none());

    let missing = root.path().join("missing");
    let error = read_source(&missing, false).unwrap_err();
    assert!(matches!(
        error,
        CliError::Source {
            operation: "inspect",
            ..
        }
    ));
}

#[test]
fn add_uses_the_shared_kind_aware_description_and_keeps_an_explicit_empty_value() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path().join("library"));
    let service = LibraryService::new(store);
    let config = root.path().join("config");
    let derived = root.path().join("derived.py");
    fs::write(&derived, b"\"\"\"Derived documentation.\"\"\"\nprint(1)\n").unwrap();

    add_with_config(
        &service,
        &config,
        AddOptions {
            source: Some(derived),
            kind: None,
            name: Some("Derived".into()),
            description: None,
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: true,
        },
    )
    .unwrap();
    assert_eq!(
        service.show("Derived").unwrap().meta.description,
        "Derived documentation."
    );

    let explicit = root.path().join("explicit.py");
    fs::write(&explicit, b"\"\"\"Must not win.\"\"\"\nprint(1)\n").unwrap();
    add_with_config(
        &service,
        &config,
        AddOptions {
            source: Some(explicit),
            kind: None,
            name: Some("Explicit empty".into()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: true,
        },
    )
    .unwrap();
    assert_eq!(service.show("Explicit empty").unwrap().meta.description, "");
}

#[test]
fn bare_add_refuses_every_unapplied_flag_and_lists_only_compatible_lanes() {
    let mut options = empty_add_options();
    options.name = Some("Named".to_owned());
    options.dependencies = vec!["requests".to_owned()];
    options.dependencies_explicit = true;
    let error = refuse_bare_add_flags(&options).unwrap_err();
    let CliError::Usage(message) = error else {
        panic!("expected a usage refusal");
    };
    assert_eq!(
        message.localize(Locale::En),
        "--name, --dep need a source — pass the path in the same command (skit add PATH …), or pick a lane outright with --edit (nothing was added)."
    );

    let mut options = empty_add_options();
    options.kind = Some("shell".to_owned());
    let error = refuse_bare_add_flags(&options).unwrap_err();
    let CliError::Usage(message) = error else {
        panic!("expected a usage refusal");
    };
    assert_eq!(
        message.localize(Locale::En),
        "--kind need a source — pass the path in the same command (skit add PATH …) (nothing was added)."
    );

    assert!(refuse_bare_add_flags(&empty_add_options()).is_ok());
    let mut empty_name = empty_add_options();
    empty_name.name = Some(String::new());
    assert!(refuse_bare_add_flags(&empty_name).is_ok());
}

#[test]
fn private_human_paths_render_entries_and_diagnostics() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "alpha", "Alpha", "Human description");
    let broken = root.path().join("scripts").join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("meta.toml"), "name = [broken").unwrap();
    let service = LibraryService::new(FileStore::new(root.path()));

    let store = FileStore::new(root.path());
    list(&service, &store, false).unwrap();
    show(&service, &store, "alpha", false).unwrap();
    write_meta(&root, "empty", "Empty", "");
    show(&service, &store, "empty", false).unwrap();

    let source = root.path().join("source.py");
    fs::write(&source, b"print('source')\n").unwrap();
    let error = add(
        &service,
        AddOptions {
            source: Some(source),
            kind: Some(" ".to_owned()),
            name: Some("Bad".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CliError::Repository(RepositoryError::InvalidMutation { .. })
    ));
}

fn listing_summary(kind: &str, mode: StorageMode, target: Option<&str>) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse("listed").unwrap(),
        name: "Listed".to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode,
        description: String::new(),
        target: target.map(str::to_owned),
    }
}

#[test]
fn summary_missing_uses_only_the_listing_projection_and_derived_copy_target() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry_dir = store.entry_dir_path(&Slug::parse("listed").unwrap());
    fs::create_dir_all(&entry_dir).unwrap();
    let payload = entry_dir.join("script.py");
    fs::write(&payload, "print('ok')\n").unwrap();

    // No meta.toml exists. A listing consumer must still find a copied target from the summary's
    // kind and entry directory instead of resolving and parsing the full entry.
    let copy = listing_summary("python", StorageMode::Copy, None);
    assert!(!summary_missing(&store, &copy));
    fs::remove_file(payload).unwrap();
    assert!(summary_missing(&store, &copy));

    let module = listing_summary("js", StorageMode::Copy, None);
    let module_payload = store.entry_dir_path(&module.slug).join("script.mjs");
    fs::write(&module_payload, "export {};\n").unwrap();
    assert!(
        !summary_missing(&store, &module),
        "a supported JavaScript stored-name variant is a healthy copy target"
    );

    // Python v0.4 uses Path.exists(), not a regular-file-only check.
    let reference_dir = root.path().join("reference-target");
    fs::create_dir(&reference_dir).unwrap();
    let reference = listing_summary(
        "python",
        StorageMode::Reference,
        Some(reference_dir.to_str().unwrap()),
    );
    assert!(!summary_missing(&store, &reference));

    // This version cannot infer target semantics for a kind written by a newer skit.
    let unknown = listing_summary(
        "future-kind",
        StorageMode::Reference,
        Some(root.path().join("gone").to_str().unwrap()),
    );
    assert!(!summary_missing(&store, &unknown));
}

#[test]
fn tui_composition_treats_an_unindexed_broken_scripts_path_as_an_empty_library() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("scripts"), "not a directory").unwrap();
    let scan = LibraryService::new(FileStore::new(root.path()))
        .list()
        .unwrap();
    assert!(scan.entries.is_empty());
    assert!(scan.diagnostics.is_empty());
}

#[test]
fn data_directory_mode_and_error_taxonomy_helpers_are_stable() {
    let root = TempDir::new().unwrap();
    assert_eq!(
        resolve_data_dir(Some(root.path().to_path_buf())).unwrap(),
        root.path()
    );
    assert!(platform_data_dir().is_some());
    assert!(resolve_data_dir(None).is_ok());
    assert_eq!(mode_name(StorageMode::Copy), "copy");
    assert_eq!(mode_name(StorageMode::Reference), "reference");

    let json_error = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let errors = [
        (
            CliError::Repository(RepositoryError::NotFound {
                query: "missing".to_owned(),
            }),
            ExitClass::Failure.code(),
        ),
        (CliError::ConfirmationRequired, ExitClass::Usage.code()),
        (CliError::Json(json_error), ExitClass::Skit.code()),
        (
            CliError::Io(io::Error::other("output")),
            ExitClass::Skit.code(),
        ),
        (
            CliError::Tui(skit_tui::TuiError::Io(io::Error::other("terminal"))),
            ExitClass::Skit.code(),
        ),
        (
            source_error("read", Path::new("source"), io::Error::other("source")),
            ExitClass::Failure.code(),
        ),
        (
            CliError::Dependencies(DependencyError::Io {
                operation: "read",
                path: "dependency".to_owned(),
                reason: "failed".to_owned(),
            }),
            ExitClass::Skit.code(),
        ),
        (CliError::DataDirectoryUnavailable, ExitClass::Skit.code()),
    ];

    for (error, expected) in errors {
        assert_eq!(error.exit_code(), i32::from(expected));
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn adapter_only_error_paths_do_not_require_process_global_configuration() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("data");
    let config_dir = root.path().join("config");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("scripts"), "not a directory").unwrap();
    let store = FileStore::new(&data);
    let service = LibraryService::new(store.clone());
    assert!(matches!(
        edit_with_config(&service, &store, &config_dir, "missing", true).unwrap_err(),
        CliError::Failure(_)
    ));
    assert!(
        config_in(
            &FileConfigStore::new(&config_dir),
            None,
            Some("orphan"),
            false,
        )
        .is_err()
    );
    let runner_error =
        validate_prompt_runner_in(&FileConfigStore::new(&config_dir), Some("missing")).unwrap_err();
    assert!(
        runner_error
            .to_string()
            .contains("prompt runner \"missing\" is not configured"),
        "{runner_error}"
    );
    let valid_store = FileStore::new(root.path().join("valid-data"));
    let valid_service = LibraryService::new(valid_store);
    assert!(
        add_with_config(
            &valid_service,
            &config_dir,
            AddOptions {
                source: None,
                kind: None,
                name: None,
                description: Some(String::new()),
                reference: false,
                command_template: None,
                prompt: false,
                executable: false,
                runner: None,
                no_interpolate: false,
                dependencies: Vec::new(),
                dependencies_explicit: false,
                requires_python: None,
                no_input: false,
            },
        )
        .is_err()
    );
}

#[test]
fn tui_run_forms_preserve_saved_values_but_never_prefill_secrets() {
    let mut entry = Entry {
        slug: Slug::parse("alpha").unwrap(),
        meta: EntryMeta::minimal("Alpha", EntryKind::parse("prompt").unwrap()),
    };
    let mut token = ParamDecl::new("token");
    token.secret = true;
    let mut count = ParamDecl::new("count");
    count.parameter_type = ParameterType::Int;
    count.prompt = "Count".to_owned();
    let saved = BTreeMap::from([
        ("token".to_owned(), "do-not-show".to_owned()),
        ("count".to_owned(), "4".to_owned()),
    ]);
    let settings = EntrySettings {
        params: vec!["token".to_owned(), "count".to_owned()],
        parameters: vec![token, count],
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut entry.meta);
    let plan = form_plan("prompt", "{{token}} {{count}}", &settings);

    let Screen::Run(form) = tui_run_form(
        &entry,
        &plan,
        &saved,
        &["codex".to_owned()],
        "codex",
        &BTreeMap::from([(
            "fast".to_owned(),
            BTreeMap::from([("count".to_owned(), "8".to_owned())]),
        )]),
        "--model fast",
        RunFormContext {
            entry_kind: "prompt".to_owned(),
            path: None,
            tokens: skit_application::tokens::TokenContext {
                cwd: "/invoke".to_owned(),
                home: None,
                env: BTreeMap::new(),
                today: "2026-08-09".to_owned(),
                now: "12-00-00".to_owned(),
            },
        },
        Locale::En,
    ) else {
        panic!("run must use the typed launch form");
    };
    assert_eq!(form.purpose(), FormPurpose::Run);
    assert_eq!(form.selector(), "alpha");
    let fields = form
        .fields()
        .iter()
        .map(|field| {
            (
                field.key.as_str(),
                field.control.value(),
                matches!(&field.control, FormControl::Text(control) if control.secret),
            )
        })
        .collect::<Vec<_>>();
    assert!(fields.contains(&("value:token", String::new(), true)));
    assert!(fields.contains(&("value:count", "4".to_owned(), false)));
    assert!(fields.contains(&("_skit_runner", "codex".to_owned(), false)));
    assert!(fields.contains(&("_skit_preset", String::new(), false)));
    assert!(fields.contains(&("_skit_args", "--model fast".to_owned(), false)));
    assert_eq!(form.context().unwrap().entry_kind, "prompt");
    assert_eq!(
        form.fields()
            .iter()
            .find(|field| field.key == "_skit_args")
            .unwrap()
            .label,
        "Extra agent arguments"
    );
}

#[test]
fn tui_add_opens_the_typed_workflow_with_owned_drafts_and_runner_history() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let drafts = data_dir.join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    fs::write(drafts.join("skit-new-script.py"), "print(1)\n").unwrap();
    fs::write(drafts.join("not-owned.py"), "print(2)\n").unwrap();
    fs::create_dir(drafts.join("skit-new-directory.py")).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        concat!(
            "[[prompt.runners]]\n",
            "name = \"my-agent\"\n",
            "argv = [\"agent\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    PromptSelectionService::new(FilePromptSelectionStore::new(&state_dir))
        .remember_runner("my-agent")
        .unwrap();

    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let Screen::Add(workflow) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Add,
        None,
    )
    .unwrap() else {
        panic!("add must use its typed workflow");
    };
    assert_eq!(workflow.source().listed_drafts().len(), 1);
    assert_eq!(
        workflow.source().listed_drafts()[0].path,
        drafts.join("skit-new-script.py")
    );
    assert_eq!(workflow.review_defaults().runner_names, ["my-agent"]);
    assert_eq!(
        workflow.review_defaults().last_runner.as_deref(),
        Some("my-agent")
    );

    assert_eq!(
        tui_split_list("alpha, beta\ngamma  delta"),
        ["alpha", "beta", "gamma", "delta"]
    );
    assert_eq!(
        tui_dependency_list("requests>=2,<3\nrich; python_version >= '3.12'"),
        ["requests>=2,<3", "rich; python_version >= '3.12'"]
    );
    for (kind, source, expected) in [
        ("js", "module.mjs", "script.mjs"),
        ("js", "module.cjs", "script.cjs"),
        ("ts", "module.mts", "script.mts"),
        ("ts", "module.cts", "script.cts"),
    ] {
        assert_eq!(
            payload_stored_name(&EntryKind::parse(kind).unwrap(), Path::new(source)),
            expected
        );
    }
}

#[test]
fn tui_add_host_reads_exact_source_and_commits_through_the_reducer_seam() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("hello.sh");
    let raw = b"#!/bin/sh\nprintf '\\377\\n'\n";
    fs::write(&source, raw).unwrap();
    let canonical = fs::canonicalize(&source).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(source.display().to_string()));
    let effects = workflow.reduce(AddAction::Continue);

    let action = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap();
    let UiAction::Add(AddAction::SourceInspected { request, result }) = action else {
        panic!("source inspection must return through the typed reducer");
    };
    let snapshot = result.unwrap();
    assert_eq!(snapshot.path, canonical);
    assert_eq!(snapshot.source_record, canonical.display().to_string());
    assert_eq!(snapshot.bytes, raw);
    assert!(snapshot.is_regular);
    assert!(!snapshot.is_directory);
    assert!(!snapshot.is_draft);
    let effects = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(snapshot),
    });
    assert!(effects.is_empty());
    assert!(workflow.review().is_some());

    let effects = workflow.reduce(AddAction::Save);
    let [
        AddEffect::Commit {
            source: Some(expected),
            ..
        },
    ] = effects.as_slice()
    else {
        panic!("a reviewed file commit must carry the byte-exact source expectation");
    };
    assert_eq!(expected.bytes, raw);
    let action = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap();
    let UiAction::Add(AddAction::CommitFinished { request, result }) = action else {
        panic!("repository completion must return through the typed reducer");
    };
    let slug = result.unwrap();
    let effects = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Ok(slug.clone()),
    });
    let UiAction::AddCompleted {
        slug: completed,
        message,
        ..
    } = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap()
    else {
        panic!("the final host effect must refresh and select the new entry");
    };
    assert_eq!(completed.as_str(), slug);
    assert_eq!(message, "Entry added");
    let entry = service.show(&slug).unwrap();
    assert_eq!(entry.meta.source, canonical.display().to_string());
}

#[test]
fn tui_add_refuses_a_source_that_changed_after_review_without_writing_an_entry() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("reviewed.sh");
    fs::write(&source, "#!/bin/sh\necho before\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(source.display().to_string()));
    let inspect = workflow.reduce(AddAction::Continue);
    let UiAction::Add(AddAction::SourceInspected { request, result }) =
        tui_add_effect(&service, &store, &state_dir, &config_dir, inspect).unwrap()
    else {
        panic!("source inspection must return to the reducer");
    };
    let _ = workflow.reduce(AddAction::SourceInspected { request, result });
    fs::write(&source, "#!/bin/sh\necho after\n").unwrap();

    let commit = workflow.reduce(AddAction::Save);
    let UiAction::Add(AddAction::CommitFinished {
        request,
        result: Err(reason),
    }) = tui_add_effect(&service, &store, &state_dir, &config_dir, commit).unwrap()
    else {
        panic!("a stale review must return a typed commit refusal");
    };
    assert!(reason.contains("source changed while the add review was open"));
    let _ = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Err(reason),
    });
    assert!(service.list().unwrap().entries.is_empty());
    assert!(workflow.problem().is_some());
}

#[test]
fn tui_add_cleanup_refusals_keep_the_committed_entry_and_the_unowned_file() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let unusable_state = root.path().join("state-is-a-file");
    fs::write(&unusable_state, "not a directory").unwrap();
    let outside = root.path().join("skit-outside.py");
    fs::write(&outside, "print('keep me')\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let created = add_command(&service, "Kept", "printf kept");

    let action = tui_add_effect(
        &service,
        &store,
        &unusable_state,
        &config_dir,
        vec![
            AddEffect::ConsumeDraft(outside.clone()),
            AddEffect::RememberRunner("agent".to_owned()),
            AddEffect::Complete(created.slug.as_str().to_owned()),
        ],
    )
    .unwrap();
    let UiAction::AddCompleted { message, slug, .. } = action else {
        panic!("auxiliary cleanup failures must not turn a committed add into failure");
    };
    assert_eq!(slug, created.slug);
    assert!(message.starts_with("Entry added\nwarning: "));
    assert!(message.contains("drafts directory"));
    assert!(outside.exists());
    assert_eq!(service.show("Kept").unwrap().slug, created.slug);
}

#[cfg(unix)]
#[test]
fn tui_add_never_treats_a_symlinked_drafts_directory_as_owned_storage() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let outside = root.path().join("outside");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, data_dir.join("drafts")).unwrap();
    let victim = outside.join("skit-victim.py");
    fs::write(&victim, "print('keep me')\n").unwrap();

    assert!(remove_owned_draft(&data_dir, &data_dir.join("drafts/skit-victim.py")).is_err());
    assert_eq!(fs::read(&victim).unwrap(), b"print('keep me')\n");
    assert!(tui_drafts(&data_dir).is_empty());
}

#[cfg(unix)]
#[test]
fn tui_add_authoring_uses_a_real_owned_draft_and_discards_only_unchanged_starters() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let editor = root.path().join("editor.sh");
    fs::write(
        &editor,
        r#"#!/bin/sh
printf "print('written')\n" >> "$1"
"#,
    )
    .unwrap();
    let mut mode = fs::metadata(&editor).unwrap().permissions();
    mode.set_mode(0o755);
    fs::set_permissions(&editor, mode).unwrap();
    FileConfigStore::new(&config_dir)
        .set("editor", editor.to_str().unwrap())
        .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut workflow = AddWorkflowState::new(Vec::new());
    let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Script));
    let UiAction::Add(AddAction::DraftEdited {
        request,
        result: Ok(Some(snapshot)),
    }) = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap()
    else {
        panic!("an edited starter must return an owned byte-exact draft");
    };
    assert!(snapshot.is_draft);
    assert!(snapshot.path.exists());
    assert_eq!(
        snapshot.bytes,
        b"#!/usr/bin/env python3\nprint('written')\n"
    );
    let _ = workflow.reduce(AddAction::DraftEdited {
        request,
        result: Ok(Some(snapshot)),
    });

    FileConfigStore::new(&config_dir)
        .set("editor", "true")
        .unwrap();
    let mut unchanged = AddWorkflowState::new(Vec::new());
    let effects = unchanged.reduce(AddAction::NewDraft(DraftKind::Prompt));
    let UiAction::Add(AddAction::DraftEdited {
        request,
        result: Ok(None),
    }) = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap()
    else {
        panic!("an unchanged prompt starter must be discarded as scaffolding");
    };
    let _ = unchanged.reduce(AddAction::DraftEdited {
        request,
        result: Ok(None),
    });
    assert_eq!(
        unchanged.notice(),
        Some(&skit_ui::AddNotice::NothingWritten)
    );
}

#[test]
fn tui_settings_exposes_source_management_and_every_parameter_axis() {
    let root = TempDir::new().unwrap();
    let directory = root.path().join("scripts").join("alpha");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "name = \"Alpha\"\n",
            "kind = \"command\"\n",
            "mode = \"copy\"\n",
            "source = \"\"\n",
            "workdir = \"invoke\"\n",
            "[[parameters]]\n",
            "name = \"count\"\n",
            "binding = \"none\"\n",
            "delivery = \"flag\"\n",
            "type = \"int\"\n",
            "default = 4\n",
            "required = true\n",
            "multiple = true\n",
            "repeat = true\n",
            "choices = [\"4\", \"8\"]\n",
            "prompt = \"Count\"\n",
            "help = \"Set the count.\"\n",
            "secret = true\n",
            "env_source = \"SKIT_COUNT\"\n",
            "env_target = \"COUNT\"\n",
            "flag = \"--count\"\n",
            "action = \"append\"\n",
        ),
    )
    .unwrap();
    let store = FileStore::new(root.path());
    store.rebuild_registry().unwrap();
    let service = LibraryService::new(store.clone());
    let entry = service.show("alpha").unwrap();

    let Screen::Form(form) = tui_settings_form(&store, &entry) else {
        panic!("settings must use a form");
    };
    let values = form
        .fields
        .iter()
        .map(|field| (field.key.as_str(), field.value.as_str()))
        .collect::<BTreeMap<_, _>>();
    for key in [
        "source:resync",
        "source:manage",
        "source:unmanage",
        "source:normalize",
        "parameter:add",
        "parameter:remove",
        "parameter:0:name",
        "parameter:0:binding",
        "parameter:0:delivery",
        "parameter:0:type",
        "parameter:0:default",
        "parameter:0:choices",
        "parameter:0:required",
        "parameter:0:multiple",
        "parameter:0:repeat",
        "parameter:0:prompt",
        "parameter:0:help",
        "parameter:0:secret",
        "parameter:0:env_source",
        "parameter:0:env_target",
        "parameter:0:flag",
        "parameter:0:action",
    ] {
        assert!(values.contains_key(key), "missing settings field {key}");
    }
    assert_eq!(values["parameter:0:default"], "4");
    assert_eq!(values["parameter:0:choices"], "4, 8");
}

#[test]
fn tui_parameter_rows_round_trip_and_validate_every_editable_axis() {
    let values = BTreeMap::from([
        ("parameter:0:name".to_owned(), "token".to_owned()),
        ("parameter:0:binding".to_owned(), "none".to_owned()),
        ("parameter:0:delivery".to_owned(), "env".to_owned()),
        ("parameter:0:type".to_owned(), "choice".to_owned()),
        ("parameter:0:default".to_owned(), "green".to_owned()),
        ("parameter:0:choices".to_owned(), "red, green".to_owned()),
        ("parameter:0:required".to_owned(), "true".to_owned()),
        ("parameter:0:multiple".to_owned(), "true".to_owned()),
        ("parameter:0:repeat".to_owned(), "true".to_owned()),
        ("parameter:0:prompt".to_owned(), "Token".to_owned()),
        ("parameter:0:help".to_owned(), "Select a token.".to_owned()),
        ("parameter:0:secret".to_owned(), "true".to_owned()),
        (
            "parameter:0:env_source".to_owned(),
            "TOKEN_SOURCE".to_owned(),
        ),
        (
            "parameter:0:env_target".to_owned(),
            "TOKEN_TARGET".to_owned(),
        ),
        ("parameter:0:flag".to_owned(), "--token".to_owned()),
        ("parameter:0:action".to_owned(), "append".to_owned()),
    ]);

    let declarations = tui_declarations_from_values(&values).unwrap();
    assert_eq!(declarations.len(), 1);
    assert_eq!(
        declarations[0],
        ParamDecl {
            name: "token".to_owned(),
            binding: ParameterBinding::None,
            delivery: ParameterDelivery::Env,
            parameter_type: ParameterType::Choice,
            default: Some(ParameterValue::String("green".to_owned())),
            required: true,
            multiple: true,
            repeat: true,
            choices: vec!["red".to_owned(), "green".to_owned()],
            prompt: "Token".to_owned(),
            help: "Select a token.".to_owned(),
            secret: true,
            env_source: "TOKEN_SOURCE".to_owned(),
            flag: "--token".to_owned(),
            action: "append".to_owned(),
            order: -1,
            env_target: "TOKEN_TARGET".to_owned(),
            degraded: false,
        }
    );

    let duplicate = BTreeMap::from([
        ("parameter:0:name".to_owned(), "same".to_owned()),
        ("parameter:1:name".to_owned(), "same".to_owned()),
    ]);
    assert!(
        tui_declarations_from_values(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate parameter")
    );
}

#[test]
fn tui_source_controls_change_only_the_stored_copy() {
    let root = TempDir::new().unwrap();
    let original = root.path().join("source.sh");
    fs::write(&original, "NAME=world\necho \"$NAME\"\n").unwrap();
    let store = FileStore::new(root.path().join("library"));
    let service = LibraryService::new(store.clone());
    add(
        &service,
        AddOptions {
            source: Some(original.clone()),
            kind: None,
            name: Some("Tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let mut entry = service.show("tool").unwrap();
    let stored = store
        .entry_dir_path(&Slug::parse("tool").unwrap())
        .join("script.sh");
    let mut managed = ParamDecl::new("NAME");
    managed.binding = ParameterBinding::Const;
    managed.delivery = ParameterDelivery::Inject;
    managed.prompt = "Keep this prompt".to_owned();
    let rewritten =
        write_managed_params("shell", "NAME=world\necho \"$NAME\"\n", &[managed]).unwrap();
    let claimed = service.claim_identity(&entry).unwrap();
    entry = service
        .commit_copy_edit(&claimed, rewritten.as_bytes(), &entry.meta.source_hash)
        .unwrap();
    let source = fs::read_to_string(&stored).unwrap();
    let (source, managed) =
        prepare_source_management("shell", StorageMode::Copy, source, true, &[], &[], &[]).unwrap();
    let rewritten = write_managed_params("shell", &source, &managed).unwrap();
    let claimed = service.claim_identity(&entry).unwrap();
    entry = service
        .commit_copy_edit(&claimed, rewritten.as_bytes(), &entry.meta.source_hash)
        .unwrap();
    assert_eq!(
        managed_params("shell", &fs::read_to_string(&stored).unwrap())[0].prompt,
        "Keep this prompt"
    );

    let source = fs::read_to_string(&stored).unwrap();
    let (source, managed) = prepare_source_management(
        "shell",
        StorageMode::Copy,
        source,
        false,
        &[],
        &[],
        &["NAME".to_owned()],
    )
    .unwrap();
    let rewritten = write_managed_params("shell", &source, &managed).unwrap();
    let claimed = service.claim_identity(&entry).unwrap();
    service
        .commit_copy_edit(&claimed, rewritten.as_bytes(), &entry.meta.source_hash)
        .unwrap();

    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "NAME=world\necho \"$NAME\"\n"
    );
    assert!(
        fs::read_to_string(stored)
            .unwrap()
            .contains("NAME=\"${NAME:-world}\"")
    );
}

#[test]
fn plain_form_collection_uses_defaults_masks_secrets_and_refuses_end_of_input() {
    let form = FormView {
        purpose: FormPurpose::Run,
        title: "Run".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: Some("alpha".to_owned()),
        fields: vec![
            FormField::text("name", "Name", "default"),
            FormField::secret("token", "Token", ""),
            FormField::text_raw("raw", "User label", ""),
        ],
        focused: 0,
        submit_label: "Run".to_owned(),
    };
    let mut output = Vec::new();
    let values = collect_plain_form(
        &form,
        skit_i18n::Locale::En,
        &mut "\ncustom\n".as_bytes(),
        &mut output,
        |_| Ok("hidden".to_owned()),
    )
    .unwrap();
    assert_eq!(values["name"], "default");
    assert_eq!(values["token"], "hidden");
    assert_eq!(values["raw"], "custom");
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Name [default]: "));
    assert!(output.contains("Token: "));
    assert!(!output.contains("hidden"));

    let error = collect_plain_form(
        &form,
        skit_i18n::Locale::En,
        &mut "".as_bytes(),
        &mut Vec::new(),
        |_| Ok(String::new()),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("ended before the form was complete")
    );
    assert!(matches!(
        plain_form_error(io::Error::new(io::ErrorKind::UnexpectedEof, "end")),
        CliError::Aborted
    ));
    assert!(matches!(
        plain_form_error(io::Error::other("read")),
        CliError::Io(_)
    ));
}

#[test]
fn interactive_run_submission_keeps_explicit_clears_and_cli_fixed_values() {
    let baseline = BTreeMap::from([
        ("name".to_owned(), "Ada".to_owned()),
        ("count".to_owned(), "2".to_owned()),
    ]);
    let submitted = BTreeMap::from([
        ("value:name".to_owned(), String::new()),
        ("value:count".to_owned(), "3".to_owned()),
    ]);
    assert_eq!(
        changed_form_values(&submitted, &baseline),
        ["count=3", "name="]
    );

    let declarations = [ParamDecl::new("name"), ParamDecl::new("count")];
    assert_eq!(
        run_fixed_values(
            &declarations,
            &["name=Grace".to_owned(), "count=".to_owned()]
        )
        .unwrap(),
        BTreeMap::from([
            ("count".to_owned(), String::new()),
            ("name".to_owned(), "Grace".to_owned()),
        ])
    );
    assert!(matches!(
        run_fixed_values(&declarations, &["missing=x".to_owned()]),
        Err(CliError::Run(RunError::UnknownSet { .. }))
    ));
    assert!(matches!(
        run_fixed_values(&declarations, &["broken".to_owned()]),
        Err(CliError::Run(RunError::InvalidSet { .. }))
    ));
}

#[test]
fn parameter_json_and_parser_helpers_cover_every_public_spelling() {
    assert_eq!(nonempty(""), None);
    assert_eq!(nonempty("value"), Some("value"));
    let source = |kind: &str, text: &str, parameters: Vec<ParamDecl>| {
        let settings = EntrySettings {
            parameters,
            ..EntrySettings::default()
        };
        form_plan(kind, text, &settings).source.as_str()
    };
    assert_eq!(source("command", "", Vec::new()), "command");
    assert_eq!(source("prompt", "", Vec::new()), "command");
    assert_eq!(source("python", "", Vec::new()), "none");
    assert_eq!(
        source("python", "", vec![ParamDecl::new("plain")]),
        "declared"
    );
    let managed_source = concat!(
        "# /// script\n",
        "# dependencies = []\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "# [[tool.skit.params]]\n",
        "# name = \"bound\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# ///\n",
        "bound = 'value'\n",
    );
    assert_eq!(source("python", managed_source, Vec::new()), "inject");
    for (source, origin) in [
        ("command", "command"),
        ("inject", "managed"),
        ("argparse", "reader"),
        ("declared", "declared"),
        ("none", "none"),
    ] {
        assert_eq!(parameter_origin(source), origin);
    }

    let mut declaration = ParamDecl::new("path");
    declaration.prompt = "Select path".to_owned();
    declaration.parameter_type = ParameterType::Path;
    declaration.delivery = ParameterDelivery::Env;
    declaration.default = Some(ParameterValue::String(String::new()));
    let record = field_json(
        &form_plan(
            "python",
            "",
            &EntrySettings {
                parameters: vec![declaration.clone()],
                ..EntrySettings::default()
            },
        )
        .fields[0],
    );
    assert_eq!(record["label"], "Select path");
    assert_eq!(record["delivers_empty"], true);
    declaration.secret = true;
    assert_eq!(
        field_json(
            &form_plan(
                "python",
                "",
                &EntrySettings {
                    parameters: vec![declaration.clone()],
                    ..EntrySettings::default()
                },
            )
            .fields[0]
        )["delivers_empty"],
        false
    );
    declaration.secret = false;
    declaration.multiple = true;
    assert_eq!(
        field_json(
            &form_plan(
                "python",
                "",
                &EntrySettings {
                    parameters: vec![declaration],
                    ..EntrySettings::default()
                },
            )
            .fields[0]
        )["delivers_empty"],
        false
    );

    assert_eq!(
        assignment("name=value", "field").unwrap(),
        ("name", "value")
    );
    assert!(assignment("=value", "field").is_err());
    assert!(assignment("value", "field").is_err());
    let mut declarations = [ParamDecl::new("item")];
    assert_eq!(
        parameter_mut(&mut declarations, "item").unwrap().name,
        "item"
    );
    assert!(parameter_mut(&mut declarations, "missing").is_err());

    for (value, expected) in [
        ("str", ParameterType::Str),
        ("int", ParameterType::Int),
        ("float", ParameterType::Float),
        ("bool", ParameterType::Bool),
        ("choice", ParameterType::Choice),
        ("path", ParameterType::Path),
    ] {
        assert_eq!(parse_parameter_type(value).unwrap(), expected);
    }
    assert!(parse_parameter_type("future").is_err());
    for (value, expected) in [
        ("inject", ParameterDelivery::Inject),
        ("env", ParameterDelivery::Env),
        ("flag", ParameterDelivery::Flag),
        ("placeholder", ParameterDelivery::Placeholder),
    ] {
        assert_eq!(parse_delivery(value).unwrap(), expected);
    }
    assert!(parse_delivery("future").is_err());
    assert!(set_bool(&mut declarations, &[], |item| &mut item.required, true).is_ok());
    assert!(!declarations[0].required);
    assert!(
        set_bool(
            &mut declarations,
            &["item".to_owned()],
            |item| &mut item.required,
            true,
        )
        .unwrap()
    );
    assert!(declarations[0].required);
    assert!(
        set_bool(
            &mut declarations,
            &["missing".to_owned()],
            |item| &mut item.required,
            false,
        )
        .is_err()
    );
}

#[test]
fn tui_scalar_helpers_reject_incomplete_and_incompatible_rows() {
    assert_eq!(
        tui_parameter_value(&ParameterValue::String("text".to_owned())),
        "text"
    );
    assert_eq!(tui_parameter_value(&ParameterValue::Integer(2)), "2");
    assert_eq!(tui_parameter_value(&ParameterValue::Float(2.5)), "2.5");
    assert_eq!(tui_parameter_value(&ParameterValue::Bool(true)), "true");
    let plain = tui_options_field("key", "Label", "Options", &[], "value");
    assert_eq!(plain.label, "Label");
    assert!(plain.label_arguments.is_empty());
    assert_eq!(
        tui_options_field(
            "key",
            "Label",
            "Options: {}",
            &["one".to_owned(), "two".to_owned()],
            "one",
        )
        .label_arguments,
        ["one, two"]
    );

    assert!(
        tui_declarations_from_values(&BTreeMap::new())
            .unwrap()
            .is_empty()
    );
    for (field, value) in [
        ("name", " "),
        ("binding", "future"),
        ("delivery", "future"),
        ("type", "future"),
    ] {
        let mut values = BTreeMap::from([("parameter:0:name".to_owned(), "item".to_owned())]);
        values.insert(format!("parameter:0:{field}"), value.to_owned());
        assert!(
            tui_declarations_from_values(&values).is_err(),
            "field={field}"
        );
    }
    let invalid_default = BTreeMap::from([
        ("parameter:0:name".to_owned(), "item".to_owned()),
        ("parameter:0:type".to_owned(), "int".to_owned()),
        ("parameter:0:default".to_owned(), "not-an-int".to_owned()),
    ]);
    assert!(tui_declarations_from_values(&invalid_default).is_err());
    let incompatible = BTreeMap::from([
        ("parameter:0:name".to_owned(), "item".to_owned()),
        ("parameter:0:type".to_owned(), "choice".to_owned()),
    ]);
    assert!(tui_declarations_from_values(&incompatible).is_err());

    assert!(tui_selector(&None).is_err());
    assert_eq!(tui_selector(&Some(String::new())).unwrap(), "");
    assert_eq!(tui_selector(&Some("item".to_owned())).unwrap(), "item");
    let values = BTreeMap::from([
        ("key".to_owned(), " value ".to_owned()),
        ("yes".to_owned(), "YES".to_owned()),
    ]);
    assert_eq!(tui_value(&values, "key"), "value");
    assert_eq!(tui_nonempty_owned(&values, "key"), Some("value".to_owned()));
    assert_eq!(tui_nonempty_owned(&values, "missing"), None);
    assert_eq!(tui_required(&values, "key").unwrap(), "value");
    assert!(tui_required(&values, "missing").is_err());
    for value in ["true", "YES", "1", "on"] {
        assert!(tui_bool(value).unwrap());
    }
    for value in ["", "false", "NO", "0", "off"] {
        assert!(!tui_bool(value).unwrap());
    }
    assert!(tui_bool("sometimes").is_err());
    assert_eq!(
        split_windows_arguments(r#"agent.exe "C:\Program Files\input.txt" """#).unwrap(),
        ["agent.exe", r"C:\Program Files\input.txt", ""]
    );
    let windows = ["agent.exe", r"C:\Program Files\input.txt", "", r#"a\"b"#];
    assert_eq!(
        split_windows_arguments(&join_windows_arguments(&windows)).unwrap(),
        windows
    );
    let executable = EntryKind::parse("exe").unwrap();
    assert_eq!(payload_stored_name(&executable, Path::new("")), "script");
    assert_eq!(
        payload_stored_name(&executable, Path::new("tool.custom")),
        "tool.custom"
    );
}

#[derive(Debug, Default)]
struct HealthProbe {
    programs: BTreeMap<String, std::path::PathBuf>,
    directories: Vec<std::path::PathBuf>,
}

impl ProgramProbe for HealthProbe {
    fn find_program(&self, name: &str) -> Option<std::path::PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.directories.iter().any(|directory| directory == path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        path.is_file()
    }
}

fn health_entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("health").unwrap(),
        meta: EntryMeta::minimal("Health", EntryKind::parse(kind).unwrap()),
    }
}

#[test]
fn doctor_launch_checks_cover_every_runtime_and_workdir_policy() {
    let root = TempDir::new().unwrap();
    let config = FileConfigStore::new(root.path());
    let mut entry = health_entry("shell");
    entry.meta.workdir = "relative".to_owned();
    assert!(
        doctor_launch_block(
            &entry,
            &EntrySettings::default(),
            &config,
            &HealthProbe::default()
        )
        .unwrap()
        .unwrap()
        .localize(Locale::En)
        .contains("must be absolute")
    );
    entry.meta.workdir = root.path().join("missing").display().to_string();
    assert!(
        doctor_launch_block(
            &entry,
            &EntrySettings::default(),
            &config,
            &HealthProbe::default()
        )
        .unwrap()
        .unwrap()
        .localize(Locale::En)
        .contains("does not exist")
    );
    entry.meta.workdir = root.path().display().to_string();
    let mut probe = HealthProbe {
        directories: vec![root.path().to_owned()],
        ..HealthProbe::default()
    };
    assert!(
        doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains("bash")
    );
    let custom_bash = root.path().join("custom-bash");
    fs::write(&custom_bash, "#!/bin/sh\n").unwrap();
    config
        .set("shell.bash_path", custom_bash.to_str().unwrap())
        .unwrap();
    assert!(
        doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains(custom_bash.to_str().unwrap())
    );

    for (kind, program) in [
        ("python", "uv"),
        ("fish", "fish"),
        ("powershell", "pwsh"),
        ("ruby", "ruby"),
        ("perl", "perl"),
        ("lua", "lua"),
        ("r", "Rscript"),
        ("command", if cfg!(windows) { "cmd.exe" } else { "sh" }),
    ] {
        let mut entry = health_entry(kind);
        entry.meta.workdir = "invoke".to_owned();
        assert!(
            doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe)
                .unwrap()
                .unwrap()
                .localize(Locale::En)
                .contains(program),
            "kind={kind}",
        );
        probe
            .programs
            .insert(program.to_owned(), std::path::PathBuf::from(program));
        assert_eq!(
            doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe).unwrap(),
            None,
            "kind={kind}",
        );
    }

    let javascript = health_entry("js");
    assert!(
        doctor_launch_block(
            &javascript,
            &EntrySettings::default(),
            &config,
            &HealthProbe::default(),
        )
        .unwrap()
        .unwrap()
        .localize(Locale::En)
        .contains("deno, bun, or node")
    );
    probe
        .programs
        .insert("node".to_owned(), std::path::PathBuf::from("node"));
    assert_eq!(
        doctor_launch_block(&javascript, &EntrySettings::default(), &config, &probe).unwrap(),
        None
    );

    let prompt = health_entry("prompt");
    let pinned = EntrySettings {
        runner: "agent".to_owned(),
        ..EntrySettings::default()
    };
    assert!(
        doctor_launch_block(&prompt, &pinned, &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains("not configured")
    );
    config
        .set_runner(
            skit_store::PromptRunner {
                name: "agent".to_owned(),
                argv: vec!["agent-bin".to_owned(), "{{prompt}}".to_owned()],
            },
            false,
        )
        .unwrap();
    assert!(
        doctor_launch_block(&prompt, &pinned, &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains("agent-bin")
    );
    probe.programs.insert(
        "agent-bin".to_owned(),
        std::path::PathBuf::from("agent-bin"),
    );
    assert_eq!(
        doctor_launch_block(&prompt, &pinned, &config, &probe).unwrap(),
        None
    );
    assert_eq!(
        doctor_launch_block(&prompt, &EntrySettings::default(), &config, &probe).unwrap(),
        None
    );
    assert_eq!(
        doctor_launch_block(
            &health_entry("exe"),
            &EntrySettings::default(),
            &config,
            &probe
        )
        .unwrap(),
        None
    );
    assert!(
        doctor_launch_block(
            &health_entry("future"),
            &EntrySettings::default(),
            &config,
            &probe,
        )
        .unwrap()
        .unwrap()
        .localize(Locale::En)
        .contains("unknown entry kind")
    );
    assert_eq!(
        interpreter_name(&EntrySettings::default(), "default"),
        "default"
    );
    assert_eq!(
        interpreter_name(
            &EntrySettings {
                interpreter: "custom".to_owned(),
                ..EntrySettings::default()
            },
            "default",
        ),
        "custom"
    );
    let custom = EntrySettings {
        interpreter: "custom-shell".to_owned(),
        ..EntrySettings::default()
    };
    let mut custom_probe = HealthProbe::default();
    custom_probe
        .programs
        .insert("custom-shell".to_owned(), root.path().join("custom-shell"));
    assert_eq!(
        doctor_launch_block(&health_entry("shell"), &custom, &config, &custom_probe).unwrap(),
        None
    );
}

#[test]
fn doctor_source_and_size_helpers_are_total_on_missing_corrupt_and_nested_paths() {
    let root = TempDir::new().unwrap();
    assert_eq!(directory_size(&root.path().join("missing")), 0);
    fs::write(root.path().join("one"), b"1234").unwrap();
    assert_eq!(directory_size(&root.path().join("one")), 0);
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("nested/two"), b"12").unwrap();
    assert_eq!(directory_size(root.path()), 6);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.path().join("one"), root.path().join("link")).unwrap();
        assert_eq!(directory_size(&root.path().join("link")), 0);
        assert_eq!(directory_size(root.path()), 10);
        std::os::unix::fs::symlink(root.path().join("nested"), root.path().join("dir-link"))
            .unwrap();
        assert_eq!(directory_size(root.path()), 10);
    }

    let library = root.path().join("library");
    let store = FileStore::new(&library);
    let directory = library.join("scripts/health");
    fs::create_dir_all(&directory).unwrap();
    let mut prompt = health_entry("prompt");
    EntrySettings {
        params: vec!["missing".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut prompt.meta);
    fs::write(directory.join("prompt.md"), "Hello {{name}}").unwrap();
    assert!(doctor_entry_drifted(&store, &prompt));
    EntrySettings {
        interpolate: false,
        params: vec!["missing".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut prompt.meta);
    assert!(!doctor_entry_drifted(&store, &prompt));
    fs::write(directory.join("prompt.md"), [0xff]).unwrap();
    assert!(!doctor_entry_drifted(&store, &prompt));
    fs::remove_file(directory.join("prompt.md")).unwrap();
    assert!(!doctor_entry_drifted(&store, &prompt));

    let mut current = health_entry("shell");
    let mut declaration = ParamDecl::new("NAME");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    fs::write(
        directory.join("script.sh"),
        write_managed_params("shell", "NAME=world\n", &[declaration]).unwrap(),
    )
    .unwrap();
    assert!(!doctor_entry_drifted(&store, &current));
    current.meta.kind = EntryKind::parse("prompt").unwrap();
    EntrySettings {
        params: vec!["name".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut current.meta);
    fs::write(directory.join("prompt.md"), "Hello {{name}}").unwrap();
    assert!(!doctor_entry_drifted(&store, &current));

    assert!(!entry_missing(&store, &health_entry("command")));
    let mut executable = health_entry("exe");
    executable.meta.mode = StorageMode::Reference;
    executable.meta.source = root.path().join("gone").display().to_string();
    assert!(entry_missing(&store, &executable));
    fs::write(root.path().join("gone"), "x").unwrap();
    assert!(!entry_missing(&store, &executable));
}

#[test]
fn shared_health_inspector_reports_typed_entry_runner_and_rebuild_facts() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[prompt]\nrunners_seeded = true\nrunners = \"broken\"\n",
    )
    .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    service
        .add(CreateEntry {
            name: "Missing program".to_owned(),
            kind: EntryKind::parse("exe").unwrap(),
            mode: StorageMode::Reference,
            source: root.path().join("gone-program").display().to_string(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    service
        .add(CreateEntry {
            name: "Missing need".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings {
                template: "printf ok".to_owned(),
                needs: vec!["skit-health-tool-that-does-not-exist".to_owned()],
                ..EntrySettings::default()
            },
        })
        .unwrap();

    let inspector = CliHealthInspector::new(&service, &store, &config_dir);
    let snapshot = inspector.inspect().unwrap();
    assert_eq!(snapshot.entry_count, 2);
    assert!(!matches!(snapshot.uv, UvHealth::Missing));
    assert!(snapshot.issues.iter().any(|issue| {
        issue.name == "Missing program" && issue.kind == HealthIssueKind::MissingTarget
    }));
    assert!(snapshot.issues.iter().any(|issue| {
        issue.name == "Missing need"
            && issue.kind
                == HealthIssueKind::MissingNeeds {
                    tools: vec!["skit-health-tool-that-does-not-exist".to_owned()],
                }
    }));
    assert_eq!(snapshot.invalid_runner_rows.len(), 1);
    assert_eq!(
        snapshot.library_size,
        health_size_text(directory_size(&data_dir.join("scripts")))
    );

    let rebuilt = inspector.rebuild().unwrap();
    assert_eq!(rebuilt.outcome.entry_count, 2);
    assert_eq!(rebuilt.snapshot.entry_count, 2);
}

#[test]
fn health_size_text_matches_the_latest_main_units_and_thresholds() {
    assert_eq!(health_size_text(0), "0 B");
    assert_eq!(health_size_text(1023), "1023 B");
    assert_eq!(health_size_text(1024), "1.0 KB");
    assert_eq!(health_size_text(1024 * 1024), "1.0 MB");
    assert_eq!(health_size_text(1024 * 1024 * 1024), "1.0 GB");
}

#[test]
fn tui_host_opens_every_frontend_neutral_screen_and_handles_simple_effects() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut entry = add_command(&service, "Alpha", "printf '%s' '{name}'");
    let declarations = entry_parameters(&store, &entry);
    let state = FormStateService::new(FileFormStateStore::new(&state_dir));
    state
        .save_last(
            &entry.slug,
            &declarations,
            Some(&BTreeMap::from([("name".to_owned(), "Ada".to_owned())])),
            None,
            false,
        )
        .unwrap();
    let mut settings = EntrySettings::from_meta(&entry.meta);
    settings.runner = "codex".to_owned();
    let claimed = service.claim_identity(&entry).unwrap();
    entry = service
        .update_settings(&claimed, &settings, "invoke")
        .unwrap();

    let broken = data_dir.join("scripts/broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("meta.toml"), "name = [broken").unwrap();
    state
        .save_preset(
            &entry.slug,
            "fast",
            &declarations,
            &BTreeMap::from([("name".to_owned(), "Grace".to_owned())]),
        )
        .unwrap();

    FileConfigStore::new(&config_dir)
        .ensure_runners_seeded()
        .unwrap();

    let Screen::Run(run_form) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Run,
        Some(entry.slug.as_str().to_owned()),
    )
    .unwrap() else {
        panic!("run must open the typed launch form");
    };
    assert!(
        run_form
            .fields()
            .iter()
            .all(|field| field.key != "_skit_runner"),
        "non-prompt entries must not show the prompt-runner picker"
    );
    let context = run_form
        .context()
        .expect("the real host must attach context");
    assert_eq!(context.entry_kind, "command");
    assert_eq!(
        context.path.as_ref().unwrap().workdir,
        std::env::current_dir().unwrap().display().to_string()
    );
    assert_eq!(
        context.tokens.cwd,
        context.path.as_ref().unwrap().invoke_cwd
    );
    assert_eq!(
        run_form
            .fields()
            .iter()
            .find(|field| field.key == "_skit_args")
            .unwrap()
            .label,
        "Extra command arguments"
    );
    for request in [
        HostRequest::Settings,
        HostRequest::Presets,
        HostRequest::Rename,
    ] {
        assert!(matches!(
            tui_open(
                &service,
                &store,
                &state_dir,
                &config_dir,
                request,
                Some(entry.slug.as_str().to_owned()),
            )
            .unwrap(),
            Screen::Form(_)
        ));
    }
    assert!(matches!(
        tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Add,
            None,
        )
        .unwrap(),
        Screen::Add(_)
    ));
    assert!(matches!(
        tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Runners,
            None,
        )
        .unwrap(),
        Screen::Runners(_)
    ));
    let Screen::Preferences(preferences) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Preferences,
        None,
    )
    .unwrap() else {
        panic!("Preferences must use its typed workflow");
    };
    assert_eq!(preferences.draft().language_options[0], "auto");
    assert_eq!(preferences.draft().runner_names.len(), 8);
    assert!(matches!(
        tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Health,
            None,
        )
        .unwrap(),
        Screen::Health(_)
    ));

    for effect in [UiEffect::None, UiEffect::Quit] {
        assert_eq!(
            tui_effect(&service, &store, &state_dir, &config_dir, effect,).unwrap(),
            UiAction::ClearStatus
        );
    }
    assert!(matches!(
        tui_effect(&service, &store, &state_dir, &config_dir, UiEffect::Reload,).unwrap(),
        UiAction::Replace { .. }
    ));
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Open {
                request: HostRequest::Run,
                selector: Some(entry.slug.as_str().to_owned()),
            },
        )
        .unwrap(),
        UiAction::Present(Screen::Run(_))
    ));

    let source = root.path().join("editable.sh");
    fs::write(&source, "echo ok\n").unwrap();
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Editable".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    FileConfigStore::new(&config_dir)
        .set("editor", "true")
        .unwrap();
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Edit {
                selector: "editable".to_owned(),
            },
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    let editable = service.show("editable").unwrap();
    let editable_values = form_values(tui_settings_form(&store, &editable));
    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Settings,
            Some(editable.slug.as_str().to_owned()),
            &editable_values,
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));

    let disposable = add_command(&service, "Disposable", "true");
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Remove {
                selector: disposable.slug.as_str().to_owned(),
            },
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    assert!(service.show("Disposable").is_err());
}

#[test]
fn tui_runner_host_preserves_editor_input_on_stale_rows_and_rechecks_prompt_pins() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        concat!(
            "[prompt]\n",
            "runners_seeded = true\n",
            "[[prompt.runners]]\n",
            "name = \"agent\"\n",
            "argv = [\"agent\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());

    let row = tui_runner_rows(&service, &config_dir)
        .unwrap()
        .into_iter()
        .find(|row| row.name.as_deref() == Some("agent"))
        .unwrap();
    let mut stale = row.key_identities.clone();
    stale[0].snapshot_token.push_str("-stale");
    let action = tui_save_runner(
        &service,
        &config_dir,
        RunnerSaveRequest {
            name: "agent".to_owned(),
            argv: vec![
                "agent".to_owned(),
                "--once".to_owned(),
                "{{prompt}}".to_owned(),
            ],
            target: RunnerSaveTarget::Named {
                name: "agent".to_owned(),
                expected: stale,
            },
        },
        RunnerSaveOwner::Manager,
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Runners(RunnerManagerAction::MutationFailed(_))
    ));
    assert_eq!(
        FileConfigStore::new(&config_dir).runners().unwrap()[0].argv,
        ["agent", "{{prompt}}"]
    );

    let add_pinned_prompt = |name: &str| {
        let mut settings = EntrySettings {
            runner: "agent".to_owned(),
            ..EntrySettings::default()
        };
        settings.params = Vec::new();
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("prompt").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"Review this".to_vec(),
                    stored_name: Some("prompt.md".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings,
            })
            .unwrap();
    };
    add_pinned_prompt("First prompt");
    let row = tui_runner_rows(&service, &config_dir)
        .unwrap()
        .into_iter()
        .find(|row| row.name.as_deref() == Some("agent"))
        .unwrap();
    assert_eq!(row.pinned_count, 1);
    add_pinned_prompt("Second prompt");
    let action = tui_remove_runner(
        &service,
        &config_dir,
        RunnerRemoveRequest::Named {
            name: "agent".to_owned(),
            expected: row.key_identities,
            expected_pinned_count: row.pinned_count,
        },
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Runners(RunnerManagerAction::MutationFailed(_))
    ));
    assert_eq!(
        FileConfigStore::new(&config_dir).runners().unwrap().len(),
        1
    );
}

#[test]
fn tui_runner_host_routes_success_to_the_exact_standalone_editor_owner() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let owner = skit_ui::RunnerEditorOwner::Run {
        selector: "prompt".to_owned(),
    };
    let action = tui_save_runner(
        &service,
        &config_dir,
        RunnerSaveRequest {
            name: "new-agent".to_owned(),
            argv: vec!["new-agent".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::New,
        },
        RunnerSaveOwner::Editor(owner.clone()),
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::RunnerEditorSaved {
            owner: saved_owner,
            ref name,
            ..
        } if saved_owner == owner && name == "new-agent"
    ));
    assert!(
        FileConfigStore::new(&config_dir)
            .runners()
            .unwrap()
            .iter()
            .any(|runner| runner.name == "new-agent")
    );
    let action = tui_effect(
        &service,
        &store,
        &root.path().join("state"),
        &config_dir,
        UiEffect::RefreshPreferencesAfterRunners,
    )
    .unwrap();
    let UiAction::RunnerManagerClosed { preferences } = action else {
        panic!("closing the runner manager must refresh typed Preferences");
    };
    assert!(
        preferences
            .draft()
            .runner_names
            .iter()
            .any(|name| name == "new-agent")
    );
}

#[test]
fn tui_run_host_publishes_source_drift_degradation_and_runtime_path_context() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");

    let mut managed = ParamDecl::new("NAME");
    managed.binding = ParameterBinding::Const;
    managed.delivery = ParameterDelivery::Inject;
    let drift_dir = data_dir.join("scripts/drift");
    fs::create_dir_all(&drift_dir).unwrap();
    fs::write(
        drift_dir.join("meta.toml"),
        "name = \"Drift\"\nkind = \"shell\"\nmode = \"copy\"\ndescription = \"\"\n",
    )
    .unwrap();
    fs::write(
        drift_dir.join("script.sh"),
        write_managed_params("shell", "echo done\n", &[managed]).unwrap(),
    )
    .unwrap();

    let dynamic_dir = data_dir.join("scripts/dynamic");
    fs::create_dir_all(&dynamic_dir).unwrap();
    fs::write(
        dynamic_dir.join("meta.toml"),
        "name = \"Dynamic\"\nkind = \"python\"\nmode = \"copy\"\ndescription = \"\"\n",
    )
    .unwrap();
    fs::write(
        dynamic_dir.join("script.py"),
        "p.add_argument('--x')\np.add_subparsers()\n",
    )
    .unwrap();

    let store = FileStore::new(&data_dir);
    store.rebuild_registry().unwrap();
    let service = LibraryService::new(store.clone());
    let Screen::Run(drift) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Run,
        Some("drift".to_owned()),
    )
    .unwrap() else {
        panic!("run must open the typed launch form");
    };
    assert!(!drift.drift_lines.is_empty());
    let context = drift.context().unwrap();
    assert_eq!(context.entry_kind, "shell");
    assert_eq!(
        context.path.as_ref().unwrap().workdir,
        std::env::current_dir().unwrap().display().to_string()
    );

    let Screen::Run(dynamic) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Run,
        Some("dynamic".to_owned()),
    )
    .unwrap() else {
        panic!("run must open the typed launch form");
    };
    assert_eq!(dynamic.degraded_reason.as_deref(), Some("subparsers"));
}

#[test]
fn typed_preferences_effects_validate_atomically_and_install_only_after_selection() {
    let root = TempDir::new().unwrap();
    let config_dir = root.path().join("config");
    let service = LibraryService::new(FileStore::new(root.path().join("data")));
    let config = FileConfigStore::new(&config_dir);
    config.set("editor", "vi").unwrap();

    let refused = skit_application::preferences::PreferencesChangeSet {
        settings: BTreeMap::from([
            ("editor".to_owned(), "micro".to_owned()),
            (
                "shell.bash_path".to_owned(),
                root.path().join("missing-bash").display().to_string(),
            ),
        ]),
    };
    assert!(matches!(
        tui_preferences_effect(&service, &config_dir, PreferencesEffect::Save(refused)).unwrap(),
        UiAction::Preferences(PreferencesAction::ValidationFailed(_))
    ));
    assert_eq!(config.get("editor").unwrap(), "vi");

    let accepted = skit_application::preferences::PreferencesChangeSet {
        settings: BTreeMap::from([
            ("editor".to_owned(), "micro".to_owned()),
            ("lang".to_owned(), "zh-TW".to_owned()),
        ]),
    };
    assert_eq!(
        tui_preferences_effect(&service, &config_dir, PreferencesEffect::Save(accepted)).unwrap(),
        UiAction::PreferencesSaved {
            locale: "zh-TW".to_owned(),
            message: "Preferences saved".to_owned(),
        }
    );
    assert_eq!(config.get("editor").unwrap(), "micro");
    assert_eq!(config.get("lang").unwrap(), "zh-TW");

    assert!(matches!(
        tui_preferences_effect(
            &service,
            &config_dir,
            PreferencesEffect::DiscoverAgentSkillTargets,
        )
        .unwrap(),
        UiAction::Preferences(PreferencesAction::PresentAgentSkillTargets(_))
    ));

    let skills_dir = root.path().join("agent/skills");
    let action = tui_preferences_effect(
        &service,
        &config_dir,
        PreferencesEffect::InstallAgentSkill {
            skills_dir: skills_dir.clone(),
        },
    )
    .unwrap();
    let UiAction::Preferences(PreferencesAction::AgentSkillInstalled { message }) = action else {
        panic!("successful install must stay in typed Preferences");
    };
    let written = skills_dir.join("skit/SKILL.md");
    assert!(message.contains(&written.display().to_string()));
    assert_eq!(
        fs::read(written).unwrap(),
        include_bytes!("../../../../skills/skit/SKILL.md")
    );
}

#[test]
fn typed_run_feedback_and_preset_effects_use_application_and_store_ports() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Snapshot", "printf '%s' '{name} {token}'");
    fs::write(root.path().join("one.txt"), "").unwrap();
    fs::write(root.path().join("two.txt"), "").unwrap();

    assert_eq!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::CountRunGlob {
                selector: entry.slug.as_str().to_owned(),
                field: 2,
                value: "*.txt".to_owned(),
                request: skit_application::form_feedback::GlobCountRequest {
                    cwd: root.path().display().to_string(),
                    pieces: vec!["*.txt".to_owned()],
                },
            },
        )
        .unwrap(),
        UiAction::SetRunGlobCount {
            field: 2,
            value: "*.txt".to_owned(),
            count: 2,
        }
    );

    let action = tui_effect(
        &service,
        &store,
        &state_dir,
        &config_dir,
        UiEffect::SaveRunPreset {
            selector: entry.slug.as_str().to_owned(),
            name: "quick".to_owned(),
            values: BTreeMap::from([
                ("name".to_owned(), "Ada".to_owned()),
                ("token".to_owned(), "do-not-store".to_owned()),
            ]),
            secret_names: std::collections::BTreeSet::from(["token".to_owned()]),
        },
    )
    .unwrap();
    let UiAction::RunPresetSaved { presets, .. } = action else {
        panic!("preset save must refresh the typed run form");
    };
    assert_eq!(
        presets["quick"],
        BTreeMap::from([("name".to_owned(), "Ada".to_owned())])
    );
    assert_eq!(
        FormStateService::new(FileFormStateStore::new(&state_dir))
            .load(&entry.slug)
            .presets["quick"],
        BTreeMap::from([("name".to_owned(), "Ada".to_owned())])
    );
}

#[test]
fn tui_rerun_replays_only_honest_valid_state_and_falls_back_to_the_form() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let state = FormStateService::new(FileFormStateStore::new(&state_dir));
    FileConfigStore::new(&config_dir)
        .set("after_run", "stay")
        .unwrap();

    let runnable = add_command(&service, "Runnable", "true");
    state
        .record_run(
            &runnable.slug,
            0,
            "2026-08-08T00:00:00Z",
            &[],
            Some(&BTreeMap::new()),
        )
        .unwrap();
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Rerun {
                selector: runnable.slug.as_str().to_owned(),
            },
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));

    let invalid = add_command(&service, "Invalid", "printf '%s' '{required}'");
    let declarations = entry_parameters(&store, &invalid);
    state
        .record_run(
            &invalid.slug,
            0,
            "2026-08-08T00:00:00Z",
            &declarations,
            Some(&BTreeMap::new()),
        )
        .unwrap();
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Rerun {
                selector: invalid.slug.as_str().to_owned(),
            },
        )
        .unwrap(),
        UiAction::Present(Screen::Run(_))
    ));

    let fresh = add_command(&service, "Fresh", "true");
    assert!(matches!(
        tui_rerun(
            &service,
            &store,
            &state_dir,
            &config_dir,
            fresh.slug.as_str(),
        )
        .unwrap(),
        UiAction::SetStatus(message) if message.contains("hasn't run yet")
    ));

    let prompt_source = root.path().join("review.prompt.md");
    fs::write(&prompt_source, "Review {{target}}.\n").unwrap();
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(prompt_source),
            kind: Some("prompt".to_owned()),
            name: Some("Review".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let prompt = service.show("Review").unwrap();
    let prompt_declarations = entry_parameters(&store, &prompt);
    state
        .record_run(
            &prompt.slug,
            0,
            "2026-08-08T00:00:00Z",
            &prompt_declarations,
            Some(&BTreeMap::from([(
                "target".to_owned(),
                "workspace".to_owned(),
            )])),
        )
        .unwrap();
    assert!(matches!(
        tui_rerun(
            &service,
            &store,
            &state_dir,
            &config_dir,
            prompt.slug.as_str(),
        )
        .unwrap(),
        UiAction::Present(Screen::Run(_))
    ));

    let scan = service.list().unwrap();
    let rerunnable = tui_rerunnable(&scan, &state_dir);
    assert!(rerunnable.contains(&runnable.slug));
    assert!(rerunnable.contains(&invalid.slug));
    assert!(!rerunnable.contains(&fresh.slug));
}

#[test]
fn tui_host_submits_every_form_without_global_process_state() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let alpha = add_command(&service, "Alpha", "printf '%s' '{name}'");
    let runnable = add_command(&service, "Runnable", "true");

    let add_values = BTreeMap::from([
        ("name".to_owned(), "Added".to_owned()),
        ("kind".to_owned(), "command".to_owned()),
        ("description".to_owned(), "From TUI".to_owned()),
        ("mode".to_owned(), "copy".to_owned()),
        ("template".to_owned(), "echo {value}".to_owned()),
        ("dependencies".to_owned(), String::new()),
        ("python".to_owned(), String::new()),
    ]);
    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Add,
            None,
            &add_values,
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    assert_eq!(service.show("Added").unwrap().meta.description, "From TUI");
    let invalid_add = BTreeMap::from([
        ("name".to_owned(), "Invalid add".to_owned()),
        ("kind".to_owned(), "command".to_owned()),
        ("template".to_owned(), "true".to_owned()),
        ("dependencies".to_owned(), "not-supported".to_owned()),
    ]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Add,
            None,
            &invalid_add,
        )
        .is_err()
    );
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Rename,
            Some(alpha.slug.as_str().to_owned()),
            &BTreeMap::from([("name".to_owned(), "Runnable".to_owned())]),
        )
        .is_err()
    );

    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Rename,
            Some(alpha.slug.as_str().to_owned()),
            &BTreeMap::from([("name".to_owned(), "Renamed Tool".to_owned())]),
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    let renamed = service.show("Renamed Tool").unwrap();

    let preferences = BTreeMap::from([
        ("lang".to_owned(), "en".to_owned()),
        ("editor".to_owned(), "vi".to_owned()),
        ("form".to_owned(), "plain".to_owned()),
        ("after_run".to_owned(), "stay".to_owned()),
        ("shell.bash_path".to_owned(), String::new()),
        ("js.runner".to_owned(), String::new()),
        ("mirror".to_owned(), "off".to_owned()),
        ("mirror.pypi".to_owned(), "off".to_owned()),
        ("mirror.github".to_owned(), "off".to_owned()),
        ("mirror.npm".to_owned(), "off".to_owned()),
    ]);
    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Preferences,
            None,
            &preferences,
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));

    let runner = BTreeMap::from([
        ("name".to_owned(), "local".to_owned()),
        ("argv".to_owned(), "printf {{prompt}}".to_owned()),
        ("remove".to_owned(), "false".to_owned()),
    ]);
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Runners,
        None,
        &runner,
    )
    .unwrap();
    let invalid_runner = BTreeMap::from([
        ("name".to_owned(), "bad".to_owned()),
        ("argv".to_owned(), "'unterminated".to_owned()),
    ]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Runners,
            None,
            &invalid_runner,
        )
        .is_err()
    );
    let invalid_definition = BTreeMap::from([
        ("name".to_owned(), "bad-definition".to_owned()),
        ("argv".to_owned(), "true".to_owned()),
    ]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Runners,
            None,
            &invalid_definition,
        )
        .is_err()
    );
    let remove_unknown = BTreeMap::from([
        ("name".to_owned(), "missing".to_owned()),
        ("remove".to_owned(), "true".to_owned()),
    ]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Runners,
            None,
            &remove_unknown,
        )
        .is_err()
    );
    let remove_runner = BTreeMap::from([
        ("name".to_owned(), "local".to_owned()),
        ("remove".to_owned(), "true".to_owned()),
    ]);
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Runners,
        None,
        &remove_runner,
    )
    .unwrap();

    let preset_save = BTreeMap::from([
        ("name".to_owned(), "empty".to_owned()),
        ("action".to_owned(), "save".to_owned()),
    ]);
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Presets,
        Some(renamed.slug.as_str().to_owned()),
        &preset_save,
    )
    .unwrap();
    let preset_delete = BTreeMap::from([
        ("name".to_owned(), "empty".to_owned()),
        ("action".to_owned(), "delete".to_owned()),
    ]);
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Presets,
        Some(renamed.slug.as_str().to_owned()),
        &preset_delete,
    )
    .unwrap();
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Presets,
            Some(renamed.slug.as_str().to_owned()),
            &preset_delete,
        )
        .is_err()
    );

    let mut settings = form_values(
        tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Settings,
            Some(renamed.slug.as_str().to_owned()),
        )
        .unwrap(),
    );
    let mut duplicate = settings.clone();
    duplicate.insert("parameter:add".to_owned(), "same same".to_owned());
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Settings,
            Some(renamed.slug.as_str().to_owned()),
            &duplicate,
        )
        .is_err()
    );
    settings.insert("name".to_owned(), "Configured".to_owned());
    settings.insert("description".to_owned(), "Configured in TUI".to_owned());
    settings.insert("workdir".to_owned(), "invoke".to_owned());
    settings.insert("template".to_owned(), "printf %s {name}".to_owned());
    settings.insert("parameter:add".to_owned(), "fresh".to_owned());
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Settings,
        Some(renamed.slug.as_str().to_owned()),
        &settings,
    )
    .unwrap();
    let configured = service.show("Configured").unwrap();
    assert_eq!(configured.meta.description, "Configured in TUI");

    let run_values = BTreeMap::from([
        ("value:name".to_owned(), "Ada".to_owned()),
        ("_skit_save_preset".to_owned(), "from-tui".to_owned()),
        ("_skit_args".to_owned(), "tail".to_owned()),
        ("_skit_dry_run".to_owned(), "false".to_owned()),
    ]);
    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Run,
            Some(configured.slug.as_str().to_owned()),
            &run_values,
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    assert!(
        FormStateService::new(FileFormStateStore::new(&state_dir))
            .load(&configured.slug)
            .presets
            .contains_key("from-tui")
    );

    let invalid_args = BTreeMap::from([("_skit_args".to_owned(), "'bad".to_owned())]);
    assert!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            runnable.slug.as_str(),
            &invalid_args,
        )
        .is_err()
    );
    assert!(matches!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            runnable.slug.as_str(),
            &BTreeMap::new(),
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    FileConfigStore::new(&config_dir)
        .set("after_run", "exit")
        .unwrap();
    assert_eq!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            runnable.slug.as_str(),
            &BTreeMap::new(),
        )
        .unwrap(),
        UiAction::Quit
    );

    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Run,
            None,
            &BTreeMap::new(),
        )
        .is_err()
    );
    assert!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            "missing",
            &BTreeMap::new(),
        )
        .is_err()
    );
}

#[test]
fn tui_python_dependencies_use_the_same_pep_723_source_as_the_cli() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    fs::write(&source, "print('ok')\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: vec!["requests>=2,<3".to_owned()],
            dependencies_explicit: true,
            requires_python: Some(">=3.12".to_owned()),
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("python-tool").unwrap();
    let mut values = form_values(tui_settings_form(&store, &entry));
    assert_eq!(values["dependencies"], "requests>=2,<3");
    values.insert("dependencies".to_owned(), "requests>=2,<3\nrich".to_owned());
    values.insert("python".to_owned(), ">=3.13".to_owned());

    tui_submit_settings(&service, &store, &state_dir, "python-tool", &values).unwrap();

    let stored = fs::read_to_string(data_dir.join("scripts/python-tool/script.py")).unwrap();
    assert!(stored.contains("requests>=2,<3"));
    assert!(stored.contains("\"rich\""));
    assert!(stored.contains("requires-python = \">=3.13\""));
    let meta = fs::read_to_string(data_dir.join("scripts/python-tool/meta.toml")).unwrap();
    let meta: toml::Value = toml::from_str(&meta).unwrap();
    assert_eq!(
        meta["dependencies"],
        toml::Value::Array(vec![
            toml::Value::String("requests>=2,<3".to_owned()),
            toml::Value::String("rich".to_owned()),
        ])
    );
    assert_eq!(meta["requires_python"].as_str(), Some(">=3.13"));
}

#[test]
fn tui_settings_source_refusal_does_not_commit_other_fields() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    fs::write(&source, "NAME=world\necho \"$NAME\"\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Shell tool".to_owned()),
            description: Some("before".to_owned()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("shell-tool").unwrap();
    let source_path = data_dir.join("scripts/shell-tool/script.sh");
    let meta_path = data_dir.join("scripts/shell-tool/meta.toml");
    let source_before = fs::read(&source_path).unwrap();
    let meta_before = fs::read(&meta_path).unwrap();
    let mut values = form_values(tui_settings_form(&store, &entry));
    values.insert("name".to_owned(), "Must not land".to_owned());
    values.insert("description".to_owned(), "must not land".to_owned());
    values.insert("source:normalize".to_owned(), "MISSING".to_owned());

    assert!(tui_submit_settings(&service, &store, &state_dir, "shell-tool", &values).is_err());
    assert_eq!(fs::read(source_path).unwrap(), source_before);
    assert_eq!(fs::read(meta_path).unwrap(), meta_before);
    assert_eq!(service.show("shell-tool").unwrap().meta.name, "Shell tool");
}

#[test]
fn tui_template_updates_reconcile_the_placeholder_schema() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Template form", "echo {old}");
    let mut values = form_values(tui_settings_form(&store, &entry));
    values.insert("template".to_owned(), "echo {new}".to_owned());

    tui_submit_settings(&service, &store, &state_dir, "template-form", &values).unwrap();

    let updated = service.show("template-form").unwrap();
    let settings = EntrySettings::from_meta(&updated.meta);
    assert_eq!(settings.params, ["new"]);
    assert_eq!(
        entry_parameters(&store, &updated)
            .into_iter()
            .map(|field| field.name)
            .collect::<Vec<_>>(),
        ["new"]
    );
}

#[test]
fn tui_selected_preset_replaces_unchanged_last_used_values() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Preset form", "echo {name}");
    let declarations = entry_parameters(&store, &entry);
    let state = FormStateService::new(FileFormStateStore::new(&state_dir));
    let last = BTreeMap::from([("name".to_owned(), "last".to_owned())]);
    let preset = BTreeMap::from([("name".to_owned(), "preset".to_owned())]);
    state
        .save_last(&entry.slug, &declarations, Some(&last), None, false)
        .unwrap();
    state
        .save_preset(&entry.slug, "work", &declarations, &preset)
        .unwrap();
    let values = BTreeMap::from([
        ("value:name".to_owned(), "last".to_owned()),
        ("_skit_preset".to_owned(), "work".to_owned()),
        ("_skit_save_preset".to_owned(), "snapshot".to_owned()),
        ("_skit_dry_run".to_owned(), "true".to_owned()),
    ]);

    tui_submit_run(
        &service,
        &store,
        &state_dir,
        &config_dir,
        entry.slug.as_str(),
        &values,
    )
    .unwrap();

    assert_eq!(
        state.load(&entry.slug).presets["snapshot"]["name"],
        "preset"
    );
}

#[test]
fn tui_run_refuses_an_unknown_boolean_value() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Boolean form", "true");
    let values = BTreeMap::from([("_skit_dry_run".to_owned(), "sometimes".to_owned())]);

    assert!(matches!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            entry.slug.as_str(),
            &values,
        ),
        Err(CliError::Usage(_))
    ));
}

/// Check that English text does not drift and that each locale fills every hole.
fn assert_localized(error: &CliError, values: &[&str]) {
    let message = error.message();
    assert_eq!(error.to_string(), message.localize(Locale::En));
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let text = message.localize(locale);
        let template = message.template();
        assert!(!text.trim().is_empty(), "{template} is empty");
        assert!(!text.contains("{}"), "{template} kept an empty hole");
        for value in values {
            assert!(text.contains(value), "{text} lost the value {value}");
        }
    }
}

#[test]
fn every_cli_error_localizes_and_keeps_its_values() {
    let io_failure = || io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");

    assert_localized(
        &CliError::Repository(RepositoryError::NotFound {
            query: "missing".to_owned(),
        }),
        &["missing"],
    );
    assert_localized(&CliError::Run(crate::run::RunError::RawConflict), &[]);
    assert_localized(
        &CliError::Dependencies(skit_runtime::DependencyError::InstallerNotFound {
            name: "npm".to_owned(),
        }),
        &["npm"],
    );
    assert_localized(
        &CliError::Json(serde_json::from_str::<serde_json::Value>("{").unwrap_err()),
        &[],
    );
    assert_localized(&CliError::Io(io_failure()), &["permission denied"]);
    assert_localized(&CliError::Tui(skit_tui::TuiError::Io(io_failure())), &[]);
    assert_localized(
        &CliError::Config(ConfigError::Encode {
            reason: "unsupported value".to_owned(),
        }),
        &["unsupported value"],
    );
    assert_localized(
        &CliError::State(StateWriteError::Encode {
            reason: "unsupported value".to_owned(),
        }),
        &["unsupported value"],
    );
    assert_localized(
        &CliError::Usage(Message::new("unknown preset: {}").with("nightly")),
        &["nightly"],
    );
    assert_localized(&CliError::ConfirmationRequired, &[]);
    assert_localized(&CliError::Aborted, &[]);
    assert_localized(&CliError::AddCancelled, &[]);
    assert_localized(
        &CliError::Source {
            operation: "read",
            path: "/data/demo.py".to_owned(),
            source: io_failure(),
        },
        &["/data/demo.py", "permission denied"],
    );
    assert_localized(&CliError::DataDirectoryUnavailable, &[]);
    assert_localized(&CliError::DirectoryUnavailable("state"), &["state"]);
}

#[test]
fn the_localized_command_tree_translates_every_description() {
    let command = translate_command(Cli::command(), Locale::ZhTw);
    assert_eq!(
        command.get_about().map(ToString::to_string).unwrap(),
        "程式、提示詞、執行檔與命令程式庫"
    );
    let list = command
        .get_subcommands()
        .find(|sub| sub.get_name() == "list")
        .expect("the list subcommand exists");
    assert_eq!(
        list.get_about().map(ToString::to_string).unwrap(),
        "列出程式庫中的項目"
    );
    // A Clap token such as `--help` must never change.
    let rendered = command.clone().render_help().to_string();
    assert!(rendered.contains("--help"));
    assert!(!rendered.contains("--說明"));

    let english = translate_command(Cli::command(), Locale::En);
    assert_eq!(
        english.get_about().map(ToString::to_string).unwrap(),
        "A script, prompt, program, and command library"
    );
}

#[test]
fn windows_argument_encoding_round_trips_every_quoting_shape() {
    // Each row is one argument list that must survive join then split byte for byte.
    let lists: &[&[&str]] = &[
        &[],
        &[""],
        &["plain"],
        &["two words"],
        &["tab\there"],
        &["", "after empty", ""],
        &[r"trailing\"],
        &[r"trailing\\"],
        &[r"C:\Program Files\tool.exe", r"--out=C:\dir\"],
        &[r#"quote"inside"#],
        &[r#"back\"slash"#],
        &[r#"a b\"#],
        &[r#"a b\\"#],
        &[r#""quoted whole""#],
        &["-", "--", "--flag=value with spaces"],
    ];

    for list in lists {
        let joined = join_windows_arguments(list);
        let split = split_windows_arguments(&joined).unwrap();
        assert_eq!(
            &split, list,
            "round trip failed for {list:?} via {joined:?}"
        );
    }

    // A trailing backslash run inside a quoted argument doubles before the closing quote.
    assert_eq!(join_windows_arguments(&[r"a b\"]), r#""a b\\""#);
    assert_eq!(join_windows_arguments(&[r"plain\"]), r"plain\");
    assert_eq!(join_windows_arguments(&[""]), "\"\"");
    // A backslash run that no quote follows stays literal on the way back.
    assert_eq!(split_windows_arguments(r"a\").unwrap(), [r"a\"]);
    assert_eq!(split_windows_arguments(r"a\\").unwrap(), [r"a\\"]);
}

#[test]
fn windows_argument_splitting_covers_padding_escapes_and_unclosed_quotes() {
    // Trailing separators end the scan without adding an empty argument.
    assert_eq!(
        split_windows_arguments("  agent.exe   run   ").unwrap(),
        ["agent.exe", "run"]
    );
    assert!(split_windows_arguments("   ").unwrap().is_empty());

    // An even backslash run before a quote keeps half the backslashes and toggles the quote.
    assert_eq!(
        split_windows_arguments(r#"agent.exe "a\\" b"#).unwrap(),
        ["agent.exe", r"a\", "b"]
    );
    // An odd run escapes the quote itself.
    assert_eq!(
        split_windows_arguments(r#"agent.exe a\"b"#).unwrap(),
        ["agent.exe", r#"a"b"#]
    );
    // Backslashes that no quote follows stay as written.
    assert_eq!(
        split_windows_arguments(r"agent.exe a\\b").unwrap(),
        ["agent.exe", r"a\\b"]
    );
    // Two quotes inside a quoted run encode one literal quote.
    assert_eq!(
        split_windows_arguments(r#"agent.exe "a""b""#).unwrap(),
        ["agent.exe", r#"a"b"#]
    );

    let error = split_windows_arguments(r#"agent.exe "unclosed"#).unwrap_err();
    assert!(matches!(error, CliError::Usage(_)));
    assert!(error.to_string().contains("invalid quoting"));
}

#[test]
fn the_command_translator_is_total_over_every_clap_field() {
    // A command can carry a long description, and an argument can carry no help at all.
    let command = translate_command(
        clap::Command::new("probe")
            .about("Library")
            .long_about("A script, prompt, program, and command library")
            .arg(clap::Arg::new("bare"))
            .arg(clap::Arg::new("described").help("Entry slug or display name")),
        Locale::ZhTw,
    );

    assert_eq!(
        command.get_about().map(ToString::to_string).unwrap(),
        "程式庫"
    );
    assert_eq!(
        command.get_long_about().map(ToString::to_string).unwrap(),
        "程式、提示詞、執行檔與命令程式庫"
    );
    let arguments = command.get_arguments().collect::<Vec<_>>();
    assert!(
        arguments
            .iter()
            .find(|argument| argument.get_id() == "bare")
            .unwrap()
            .get_help()
            .is_none()
    );
    assert_eq!(
        arguments
            .iter()
            .find(|argument| argument.get_id() == "described")
            .unwrap()
            .get_help()
            .map(ToString::to_string)
            .unwrap(),
        "項目短名或顯示名稱"
    );
}

#[test]
fn tui_settings_refuse_axes_that_do_not_apply_to_the_entry_kind() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Command form", "echo ok");
    let base = form_values(tui_settings_form(&store, &entry));

    for (field, value, detail) in [
        (
            "interpreter",
            "bash",
            "the entry does not use a pinnable interpreter",
        ),
        (
            "dependencies",
            "rich",
            "package dependencies apply only to Python and JavaScript entries",
        ),
        (
            "python",
            ">=3.12",
            "a Python constraint applies only to Python entries",
        ),
    ] {
        let mut values = base.clone();
        values.insert(field.to_owned(), value.to_owned());
        let error =
            tui_submit_settings(&service, &store, &state_dir, "command-form", &values).unwrap_err();
        assert!(error.to_string().contains(detail), "{error}");
    }
}

#[test]
fn tui_settings_accept_a_pinnable_interpreter_and_a_managed_binding() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    fs::write(&source, "NAME=world\necho \"$NAME\"\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Pin tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("pin-tool").unwrap();
    let mut values = form_values(tui_settings_form(&store, &entry));
    values.insert("interpreter".to_owned(), "/opt/bash".to_owned());
    values.insert("source:manage".to_owned(), "NAME".to_owned());

    tui_submit_settings(&service, &store, &state_dir, "pin-tool", &values).unwrap();

    let updated = service.show("pin-tool").unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).interpreter,
        "/opt/bash"
    );

    // A second submission keeps the managed binding the form reports back.
    let values = form_values(tui_settings_form(&store, &updated));
    tui_submit_settings(&service, &store, &state_dir, "pin-tool", &values).unwrap();
    let stored = fs::read_to_string(data_dir.join("scripts/pin-tool/script.sh")).unwrap();
    assert!(stored.contains("name = \"NAME\""), "{stored}");
}

#[test]
fn tui_settings_refuse_to_manage_a_parameter_the_form_unbound() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    fs::write(&source, "NAME=world\necho \"$NAME\"\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Shell tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("shell-tool").unwrap();
    let mut values = form_values(tui_settings_form(&store, &entry));
    values.insert("source:manage".to_owned(), "NAME".to_owned());
    tui_submit_settings(&service, &store, &state_dir, "shell-tool", &values).unwrap();

    // The stored source now manages NAME. Clearing its binding needs source:unmanage.
    let managed = service.show("shell-tool").unwrap();
    let mut values = form_values(tui_settings_form(&store, &managed));
    values.insert("source:resync".to_owned(), "true".to_owned());
    let binding_key = values
        .keys()
        .find(|key| key.ends_with(":binding"))
        .expect("the managed parameter has a binding field")
        .clone();
    values.insert(binding_key, "none".to_owned());

    let error =
        tui_submit_settings(&service, &store, &state_dir, "shell-tool", &values).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("use source:unmanage to remove the source binding for NAME"),
        "{error}"
    );
}

#[test]
fn tui_settings_manage_source_parameters_without_losing_non_utf8_bytes() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    let original = b"NAME=world\n# caf\xe9\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Bytes tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("bytes-tool").unwrap();
    let mut requested = form_values(tui_settings_form(&store, &entry));
    requested.insert("source:manage".to_owned(), "NAME".to_owned());

    tui_submit_settings(&service, &store, &state_dir, "bytes-tool", &requested).unwrap();

    let written = fs::read(data_dir.join("scripts/bytes-tool/script.sh")).unwrap();
    assert!(written.contains(&0xe9));
    assert!(written.ends_with(original));
    assert!(
        written
            .windows(b"name = \"NAME\"".len())
            .any(|row| row == b"name = \"NAME\"")
    );
}

#[test]
fn tui_settings_keep_an_unchanged_non_utf8_python_copy_byte_exact() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# coding: latin-1\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("python-bytes").unwrap();
    let values = form_values(tui_settings_form(&store, &entry));
    tui_submit_settings(&service, &store, &state_dir, "python-bytes", &values).unwrap();

    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
}

#[test]
fn tui_settings_store_a_dependency_edit_for_non_utf8_python_without_a_block() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# coding: latin-1\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("python-bytes").unwrap();
    let mut values = form_values(tui_settings_form(&store, &entry));
    values.insert("dependencies".to_owned(), "httpx>=0.28".to_owned());

    tui_submit_settings(&service, &store, &state_dir, "python-bytes", &values).unwrap();

    let updated = service.show("python-bytes").unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).dependencies,
        ["httpx>=0.28"]
    );
    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
}

#[test]
fn tui_settings_refuse_an_edit_to_a_non_utf8_authoritative_python_block() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# /// script\n# dependencies = [\"requests\"]\n# ///\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("python-bytes").unwrap();
    let before_settings = EntrySettings::from_meta(&entry.meta);
    let mut values = form_values(tui_settings_form(&store, &entry));
    values.insert("dependencies".to_owned(), String::new());

    let error =
        tui_submit_settings(&service, &store, &state_dir, "python-bytes", &values).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Python bytes's stored copy isn't valid UTF-8, so skit can't rewrite the script's own dependency block — and that block is what uv reads. Edit it in the script itself: skit edit Python bytes"
    );
    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
    assert_eq!(
        EntrySettings::from_meta(&service.show("python-bytes").unwrap().meta),
        before_settings
    );
}

#[test]
fn deps_keep_non_utf8_python_bytes_when_metadata_can_deliver_the_edit() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# coding: latin-1\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();

    deps(
        &service,
        &store,
        DepsArgs {
            selector: "python-bytes".to_owned(),
            dependencies: vec![" httpx>=0.28 ".to_owned()],
            clear: false,
            requires_python: None,
            needs: Vec::new(),
            clear_needs: false,
            json: false,
        },
    )
    .unwrap();

    let updated = service.show("python-bytes").unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).dependencies,
        ["httpx>=0.28"]
    );
    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
}

#[test]
fn deps_refuse_a_non_utf8_authoritative_block_before_any_metadata_write() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# /// script\n# dependencies = [\"requests\"]\n# ///\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let before = service.show("python-bytes").unwrap();

    let error = deps(
        &service,
        &store,
        DepsArgs {
            selector: "python-bytes".to_owned(),
            dependencies: Vec::new(),
            clear: true,
            requires_python: None,
            needs: vec!["uv".to_owned()],
            clear_needs: false,
            json: false,
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("what uv reads"), "{error}");
    assert_eq!(service.show("python-bytes").unwrap().meta, before.meta);
    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
}

#[test]
fn deps_distinguish_an_untouched_axis_from_an_explicit_clear() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    fs::write(
        &source,
        concat!(
            "# /// script\n",
            "# dependencies = [\"requests\"]\n",
            "# requires-python = \">=3.11\"\n",
            "# ///\n",
            "print(1)\n",
        ),
    )
    .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Axes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();

    deps(
        &service,
        &store,
        DepsArgs {
            selector: "axes".to_owned(),
            dependencies: Vec::new(),
            clear: false,
            requires_python: Some(">=3.12".to_owned()),
            needs: Vec::new(),
            clear_needs: false,
            json: false,
        },
    )
    .unwrap();
    let pinned = fs::read_to_string(data_dir.join("scripts/axes/script.py")).unwrap();
    assert!(pinned.contains("requests"));
    assert!(pinned.contains(">=3.12"));

    deps(
        &service,
        &store,
        DepsArgs {
            selector: "axes".to_owned(),
            dependencies: Vec::new(),
            clear: true,
            requires_python: None,
            needs: Vec::new(),
            clear_needs: false,
            json: false,
        },
    )
    .unwrap();
    let cleared = fs::read_to_string(data_dir.join("scripts/axes/script.py")).unwrap();
    assert!(!cleared.contains("requests"));
    assert!(cleared.contains(">=3.12"));
}

#[test]
fn every_agent_skill_command_example_matches_the_real_cli_tree() {
    let skill = include_str!("../../../../skills/skit/SKILL.md");
    let mut checked = 0;
    let mut in_bash = false;

    for line in skill.lines().map(str::trim) {
        if line == "```bash" {
            in_bash = true;
            continue;
        }
        if line == "```" {
            in_bash = false;
            continue;
        }
        if !in_bash || !line.starts_with("skit ") {
            continue;
        }
        let command = line.split_once(" #").map_or(line, |(command, _)| command);
        let arguments =
            shlex::split(command).unwrap_or_else(|| panic!("invalid shell line: {line}"));
        Cli::try_parse_from(&arguments)
            .unwrap_or_else(|error| panic!("invalid Agent Skill command `{command}`: {error}"));
        checked += 1;
    }

    assert!(checked >= 35, "only checked {checked} Agent Skill commands");
}

#[test]
fn clap_error_localization_keeps_every_context_value_shape_verbatim() {
    use clap::{builder::StyledStr, error::ErrorKind};

    let mut error = clap::Error::raw(
        ErrorKind::InvalidValue,
        "Print help | Entry added | Entry removed | Entry renamed",
    );
    error.insert(
        ContextKind::ValidValue,
        ContextValue::Strings(vec!["Print help".to_owned(), "Entry added".to_owned()]),
    );
    error.insert(
        ContextKind::PriorArg,
        ContextValue::StyledStr(StyledStr::from("Entry removed")),
    );
    error.insert(
        ContextKind::ValidSubcommand,
        ContextValue::StyledStrs(vec![StyledStr::from("Entry renamed")]),
    );
    error.insert(ContextKind::Custom, ContextValue::None);

    assert_eq!(
        localized_clap_error(&error, Locale::ZhCn),
        "错误：Print help | Entry added | Entry removed | Entry renamed"
    );
}
