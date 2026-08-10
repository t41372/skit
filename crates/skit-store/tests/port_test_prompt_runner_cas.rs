//! Public-API CAS/snapshot ports for prompt-runner management from
//! `origin/main@206f9ef:tests/test_prompt_kind.py`.
//!
//! A failed comparison is expected to stay red if Rust differs from Python. No production repair
//! belongs in this test-only branch.

use std::fs;

use skit_store::{FileConfigStore, PromptRunner};
use tempfile::TempDir;
use toml::{Table, Value};

fn runner(name: &str, argv: &[&str]) -> PromptRunner {
    PromptRunner {
        name: name.to_owned(),
        argv: argv.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn argv(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| Value::String((*value).to_owned()))
            .collect(),
    )
}

fn runner_value(name: &str, argv: Value) -> Value {
    Value::Table(Table::from_iter([
        ("name".to_owned(), Value::String(name.to_owned())),
        ("argv".to_owned(), argv),
    ]))
}

fn document_with_rows(rows: Vec<Value>) -> Table {
    Table::from_iter([(
        "prompt".to_owned(),
        Value::Table(Table::from_iter([
            ("runners_seeded".to_owned(), Value::Boolean(true)),
            ("runners".to_owned(), Value::Array(rows)),
        ])),
    )])
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

fn rows_mut(document: &mut Table) -> &mut Vec<Value> {
    document
        .get_mut("prompt")
        .and_then(Value::as_table_mut)
        .and_then(|prompt| prompt.get_mut("runners"))
        .and_then(Value::as_array_mut)
        .unwrap()
}

#[test]
fn test_raw_row_remove_snapshot_includes_unknown_fields_and_container_value() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let mut bad = match runner_value("bad", argv(&["bad"])) {
        Value::Table(table) => table,
        _ => unreachable!(),
    };
    bad.insert("future".to_owned(), Value::Integer(1));
    write_document(&root, &document_with_rows(vec![Value::Table(bad)]));
    let expected = store.runner_rows().unwrap().remove(0);

    let mut changed = read_document(&root);
    rows_mut(&mut changed)[0]
        .as_table_mut()
        .unwrap()
        .insert("future".to_owned(), Value::Integer(2));
    write_document(&root, &changed);

    assert!(!store.remove_runner_row_if_unchanged(&expected).unwrap());
    assert_eq!(
        rows_mut(&mut read_document(&root))[0]
            .as_table()
            .unwrap()
            .get("future")
            .and_then(Value::as_integer),
        Some(2)
    );

    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::String("before".to_owned()),
        )]),
    );
    let expected_container = store.runner_rows().unwrap().remove(0);
    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::String("after".to_owned()),
        )]),
    );
    assert!(!store
        .remove_runner_row_if_unchanged(&expected_container)
        .unwrap());
    assert_eq!(
        read_document(&root).get("prompt").and_then(Value::as_str),
        Some("after")
    );
}

#[test]
fn test_runner_raw_snapshots_are_recursively_type_sensitive() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let mut bad = match runner_value("bad", argv(&["bad"])) {
        Value::Table(table) => table,
        _ => unreachable!(),
    };
    bad.insert(
        "future".to_owned(),
        Value::Table(Table::from_iter([(
            "nested".to_owned(),
            Value::Array(vec![
                Value::Integer(1),
                Value::Table(Table::from_iter([(
                    "flag".to_owned(),
                    Value::Integer(0),
                )])),
            ]),
        )])),
    );
    write_document(&root, &document_with_rows(vec![Value::Table(bad)]));
    let expected = store.runner_rows().unwrap().remove(0);

    let mut changed = read_document(&root);
    rows_mut(&mut changed)[0]
        .as_table_mut()
        .unwrap()
        .insert(
            "future".to_owned(),
            Value::Table(Table::from_iter([(
                "nested".to_owned(),
                Value::Array(vec![
                    Value::Boolean(true),
                    Value::Table(Table::from_iter([(
                        "flag".to_owned(),
                        Value::Boolean(false),
                    )])),
                ]),
            )])),
        );
    write_document(&root, &changed);

    assert!(!store.remove_runner_row_if_unchanged(&expected).unwrap());
}

#[test]
fn test_runner_stable_key_remove_refuses_blank_without_seeding_or_deleting_rows() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert!(!store.remove_runner("   ").unwrap());
    assert!(!root.path().join("config.toml").exists());

    let rows = vec![
        runner_value(" ", argv(&["one", "{{prompt}}"])),
        Value::Table(Table::from_iter([(
            "argv".to_owned(),
            argv(&["two", "{{prompt}}"]),
        )])),
    ];
    write_document(&root, &document_with_rows(rows));
    let before = fs::read(root.path().join("config.toml")).unwrap();

    assert!(!store.remove_runner("").unwrap());
    assert_eq!(fs::read(root.path().join("config.toml")).unwrap(), before);
}

#[test]
fn test_runner_edit_snapshot_checks_only_the_target_key() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_document(
        &root,
        &document_with_rows(vec![
            runner_value("victim", argv(&["old", "{{prompt}}"])),
            runner_value("other", argv(&["other", "{{prompt}}"])),
        ]),
    );
    let expected = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some("victim"))
        .collect::<Vec<_>>();
    let mut changed = read_document(&root);
    rows_mut(&mut changed)[1]
        .as_table_mut()
        .unwrap()
        .insert("argv".to_owned(), argv(&["unrelated", "{{prompt}}"]));
    write_document(&root, &changed);

    assert!(store
        .set_runner_if_unchanged(runner("victim", &["mine", "{{prompt}}"]), &expected)
        .unwrap());
    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .find(|runner| runner.name == "victim")
            .unwrap(),
        runner("victim", &["mine", "{{prompt}}"])
    );
    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .find(|runner| runner.name == "other")
            .unwrap(),
        runner("other", &["unrelated", "{{prompt}}"])
    );

    let expected = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some("victim"))
        .collect::<Vec<_>>();
    let concurrent = runner("victim", &["external", "{{prompt}}"]);
    assert!(store.set_runner(concurrent.clone(), true).unwrap());
    assert!(!store
        .set_runner_if_unchanged(runner("victim", &["old", "{{prompt}}"]), &expected)
        .unwrap());
    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .find(|runner| runner.name == "victim")
            .unwrap(),
        concurrent
    );
}

#[test]
fn test_exact_row_repair_can_name_a_recognizable_anonymous_command() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let anonymous = Value::Table(Table::from_iter([(
        "argv".to_owned(),
        argv(&["valuable-agent", "--model", "x", "{{prompt}}"]),
    )]));
    write_document(
        &root,
        &document_with_rows(vec![
            anonymous,
            Value::String("untouched".to_owned()),
        ]),
    );
    let expected = store.runner_rows().unwrap().remove(0);
    let replacement = runner(
        "valuable",
        &["valuable-agent", "--model", "x", "{{prompt}}"],
    );

    assert!(store
        .replace_runner_row_if_unchanged(replacement.clone(), &expected)
        .unwrap());
    assert_eq!(store.runners().unwrap(), [replacement]);
    let mut document = read_document(&root);
    assert_eq!(
        rows_mut(&mut document)[1],
        Value::String("untouched".to_owned())
    );
}

#[test]
fn test_exact_row_repair_refuses_a_stale_snapshot_or_colliding_new_name() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let anonymous = Value::Table(Table::from_iter([(
        "argv".to_owned(),
        argv(&["valuable", "{{prompt}}"]),
    )]));
    write_document(
        &root,
        &document_with_rows(vec![
            anonymous,
            runner_value("taken", argv(&["taken", "{{prompt}}"])),
        ]),
    );
    let expected = store.runner_rows().unwrap().remove(0);
    let mut changed = read_document(&root);
    rows_mut(&mut changed)[0]
        .as_table_mut()
        .unwrap()
        .insert("future".to_owned(), Value::Boolean(true));
    write_document(&root, &changed);

    assert!(!store
        .replace_runner_row_if_unchanged(
            runner("fresh", &["valuable", "{{prompt}}"]),
            &expected,
        )
        .unwrap());

    let expected = store.runner_rows().unwrap().remove(0);
    assert!(store
        .replace_runner_row_if_unchanged(
            runner("taken", &["valuable", "{{prompt}}"]),
            &expected,
        )
        .is_err());
}

#[test]
fn test_name_remove_snapshot_checks_only_target_key() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_document(
        &root,
        &document_with_rows(vec![
            runner_value("victim", argv(&["old", "{{prompt}}"])),
            runner_value("other", argv(&["other", "{{prompt}}"])),
        ]),
    );
    let expected = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some("victim"))
        .collect::<Vec<_>>();
    let mut changed = read_document(&root);
    rows_mut(&mut changed).insert(
        0,
        runner_value("unrelated", argv(&["unrelated", "{{prompt}}"])),
    );
    write_document(&root, &changed);

    assert!(store
        .remove_runner_if_unchanged("victim", &expected)
        .unwrap());
    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .map(|runner| runner.name)
            .collect::<Vec<_>>(),
        ["unrelated", "other"]
    );
}

#[test]
fn test_runner_remove_helpers_report_absent_targets_and_bad_shapes_without_writing() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    write_document(
        &root,
        &document_with_rows(vec![runner_value(
            "kept",
            argv(&["kept", "{{prompt}}"]),
        )]),
    );
    let before = fs::read(root.path().join("config.toml")).unwrap();

    assert!(!store.remove_runner("ghost").unwrap());
    assert!(!store.remove_runner_row(99).unwrap());
    assert_eq!(fs::read(root.path().join("config.toml")).unwrap(), before);

    write_document(
        &root,
        &Table::from_iter([(
            "prompt".to_owned(),
            Value::String("scalar".to_owned()),
        )]),
    );
    assert!(!store.remove_runner_row(0).unwrap());
}
