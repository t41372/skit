use std::{collections::BTreeMap, fs, io, path::Path};

use skit_application::{ExitClass, LibraryService, RepositoryError};
use skit_domain::{
    Entry, EntryKind, EntryMeta, Slug, StorageMode,
    parameters::{ParamDecl, ParameterType},
};
use skit_store::FileStore;
use skit_ui::{FormPurpose, Screen};
use tempfile::TempDir;

use super::{
    AddOptions, Cli, CliError, Command, add, execute, list, mode_name, platform_data_dir,
    read_source, resolve_data_dir, show, source_default_name, source_error, stored_name,
    tui_add_form, tui_run_form, tui_split_list,
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
        meta: EntryMeta::minimal("Alpha", EntryKind::parse("command").unwrap()),
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
