//! Public-API ports of the prompt-runner registry contracts in
//! `origin/main@206f9ef:tests/test_prompt_kind.py`.
//!
//! The Python configuration semantics remain authoritative. Red assertions are kept as parity
//! findings; production config code is not changed in this branch.

use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};

use skit_store::{FileConfigStore, PromptRunner};
use tempfile::TempDir;
use toml::{Table, Value};

fn runner(name: &str, argv: &[&str]) -> PromptRunner {
    PromptRunner {
        name: name.to_owned(),
        argv: argv.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn runner_value(name: &str, argv: Value) -> Value {
    Value::Table(Table::from_iter([
        ("name".to_owned(), Value::String(name.to_owned())),
        ("argv".to_owned(), argv),
    ]))
}

fn argv(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| Value::String((*value).to_owned()))
            .collect(),
    )
}

fn write_document(root: &TempDir, document: &Table) {
    fs::write(
        root.path().join("config.toml"),
        toml::to_string_pretty(document).unwrap(),
    )
    .unwrap();
}

fn read_document(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(root.path().join("config.toml")).unwrap()).unwrap()
}

#[test]
fn test_load_prompt_runners_is_read_only_before_seeding() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    let expected = [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
        "copilot",
        "cursor",
        "pi",
    ];
    assert_eq!(
        store
            .runner_rows()
            .unwrap()
            .iter()
            .map(|row| row.name.as_deref().unwrap())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        store
            .runner_rows()
            .unwrap()
            .iter()
            .all(|row| row.reason.is_none())
    );
    assert_eq!(
        store
            .runners()
            .unwrap()
            .iter()
            .map(|runner| runner.name.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(!root.path().join("config.toml").exists());
}

#[test]
fn test_ensure_seeded_materializes_once_and_empty_stays_empty() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    store.ensure_runners_seeded().unwrap();
    assert!(root.path().join("config.toml").is_file());
    for name in [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
        "copilot",
        "cursor",
        "pi",
    ] {
        assert!(store.remove_runner(name).unwrap());
    }
    assert!(store.runners().unwrap().is_empty());

    store.ensure_runners_seeded().unwrap();
    assert!(store.runners().unwrap().is_empty());
}

#[test]
fn test_marker_alone_counts_as_seeded_and_stays_empty() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([(
                "runners_seeded".to_owned(),
                Value::Boolean(true),
            )])),
        )]),
    );

    assert!(store.runners().unwrap().is_empty());
    store.ensure_runners_seeded().unwrap();
    assert!(store.runners().unwrap().is_empty());
}

#[test]
fn test_hand_authored_rows_without_marker_count_as_seeded() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([(
                "runners".to_owned(),
                Value::Array(vec![runner_value("mine", argv(&["m", "{{prompt}}"]))]),
            )])),
        )]),
    );

    assert_eq!(
        store.runners().unwrap(),
        [runner("mine", &["m", "{{prompt}}"])],
    );
    store.ensure_runners_seeded().unwrap();
    assert_eq!(
        store.runners().unwrap(),
        [runner("mine", &["m", "{{prompt}}"])],
    );
}

#[test]
fn test_malformed_runner_rows_are_skipped_and_reported() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let rows = vec![
        runner_value("good", argv(&["g", "{{prompt}}"])),
        runner_value("bad-no-slot", argv(&["g"])),
        runner_value("", argv(&["g", "{{prompt}}"])),
        runner_value("bad-argv", Value::String("not-a-list".to_owned())),
        runner_value(
            "bad-token-type",
            Value::Array(vec![Value::String("g".to_owned()), Value::Integer(3)]),
        ),
        Value::String("not-a-table".to_owned()),
    ];
    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([
                ("runners_seeded".to_owned(), Value::Boolean(true)),
                ("runners".to_owned(), Value::Array(rows)),
            ])),
        )]),
    );

    assert_eq!(
        store.runners().unwrap(),
        [runner("good", &["g", "{{prompt}}"])],
    );
    let rows = store.runner_rows().unwrap();
    assert_eq!(rows[1].reason.as_deref(), Some("prompt-slot-count"));
    assert_eq!(rows[2].reason.as_deref(), Some("name"));
    assert_eq!(
        rows[2].argv.as_deref(),
        Some(&["g".to_owned(), "{{prompt}}".to_owned()][..])
    );
    assert!(rows[2].descriptor.starts_with('{'));
    assert_eq!(store.invalid_runner_rows().unwrap().len(), 5);
}

#[test]
fn test_duplicate_normalized_runner_names_keep_first_and_are_reported() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([
                ("runners_seeded".to_owned(), Value::Boolean(true)),
                (
                    "runners".to_owned(),
                    Value::Array(vec![
                        runner_value("same", argv(&["first", "{{prompt}}"])),
                        runner_value(" same ", argv(&["second", "{{prompt}}"])),
                    ]),
                ),
            ])),
        )]),
    );

    assert_eq!(
        store.runners().unwrap(),
        [runner("same", &["first", "{{prompt}}"])],
    );
    assert_eq!(store.invalid_runner_rows().unwrap(), ["same"]);
}

#[test]
fn test_runners_section_of_wrong_type_degrades_without_being_repaired_by_seed_read() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([
                ("runners_seeded".to_owned(), Value::Boolean(true)),
                (
                    "runners".to_owned(),
                    Value::String("garbage".to_owned()),
                ),
            ])),
        )]),
    );
    assert!(store.runners().unwrap().is_empty());
    assert_eq!(store.invalid_runner_rows().unwrap(), ["prompt.runners"]);

    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::String("not-a-table".to_owned()),
        )]),
    );
    assert!(store.runners().unwrap().is_empty());
    assert_eq!(store.invalid_runner_rows().unwrap(), ["prompt"]);
    let before = fs::read(root.path().join("config.toml")).unwrap();
    store.ensure_runners_seeded().unwrap();
    assert_eq!(fs::read(root.path().join("config.toml")).unwrap(), before);
}

#[test]
fn test_targeted_runner_mutations_preserve_unrelated_malformed_rows() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let malformed = Value::Table(Table::from_iter([
        ("name".to_owned(), Value::String("typo".to_owned())),
        ("argv".to_owned(), argv(&["mycli", "{{promt}}"])),
        ("future".to_owned(), Value::Integer(7)),
    ]));
    let anonymous = Value::String("not-a-table".to_owned());
    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([
                ("runners_seeded".to_owned(), Value::Boolean(true)),
                (
                    "runners".to_owned(),
                    Value::Array(vec![malformed.clone(), anonymous.clone()]),
                ),
            ])),
        )]),
    );

    assert!(
        !store
            .set_runner(runner("good", &["good", "{{prompt}}"]), false)
            .unwrap()
    );
    let after_add = read_document(&root);
    let rows = after_add
        .get("prompt")
        .and_then(Value::as_table)
        .and_then(|prompt| prompt.get("runners"))
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(&rows[..2], &[malformed.clone(), anonymous.clone()]);

    assert!(store.remove_runner("good").unwrap());
    let after_remove = read_document(&root);
    let rows = after_remove
        .get("prompt")
        .and_then(Value::as_table)
        .and_then(|prompt| prompt.get("runners"))
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(rows, &[malformed, anonymous]);
}

#[test]
fn test_explicit_runner_replace_repairs_same_name_malformed_rows_only() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let untouched = runner_value("other", argv(&["other"]));
    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::Table(Table::from_iter([
                ("runners_seeded".to_owned(), Value::Boolean(true)),
                (
                    "runners".to_owned(),
                    Value::Array(vec![
                        runner_value(" typo ", argv(&["old"])),
                        untouched.clone(),
                        runner_value("typo", Value::String("also-bad".to_owned())),
                    ]),
                ),
            ])),
        )]),
    );
    let replacement = runner("typo", &["fixed", "{{prompt}}"]);

    assert!(store.set_runner(replacement.clone(), false).is_err());
    assert!(store.set_runner(replacement.clone(), true).unwrap());
    assert_eq!(store.runners().unwrap(), [replacement]);
    let document = read_document(&root);
    let rows = document
        .get("prompt")
        .and_then(Value::as_table)
        .and_then(|prompt| prompt.get("runners"))
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], untouched);
}

#[test]
fn test_runner_targeted_transactions_do_not_lose_concurrent_distinct_adds() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(FileConfigStore::new(root.path()));
    store.ensure_runners_seeded().unwrap();
    for name in [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
        "copilot",
        "cursor",
        "pi",
    ] {
        store.remove_runner(name).unwrap();
    }
    let barrier = Arc::new(Barrier::new(2));

    let handles = [
        runner("one", &["one", "{{prompt}}"]),
        runner("two", &["two", "{{prompt}}"]),
    ]
    .into_iter()
    .map(|runner| {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            store.set_runner(runner, false).unwrap();
        })
    })
    .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .map(|runner| runner.name)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["one".to_owned(), "two".to_owned()])
    );
}

#[test]
fn test_runner_transaction_and_non_runner_config_update_preserve_each_other() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(FileConfigStore::new(root.path()));
    store.ensure_runners_seeded().unwrap();
    for name in [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
        "copilot",
        "cursor",
        "pi",
    ] {
        store.remove_runner(name).unwrap();
    }
    let barrier = Arc::new(Barrier::new(2));

    let runner_store = Arc::clone(&store);
    let runner_barrier = Arc::clone(&barrier);
    let add = thread::spawn(move || {
        runner_barrier.wait();
        runner_store
            .set_runner(runner("agent", &["agent", "{{prompt}}"]), false)
            .unwrap();
    });
    let setting_store = Arc::clone(&store);
    let setting_barrier = Arc::clone(&barrier);
    let set = thread::spawn(move || {
        setting_barrier.wait();
        setting_store.set("editor", "code --wait").unwrap();
    });

    add.join().unwrap();
    set.join().unwrap();
    assert_eq!(
        store.runners().unwrap(),
        [runner("agent", &["agent", "{{prompt}}"])],
    );
    assert_eq!(store.get("editor").unwrap(), "code --wait");
}

#[test]
fn test_set_runner_rejects_every_invalid_prompt_slot_shape_without_mutating_config() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("editor", "vim").unwrap();
    let before = fs::read(root.path().join("config.toml")).unwrap();

    for invalid in [
        runner("empty", &[]),
        runner("no-slot", &["agent"]),
        runner("twice", &["agent", "{{prompt}}", "{{prompt}}"]),
        runner("binary", &["{{prompt}}"]),
        runner("stray", &["agent", "{{other}}", "{{prompt}}"]),
    ] {
        let error = store.set_runner(invalid, false).unwrap_err();
        assert!(error.is_usage());
        assert_eq!(fs::read(root.path().join("config.toml")).unwrap(), before);
    }
}

#[test]
fn test_set_runner_accepts_single_braces_as_literal_text() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert!(
        !store
            .set_runner(
                runner("literal", &["agent", "{lit} {{prompt}}"]),
                false,
            )
            .unwrap()
    );
    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .find(|runner| runner.name == "literal")
            .unwrap(),
        runner("literal", &["agent", "{lit} {{prompt}}"])
    );
}
