use std::fs;

use skit_store::{ConfigError, FileConfigStore, PromptRunner};
use tempfile::TempDir;

fn runner(name: &str, program: &str) -> PromptRunner {
    PromptRunner {
        name: name.to_owned(),
        argv: vec![program.to_owned(), "{{prompt}}".to_owned()],
    }
}

#[test]
fn named_edit_compares_only_the_complete_target_key_snapshot() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set_runner(runner("victim", "old"), false).unwrap();
    store.set_runner(runner("other", "other"), false).unwrap();
    let expected = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some("victim"))
        .collect::<Vec<_>>();

    store.set_runner(runner("other", "external"), true).unwrap();
    assert!(
        store
            .set_runner_if_unchanged(runner("victim", "mine"), &expected)
            .unwrap()
    );
    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .find(|row| row.name == "victim")
            .unwrap()
            .argv[0],
        "mine"
    );

    let stale = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some("victim"))
        .collect::<Vec<_>>();
    store.set_runner(runner("victim", "newer"), true).unwrap();
    assert!(
        !store
            .set_runner_if_unchanged(runner("victim", "stale-write"), &stale)
            .unwrap()
    );
    assert_eq!(
        store
            .runners()
            .unwrap()
            .into_iter()
            .find(|row| row.name == "victim")
            .unwrap()
            .argv[0],
        "newer"
    );
}

#[test]
fn named_edit_coalesces_selected_duplicate_key_and_keeps_future_siblings() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        r#"[prompt]
runners = [
  { name = "same", argv = ["first", "{{prompt}}"], future = 1 },
  { name = "same", argv = ["broken"] },
  { name = "other", argv = ["other", "{{prompt}}"], future = 9 },
]
"#,
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());
    let expected = store
        .runner_rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.name.as_deref() == Some("same"))
        .collect::<Vec<_>>();

    assert!(
        store
            .set_runner_if_unchanged(runner("same", "fixed"), &expected)
            .unwrap()
    );
    let text = fs::read_to_string(path).unwrap();
    assert_eq!(text.matches("name = \"same\"").count(), 1, "{text}");
    assert!(text.contains("future = 9"), "{text}");
    assert!(text.contains("\"fixed\""), "{text}");
}

#[test]
fn raw_row_repair_is_index_and_snapshot_checked_and_preserves_siblings() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        r#"[prompt]
runners = [
  { name = "", argv = ["valuable", "{{prompt}}"], future = 1 },
  { name = "other", argv = ["other", "{{prompt}}"], future = 9 },
]
"#,
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());
    let expected = store.runner_rows().unwrap().remove(0);
    assert!(
        store
            .replace_runner_row_if_unchanged(runner("valuable", "valuable"), &expected)
            .unwrap()
    );
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("name = \"valuable\""), "{text}");
    assert!(text.contains("future = 9"), "{text}");

    let stale = store.runner_rows().unwrap().remove(0);
    let mut document = fs::read_to_string(&path).unwrap();
    document = document.replace(
        "\"valuable\", \"{{prompt}}\"",
        "\"external\", \"{{prompt}}\"",
    );
    fs::write(&path, document).unwrap();
    assert!(
        !store
            .replace_runner_row_if_unchanged(runner("valuable", "mine"), &stale)
            .unwrap()
    );
    assert!(fs::read_to_string(path).unwrap().contains("external"));
}

#[test]
fn raw_row_repair_refuses_a_name_collision() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        r#"[prompt]
runners = [
  { argv = ["valuable", "{{prompt}}"] },
  { name = "taken", argv = ["taken", "{{prompt}}"] },
]
"#,
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());
    let expected = store.runner_rows().unwrap().remove(0);
    assert!(matches!(
        store.replace_runner_row_if_unchanged(runner("taken", "valuable"), &expected),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn opaque_identity_token_includes_raw_shape_and_container_kind() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(&path, "prompt = \"garbage\"\n").unwrap();
    let store = FileConfigStore::new(root.path());
    let prompt_container = store.runner_rows().unwrap().remove(0);
    let token = prompt_container.snapshot_token();

    fs::write(&path, "[prompt]\nrunners = \"garbage\"\n").unwrap();
    let runners_container = store.runner_rows().unwrap().remove(0);
    assert_ne!(token, runners_container.snapshot_token());

    fs::write(
        &path,
        "[prompt]\nrunners = [{ name = \"x\", argv = [\"x\"] }]\n",
    )
    .unwrap();
    let first = store.runner_rows().unwrap().remove(0);
    fs::write(
        &path,
        "[prompt]\nrunners = [{ name = \"x\", argv = [\"x\"], future = 1 }]\n",
    )
    .unwrap();
    let changed = store.runner_rows().unwrap().remove(0);
    assert_ne!(first.snapshot_token(), changed.snapshot_token());

    fs::write(
        &path,
        "[prompt]\nrunners = [{ name = \"x\", argv = [\"x\"], future = 1979-05-27T07:32:00Z }]\n",
    )
    .unwrap();
    let datetime = store.runner_rows().unwrap().remove(0);
    fs::write(
        path,
        "[prompt]\nrunners = [{ name = \"x\", argv = [\"x\"], future = \"1979-05-27T07:32:00Z\" }]\n",
    )
    .unwrap();
    let string = store.runner_rows().unwrap().remove(0);
    assert_ne!(
        datetime.snapshot_token(),
        string.snapshot_token(),
        "opaque CAS identity must preserve the TOML value kind"
    );
}

#[test]
fn row_cas_refuses_container_tokens_and_disappeared_arrays() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(&path, "prompt = \"garbage\"\n").unwrap();
    let store = FileConfigStore::new(root.path());
    let container = store.runner_rows().unwrap().remove(0);
    assert!(
        !store
            .replace_runner_row_if_unchanged(runner("fixed", "fixed"), &container)
            .unwrap()
    );

    fs::write(
        &path,
        "[prompt]\nrunners = [{ name = \"old\", argv = [\"old\", \"{{prompt}}\"] }]\n",
    )
    .unwrap();
    let row = store.runner_rows().unwrap().remove(0);
    fs::write(&path, "[prompt]\nrunners = \"gone\"\n").unwrap();
    assert!(
        !store
            .replace_runner_row_if_unchanged(runner("new", "new"), &row)
            .unwrap()
    );
    assert!(!store.remove_runner_row_if_unchanged(&row).unwrap());
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "[prompt]\nrunners = \"gone\"\n"
    );
}

/// A configuration file the user is still repairing by hand.
const MALFORMED: &str = "not = [valid\n";

/// A refused compare-and-set must leave a malformed file exactly as the user wrote it.
///
/// The store repairs a malformed file on its first real write. A refusal is not a write: the
/// user's own bytes stay until a mutation commits, so a stale expectation can never replace a
/// file the user is still repairing by hand.
#[test]
fn a_refused_single_row_mutation_keeps_a_malformed_file_and_its_defaults() {
    let stale_row = {
        let source = TempDir::new().unwrap();
        let store = FileConfigStore::new(source.path());
        store.set_runner(runner("my-agent", "old"), false).unwrap();
        store
            .runner_rows()
            .unwrap()
            .into_iter()
            .find(|row| row.name.as_deref() == Some("my-agent"))
            .unwrap()
    };

    for refuse in [
        |store: &FileConfigStore, row: &skit_store::PromptRunnerRow| {
            store.set_runner_if_unchanged(runner("my-agent", "mine"), std::slice::from_ref(row))
        },
        |store: &FileConfigStore, row: &skit_store::PromptRunnerRow| {
            store.replace_runner_row_if_unchanged(runner("my-agent", "mine"), row)
        },
        |store: &FileConfigStore, row: &skit_store::PromptRunnerRow| {
            store.remove_runner_row_if_unchanged(row)
        },
        |store: &FileConfigStore, row: &skit_store::PromptRunnerRow| {
            store.remove_runner_if_unchanged("my-agent", std::slice::from_ref(row))
        },
    ] {
        let root = TempDir::new().unwrap();
        let path = root.path().join("config.toml");
        fs::write(&path, MALFORMED).unwrap();
        let store = FileConfigStore::new(root.path());

        assert!(!refuse(&store, &stale_row).unwrap());

        assert_eq!(fs::read_to_string(&path).unwrap(), MALFORMED);
        assert!(!root.path().join("config.toml.bak").exists());
    }
}

/// A refused mutation on a fresh configuration must not seed the default agents either.
#[test]
fn a_refused_single_row_mutation_leaves_a_fresh_configuration_unwritten() {
    let source = TempDir::new().unwrap();
    let seeded = FileConfigStore::new(source.path());
    seeded.set_runner(runner("my-agent", "old"), false).unwrap();
    let stale = seeded
        .runner_rows()
        .unwrap()
        .into_iter()
        .find(|row| row.name.as_deref() == Some("my-agent"))
        .unwrap();

    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert!(
        !store
            .set_runner_if_unchanged(runner("my-agent", "mine"), std::slice::from_ref(&stale))
            .unwrap()
    );

    assert!(!root.path().join("config.toml").exists());
}

/// A removal that finds no such agent is not a write either.
///
/// `remove_runner` reports `false` when the name was never there. The store keeps its hands off
/// the file in that case: it creates no configuration for a name it did not remove, and it leaves
/// a malformed file for the user to repair.
#[test]
fn a_removal_that_finds_no_such_agent_writes_nothing() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());

    assert!(!store.remove_runner("no-such-agent").unwrap());

    assert!(!root.path().join("config.toml").exists());

    let malformed = TempDir::new().unwrap();
    let path = malformed.path().join("config.toml");
    fs::write(&path, MALFORMED).unwrap();

    assert!(
        !FileConfigStore::new(malformed.path())
            .remove_runner("no-such-agent")
            .unwrap()
    );

    assert_eq!(fs::read_to_string(&path).unwrap(), MALFORMED);
    assert!(!malformed.path().join("config.toml.bak").exists());
}
