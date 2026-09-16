use std::{collections::BTreeMap, fs, fs::OpenOptions, sync::mpsc, thread, time::Duration};

use skit_store::{
    ConfigError, FileConfigStore, FileRunnerManagementStore, PreferencesCommit, PromptRunner,
    PromptRunnerRow, RunnerManagementStoreError, RunnerMutation,
};
use tempfile::TempDir;

fn runner(name: &str, program: &str) -> PromptRunner {
    PromptRunner {
        name: name.to_owned(),
        argv: vec![program.to_owned(), "{{prompt}}".to_owned()],
    }
}

fn management(data_dir: &TempDir, config_dir: &TempDir) -> FileRunnerManagementStore {
    FileRunnerManagementStore::new(data_dir.path(), config_dir.path())
}

fn config_bytes(config_dir: &TempDir) -> Vec<u8> {
    fs::read(config_dir.path().join("config.toml")).unwrap()
}

fn document(config_dir: &TempDir) -> toml::Table {
    String::from_utf8(config_bytes(config_dir))
        .unwrap()
        .parse()
        .unwrap()
}

fn stored_rows(config_dir: &TempDir) -> Vec<toml::Value> {
    document(config_dir)["prompt"]["runners"]
        .as_array()
        .unwrap()
        .clone()
}

fn stored_names(config_dir: &TempDir) -> Vec<String> {
    stored_rows(config_dir)
        .iter()
        .map(|row| {
            row.get("name")
                .and_then(toml::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

fn rows_named(store: &FileConfigStore, name: &str) -> Vec<PromptRunnerRow> {
    store
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some(name))
        .collect()
}

fn row_at(store: &FileConfigStore, index: usize) -> PromptRunnerRow {
    store.runner_rows().unwrap().remove(index)
}

fn argv_of(store: &FileConfigStore, name: &str) -> Vec<String> {
    store
        .runners()
        .unwrap()
        .into_iter()
        .find(|row| row.name == name)
        .unwrap()
        .argv
}

fn seeded(config_dir: &TempDir) -> FileConfigStore {
    let store = FileConfigStore::new(config_dir.path());
    store.ensure_runners_seeded().unwrap();
    store
}

fn written(config_dir: &TempDir, text: &str) -> FileConfigStore {
    fs::write(config_dir.path().join("config.toml"), text).unwrap();
    FileConfigStore::new(config_dir.path())
}

fn write_entry(data_dir: &TempDir, slug: &str, kind: &str, pinned_runner: &str) {
    let directory = data_dir.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        format!("name = {slug:?}\nkind = {kind:?}\nmode = \"copy\"\nrunner = {pinned_runner:?}\n"),
    )
    .unwrap();
}

fn one_setting(key: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(key.to_owned(), value.to_owned())])
}

fn added(name: &str) -> RunnerMutation {
    RunnerMutation::Add {
        runner: runner(name, name),
    }
}

/// The virtual default rows a fresh config reports must survive materialization.
///
/// Preferences opens without a write, so every staged mutation compares tokens that were
/// read before the file existed.
#[test]
fn virtual_default_rows_keep_their_tokens_after_seeding() {
    let config_dir = TempDir::new().unwrap();
    let store = FileConfigStore::new(config_dir.path());
    let before = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .map(|row| row.snapshot_token())
        .collect::<Vec<_>>();
    assert!(!config_dir.path().join("config.toml").exists());

    store.ensure_runners_seeded().unwrap();

    let after = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .map(|row| row.snapshot_token())
        .collect::<Vec<_>>();
    assert_eq!(before, after);
}

#[test]
fn an_unseeded_default_row_edit_commits_and_materializes_every_default() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = FileConfigStore::new(config_dir.path());
    let before = store.runner_rows().unwrap();
    let expected = rows_named(&store, "claude");

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &BTreeMap::new(),
                &[RunnerMutation::ReplaceNamed {
                    runner: runner("claude", "mine"),
                    expected,
                }],
            )
            .unwrap(),
        PreferencesCommit::Committed
    );

    let after = store.runner_rows().unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(
        document(&config_dir)["prompt"]["runners_seeded"].as_bool(),
        Some(true)
    );
    for (old, new) in before
        .iter()
        .zip(&after)
        .filter(|(old, _)| old.name.as_deref() != Some("claude"))
    {
        assert_eq!(old.snapshot_token(), new.snapshot_token());
    }
    assert_eq!(argv_of(&store, "claude")[0], "mine");
}

#[test]
fn one_batch_adds_edits_removes_and_writes_settings() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = seeded(&config_dir);

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &one_setting("editor", "vim"),
                &[
                    added("my-agent"),
                    RunnerMutation::ReplaceNamed {
                        runner: runner("codex", "mine"),
                        expected: rows_named(&store, "codex"),
                    },
                    RunnerMutation::RemoveNamed {
                        name: "opencode".to_owned(),
                        expected: rows_named(&store, "opencode"),
                        expected_pinned_count: 0,
                    },
                ],
            )
            .unwrap(),
        PreferencesCommit::Committed
    );

    let document = document(&config_dir);
    assert_eq!(document["editor"].as_str(), Some("vim"));
    let names = stored_names(&config_dir);
    assert!(!names.contains(&"opencode".to_owned()));
    assert_eq!(names.last().map(String::as_str), Some("my-agent"));
    assert_eq!(argv_of(&store, "codex")[0], "mine");
    assert_eq!(argv_of(&store, "my-agent")[0], "my-agent");
}

fn stale_named_edit(store: &FileConfigStore) -> RunnerMutation {
    let expected = rows_named(store, "codex");
    store.set_runner(runner("codex", "external"), true).unwrap();
    RunnerMutation::ReplaceNamed {
        runner: runner("codex", "mine"),
        expected,
    }
}

fn stale_named_removal(store: &FileConfigStore) -> RunnerMutation {
    let expected = rows_named(store, "codex");
    store.set_runner(runner("codex", "external"), true).unwrap();
    RunnerMutation::RemoveNamed {
        name: "codex".to_owned(),
        expected,
        expected_pinned_count: 0,
    }
}

fn stale_row_repair(store: &FileConfigStore) -> RunnerMutation {
    let expected = row_at(store, 1);
    store.set_runner(runner("codex", "external"), true).unwrap();
    RunnerMutation::RepairRow {
        runner: runner("codex", "mine"),
        expected,
    }
}

fn stale_row_removal(store: &FileConfigStore) -> RunnerMutation {
    let expected = row_at(store, 1);
    store.set_runner(runner("codex", "external"), true).unwrap();
    RunnerMutation::RemoveRow { expected }
}

#[test]
fn a_stale_expectation_refuses_the_complete_batch() {
    for build in [
        stale_named_edit,
        stale_named_removal,
        stale_row_repair,
        stale_row_removal,
    ] {
        let data_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();
        let store = seeded(&config_dir);
        let stale = build(&store);
        let before = config_bytes(&config_dir);

        assert_eq!(
            management(&data_dir, &config_dir)
                .commit_preferences(&one_setting("editor", "vim"), &[added("my-agent"), stale])
                .unwrap(),
            PreferencesCommit::RowsChanged
        );
        assert_eq!(config_bytes(&config_dir), before);
    }
}

#[test]
fn a_stale_pin_count_refuses_the_batch_and_names_the_runner() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    write_entry(&data_dir, "pinned", "prompt", "codex");
    let store = seeded(&config_dir);
    let before = config_bytes(&config_dir);

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &one_setting("editor", "vim"),
                &[
                    added("my-agent"),
                    RunnerMutation::RemoveNamed {
                        name: "codex".to_owned(),
                        expected: rows_named(&store, "codex"),
                        expected_pinned_count: 0,
                    },
                ],
            )
            .unwrap(),
        PreferencesCommit::PinsChanged {
            name: "codex".to_owned(),
            actual: 1,
        }
    );
    assert_eq!(config_bytes(&config_dir), before);
}

#[test]
fn malformed_and_future_rows_survive_a_repairing_batch() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = written(
        &config_dir,
        r#"[prompt]
runners_seeded = true
runners = [
  { name = "valid", argv = ["old", "{{prompt}}"], keep = "yes" },
  { name = "broken", argv = ["no marker"], future = 7 },
  "future-shape",
  { name = "other", argv = ["other", "{{prompt}}"], future = 9 },
]
"#,
    );

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &BTreeMap::new(),
                &[
                    RunnerMutation::ReplaceNamed {
                        runner: runner("valid", "new"),
                        expected: rows_named(&store, "valid"),
                    },
                    RunnerMutation::RepairRow {
                        runner: runner("broken", "broken"),
                        expected: row_at(&store, 1),
                    },
                ],
            )
            .unwrap(),
        PreferencesCommit::Committed
    );

    let rows = stored_rows(&config_dir);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["keep"].as_str(), Some("yes"));
    assert_eq!(rows[0]["argv"][0].as_str(), Some("new"));
    assert!(rows[1].get("future").is_none());
    assert_eq!(rows[1]["argv"][1].as_str(), Some("{{prompt}}"));
    assert_eq!(rows[2].as_str(), Some("future-shape"));
    assert_eq!(rows[3]["future"].as_integer(), Some(9));
}

#[test]
fn a_duplicate_add_refuses_the_batch() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    seeded(&config_dir);
    let before = config_bytes(&config_dir);

    assert!(matches!(
        management(&data_dir, &config_dir)
            .commit_preferences(&one_setting("editor", "vim"), &[added("codex")]),
        Err(RunnerManagementStoreError::Config(ConfigError::Invalid(_)))
    ));
    assert_eq!(config_bytes(&config_dir), before);
}

#[test]
fn a_removed_name_can_return_in_the_same_batch() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = seeded(&config_dir);

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &BTreeMap::new(),
                &[
                    RunnerMutation::RemoveNamed {
                        name: "codex".to_owned(),
                        expected: rows_named(&store, "codex"),
                        expected_pinned_count: 0,
                    },
                    RunnerMutation::Add {
                        runner: runner("codex", "fresh"),
                    },
                ],
            )
            .unwrap(),
        PreferencesCommit::Committed
    );

    let names = stored_names(&config_dir);
    assert_eq!(names.iter().filter(|name| *name == "codex").count(), 1);
    assert_eq!(names.last().map(String::as_str), Some("codex"));
    assert_eq!(argv_of(&store, "codex")[0], "fresh");
}

fn four_rows(config_dir: &TempDir) -> FileConfigStore {
    written(
        config_dir,
        r#"[prompt]
runners_seeded = true
runners = [
  { name = "claude", argv = ["claude", "{{prompt}}"] },
  { name = "codex", argv = ["codex", "{{prompt}}"] },
  { name = "broken", argv = ["no marker"] },
  { name = "opencode", argv = ["opencode", "{{prompt}}"] },
]
"#,
    )
}

#[test]
fn a_row_address_follows_an_earlier_removal_in_the_same_batch() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = four_rows(&config_dir);

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &BTreeMap::new(),
                &[
                    RunnerMutation::RemoveNamed {
                        name: "claude".to_owned(),
                        expected: rows_named(&store, "claude"),
                        expected_pinned_count: 0,
                    },
                    RunnerMutation::RepairRow {
                        runner: runner("broken", "broken"),
                        expected: row_at(&store, 2),
                    },
                ],
            )
            .unwrap(),
        PreferencesCommit::Committed
    );

    assert_eq!(stored_names(&config_dir), ["codex", "broken", "opencode"]);
    assert_eq!(argv_of(&store, "broken"), ["broken", "{{prompt}}"]);
    assert_eq!(argv_of(&store, "opencode")[0], "opencode");
}

#[test]
fn raw_row_removals_address_the_rows_the_caller_read() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = four_rows(&config_dir);

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &BTreeMap::new(),
                &[
                    added("my-agent"),
                    RunnerMutation::RemoveRow {
                        expected: row_at(&store, 0),
                    },
                    RunnerMutation::RemoveRow {
                        expected: row_at(&store, 2),
                    },
                ],
            )
            .unwrap(),
        PreferencesCommit::Committed
    );

    assert_eq!(stored_names(&config_dir), ["codex", "opencode", "my-agent"]);
}

#[test]
fn a_row_an_earlier_mutation_removed_refuses_the_batch() {
    for repair in [true, false] {
        let data_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();
        let store = four_rows(&config_dir);
        let expected = row_at(&store, 1);
        let second = if repair {
            RunnerMutation::RepairRow {
                runner: runner("codex", "mine"),
                expected,
            }
        } else {
            RunnerMutation::RemoveRow { expected }
        };
        let before = config_bytes(&config_dir);

        assert_eq!(
            management(&data_dir, &config_dir)
                .commit_preferences(
                    &BTreeMap::new(),
                    &[
                        RunnerMutation::RemoveNamed {
                            name: "codex".to_owned(),
                            expected: rows_named(&store, "codex"),
                            expected_pinned_count: 0,
                        },
                        second,
                    ],
                )
                .unwrap(),
            PreferencesCommit::RowsChanged
        );
        assert_eq!(config_bytes(&config_dir), before);
    }
}

#[test]
fn invalid_input_refuses_before_any_lock() {
    for mutation in [
        RunnerMutation::RemoveNamed {
            name: "  ".to_owned(),
            expected: Vec::new(),
            expected_pinned_count: 0,
        },
        RunnerMutation::Add {
            runner: PromptRunner {
                name: "empty".to_owned(),
                argv: Vec::new(),
            },
        },
    ] {
        let data_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();

        assert!(matches!(
            management(&data_dir, &config_dir)
                .commit_preferences(&one_setting("editor", "vim"), &[mutation]),
            Err(RunnerManagementStoreError::Config(ConfigError::Usage(_)))
        ));
        assert!(!config_dir.path().join("config.toml").exists());
        assert!(!config_dir.path().join("config.lock").exists());
        assert!(!data_dir.path().join("registry.native.lock").exists());
    }
}

fn removal_batch(store: &FileConfigStore) -> Vec<RunnerMutation> {
    vec![RunnerMutation::RemoveNamed {
        name: "codex".to_owned(),
        expected: rows_named(store, "codex"),
        expected_pinned_count: 0,
    }]
}

#[test]
fn namespace_lock_failure_never_writes_config() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = seeded(&config_dir);
    let batch = removal_batch(&store);
    fs::create_dir(data_dir.path().join("registry.native.lock")).unwrap();
    let before = config_bytes(&config_dir);

    assert!(matches!(
        management(&data_dir, &config_dir)
            .commit_preferences(&one_setting("editor", "vim"), &batch),
        Err(RunnerManagementStoreError::Library(_))
    ));
    assert_eq!(config_bytes(&config_dir), before);
}

#[test]
fn a_batch_without_a_named_removal_needs_no_namespace_lock() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = seeded(&config_dir);
    fs::create_dir(data_dir.path().join("registry.native.lock")).unwrap();

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(&one_setting("editor", "vim"), &[added("my-agent")])
            .unwrap(),
        PreferencesCommit::Committed
    );
    assert_eq!(argv_of(&store, "my-agent")[0], "my-agent");
}

#[test]
fn library_scan_failure_never_writes_config() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = seeded(&config_dir);
    let batch = removal_batch(&store);
    fs::write(data_dir.path().join("scripts"), b"not a directory").unwrap();
    let before = config_bytes(&config_dir);

    assert!(matches!(
        management(&data_dir, &config_dir)
            .commit_preferences(&one_setting("editor", "vim"), &batch),
        Err(RunnerManagementStoreError::Library(_))
    ));
    assert_eq!(config_bytes(&config_dir), before);
}

#[test]
fn config_lock_failure_never_writes_config() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = seeded(&config_dir);
    let batch = removal_batch(&store);
    fs::remove_file(config_dir.path().join("config.lock")).unwrap();
    fs::create_dir(config_dir.path().join("config.lock")).unwrap();
    let before = config_bytes(&config_dir);

    assert!(matches!(
        management(&data_dir, &config_dir)
            .commit_preferences(&one_setting("editor", "vim"), &batch),
        Err(RunnerManagementStoreError::Config(_))
    ));
    assert_eq!(config_bytes(&config_dir), before);
}

#[test]
fn settings_without_runner_mutations_match_a_plain_settings_write() {
    let data_dir = TempDir::new().unwrap();
    let expected_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let text = "editor = \"vi\"\nfuture = 3\n";
    let expected_store = written(&expected_dir, text);
    written(&config_dir, text);
    let settings = BTreeMap::from([
        ("editor".to_owned(), "vim".to_owned()),
        ("mirror.pypi".to_owned(), "tsinghua".to_owned()),
        ("mirror".to_owned(), "on".to_owned()),
    ]);
    expected_store.set_many(&settings).unwrap();

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(&settings, &[])
            .unwrap(),
        PreferencesCommit::Committed
    );

    assert_eq!(config_bytes(&config_dir), config_bytes(&expected_dir));
    assert!(!document(&config_dir).contains_key("prompt"));
}

#[test]
fn the_batch_waits_for_the_config_lock() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = seeded(&config_dir);
    let batch = vec![
        added("my-agent"),
        RunnerMutation::RemoveNamed {
            name: "codex".to_owned(),
            expected: rows_named(&store, "codex"),
            expected_pinned_count: 0,
        },
    ];
    let before = config_bytes(&config_dir);
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .open(config_dir.path().join("config.lock"))
        .unwrap();
    guard.lock().unwrap();

    let management = management(&data_dir, &config_dir);
    let (done_sender, done_receiver) = mpsc::channel();
    let commit = thread::spawn(move || {
        let result = management.commit_preferences(&one_setting("editor", "vim"), &batch);
        done_sender.send(()).unwrap();
        result
    });
    assert!(
        done_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err(),
        "the transaction must wait for the configuration lock"
    );
    assert_eq!(config_bytes(&config_dir), before);

    drop(guard);
    assert_eq!(
        commit.join().unwrap().unwrap(),
        PreferencesCommit::Committed
    );
    assert_eq!(document(&config_dir)["editor"].as_str(), Some("vim"));
    assert!(!stored_names(&config_dir).contains(&"codex".to_owned()));
}

const MALFORMED: &str = "not = [valid\n";

fn malformed(config_dir: &TempDir) -> FileConfigStore {
    written(config_dir, MALFORMED)
}

/// Write a malformed file and return rows that no longer describe it.
fn stale_expectation(config_dir: &TempDir) -> Vec<PromptRunnerRow> {
    let scratch = TempDir::new().unwrap();
    let store = FileConfigStore::new(scratch.path());
    store.set_runner(runner("codex", "external"), true).unwrap();
    let expected = rows_named(&store, "codex");
    malformed(config_dir);
    expected
}

#[test]
fn a_refused_batch_keeps_a_malformed_file_as_the_user_wrote_it() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let expected = stale_expectation(&config_dir);

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &one_setting("editor", "vim"),
                &[RunnerMutation::ReplaceNamed {
                    runner: runner("codex", "mine"),
                    expected,
                }],
            )
            .unwrap(),
        PreferencesCommit::RowsChanged
    );

    assert_eq!(config_bytes(&config_dir), MALFORMED.as_bytes());
    assert!(!config_dir.path().join("config.toml.bak").exists());
}

#[test]
fn a_stale_pin_count_keeps_a_malformed_file_as_the_user_wrote_it() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    write_entry(&data_dir, "pinned", "prompt", "codex");
    let expected = stale_expectation(&config_dir);

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &one_setting("editor", "vim"),
                &[RunnerMutation::RemoveNamed {
                    name: "codex".to_owned(),
                    expected,
                    expected_pinned_count: 0,
                }],
            )
            .unwrap(),
        PreferencesCommit::PinsChanged {
            name: "codex".to_owned(),
            actual: 1,
        }
    );

    assert_eq!(config_bytes(&config_dir), MALFORMED.as_bytes());
    assert!(!config_dir.path().join("config.toml.bak").exists());
}

#[test]
fn a_committed_batch_repairs_a_malformed_file_and_keeps_a_backup() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = malformed(&config_dir);
    let expected = rows_named(&store, "codex");

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(
                &one_setting("editor", "vim"),
                &[RunnerMutation::ReplaceNamed {
                    runner: runner("codex", "mine"),
                    expected,
                }],
            )
            .unwrap(),
        PreferencesCommit::Committed
    );

    assert_eq!(document(&config_dir)["editor"].as_str(), Some("vim"));
    assert_eq!(argv_of(&store, "codex")[0], "mine");
    assert_eq!(
        fs::read(config_dir.path().join("config.toml.bak")).unwrap(),
        MALFORMED.as_bytes()
    );
}

#[test]
fn an_empty_change_on_a_malformed_file_recovers_like_a_settings_write() {
    let data_dir = TempDir::new().unwrap();
    let expected_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let expected_store = malformed(&expected_dir);
    malformed(&config_dir);
    // The stored file has no editor key, so this write changes no value.
    let settings = one_setting("editor", "");
    expected_store.set_many(&settings).unwrap();

    assert_eq!(
        management(&data_dir, &config_dir)
            .commit_preferences(&settings, &[])
            .unwrap(),
        PreferencesCommit::Committed
    );

    assert_eq!(config_bytes(&config_dir), config_bytes(&expected_dir));
    assert_eq!(
        fs::read(config_dir.path().join("config.toml.bak")).unwrap(),
        MALFORMED.as_bytes()
    );
    assert_eq!(
        fs::read(expected_dir.path().join("config.toml.bak")).unwrap(),
        MALFORMED.as_bytes()
    );
}

#[test]
fn an_invalid_setting_refuses_before_any_lock() {
    let data_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let store = seeded(&config_dir);
    let batch = removal_batch(&store);
    let before = config_bytes(&config_dir);
    fs::remove_file(config_dir.path().join("config.lock")).unwrap();

    assert!(matches!(
        management(&data_dir, &config_dir)
            .commit_preferences(&one_setting("form", "neither"), &batch),
        Err(RunnerManagementStoreError::Config(ConfigError::Usage(_)))
    ));
    assert_eq!(config_bytes(&config_dir), before);
    assert!(!config_dir.path().join("config.lock").exists());
    assert!(!data_dir.path().join("registry.native.lock").exists());
}
