use std::fs;

use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }
}

#[test]
fn manage_and_unmanage_write_only_the_stored_copy() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("tool.py");
    fs::write(&original, "WIDTH = 800\nprint(WIDTH)\n").unwrap();
    sandbox
        .command()
        .args(["add", original.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();

    sandbox
        .command()
        .args(["params", "tool", "--manage", "WIDTH"])
        .assert()
        .success();
    let stored = sandbox.data.path().join("scripts/tool/script.py");
    let text = fs::read_to_string(&stored).unwrap();
    assert!(text.contains("[[tool.skit.params]]"));
    assert!(text.contains("name = \"WIDTH\""));
    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "WIDTH = 800\nprint(WIDTH)\n"
    );

    let output = sandbox
        .command()
        .args(["params", "tool", "--json"])
        .output()
        .unwrap();
    let params: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(params["parameters"][0]["name"], "WIDTH");
    assert_eq!(params["parameters"][0]["binding"], "const");

    sandbox
        .command()
        .args(["params", "tool", "--unmanage", "WIDTH"])
        .assert()
        .success();
    assert!(
        !fs::read_to_string(stored)
            .unwrap()
            .contains("[[tool.skit.params]]")
    );
}

#[test]
fn shell_normalize_is_explicit_and_never_changes_the_original() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("tool.sh");
    fs::write(&original, "NAME=world\necho \"$NAME\"\n").unwrap();
    sandbox
        .command()
        .args(["add", original.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["params", "tool", "--normalize", "NAME"])
        .assert()
        .success();
    assert!(
        fs::read_to_string(sandbox.data.path().join("scripts/tool/script.sh"))
            .unwrap()
            .contains("NAME=${NAME:-world}")
    );
    assert_eq!(
        fs::read_to_string(original).unwrap(),
        "NAME=world\necho \"$NAME\"\n"
    );
}

#[test]
fn python_deps_update_the_existing_pep723_block_without_losing_extensions() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("tool.py");
    fs::write(
        &original,
        r#"# /// script
# future = "keep"
# dependencies = ["old"]
# requires-python = ">=3.11"
# ///
print('ok')
"#,
    )
    .unwrap();
    sandbox
        .command()
        .args(["add", original.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();
    sandbox
        .command()
        .args([
            "deps",
            "tool",
            "--dep",
            "requests>=2,<3",
            "--dep",
            "rich",
            "--python",
            ">=3.12",
        ])
        .assert()
        .success();

    let stored = fs::read_to_string(sandbox.data.path().join("scripts/tool/script.py")).unwrap();
    assert!(stored.contains("future = \"keep\""));
    assert!(stored.contains("requests>=2,<3"));
    assert!(stored.contains("requires-python = \">=3.12\""));
    let output = sandbox
        .command()
        .args(["deps", "tool", "--json"])
        .output()
        .unwrap();
    let deps: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        deps["dependencies"],
        serde_json::json!(["requests>=2,<3", "rich"])
    );
    assert_eq!(deps["requires_python"], ">=3.12");
}
