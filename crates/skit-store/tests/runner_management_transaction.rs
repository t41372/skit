use std::fs;

use skit_store::{
    FileConfigStore, FileRunnerManagementStore, PromptRunner, RunnerManagementStoreError,
    RunnerRemovalCas,
};
use tempfile::TempDir;

fn runner(name: &str, program: &str) -> PromptRunner {
    PromptRunner {
        name: name.to_owned(),
        argv: vec![program.to_owned(), "{{prompt}}".to_owned()],
    }
}

fn write_entry(data_dir: &TempDir, slug: &str, kind: &str, pinned_runner: &str) {
    let directory = data_dir.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        format!(
            "name = {slug:?}\nkind = {kind:?}\nmode = \"copy\"\nrunner = {pinned_runner:?}\n"
        ),
    )
    .unwrap();
}

fn seeded_target(config_dir: &TempDir) -> (FileConfigStore, Vec<skit_store::PromptRunnerRow>) {
    let config = FileConfigStore::new(config_dir.path());
    config.set_runner(runner("victim", "old"), false).unwrap();
    let expected = config
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some("victim"))
        .collect();
    (config, expected)
}

#[test]
fn named_removal_atomically_checks_rows_and_prompt_pins() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    write_entry(&data_dir, "pinned", "prompt", "victim");
    let (config, expected) = seeded_target(&config_dir);
    let management = FileRunnerManagementStore::new(data_dir.path(), config_dir.path());

    assert_eq!(
        management
            .remove_named_if_unchanged("victim", &expected, 1)
            .unwrap(),
        RunnerRemovalCas::Removed
    );
    assert!(
        config
            .runners()
            .unwrap()
            .iter()
            .all(|runner| runner.name != "victim")
    );
}

#[test]
fn named_removal_refuses_stale_rows_without_changing_config() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let (config, expected) = seeded_target(&config_dir);
    config.set_runner(runner("victim", "newer"), true).unwrap();
    let before = fs::read(config_dir.path().join("config.toml")).unwrap();
    let management = FileRunnerManagementStore::new(data_dir.path(), config_dir.path());

    assert_eq!(
        management
            .remove_named_if_unchanged("victim", &expected, 0)
            .unwrap(),
        RunnerRemovalCas::RowsChanged
    );
    assert_eq!(
        fs::read(config_dir.path().join("config.toml")).unwrap(),
        before
    );
}

#[test]
fn named_removal_refuses_stale_pin_count_without_changing_config() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    write_entry(&data_dir, "new-pin", "prompt", "victim");
    let (_config, expected) = seeded_target(&config_dir);
    let before = fs::read(config_dir.path().join("config.toml")).unwrap();
    let management = FileRunnerManagementStore::new(data_dir.path(), config_dir.path());

    assert_eq!(
        management
            .remove_named_if_unchanged("victim", &expected, 0)
            .unwrap(),
        RunnerRemovalCas::PinsChanged { actual: 1 }
    );
    assert_eq!(
        fs::read(config_dir.path().join("config.toml")).unwrap(),
        before
    );
}

#[test]
fn unrelated_config_and_entry_edits_do_not_block_targeted_removal() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let (_config, expected) = seeded_target(&config_dir);
    let path = config_dir.path().join("config.toml");
    let mut text = fs::read_to_string(&path).unwrap();
    text.insert_str(0, "language = \"zh-TW\"\n");
    fs::write(&path, text).unwrap();
    write_entry(&data_dir, "other-prompt", "prompt", "other");
    write_entry(&data_dir, "not-a-prompt", "python", "victim");
    let management = FileRunnerManagementStore::new(data_dir.path(), config_dir.path());

    assert_eq!(
        management
            .remove_named_if_unchanged("victim", &expected, 0)
            .unwrap(),
        RunnerRemovalCas::Removed
    );
    assert!(fs::read_to_string(path).unwrap().contains("language = \"zh-TW\""));
}

#[test]
fn namespace_lock_failure_never_writes_config() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let (_config, expected) = seeded_target(&config_dir);
    fs::create_dir(data_dir.path().join("registry.native.lock")).unwrap();
    let before = fs::read(config_dir.path().join("config.toml")).unwrap();
    let management = FileRunnerManagementStore::new(data_dir.path(), config_dir.path());

    assert!(matches!(
        management.remove_named_if_unchanged("victim", &expected, 0),
        Err(RunnerManagementStoreError::Library(_))
    ));
    assert_eq!(
        fs::read(config_dir.path().join("config.toml")).unwrap(),
        before
    );
}

#[test]
fn library_scan_failure_never_writes_config() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let (_config, expected) = seeded_target(&config_dir);
    fs::write(data_dir.path().join("scripts"), b"not a directory").unwrap();
    let before = fs::read(config_dir.path().join("config.toml")).unwrap();
    let management = FileRunnerManagementStore::new(data_dir.path(), config_dir.path());

    assert!(matches!(
        management.remove_named_if_unchanged("victim", &expected, 0),
        Err(RunnerManagementStoreError::Library(_))
    ));
    assert_eq!(
        fs::read(config_dir.path().join("config.toml")).unwrap(),
        before
    );
}

#[test]
fn config_lock_failure_never_writes_config() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let (_config, expected) = seeded_target(&config_dir);
    fs::remove_file(config_dir.path().join("config.lock")).unwrap();
    fs::create_dir(config_dir.path().join("config.lock")).unwrap();
    let before = fs::read(config_dir.path().join("config.toml")).unwrap();
    let management = FileRunnerManagementStore::new(data_dir.path(), config_dir.path());

    assert!(matches!(
        management.remove_named_if_unchanged("victim", &expected, 0),
        Err(RunnerManagementStoreError::Config(_))
    ));
    assert_eq!(
        fs::read(config_dir.path().join("config.toml")).unwrap(),
        before
    );
}
