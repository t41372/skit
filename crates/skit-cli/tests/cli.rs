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
dependencies = ["requests>=2"]
needs = ["git"]
interpreter = "python3.14"
"#,
    )
    .unwrap();
    fs::write(
        dir.join("script.py"),
        "import argparse\np = argparse.ArgumentParser()\np.add_argument('--output', default='result.txt', choices=['result.txt', 'other.txt'], help='Output file')\np.parse_args()\n",
    )
    .unwrap();
    // `registry.toml` is the authoritative membership index. A row with no stamp cannot be
    // trusted, so the store re-reads `meta.toml` above, which stays the single truth.
    fs::write(root.path().join("registry.toml"), "[entries.hello]\n").unwrap();
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
fn missing_show_entries_keep_the_management_exit_one() {
    let root = TempDir::new().unwrap();
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", root.path())
        .args(["show", "missing", "--json"])
        .assert()
        .code(1)
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
        .stdout(predicate::str::contains("Python"))
        .stdout(predicate::str::contains("A friendly script"));
}

#[test]
fn human_show_and_params_expose_the_discovery_context() {
    let root = library();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    fs::create_dir_all(state.path().join("values")).unwrap();
    fs::write(
        state.path().join("values/hello.toml"),
        "[values]\noutput = \"other.txt\"\n[presets.fast]\noutput = \"result.txt\"\n",
    )
    .unwrap();

    let mut show = assert_cmd::cargo::cargo_bin_cmd!("skit");
    show.env("SKIT_DATA_DIR", root.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .args(["show", "hello"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Source: /tmp/hello.py"))
        .stdout(predicate::str::contains("Interpreter: python3.14"))
        .stdout(predicate::str::contains("Dependencies: requests>=2"))
        .stdout(predicate::str::contains("Needs: git"))
        .stdout(predicate::str::contains("Parameter"))
        .stdout(predicate::str::contains("output"))
        .stdout(predicate::str::contains("Presets: fast"))
        .stdout(predicate::str::contains("skit run Hello"));

    let mut params = assert_cmd::cargo::cargo_bin_cmd!("skit");
    params
        .env("SKIT_DATA_DIR", root.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .args(["params", "hello"])
        .assert()
        .success()
        // The argparse form IS the interface (reader-driven), so the v0.4 read view is
        // the one plain line with no --manage advice; the discovery context (defaults,
        // choices, help) belongs to `show` above and to the run form.
        .stdout(predicate::str::contains("Hello has no managed parameters."))
        .stdout(predicate::str::contains("--manage").not());
}
