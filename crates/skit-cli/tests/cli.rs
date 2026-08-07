use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

fn library() -> TempDir {
    let root = TempDir::new().unwrap();
    let dir = root.path().join("scripts").join("hello");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("meta.toml"),
        r#"
schema = 1
name = "Hello"
kind = "python"
mode = "copy"
source = "/tmp/hello.py"
source_hash = "sha256:abc"
added_at = "2026-08-07T00:00:00+00:00"
id = "0123456789abcdef0123456789abcdef"
workdir = "origin"
description = "A friendly script"
"#,
    )
    .unwrap();
    root
}

#[test]
fn list_json_is_stable_machine_output() {
    let root = library();
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", root.path())
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""slug":"hello""#))
        .stdout(predicate::str::contains(r#""kind":"python""#));
}

#[test]
fn show_json_reads_the_existing_metadata_layout() {
    let root = library();
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", root.path())
        .args(["show", "hello", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""name":"Hello""#))
        .stdout(predicate::str::contains(
            r#""id":"0123456789abcdef0123456789abcdef""#,
        ));
}

#[test]
fn missing_entries_keep_exit_127() {
    let root = TempDir::new().unwrap();
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", root.path())
        .args(["show", "missing", "--json"])
        .assert()
        .code(127)
        .stderr(predicate::str::contains("missing"));
}

#[test]
fn human_list_is_readable_without_json_parsing() {
    let root = library();
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", root.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello"))
        .stdout(predicate::str::contains("python"))
        .stdout(predicate::str::contains("A friendly script"));
}
