use std::{collections::BTreeMap, fs, io, path::Path};

use clap::{CommandFactory as _, Parser as _};
use skit_application::{ExitClass, LibraryService, RepositoryError};
use skit_domain::{
    Entry, EntryKind, EntryMeta, Slug, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_store::FileStore;
use skit_ui::{FormField, FormPurpose, FormView, Screen};
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
}

fn add_command(service: &LibraryService<FileStore>, name: &str, template: &str) -> Entry {
    add(
        service,
        AddOptions {
            source: None,
            kind: Some("command".to_owned()),
            name: Some(name.to_owned()),
            description: String::new(),
            reference: false,
            command_template: Some(template.to_owned()),
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
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
fn completion_adapters_degrade_to_empty_for_corrupt_or_irrelevant_state() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("scripts"), "not a directory").unwrap();
    assert!(entry_candidates_from(&FileStore::new(root.path())).is_empty());

    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("config.toml"), "[").unwrap();
    assert!(runner_candidates_from(&FileConfigStore::new(&config)).is_empty());

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
            skit_i18n::text(skit_i18n::Locale::ZhCn, message) == message
                || skit_i18n::text(skit_i18n::Locale::ZhTw, message) == message
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

    let (bytes, permissions) = read_source(&source).unwrap();

    assert_eq!(bytes, b"alpha\r\nbeta\r\n");
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
        .map(|(kind, _)| stored_name(kind, &source)),
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
    assert!(permissions.unix_mode.is_some());
    #[cfg(not(unix))]
    assert!(permissions.unix_mode.is_none());

    let missing = root.path().join("missing");
    let error = read_source(&missing).unwrap_err();
    assert!(matches!(
        error,
        CliError::Source {
            operation: "open",
            ..
        }
    ));
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
            description: String::new(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CliError::Repository(RepositoryError::InvalidMutation { .. })
    ));
}

#[test]
fn tui_composition_refuses_before_terminal_start_when_the_library_cannot_scan() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("scripts"), "not a directory").unwrap();

    for command in [Some(Command::Tui), None] {
        let error = execute(Cli {
            data_dir: Some(root.path().to_path_buf()),
            install_completion: false,
            show_completion: false,
            command,
        })
        .unwrap_err();
        assert!(matches!(
            error,
            CliError::Repository(RepositoryError::Io {
                operation: "scan",
                ..
            })
        ));
    }
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
            ExitClass::NotFound.code(),
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
            ExitClass::Skit.code(),
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
        CliError::Repository(_)
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
                description: String::new(),
                reference: false,
                command_template: None,
                prompt: false,
                executable: false,
                runner: None,
                no_interpolate: false,
                dependencies: Vec::new(),
                dependencies_explicit: false,
                requires_python: None,
            },
        )
        .is_err()
    );
}

#[test]
fn tui_run_forms_preserve_saved_values_but_never_prefill_secrets() {
    let entry = Entry {
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

    let Screen::Form(form) = tui_run_form(
        &entry,
        &[token, count],
        &saved,
        &["codex".to_owned()],
        &["fast".to_owned()],
    ) else {
        panic!("run must use a form");
    };
    assert_eq!(form.purpose, FormPurpose::Run);
    assert_eq!(form.selector.as_deref(), Some("alpha"));
    let fields = form
        .fields
        .iter()
        .map(|field| (field.key.as_str(), field.value.as_str(), field.secret))
        .collect::<Vec<_>>();
    assert!(fields.contains(&("value:token", "", true)));
    assert!(fields.contains(&("value:count", "4", false)));
    assert!(fields.contains(&("_skit_runner", "codex", false)));
    assert!(fields.contains(&("_skit_preset", "", false)));
}

#[test]
fn tui_add_form_and_list_parser_cover_all_authoring_axes() {
    let Screen::Form(form) = tui_add_form() else {
        panic!("add must use a form");
    };
    assert_eq!(form.purpose, FormPurpose::Add);
    let keys = form
        .fields
        .iter()
        .map(|field| field.key.as_str())
        .collect::<Vec<_>>();
    for key in [
        "source",
        "name",
        "kind",
        "description",
        "mode",
        "template",
        "runner",
        "dependencies",
        "python",
    ] {
        assert!(keys.contains(&key));
    }
    assert_eq!(
        tui_split_list("alpha, beta\ngamma  delta"),
        ["alpha", "beta", "gamma", "delta"]
    );
    assert_eq!(
        tui_dependency_list("requests>=2,<3\nrich; python_version >= '3.12'"),
        ["requests>=2,<3", "rich; python_version >= '3.12'"]
    );
    assert_eq!(stored_name("js", Path::new("module.mjs")), "script.mjs");
    assert_eq!(stored_name("js", Path::new("module.cjs")), "script.cjs");
    assert_eq!(stored_name("ts", Path::new("module.mts")), "script.mts");
    assert_eq!(stored_name("ts", Path::new("module.cts")), "script.cts");
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
            description: String::new(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
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
            .contains("NAME=${NAME:-world}")
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
fn parameter_json_and_parser_helpers_cover_every_public_spelling() {
    assert_eq!(nonempty(""), None);
    assert_eq!(nonempty("value"), Some("value"));
    assert_eq!(parameter_source("command", "", &[]), "command");
    assert_eq!(parameter_source("prompt", "", &[]), "command");
    assert_eq!(parameter_source("python", "", &[]), "none");
    assert_eq!(
        parameter_source("python", "", &[ParamDecl::new("plain")]),
        "declared"
    );
    let mut bound = ParamDecl::new("bound");
    bound.binding = ParameterBinding::Const;
    assert_eq!(parameter_source("python", "", &[bound]), "inject");
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
    let record = field_json(&declaration);
    assert_eq!(record["label"], "Select path");
    assert_eq!(record["delivers_empty"], true);
    declaration.secret = true;
    assert_eq!(field_json(&declaration)["delivers_empty"], false);
    declaration.secret = false;
    declaration.multiple = true;
    assert_eq!(field_json(&declaration)["delivers_empty"], false);

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
    let future = tui_preference_field("future.setting".to_owned(), "value".to_owned());
    assert_eq!(future.label, "future.setting");
    assert!(!future.translate_label);
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
    assert_eq!(fallback_stored_name(Path::new("")), "script");
    assert_eq!(
        fallback_stored_name(Path::new("tool.custom")),
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
    config.set("shell.bash_path", "/custom/bash").unwrap();
    assert!(
        doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains("/custom/bash")
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
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("nested/two"), b"12").unwrap();
    assert_eq!(directory_size(root.path()), 6);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.path().join("one"), root.path().join("link")).unwrap();
        assert!(directory_size(&root.path().join("link")) > 0);
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

    for request in [
        HostRequest::Run,
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
    for request in [
        HostRequest::Add,
        HostRequest::Preferences,
        HostRequest::Runners,
    ] {
        assert!(matches!(
            tui_open(&service, &store, &state_dir, &config_dir, request, None,).unwrap(),
            Screen::Form(_)
        ));
    }
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
        Screen::Report(_)
    ));

    for effect in [UiEffect::None, UiEffect::Quit] {
        assert_eq!(
            tui_effect(&service, &store, &state_dir, &config_dir, effect,).unwrap(),
            UiAction::ClearStatus
        );
    }
    assert!(matches!(
        tui_effect(&service, &store, &state_dir, &config_dir, UiEffect::Reload,).unwrap(),
        UiAction::Replace(_)
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
        UiAction::Present(Screen::Form(_))
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
            description: String::new(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
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
        ("value:ignored".to_owned(), String::new()),
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
            description: String::new(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: vec!["requests>=2,<3".to_owned()],
            dependencies_explicit: true,
            requires_python: Some(">=3.12".to_owned()),
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
    assert!(!meta.contains("dependencies ="));
    assert!(!meta.contains("requires_python ="));
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
            description: "before".to_owned(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
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
    assert_localized(&CliError::ConfirmationRequiredFor("runner remove"), &[]);
    assert_localized(&CliError::Aborted, &[]);
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
            description: String::new(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
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
            description: String::new(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
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
fn tui_settings_refuse_source_operations_on_bytes_that_are_not_utf8() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    fs::write(&source, "NAME=world\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Bytes tool".to_owned()),
            description: String::new(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
        },
    )
    .unwrap();
    let entry = service.show("bytes-tool").unwrap();
    let values = form_values(tui_settings_form(&store, &entry));
    fs::write(data_dir.join("scripts/bytes-tool/script.sh"), [0xff, 0xfe]).unwrap();

    let mut requested = values.clone();
    requested.insert("source:resync".to_owned(), "true".to_owned());
    let error =
        tui_submit_settings(&service, &store, &state_dir, "bytes-tool", &requested).unwrap_err();
    assert!(error.to_string().contains("not valid UTF-8"), "{error}");
}

#[test]
fn tui_settings_refuse_a_python_copy_that_is_not_utf8() {
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
            name: Some("Python bytes".to_owned()),
            description: String::new(),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
        },
    )
    .unwrap();
    let entry = service.show("python-bytes").unwrap();
    let values = form_values(tui_settings_form(&store, &entry));
    fs::write(
        data_dir.join("scripts/python-bytes/script.py"),
        [0xff, 0xfe],
    )
    .unwrap();

    let error =
        tui_submit_settings(&service, &store, &state_dir, "python-bytes", &values).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("the Python stored copy is not valid UTF-8"),
        "{error}"
    );
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
