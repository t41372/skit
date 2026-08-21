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

    fn command_in(&self, locale: &str) -> assert_cmd::Command {
        let mut command = self.command();
        command.env("SKIT_LANG", locale);
        command
    }

    fn add_copy(&self, file: &str, name: &str, source: &str) {
        let original = self.data.path().join(file);
        fs::write(&original, source).unwrap();
        self.command()
            .args(["add", original.to_str().unwrap(), "--name", name])
            .assert()
            .success();
    }
}

#[test]
fn test_normalize_mixed_batch_reports_each_name() {
    let sandbox = Sandbox::new();
    sandbox.add_copy(
        "mixed.sh",
        "Mixed",
        "#!/usr/bin/env bash\nWIDTH=800\nreadonly MAX=100\n",
    );
    let stored = sandbox.data.path().join("scripts/mixed/script.sh");
    sandbox
        .command()
        .args(["params", "mixed", "--manage", "WIDTH"])
        .assert()
        .success();
    let source_before = fs::read_to_string(&stored).unwrap();
    let output = sandbox
        .command()
        .args([
            "params",
            "mixed",
            "--normalize",
            "WIDTH",
            "--normalize",
            "MAX",
            "--normalize",
            "NOPE",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let parameters = json["parameters"].as_array().unwrap();
    assert!(parameters.iter().any(|row| {
        row["name"] == "WIDTH" && row["binding"] == "envdefault" && row["delivery"] == "env"
    }));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("MAX is readonly"), "{stderr}");
    assert!(stderr.contains("NOPE isn't a plain constant"), "{stderr}");
    assert!(!stderr.contains("WIDTH"), "{stderr}");
    assert_eq!(
        fs::read_to_string(stored).unwrap(),
        source_before
            .replace("kind = \"const\"", "kind = \"envdefault\"")
            .replace("WIDTH=800", "WIDTH=\"${WIDTH:-800}\"")
    );
}

#[test]
fn test_cli_normalize_reports_refusals() {
    let cases = [
        (
            "en",
            "MAX is readonly, so the script could never take a value from the environment; skipped.",
        ),
        (
            "zh-CN",
            "MAX 是 readonly,脚本永远不可能从环境变量取值;已跳过。",
        ),
        (
            "zh-TW",
            "MAX 是 readonly,腳本永遠不可能從環境變數取值;已略過。",
        ),
    ];
    for (locale, expected) in cases {
        let sandbox = Sandbox::new();
        sandbox.add_copy("readonly.sh", "Readonly", "readonly MAX=100\n");
        let stored = sandbox.data.path().join("scripts/readonly/script.sh");
        let source_before = fs::read(&stored).unwrap();
        let meta = sandbox.data.path().join("scripts/readonly/meta.toml");
        let meta_before = fs::read(&meta).unwrap();
        let output = sandbox
            .command_in(locale)
            .args(["params", "readonly", "--normalize", "MAX"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap().trim(), expected);
        assert_eq!(fs::read(stored).unwrap(), source_before);
        assert_eq!(fs::read(meta).unwrap(), meta_before);
    }
}

#[test]
fn test_cli_normalize_refuses_a_non_shell_kind() {
    let cases = [
        (
            "en",
            "Plain has no --normalize: it is a shell idiom (VAR=value -> VAR=\"${VAR:-value}\").",
        ),
        (
            "zh-CN",
            "Plain 没有 --normalize:那是 shell 的写法(VAR=value -> VAR=\"${VAR:-value}\")。",
        ),
        (
            "zh-TW",
            "Plain 沒有 --normalize:那是 shell 的寫法(VAR=value -> VAR=\"${VAR:-value}\")。",
        ),
    ];
    for (locale, expected) in cases {
        let sandbox = Sandbox::new();
        sandbox.add_copy("plain.py", "Plain", "WIDTH = 800\n");
        let stored = sandbox.data.path().join("scripts/plain/script.py");
        let source_before = fs::read(&stored).unwrap();
        let meta = sandbox.data.path().join("scripts/plain/meta.toml");
        let meta_before = fs::read(&meta).unwrap();
        let output = sandbox
            .command_in(locale)
            .args(["params", "plain", "--normalize", "WIDTH"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap().trim(), expected);
        assert_eq!(fs::read(stored).unwrap(), source_before);
        assert_eq!(fs::read(meta).unwrap(), meta_before);
    }
}

#[test]
fn normalize_receipt_is_exact_in_each_supported_locale() {
    let cases = [
        (
            "en",
            "Normalized WIDTH in Tool: delivered as environment variables from now on (no temporary copy, and $0 stays your real file).",
        ),
        (
            "zh-CN",
            "已规范化 Tool 中的 WIDTH:今后用环境变量传值(不再写临时副本,$0 也仍指向你的真实文件)。",
        ),
        (
            "zh-TW",
            "已正規化 Tool 中的 WIDTH:今後用環境變數傳值(不再寫臨時副本,$0 也仍指向你的真實檔案)。",
        ),
    ];
    for (locale, expected) in cases {
        let sandbox = Sandbox::new();
        sandbox.add_copy("tool.sh", "Tool", "WIDTH=800\n");
        sandbox
            .command()
            .args(["params", "tool", "--manage", "WIDTH"])
            .assert()
            .success();
        let output = sandbox
            .command_in(locale)
            .args(["params", "tool", "--normalize", "WIDTH"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
    }
}

#[test]
fn test_cli_normalize_refuses_reference_mode() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("reference.sh");
    fs::write(&original, "WIDTH=800\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            original.to_str().unwrap(),
            "--ref",
            "--name",
            "Reference",
        ])
        .assert()
        .success();
    let original_before = fs::read(&original).unwrap();
    let meta = sandbox.data.path().join("scripts/reference/meta.toml");
    let meta_before = fs::read(&meta).unwrap();
    let output = sandbox
        .command()
        .args(["params", "reference", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "Reference is in reference mode, and skit never writes the original file. Change the line to VAR=\"${VAR:-value}\" in the source directly."
    );
    assert_eq!(fs::read(original).unwrap(), original_before);
    assert_eq!(fs::read(meta).unwrap(), meta_before);
}

#[test]
fn test_cli_normalize_without_a_stored_copy() {
    let sandbox = Sandbox::new();
    sandbox.add_copy("missing.sh", "Missing", "WIDTH=800\n");
    let entry_dir = sandbox.data.path().join("scripts/missing");
    fs::remove_file(entry_dir.join("script.sh")).unwrap();
    let meta_before = fs::read(entry_dir.join("meta.toml")).unwrap();
    let output = sandbox
        .command()
        .args(["params", "missing", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "Missing has no stored copy to edit."
    );
    assert_eq!(fs::read(entry_dir.join("meta.toml")).unwrap(), meta_before);

    let unknown = sandbox
        .command()
        .args(["params", "unknown", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(1));
    assert!(unknown.stdout.is_empty());
    assert!(
        String::from_utf8(unknown.stderr)
            .unwrap()
            .contains("entry not found: unknown")
    );
}

#[test]
fn normalize_refuses_non_utf8_copy_without_rewriting_source_or_meta() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("binary.sh");
    fs::write(&original, b"WIDTH=800\n\xff").unwrap();
    sandbox
        .command()
        .args(["add", original.to_str().unwrap(), "--name", "Binary"])
        .assert()
        .success();
    let entry_dir = sandbox.data.path().join("scripts/binary");
    let source = entry_dir.join("script.sh");
    let source_before = fs::read(&source).unwrap();
    let meta = entry_dir.join("meta.toml");
    let meta_before = fs::read(&meta).unwrap();
    let output = sandbox
        .command()
        .args(["params", "binary", "--normalize", "WIDTH"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "Binary isn't valid UTF-8, so --normalize can't rewrite it safely; nothing was changed — its constants keep being injected into a temporary copy."
    );
    assert_eq!(fs::read(source).unwrap(), source_before);
    assert_eq!(fs::read(meta).unwrap(), meta_before);
}

#[test]
fn test_cli_normalize_warning_renderer_covers_every_code() {
    let cases = [
        ("absent", "", "X", "X isn't a plain constant"),
        ("duplicate", "X=1\nX=2\n", "X", "assigned more than once"),
        ("readonly", "readonly X=1\n", "X", "X is readonly"),
        (
            "already",
            "X=\"${X:-1}\"\n",
            "X",
            "X already reads from the environment",
        ),
        (
            "unsafe",
            "X='a$b'\n",
            "X",
            "can't be moved into ${...:-...}",
        ),
        ("syntax", "if {\nX=1\n", "X", "Could not parse the script"),
    ];
    for (file, source, name, expected) in cases {
        let sandbox = Sandbox::new();
        sandbox.add_copy(&format!("{file}.sh"), file, source);
        let output = sandbox
            .command()
            .args(["params", file, "--normalize", name])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{file}");
        assert!(output.stdout.is_empty(), "{file}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(expected), "{file}: {stderr}");
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
fn source_shared_edits_warn_and_commit_valid_siblings_without_persisting_secrets() {
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
            "--no-secret",
            "TOKEN",
        ])
        .assert()
        .success();
    let stored = sandbox.data.path().join("scripts/managed/script.py");
    let meta = sandbox.data.path().join("scripts/managed/meta.toml");
    let source_before = fs::read(&stored).unwrap();
    assert!(
        !String::from_utf8_lossy(&source_before).contains("secret = true"),
        "the fixture must exercise a real public-to-secret transition"
    );
    let meta_before = fs::read(&meta).unwrap();

    sandbox
        .command()
        .args([
            "params",
            "managed",
            "--prompt",
            "missing=Label",
            "--secret",
            "TOKEN",
        ])
        .assert()
        .success()
        .stderr(predicate_str::contains(
            "missing isn't a managed parameter; skipped.",
        ));

    let source_after = fs::read(&stored).unwrap();
    assert_ne!(source_after, source_before);
    let source_after = String::from_utf8(source_after).unwrap();
    assert!(source_after.contains("secret = true"));
    assert!(!source_after.contains("Label"));
    assert_ne!(fs::read(meta).unwrap(), meta_before);
    assert_eq!(
        fs::read(&original).unwrap(),
        b"TOKEN = 'value'\nprint(TOKEN)\n"
    );
    let state =
        fs::read_to_string(sandbox.state.path().join("values/managed.toml")).unwrap_or_default();
    assert!(!state.contains("TOKEN"), "{state}");
    assert!(!state.contains("value"), "{state}");
    assert_eq!(fs::read_dir(sandbox.config.path()).unwrap().count(), 0);
}

#[test]
fn a_legacy_non_rider_stays_declared_without_becoming_an_effective_parameter() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("tool.sh");
    fs::write(&original, "echo ok\n").unwrap();
    sandbox
        .command()
        .args(["add", original.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();
    let stored = sandbox.data.path().join("scripts/tool/script.sh");
    let meta = sandbox.data.path().join("scripts/tool/meta.toml");
    let mut meta_text = fs::read_to_string(&meta).unwrap();
    meta_text
        .push_str("\n[[parameters]]\nname = \"legacy\"\ntype = \"str\"\ndelivery = \"inject\"\n");
    fs::write(&meta, meta_text).unwrap();
    let source_before = fs::read(&stored).unwrap();
    let meta_before = fs::read(&meta).unwrap();

    let output = sandbox
        .command()
        .args(["params", "tool", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        record["declared"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "legacy")
    );
    assert!(
        record["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["name"] != "legacy")
    );
    assert_eq!(fs::read(stored).unwrap(), source_before);
    assert_eq!(fs::read(meta).unwrap(), meta_before);
    assert_eq!(fs::read_dir(sandbox.state.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(sandbox.config.path()).unwrap().count(), 0);
}

#[test]
fn test_cli_normalize_turns_a_const_into_an_env_param() {
    let sandbox = Sandbox::new();
    let original = sandbox.data.path().join("tool.sh");
    fs::write(
        &original,
        "#!/usr/bin/env bash\nWIDTH=800\nDEPTH=3\necho \"$WIDTH $DEPTH\"\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add", original.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["params", "tool", "--manage", "WIDTH", "--manage", "DEPTH"])
        .assert()
        .success();
    let normalized = sandbox
        .command()
        .args(["params", "tool", "--normalize", "WIDTH"])
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
    let stored = fs::read_to_string(sandbox.data.path().join("scripts/tool/script.sh")).unwrap();
    assert!(stored.contains("WIDTH=\"${WIDTH:-800}\""));
    assert!(stored.contains("DEPTH=3"));
    assert!(stored.contains("kind = \"envdefault\""));
    let shown: Value = serde_json::from_slice(
        &sandbox
            .command()
            .args(["show", "tool", "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let fields = shown["fields"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|field| field["key"] == "WIDTH" && field["source"] == "env")
    );
    assert!(
        fields
            .iter()
            .any(|field| field["key"] == "DEPTH" && field["source"] == "inject")
    );
    assert_eq!(
        fs::read_to_string(original).unwrap(),
        "#!/usr/bin/env bash\nWIDTH=800\nDEPTH=3\necho \"$WIDTH $DEPTH\"\n"
    );
}

#[cfg(unix)]
#[test]
fn test_cli_normalized_param_runs_through_the_environment() {
    let sandbox = Sandbox::new();
    sandbox.add_copy(
        "runner.sh",
        "Runner",
        "#!/usr/bin/env bash\nWIDTH=800\necho \"w=$WIDTH\"\n",
    );
    sandbox
        .command()
        .args(["params", "runner", "--manage", "WIDTH"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["params", "runner", "--normalize", "WIDTH"])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["run", "runner", "--set", "WIDTH=1200", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("w=1200"), "{text}");
    assert!(text.contains("WIDTH=1200"), "{text}");
    // Rust launches every copy entry from an identity-checked `.run-*` snapshot. The important
    // normalization contract is that parameter delivery needs no second injected source rewrite.
    assert!(text.contains("scripts/runner/.run-"), "{text}");
    assert!(!text.contains(".injected-"), "{text}");
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
    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "param([string]$Name = 'World')\nWrite-Output $Name\n"
    );

    // The public consumer keeps the reader as the owning source and appends the declared rider.
    // Reading the plan changes neither metadata nor either source copy.
    let show = sandbox
        .command()
        .args(["show", "ps", "--json"])
        .output()
        .unwrap();
    assert!(show.status.success());
    assert!(show.stderr.is_empty());
    let shown: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(shown["param_source"], "argparse");
    let fields = shown["fields"].as_array().unwrap();
    assert_eq!(
        fields
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Name", "API_TOKEN"]
    );
    assert_eq!(fields[0]["source"], "flag");
    assert_eq!(fields[0]["flag"], "-Name");
    assert_eq!(fields[0]["default"], "World");
    assert_eq!(fields[1]["source"], "env");
    assert_eq!(fields[1]["default"], Value::Null);
    assert_eq!(
        fs::read_to_string(sandbox.data.path().join("scripts/ps/meta.toml")).unwrap(),
        meta
    );
    assert_eq!(
        fs::read_to_string(sandbox.data.path().join("scripts/ps/script.ps1")).unwrap(),
        stored
    );
    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "param([string]$Name = 'World')\nWrite-Output $Name\n"
    );
}
