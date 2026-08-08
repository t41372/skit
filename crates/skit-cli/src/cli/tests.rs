use std::{collections::BTreeMap, fs, io, path::Path};

use clap::Parser as _;
use skit_application::{ExitClass, LibraryService, RepositoryError};
use skit_domain::{
    Entry, EntryKind, EntryMeta, Slug, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_store::FileStore;
use skit_ui::{FormField, FormPurpose, FormView, Screen};
use tempfile::TempDir;

use super::{
    AddOptions, Cli, CliError, Command, add, apply_source_management, collect_plain_form,
    entry_candidates_from, execute, list, mode_name, platform_data_dir, preset_candidates_from,
    read_source, resolve_data_dir, runner_candidates_from, show, source_default_name, source_error,
    stored_name, tui_add_form, tui_declarations_from_values, tui_run_form, tui_settings_form,
    tui_split_list, user_confirmed,
};

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
        (CliError::DataDirectoryUnavailable, ExitClass::Skit.code()),
    ];

    for (error, expected) in errors {
        assert_eq!(error.exit_code(), i32::from(expected));
        assert!(!error.to_string().is_empty());
    }
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
            requires_python: None,
        },
    )
    .unwrap();
    let entry = service.show("tool").unwrap();

    apply_source_management(
        &service,
        &store,
        entry,
        false,
        &[],
        &[],
        &["NAME".to_owned()],
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "NAME=world\necho \"$NAME\"\n"
    );
    assert!(
        fs::read_to_string(
            store
                .entry_dir_path(&Slug::parse("tool").unwrap())
                .join("script.sh")
        )
        .unwrap()
        .contains("NAME=${NAME:-world}")
    );
}

#[test]
fn plain_form_collection_uses_defaults_masks_secrets_and_refuses_end_of_input() {
    let form = FormView {
        purpose: FormPurpose::Run,
        title: "Run".to_owned(),
        selector: Some("alpha".to_owned()),
        fields: vec![
            FormField::text("name", "Name", "default"),
            FormField::secret("token", "Token", ""),
        ],
        focused: 0,
        submit_label: "Run".to_owned(),
    };
    let mut output = Vec::new();
    let values = collect_plain_form(&form, &mut "\n".as_bytes(), &mut output, |_| {
        Ok("hidden".to_owned())
    })
    .unwrap();
    assert_eq!(values["name"], "default");
    assert_eq!(values["token"], "hidden");
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Name [default]: "));
    assert!(output.contains("Token: "));
    assert!(!output.contains("hidden"));

    let error = collect_plain_form(&form, &mut "".as_bytes(), &mut Vec::new(), |_| {
        Ok(String::new())
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("ended before the form was complete")
    );
}
