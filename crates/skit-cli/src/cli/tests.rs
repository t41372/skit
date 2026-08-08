use std::{fs, io, path::Path};

use skit_application::{ExitClass, LibraryService, RepositoryError};
use skit_domain::StorageMode;
use skit_store::FileStore;
use tempfile::TempDir;

use super::{
    AddOptions, Cli, CliError, Command, add, execute, list, mode_name, platform_data_dir,
    read_source, resolve_data_dir, show, source_default_name, source_error, stored_name,
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
