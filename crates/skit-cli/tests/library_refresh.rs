use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions,
    library_detail::LibraryPromptRunner,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn a_fresh_library_refresh_rewrites_no_product_input() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let store = FileStore::new(data.path());
    let source = b"printf '\xff'\r\n";
    let entry = store
        .create(CreateEntry {
            name: "Exact".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/exact.sh".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: source.to_vec(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let values_dir = state.path().join("values");
    fs::create_dir_all(&values_dir).unwrap();
    let state_path = values_dir.join(format!("{}.toml", entry.slug.as_str()));
    fs::write(&state_path, "[values]\nname = \"Ada\"\n").unwrap();
    let config_path = config.path().join("config.toml");
    fs::write(
        &config_path,
        "[prompt]\nrunners = [{ name = \"agent\", argv = [\"agent\", \"{{prompt}}\"] }]\n",
    )
    .unwrap();
    let entry_dir = data.path().join("scripts").join(entry.slug.as_str());
    let paths = [
        entry_dir.join("meta.toml"),
        entry_dir.join("script.sh"),
        data.path().join("registry.toml"),
        state_path,
        config_path,
    ];
    let before = paths
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect::<Vec<_>>();

    let surface = skit_cli::library_surface(&store, state.path(), config.path()).unwrap();

    assert_eq!(surface.details.len(), 1);
    for (path, before) in paths.iter().zip(before) {
        assert_eq!(
            fs::read(path).unwrap(),
            before,
            "{} changed",
            path.display()
        );
    }
}

#[test]
fn corrupt_runner_configuration_degrades_without_hiding_the_library_or_rewriting() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let store = FileStore::new(data.path());
    store
        .create(CreateEntry {
            name: "Prompt".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/prompt.md".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"Hello".to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                runner: "missing".to_owned(),
                ..EntrySettings::default()
            },
        })
        .unwrap();
    let config_path = config.path().join("config.toml");
    let corrupt = b"[[[broken";
    fs::write(&config_path, corrupt).unwrap();

    let surface = skit_cli::library_surface(&store, state.path(), config.path()).unwrap();

    assert_eq!(surface.details.len(), 1);
    assert_eq!(
        surface.details.values().next().unwrap().prompt_runner,
        Some(LibraryPromptRunner::Missing("missing".to_owned()))
    );
    assert_eq!(fs::read(&config_path).unwrap(), corrupt);
    assert!(!config.path().join("config.toml.bak").exists());
}
