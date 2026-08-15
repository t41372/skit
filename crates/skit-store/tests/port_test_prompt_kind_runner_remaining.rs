use std::fs;

use skit_store::{FileConfigStore, PromptRunner};
use tempfile::TempDir;

fn store(root: &TempDir) -> FileConfigStore {
    FileConfigStore::new(root.path())
}

fn path(root: &TempDir) -> std::path::PathBuf {
    root.path().join("config.toml")
}

fn write(root: &TempDir, text: &str) {
    fs::create_dir_all(root.path()).unwrap();
    fs::write(path(root), text).unwrap();
}

fn text(root: &TempDir) -> String {
    fs::read_to_string(path(root)).unwrap_or_default()
}

fn runner(name: &str, argv: &[&str]) -> PromptRunner {
    PromptRunner {
        name: name.to_owned(),
        argv: argv.iter().map(|value| (*value).to_owned()).collect(),
    }
}

#[test]
fn test_targeted_runner_savers_refuse_malformed_containers_and_handle_absent_section() {
    // An exact row snapshot from one config is not a license to create that row in an unrelated
    // or absent list. The targeted repair must compare the raw identity and return false without
    // writing anything.
    let source = TempDir::new().unwrap();
    write(
        &source,
        "[prompt]\nrunners_seeded = true\nrunners = [{ argv = [\"x\", \"{{prompt}}\"] }]\n",
    );
    let expected = store(&source).runner_rows().unwrap().remove(0);

    let absent = TempDir::new().unwrap();
    let absent_store = store(&absent);
    assert!(!absent_store
        .replace_runner_row_if_unchanged(runner("x", &["x", "{{prompt}}"]), &expected)
        .unwrap());
    assert!(!path(&absent).exists(), "stale row repair created a runner section from nothing");

    let scalar_prompt = TempDir::new().unwrap();
    write(&scalar_prompt, "prompt = \"bad\"\n");
    let before = text(&scalar_prompt);
    let error = store(&scalar_prompt)
        .set_runner(runner("x", &["x", "{{prompt}}"]), false)
        .expect_err("a scalar prompt section must not be silently repaired by a targeted runner save");
    assert!(error.to_string().contains("table"), "{error}");
    assert_eq!(text(&scalar_prompt), before, "failed targeted save rewrote malformed prompt container");

    let scalar_rows = TempDir::new().unwrap();
    write(&scalar_rows, "[prompt]\nrunners = \"bad\"\n");
    let before = text(&scalar_rows);
    let error = store(&scalar_rows)
        .set_runner(runner("x", &["x", "{{prompt}}"]), false)
        .expect_err("a scalar prompt.runners value must not be silently repaired by a targeted runner save");
    assert!(error.to_string().contains("array") || error.to_string().contains("list"), "{error}");
    assert_eq!(text(&scalar_rows), before, "failed targeted save rewrote malformed runner container");
}

#[test]
fn test_runner_stable_key_remove_refuses_blank_without_seeding_or_deleting_rows() {
    let fresh = TempDir::new().unwrap();
    let config = store(&fresh);
    assert!(!config.remove_runner("   ").unwrap());
    assert!(!path(&fresh).exists(), "blank stable-key removal seeded prompt runners");

    let raw = TempDir::new().unwrap();
    let rows = concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \" \" , argv = [\"one\", \"{{prompt}}\"] }, ",
        "{ argv = [\"two\", \"{{prompt}}\"] }",
        "]\n",
    );
    write(&raw, rows);
    let before = text(&raw);
    assert!(!store(&raw).remove_runner("").unwrap());
    assert_eq!(text(&raw), before, "blank stable-key removal deleted anonymous/blank raw rows");
}
