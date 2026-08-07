use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

fn command(root: &TempDir) -> assert_cmd::Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command.env("SKIT_DATA_DIR", root.path());
    command
}

#[test]
fn add_copy_then_describe_rename_and_remove_is_a_complete_cli_slice() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("hello.py");
    fs::write(&source, b"print('hello')\r\n").unwrap();

    command(&root)
        .args([
            "add",
            source.to_str().unwrap(),
            "--kind",
            "python",
            "--name",
            "Hello",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added: Hello (hello)"));
    assert_eq!(
        fs::read(root.path().join("scripts/hello/script.py")).unwrap(),
        b"print('hello')\r\n"
    );

    command(&root)
        .args(["describe", "hello", "Friendly script"])
        .assert()
        .success();
    command(&root)
        .args(["rename", "hello", "Greeting Tool"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Greeting Tool (greeting-tool)"));
    command(&root)
        .args(["show", "greeting-tool", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            r#""description":"Friendly script""#,
        ));

    command(&root)
        .args(["remove", "greeting-tool"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));
    command(&root)
        .args(["remove", "greeting-tool", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed: Greeting Tool"));
    assert!(!root.path().join("scripts/greeting-tool").exists());
}

#[test]
fn add_reference_hashes_but_never_copies_the_original() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("linked.sh");
    fs::write(&source, b"#!/bin/sh\necho linked\n").unwrap();

    command(&root)
        .args([
            "add",
            source.to_str().unwrap(),
            "--kind",
            "shell",
            "--name",
            "Linked",
            "--reference",
        ])
        .assert()
        .success();

    assert!(!root.path().join("scripts/linked/script.sh").exists());
    command(&root)
        .args(["show", "linked", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""mode":"reference""#))
        .stdout(predicate::str::contains("sha256:"));
}

#[test]
fn duplicate_add_is_usage_error_and_preserves_the_first_entry() {
    let root = TempDir::new().unwrap();
    let first = root.path().join("first.py");
    let second = root.path().join("second.py");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();

    command(&root)
        .args([
            "add",
            first.to_str().unwrap(),
            "--kind",
            "python",
            "--name",
            "Same",
        ])
        .assert()
        .success();
    command(&root)
        .args([
            "add",
            second.to_str().unwrap(),
            "--kind",
            "python",
            "--name",
            "Same",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("already"));

    assert_eq!(
        fs::read(root.path().join("scripts/same/script.py")).unwrap(),
        b"first"
    );
}
