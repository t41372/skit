use std::fs;

use predicates::str as predicate_str;
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

    let managed = sandbox
        .command()
        .args(["params", "tool", "--manage", "WIDTH"])
        .output()
        .unwrap();
    assert!(managed.status.success());
    let managed_output = format!(
        "{}{}",
        String::from_utf8_lossy(&managed.stdout),
        String::from_utf8_lossy(&managed.stderr)
    );
    assert!(
        managed_output.contains("Updated Tool. Managed parameters: WIDTH"),
        "{managed_output}"
    );
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

    let unmanaged = sandbox
        .command()
        .args(["params", "tool", "--unmanage", "WIDTH"])
        .output()
        .unwrap();
    assert!(unmanaged.status.success());
    let unmanaged_output = format!(
        "{}{}",
        String::from_utf8_lossy(&unmanaged.stdout),
        String::from_utf8_lossy(&unmanaged.stderr)
    );
    assert!(
        unmanaged_output.contains("Updated Tool. Managed parameters: —"),
        "{unmanaged_output}"
    );
    assert!(
        !fs::read_to_string(stored)
            .unwrap()
            .contains("[[tool.skit.params]]")
    );
}

#[test]
fn managed_parameter_tweaks_stay_in_source_and_declared_schema_flags_refuse() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("managed.py");
    fs::write(&original, "TOKEN = 'value'\nprint(TOKEN)\n").unwrap();
    sandbox
        .command()
        .args(["add", original.to_str().unwrap(), "--name", "Managed"])
        .assert()
        .success();
    sandbox
        .command()
        .args([
            "params",
            "managed",
            "--manage",
            "TOKEN",
            "--secret",
            "TOKEN",
            "--prompt",
            "TOKEN=Access token",
        ])
        .assert()
        .success();

    let stored = sandbox.data.path().join("scripts/managed/script.py");
    let text = fs::read_to_string(&stored).unwrap();
    assert!(text.contains("secret = true"));
    assert!(text.contains("prompt = \"Access token\""));
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/managed/meta.toml")).unwrap();
    assert!(!meta.contains("[[parameters]]"));

    let before_source = fs::read(&stored).unwrap();
    let before_meta = fs::read(sandbox.data.path().join("scripts/managed/meta.toml")).unwrap();
    // A kind that owns its schema in the file refuses every declared-schema flag with exit 1 and
    // names the two flags that do apply (`src/skit/cli.py:4286-4294`). It is an operation that
    // cannot succeed, not a malformed command line, so it is not the usage class.
    for flag in [
        vec!["--add", "other"],
        vec!["--add", ""],
        vec!["--type", "TOKEN=int"],
        vec!["--rm", "TOKEN"],
    ] {
        let mut command = sandbox.command();
        command.args(["params", "managed"]).args(&flag);
        command
            .assert()
            .code(1)
            .stderr(predicate_str::contains(
                "Managed manages its parameters from the script itself — use --manage / --unmanage, or edit the [tool.skit] block.",
            ));
    }
    assert_eq!(fs::read(stored).unwrap(), before_source);
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/managed/meta.toml")).unwrap(),
        before_meta
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
    let normalized = sandbox
        .command()
        .args(["params", "tool", "--normalize", "NAME"])
        .output()
        .unwrap();
    assert!(normalized.status.success());
    let normalized_output = format!(
        "{}{}",
        String::from_utf8_lossy(&normalized.stdout),
        String::from_utf8_lossy(&normalized.stderr)
    );
    assert!(
        !normalized_output.contains("Updated "),
        "{normalized_output}"
    );
    // The canonical form keeps the expansion inside double quotes, so a default that holds
    // spaces or globs stays one word (`src/skit/langs/shell/normalize.py:125` emits
    // `f'"${{{name}:-{literal}}}"'`).
    assert!(
        fs::read_to_string(sandbox.data.path().join("scripts/tool/script.sh"))
            .unwrap()
            .contains("NAME=\"${NAME:-world}\"")
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

/// PowerShell keeps no schema in its own file, so a declared rider is legitimate there.
///
/// The refusal above is gated on `params_io`, which version 0.4 sets for python, shell, fish and
/// the JavaScript kinds only (`src/skit/langs/registry.py:61`, `:130`, `:156`, `:201`). The
/// PowerShell spec deliberately has neither: its `param()` block is a read-only CLI surface and
/// "injection is out of scope for PowerShell in v1 — the reader assembles real `-Name value` flags
/// instead" (`src/skit/langs/registry.py:221-238`). So `has_params_io` is false, the declared
/// branch applies, and `--add` is accepted.
#[test]
fn powershell_takes_declared_riders_because_it_writes_no_schema_into_its_source() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("tool.ps1");
    fs::write(
        &original,
        "param([string]$Name = 'World')\nWrite-Output $Name\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add", original.to_str().unwrap(), "--name", "Ps"])
        .assert()
        .success();

    sandbox
        .command()
        .args([
            "params",
            "ps",
            "--add",
            "API_TOKEN",
            "--deliver",
            "API_TOKEN=env",
        ])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["params", "ps", "--json"])
        .output()
        .unwrap();
    let params: Value = serde_json::from_slice(&output.stdout).unwrap();
    let declared = params["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|parameter| parameter["name"] == "API_TOKEN");
    assert!(declared, "the rider did not survive: {params}");

    // The rider lives in entry metadata, never in the user's own script.
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/ps/meta.toml")).unwrap();
    assert!(meta.contains("API_TOKEN"), "{meta}");
    let stored = fs::read_to_string(sandbox.data.path().join("scripts/ps/script.ps1")).unwrap();
    assert!(!stored.contains("API_TOKEN"), "{stored}");
}
