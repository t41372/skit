use std::fs;

use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

#[path = "support/temp_root.rs"]
mod temp_root;

use temp_root::TempRoot;

#[path = "support/shim.rs"]
mod shim;

struct Sandbox {
    data: TempRoot,
    state: TempRoot,
    config: TempRoot,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempRoot::new(),
            state: TempRoot::new(),
            config: TempRoot::new(),
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

    fn add_command(&self, name: &str, template: &str) {
        self.command()
            .args(["add", "--cmd", template, "--name", name])
            .assert()
            .success();
    }
}

#[test]
fn list_json_keeps_the_v040_array_and_complete_row_shape() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");

    let output = sandbox.command().args(["list", "--json"]).output().unwrap();
    assert!(output.status.success());
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = rows.as_array().expect("v0.4 list JSON is an array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "Demo");
    assert_eq!(rows[0]["slug"], "demo");
    assert_eq!(rows[0]["kind"], "command");
    assert_eq!(rows[0]["mode"], "reference");
    assert_eq!(rows[0]["missing"], false);
    assert!(rows[0].get("last_run_at").is_some());
    assert!(rows[0].get("last_exit").is_some());
}

#[test]
fn list_keeps_latest_main_slug_order_when_display_names_cross() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Alpha", "printf alpha");
    sandbox.add_command("Zulu", "printf zulu");
    sandbox
        .command()
        .args(["rename", "alpha", "Zulu display"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["rename", "zulu", "Alpha display"])
        .assert()
        .success();

    let output = sandbox.command().args(["list", "--json"]).output().unwrap();
    assert!(output.status.success());
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| row["slug"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["alpha", "zulu"]
    );
}

#[test]
fn human_list_keeps_the_localized_table_kind_labels_and_literal_markup() {
    let sandbox = Sandbox::new();
    sandbox.add_command("[blue]Demo[/blue]", "printf ok");

    let english = sandbox.command().arg("list").output().unwrap();
    assert!(english.status.success());
    let english = String::from_utf8(english.stdout).unwrap();
    for text in ["Name", "Kind", "Description", "Command", "—"] {
        assert!(english.contains(text), "missing {text:?} in:\n{english}");
    }
    assert!(english.contains("[blue]Demo[/blue]"));

    let traditional = sandbox
        .command()
        .env("SKIT_LANG", "zh-TW")
        .arg("list")
        .output()
        .unwrap();
    assert!(traditional.status.success());
    let traditional = String::from_utf8(traditional.stdout).unwrap();
    for text in ["名稱", "類型", "說明", "指令", "—"] {
        assert!(
            traditional.contains(text),
            "missing {text:?} in:\n{traditional}"
        );
    }
}

#[test]
fn human_list_marks_a_missing_copy_target_without_reading_full_metadata() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("source.py");
    fs::write(&source, "print('ok')\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "Gone",
            "--no-input",
        ])
        .assert()
        .success();
    // Join one name at a time, the way the store joins them, so the host chooses the separator.
    let payload = sandbox
        .data
        .path()
        .join("scripts")
        .join("gone")
        .join("script.py");
    fs::remove_file(&payload).unwrap();

    let output = sandbox.command().arg("list").output().unwrap();
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains(&format!("⚠ missing: {}", payload.display())));
}

#[test]
fn human_list_empty_library_keeps_the_localized_discovery_hint() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .arg("list")
        .assert()
        .success()
        .stdout("No entries yet. Add one with: skit add <path>\n");
    sandbox
        .command()
        .env("SKIT_LANG", "zh-TW")
        .arg("list")
        .assert()
        .success()
        .stdout("還沒有任何條目。用 skit add <path> 加入一個。\n");
}

#[test]
fn show_json_keeps_the_v040_automation_record() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");
    let output = sandbox
        .command()
        .args(["show", "demo", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();

    for key in [
        "name",
        "slug",
        "kind",
        "mode",
        "description",
        "source",
        "workdir",
        "interpreter",
        "missing",
        "dependencies",
        "requires_python",
        "needs",
        "template",
        "param_source",
        "param_origin",
        "degraded_reason",
        "drift",
        "fields",
        "presets",
        "last_run_at",
        "last_exit",
    ] {
        assert!(record.get(key).is_some(), "missing JSON key: {key}");
    }
    assert_eq!(record["param_source"], "command");
    assert_eq!(record["fields"][0]["key"], "name");
    assert_eq!(record["degraded_reason"], "");
}

#[test]
fn human_show_keeps_the_v040_discovery_view_for_command_entries() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args([
            "add",
            "--cmd",
            "printf '%s' {token}",
            "--name",
            "Demo",
            "--description",
            "[red]literal[/red]",
        ])
        .assert()
        .success();
    let meta_path = sandbox.data.path().join("scripts/demo/meta.toml");
    let mut meta = fs::read_to_string(&meta_path).unwrap();
    meta.push_str(concat!(
        "\n[[parameters]]\n",
        "name = \"token\"\n",
        "delivery = \"placeholder\"\n",
        "type = \"str\"\n",
        "default = \"visible-only-in-source\"\n",
        "required = true\n",
        "secret = true\n",
        "env_source = \"API_TOKEN\"\n",
        "help = \"Authentication token\"\n",
    ));
    fs::write(meta_path, meta).unwrap();

    let output = sandbox.command().args(["show", "demo"]).output().unwrap();
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("Demo  (Command · reference)"));
    assert!(output.contains("[red]literal[/red]"));
    assert!(!output.contains("Source:"));
    assert!(!output.contains("Missing:"));
    assert!(output.contains("Command template: printf '%s' {token}"));
    for text in [
        "Parameter",
        "Type",
        "Required",
        "Default",
        "Choices",
        "Secret",
        "Help",
        "token",
        "•••",
        "yes ← $API_TOKEN",
        "Authentication token",
    ] {
        assert!(output.contains(text), "missing {text:?} in:\n{output}");
    }
    assert!(!output.contains("visible-only-in-source"));
    assert!(output.contains("Run it: skit run Demo"));
}

#[test]
fn human_show_uses_the_missing_marker_and_localized_kind_label() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("[red]gone.py");
    fs::write(&source, "print('ok')\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "Gone",
            "--reference",
            "--no-input",
        ])
        .assert()
        .success();
    fs::remove_file(&source).unwrap();

    let output = sandbox
        .command()
        .env("SKIT_LANG", "zh-TW")
        .args(["show", "gone"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("Gone  (Python · reference)"));
    assert!(output.contains(&format!("⚠ 遺失:{}", source.display())));
    assert!(!output.contains("遺失：是"));
    assert!(output.contains("沒有表單欄位——接在 -- 之後的參數會透傳給腳本。"));
    assert!(output.contains("執行:skit run Gone"));
}

#[test]
fn show_reconciles_live_fields_defaults_and_complete_drift_explanations() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("managed.py");
    fs::write(
        &source,
        concat!(
            "# /// script\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"KEEP\"\n",
            "# kind = \"const\"\n",
            "# type = \"str\"\n",
            "# default = \"stored\"\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"GONE\"\n",
            "# kind = \"const\"\n",
            "# type = \"str\"\n",
            "# default = \"gone\"\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"CHANGED\"\n",
            "# kind = \"const\"\n",
            "# type = \"str\"\n",
            "# default = \"old type\"\n",
            "# ///\n",
            "KEEP = \"stored\"\n",
            "GONE = \"gone\"\n",
            "CHANGED = \"old type\"\n",
        ),
    )
    .unwrap();
    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "Managed",
            "--no-input",
        ])
        .assert()
        .success();
    let payload = sandbox.data.path().join("scripts/managed/script.py");
    let changed = fs::read_to_string(&payload)
        .unwrap()
        .replace("KEEP = \"stored\"", "KEEP = \"current\"")
        .replace("GONE = \"gone\"\n", "")
        .replace("CHANGED = \"old type\"", "CHANGED = 42");
    fs::write(payload, changed).unwrap();

    let json = sandbox
        .command()
        .args(["show", "managed", "--json"])
        .output()
        .unwrap();
    assert!(json.status.success());
    let record: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(record["drift"], true);
    assert_eq!(
        record["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["KEEP", "CHANGED"]
    );
    assert_eq!(record["fields"][0]["default"], "current");
    assert_eq!(record["fields"][1]["default"], "old type");

    let human = sandbox
        .command()
        .args(["show", "managed"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout).unwrap();
    for line in [
        "The parameter definitions for Managed have drifted from the script:",
        "  GONE: injection target no longer exists (dropped from this run's form)",
        "  CHANGED: type changed from str to int in the source (still injected — double-check the value)",
        "To refresh the definitions, run: skit params Managed --resync",
    ] {
        assert!(
            human.lines().any(|actual| actual == line),
            "missing {line:?} in:\n{human}"
        );
    }
}

#[test]
fn preset_list_json_keeps_the_v040_direct_name_map() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");
    fs::create_dir_all(sandbox.state.path().join("values")).unwrap();
    fs::write(
        sandbox.state.path().join("values/demo.toml"),
        "[presets.favorite]\nname = \"Ada\"\n",
    )
    .unwrap();

    let output = sandbox
        .command()
        .args(["preset", "list", "demo", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record, serde_json::json!({"favorite": {"name": "Ada"}}));
}

#[test]
fn preset_human_list_keeps_values_order_and_the_empty_discovery_hint() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");
    fs::create_dir_all(sandbox.state.path().join("values")).unwrap();
    fs::write(
        sandbox.state.path().join("values/demo.toml"),
        concat!(
            "[presets.zeta]\n",
            "city = \"Taipei\"\n",
            "count = \"2\"\n",
            "[presets.alpha]\n",
            "city = \"Tokyo\"\n",
        ),
    )
    .unwrap();

    sandbox
        .command()
        .args(["preset", "list", "demo"])
        .assert()
        .success()
        .stdout("  alpha: city=Tokyo\n  zeta: city=Taipei, count=2\n");

    fs::remove_file(sandbox.state.path().join("values/demo.toml")).unwrap();
    sandbox
        .command()
        .args(["preset", "list", "demo"])
        .assert()
        .success()
        .stdout("No presets for Demo yet. Create one with: skit run Demo --save-preset <preset>\n");
    sandbox
        .command()
        .env("SKIT_LANG", "zh-TW")
        .args(["preset", "list", "demo"])
        .assert()
        .success()
        .stdout("Demo 還沒有參數組合。建立一個：skit run Demo --save-preset <組合名>\n");
}

#[test]
fn preset_save_delete_and_unknown_messages_keep_latest_main_context() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");

    sandbox
        .command()
        .args(["preset", "save", "demo", "favorite"])
        .assert()
        .success()
        .stdout("Preset \"favorite\" saved for Demo.\n");

    sandbox
        .command()
        .args(["preset", "delete", "demo", "missing"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "Unknown preset \"missing\". Available: favorite",
        ));
    sandbox
        .command()
        .args(["preset", "delete", "demo", "favorite"])
        .assert()
        .success()
        .stdout("Preset \"favorite\" deleted from Demo.\n");
}

#[test]
fn preset_save_uses_complete_prefill_and_from_last_requires_an_honest_snapshot() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");
    sandbox
        .command()
        .args(["params", "demo", "--default", "name=Ada"])
        .assert()
        .success();

    sandbox
        .command()
        .args(["preset", "save", "demo", "prefill"])
        .assert()
        .success();
    let presets: Value = serde_json::from_slice(
        &sandbox
            .command()
            .args(["preset", "list", "demo", "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(presets["prefill"]["name"], "Ada");

    let missing = sandbox
        .command()
        .args(["preset", "save", "demo", "missing", "--from-last"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    let presets: Value = serde_json::from_slice(
        &sandbox
            .command()
            .args(["preset", "list", "demo", "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(presets.get("missing").is_none());

    sandbox
        .command()
        .args(["preset", "delete", "demo", "prefill", "--no-input"])
        .assert()
        .success();
}

#[test]
fn config_json_keeps_the_v040_direct_key_map_and_final_value() {
    let sandbox = Sandbox::new();
    let written = sandbox
        .command()
        .args(["config", "form", "plain", "--json"])
        .output()
        .unwrap();
    assert!(written.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&written.stdout).unwrap(),
        serde_json::json!({"form": "plain"})
    );

    let read = sandbox
        .command()
        .args(["config", "form", "--json"])
        .output()
        .unwrap();
    assert!(read.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&read.stdout).unwrap(),
        serde_json::json!({"form": "plain"})
    );
}

#[test]
fn config_json_projects_hand_edited_values_without_mutating_the_file() {
    let sandbox = Sandbox::new();
    let path = sandbox.config.path().join("config.toml");
    let source = concat!(
        "language = \"fr-FR\"\n",
        "form = \"dialog\"\n",
        "after_run = \"loop\"\n",
        "future = 7\n",
        "js = \"future shape\"\n",
    );
    fs::write(&path, source).unwrap();

    let output = sandbox
        .command()
        .args(["config", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let settings: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(settings["lang"], "");
    assert_eq!(settings["form"], "tui");
    assert_eq!(settings["after_run"], "exit");
    assert_eq!(settings["js.runner"], "");
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn config_lang_canonicalizes_supported_v040_tags_and_auto_clears() {
    let sandbox = Sandbox::new();
    for (input, expected) in [("zh_tw.UTF-8", "zh-TW"), ("EN", "en"), ("en-xa", "en-XA")] {
        let output = sandbox
            .command()
            .args(["config", "lang", input, "--json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            serde_json::json!({"lang": expected})
        );
    }

    let rejected = sandbox
        .command()
        .args(["config", "lang", "fr-FR", "--json"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));

    let cleared = sandbox
        .command()
        .args(["config", "lang", "AUTO", "--json"])
        .output()
        .unwrap();
    assert!(cleared.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&cleared.stdout).unwrap(),
        serde_json::json!({"lang": ""})
    );
    let document = fs::read_to_string(sandbox.config.path().join("config.toml"))
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert!(!document.contains_key("language"));
}

#[test]
fn config_repairs_corrupt_toml_only_after_an_exact_backup() {
    let sandbox = Sandbox::new();
    let path = sandbox.config.path().join("config.toml");
    let backup = sandbox.config.path().join("config.toml.bak");
    let corrupt = "language = \"zh-TW\"\ninvalid = [使用者資料";
    fs::write(&path, corrupt).unwrap();

    let read = sandbox
        .command()
        .args(["config", "--json"])
        .output()
        .unwrap();
    assert!(read.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&read.stdout).unwrap()["lang"],
        ""
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), corrupt);
    assert!(!backup.exists());

    let write = sandbox
        .command()
        .args(["config", "editor", "  vim  ", "--json"])
        .output()
        .unwrap();
    assert!(write.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&write.stdout).unwrap(),
        serde_json::json!({"editor": "vim"})
    );
    assert_eq!(fs::read_to_string(backup).unwrap(), corrupt);
    let stderr = String::from_utf8(write.stderr).unwrap();
    assert_eq!(
        stderr.trim(),
        format!(
            "{} is corrupt and could not be parsed. It has been backed up to {} before this change; recover any lost settings from that file.",
            path.display(),
            sandbox.config.path().join("config.toml.bak").display(),
        )
    );
}

#[test]
fn config_bash_path_validates_the_trimmed_file_and_keeps_usage_exit_two() {
    let sandbox = Sandbox::new();
    let config_path = sandbox.config.path().join("config.toml");
    let bash = sandbox.config.path().join("bash");
    fs::write(&bash, "").unwrap();
    fs::write(&config_path, "future = 7\n").unwrap();

    let missing = sandbox.config.path().join("missing");
    let rejected = sandbox
        .command()
        .args([
            "config",
            "shell.bash_path",
            missing.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    assert_eq!(fs::read_to_string(&config_path).unwrap(), "future = 7\n");

    let padded = format!("  {}  ", bash.display());
    let accepted = sandbox
        .command()
        .args(["config", "shell.bash_path", &padded, "--json"])
        .output()
        .unwrap();
    assert!(accepted.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&accepted.stdout).unwrap(),
        serde_json::json!({"shell.bash_path": bash.display().to_string()})
    );
}

#[test]
fn management_missing_entries_keep_exit_one_while_run_keeps_127() {
    let sandbox = Sandbox::new();
    for arguments in [
        vec!["show", "missing", "--json"],
        vec!["describe", "missing", "description"],
        vec!["rename", "missing", "New name"],
        vec!["remove", "missing", "--yes"],
        vec!["edit", "missing", "--no-input"],
        vec!["params", "missing", "--json"],
        vec!["deps", "missing", "--json"],
        vec!["preset", "save", "missing", "demo", "--from-last"],
        vec!["preset", "list", "missing", "--json"],
        vec!["preset", "delete", "missing", "demo", "--yes"],
    ] {
        let output = sandbox.command().args(&arguments).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let run = sandbox
        .command()
        .args(["run", "missing", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(127));
}

#[test]
fn ordinary_management_refusals_keep_v040_exit_codes() {
    let sandbox = Sandbox::new();
    let missing_file = sandbox
        .command()
        .args(["add", "/definitely/missing/skit-source.py", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(missing_file.status.code(), Some(1));

    let unknown_config = sandbox
        .command()
        .args(["config", "unknown-key", "--json"])
        .output()
        .unwrap();
    assert_eq!(unknown_config.status.code(), Some(2));

    let duplicate_runner = sandbox
        .command()
        .args(["runner", "add", "claude", "claude", "{{prompt}}"])
        .output()
        .unwrap();
    assert_eq!(duplicate_runner.status.code(), Some(1));
}

#[test]
fn dry_run_is_read_only_for_legacy_identity_and_runner_is_prompt_only() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "printf '%s' {name}");
    let metadata = sandbox.data.path().join("scripts/demo/meta.toml");
    let without_identity = fs::read_to_string(&metadata)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with("id = "))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&metadata, &without_identity).unwrap();

    let preview = sandbox
        .command()
        .args([
            "run",
            "demo",
            "--set",
            "name=Ada",
            "--dry-run",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert_eq!(fs::read_to_string(&metadata).unwrap(), without_identity);

    let runner = sandbox
        .command()
        .args([
            "run",
            "demo",
            "--runner",
            "claude",
            "--dry-run",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(runner.status.code(), Some(2));
}

#[test]
fn runner_list_all_json_keeps_v040_zero_based_repair_rows() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["runner", "list", "--all", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["row"], 0);
}

#[test]
fn runner_management_keeps_malformed_container_and_reason_contracts() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.config.path()).unwrap();
    fs::write(
        sandbox.config.path().join("config.toml"),
        "language = \"zh-TW\"\nprompt = \"garbage\"\n",
    )
    .unwrap();

    let listed = sandbox
        .command()
        .args(["runner", "list", "--all", "--json"])
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let rows: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(rows[0]["row"], Value::Null);
    assert_eq!(rows[0]["reason"], "prompt-section-not-table");
    assert_eq!(rows[0]["descriptor"], "prompt");

    let removed = sandbox
        .command()
        .args(["runner", "remove", "--row", "container", "--yes"])
        .output()
        .unwrap();
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    let document = fs::read_to_string(sandbox.config.path().join("config.toml")).unwrap();
    assert!(document.contains("language = \"zh-TW\""), "{document}");
    assert!(document.contains("runners = []"), "{document}");
}

#[test]
fn runner_management_rejects_unsupported_holes_and_valid_raw_row_deletion() {
    let sandbox = Sandbox::new();
    let invalid = sandbox
        .command()
        .args(["runner", "add", "mine", "agent", "{{other}}", "{{prompt}}"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));

    let valid_row = sandbox
        .command()
        .args(["runner", "remove", "--row", "0", "--yes"])
        .output()
        .unwrap();
    assert_eq!(valid_row.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&valid_row.stderr).contains("runner remove"),
        "{}",
        String::from_utf8_lossy(&valid_row.stderr)
    );
}

#[test]
fn runner_management_keeps_latest_main_duplicate_messages_commands_and_pin_warning() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["runner", "add", "mine", "agent", "{{prompt}}"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Runner mine added:")
                .and(predicate::str::contains("{{prompt}}")),
        );
    sandbox
        .command()
        .args(["runner", "add", "mine", "other", "{{prompt}}"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "already exists — pass --force to replace its command",
        ));
    sandbox
        .command()
        .args(["runner", "add", "mine", "--force", "other", "{{prompt}}"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Runner mine updated:"));
    sandbox
        .command()
        .args(["runner", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("mine").and(predicate::str::contains("{{prompt}}")));

    sandbox
        .command()
        .args([
            "add", "-", "--prompt", "--name", "Review", "--runner", "mine",
        ])
        .write_stdin("Review this.\n")
        .assert()
        .success();
    sandbox
        .command()
        .args(["runner", "remove", "mine", "--yes"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 prompt pins this runner")
                .and(predicate::str::contains("Runner mine removed.")),
        );
}

#[test]
fn agent_named_targets_keep_user_project_and_cross_agent_directories() {
    let sandbox = Sandbox::new();
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();

    sandbox
        .command()
        .env("HOME", home.path())
        .current_dir(project.path())
        .args(["agent", "install", "claude"])
        .assert()
        .success();
    sandbox
        .command()
        .env("HOME", home.path())
        .current_dir(project.path())
        .args(["agent", "install", "codex", "--project"])
        .assert()
        .success();
    sandbox
        .command()
        .env("HOME", home.path())
        .current_dir(project.path())
        .args(["agent", "install", "agents"])
        .assert()
        .success();

    assert!(home.path().join(".claude/skills/skit/SKILL.md").is_file());
    assert!(project.path().join(".codex/skills/skit/SKILL.md").is_file());
    assert!(
        project
            .path()
            .join(".agents/skills/skit/SKILL.md")
            .is_file()
    );
    assert!(!home.path().join(".agents").exists());
}

#[test]
fn add_reads_stdin_and_accepts_python_dependency_options_without_prompting() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args([
            "add",
            "-",
            "--name",
            "Pipe tool",
            "--kind",
            "python",
            "--dep",
            "requests>=2",
            "--python",
            ">=3.12",
            "--no-input",
        ])
        .write_stdin("print('pipe')\n")
        .assert()
        .success();
    let stored =
        fs::read_to_string(sandbox.data.path().join("scripts/pipe-tool/script.py")).unwrap();
    assert!(stored.contains("# /// script"));
    assert!(stored.contains("dependencies = [\"requests>=2\"]"));
    assert!(stored.contains("requires-python = \">=3.12\""));
    assert!(stored.ends_with("print('pipe')\n"));
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/pipe-tool/meta.toml")).unwrap();
    assert!(!meta.contains("dependencies ="));
    assert!(!meta.contains("requires_python ="));
    let deps = sandbox
        .command()
        .args(["deps", "pipe-tool", "--json"])
        .output()
        .unwrap();
    let deps: Value = serde_json::from_slice(&deps.stdout).unwrap();
    assert_eq!(deps["dependencies"], serde_json::json!(["requests>=2"]));
    assert_eq!(deps["requires_python"], ">=3.12");
}

#[test]
fn params_supports_the_remaining_declared_schema_options() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo {name}");
    sandbox
        .command()
        .args([
            "params",
            "demo",
            "--help-text",
            "name=Shown beside the field.",
            "--prompt",
            "name=Your name",
            "--env-source",
            "name=SKIT_TEST_NAME",
            "--secret",
            "name",
        ])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "demo", "--json"])
        .output()
        .unwrap();
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    let field = &record["parameters"][0];
    assert_eq!(field["help"], "Shown beside the field.");
    assert_eq!(field["prompt"], "Your name");
    assert_eq!(field["env_source"], "SKIT_TEST_NAME");
    assert_eq!(field["secret"], true);
}

#[test]
fn params_json_keeps_discovery_defaults_declared_and_state_surfaces() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.sh");
    fs::write(&source, "NAME=world\necho \"$NAME\"\n").unwrap();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Tool"])
        .assert()
        .success();
    fs::create_dir_all(sandbox.state.path().join("values")).unwrap();
    fs::write(
        sandbox.state.path().join("values/tool.toml"),
        "[values]\nNAME = \"remembered\"\n",
    )
    .unwrap();

    let output = sandbox
        .command()
        .args(["params", "tool", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    for key in [
        "params",
        "parameters",
        "current_defaults",
        "last_values",
        "unmanaged",
        "placeholders",
        "declared",
    ] {
        assert!(record.get(key).is_some(), "missing params JSON key: {key}");
    }
    assert_eq!(record["params"], serde_json::json!([]));
    assert_eq!(record["current_defaults"], serde_json::json!({}));
    assert_eq!(record["last_values"]["NAME"], "remembered");
    assert_eq!(record["unmanaged"], serde_json::json!(["NAME"]));

    sandbox
        .command()
        .args(["params", "tool", "--manage", "NAME"])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "tool", "--json"])
        .output()
        .unwrap();
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["params"][0]["name"], "NAME");
    assert_eq!(record["current_defaults"]["NAME"], "world");
    assert_eq!(record["unmanaged"], serde_json::json!([]));
}

#[test]
fn params_cli_can_set_every_frontend_neutral_parameter_axis() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo {name}");
    sandbox
        .command()
        .args([
            "params",
            "demo",
            "--binding",
            "name=none",
            "--multiple",
            "name",
            "--repeat",
            "name",
            "--env-target",
            "name=SKIT_NAME",
        ])
        .assert()
        .success();
    let parameter = |sandbox: &Sandbox, selector: &str, index: usize| -> Value {
        let output = sandbox
            .command()
            .args(["params", selector, "--json"])
            .output()
            .unwrap();
        let record: Value = serde_json::from_slice(&output.stdout).unwrap();
        record["parameters"][index].clone()
    };
    let field = parameter(&sandbox, "demo", 0);
    assert_eq!(field["binding"], "none");
    assert_eq!(field["multiple"], true);
    assert_eq!(field["repeat"], true);
    assert_eq!(field["env_target"], "SKIT_NAME");
    // A non-bool row carries no action: an edit that moves a type off bool sheds the stale
    // value (`src/skit/params.py:492-493` `if decl.type != "bool": decl.action = ""`).
    assert_eq!(field["action"], "");

    // The action axis belongs to a bool flag. Declaring one with no action records
    // store_true, because "pass the flag when on" is what the checkbox means
    // (`src/skit/params.py:488-491`).
    let executable = sandbox.state.path().join("program");
    fs::write(&executable, b"program").unwrap();
    sandbox
        .command()
        .arg("add")
        .arg(&executable)
        .args(["--exe", "--name", "Bools", "--no-input"])
        .assert()
        .success();
    sandbox
        .command()
        .args([
            "params",
            "bools",
            "--add",
            "verbose",
            "--type",
            "verbose=bool",
            "--deliver",
            "verbose=flag",
            "--flag",
            "verbose=--verbose",
        ])
        .assert()
        .success();
    assert_eq!(parameter(&sandbox, "bools", 0)["action"], "store_true");

    sandbox
        .command()
        .args(["params", "bools", "--action", "verbose=store_false"])
        .assert()
        .success();
    assert_eq!(parameter(&sandbox, "bools", 0)["action"], "store_false");

    // Moving the same row off bool sheds the action again.
    sandbox
        .command()
        .args(["params", "bools", "--type", "verbose=str"])
        .assert()
        .success();
    assert_eq!(parameter(&sandbox, "bools", 0)["action"], "");

    sandbox
        .command()
        .args([
            "params",
            "demo",
            "--no-multiple",
            "name",
            "--no-repeat",
            "name",
        ])
        .assert()
        .success();
}

#[test]
fn params_refuses_inapplicable_or_order_dependent_operations_without_a_write() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo {name}");
    let meta_path = sandbox.data.path().join("scripts/demo/meta.toml");
    let before = fs::read(&meta_path).unwrap();

    for (arguments, exit_code) in [
        (vec!["params", "demo", "--runner", ""], 1),
        (vec!["params", "demo", "--no-interpolate"], 1),
        (vec!["params", "demo", "--interpreter", "bash"], 2),
        (vec!["params", "demo", "--workdir", "relative/path"], 2),
        (vec!["params", "demo", "--runner", "", "--add", "other"], 2),
        (
            vec!["params", "demo", "--template", "", "--add", "other"],
            2,
        ),
    ] {
        sandbox.command().args(arguments).assert().code(exit_code);
        assert_eq!(fs::read(&meta_path).unwrap(), before);
    }
}

#[test]
fn show_json_reports_reader_field_sources_and_effective_python_metadata() {
    let sandbox = Sandbox::new();
    let python = sandbox.data.path().join("reader.py");
    fs::write(
        &python,
        "import argparse\np = argparse.ArgumentParser()\np.add_argument('--count', type=int)\n",
    )
    .unwrap();
    sandbox
        .command()
        .args([
            "add",
            python.to_str().unwrap(),
            "--name",
            "Reader",
            "--dep",
            "requests>=2",
            "--python",
            ">=3.12",
        ])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["show", "reader", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["dependencies"], serde_json::json!(["requests>=2"]));
    assert_eq!(record["requires_python"], ">=3.12");
    assert_eq!(record["param_source"], "argparse");
    assert_eq!(record["param_origin"], "reader");
    assert_eq!(record["fields"][0]["source"], "flag");
}

#[test]
fn show_json_keeps_the_complete_latest_main_argparse_field_contract() {
    let sandbox = Sandbox::new();
    let python = sandbox.data.path().join("schema.py");
    fs::write(
        &python,
        concat!(
            "import argparse\n",
            "ap = argparse.ArgumentParser()\n",
            "ap.add_argument('src')\n",
            "ap.add_argument('--width', type=int, default=800, help='target width')\n",
            "ap.add_argument('--fmt', choices=['png', 'jpg'], default='png')\n",
            "ap.add_argument('--force', action='store_true')\n",
            "ap.parse_args()\n",
        ),
    )
    .unwrap();
    sandbox
        .command()
        .args([
            "add",
            python.to_str().unwrap(),
            "--name",
            "Schema",
            "--no-input",
        ])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["show", "schema", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["param_source"], "argparse");
    assert_eq!(record["param_origin"], "reader");
    assert_eq!(record["degraded_reason"], "");
    let fields = record["fields"].as_array().unwrap();
    assert_eq!(
        fields
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["src", "width", "fmt", "force"]
    );
    assert_eq!(fields[0]["required"], true);
    assert_eq!(fields[0]["default"], Value::Null);
    assert_eq!(fields[1]["type"], "int");
    assert_eq!(fields[1]["default"], "800");
    assert_eq!(fields[1]["help"], "target width");
    assert_eq!(fields[1]["flag"], "--width");
    assert_eq!(fields[2]["type"], "choice");
    assert_eq!(fields[2]["choices"], serde_json::json!(["png", "jpg"]));
    assert_eq!(fields[3]["type"], "bool");
    assert_eq!(fields[3]["action"], "store_true");
    assert_eq!(fields[3]["default"], "false");
    for field in fields {
        for key in [
            "key",
            "label",
            "type",
            "source",
            "required",
            "secret",
            "multiple",
            "repeat",
            "degraded",
            "choices",
            "default",
            "help",
            "flag",
            "action",
            "env_source",
            "delivers_empty",
        ] {
            assert!(field.get(key).is_some(), "field is missing {key}: {field}");
        }
    }
}

#[test]
fn show_reports_a_dynamic_python_cli_surface_without_inventing_fields() {
    let sandbox = Sandbox::new();
    let python = sandbox.data.path().join("dynamic.py");
    fs::write(
        &python,
        concat!(
            "import argparse\n",
            "ap = argparse.ArgumentParser()\n",
            "sub = ap.add_subparsers()\n",
            "child = sub.add_parser('x')\n",
            "child.add_argument('--value')\n",
            "ap.parse_args()\n",
        ),
    )
    .unwrap();
    sandbox
        .command()
        .args([
            "add",
            python.to_str().unwrap(),
            "--name",
            "Dynamic",
            "--no-input",
        ])
        .assert()
        .success();

    let json = sandbox
        .command()
        .args(["show", "dynamic", "--json"])
        .output()
        .unwrap();
    assert!(json.status.success());
    let record: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(record["param_source"], "argparse");
    assert_eq!(record["degraded_reason"], "subparsers");
    assert_eq!(record["fields"], serde_json::json!([]));

    sandbox
        .command()
        .args(["show", "dynamic"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "skit could not model this script's own arguments; pass them after -- instead.\n",
        ));
}

#[test]
fn show_json_keeps_click_multiple_as_a_repeated_flag_contract() {
    let sandbox = Sandbox::new();
    let python = sandbox.data.path().join("clicker.py");
    fs::write(
        &python,
        concat!(
            "import click\n",
            "@click.command()\n",
            "@click.option('--tag', multiple=True)\n",
            "def main(tag):\n",
            "    pass\n",
        ),
    )
    .unwrap();
    sandbox
        .command()
        .args([
            "add",
            python.to_str().unwrap(),
            "--name",
            "Clicker",
            "--no-input",
        ])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["show", "clicker", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["fields"][0]["key"], "tag");
    assert_eq!(record["fields"][0]["multiple"], true);
    assert_eq!(record["fields"][0]["repeat"], true);
}

#[test]
fn show_json_distinguishes_shell_empty_fallback_from_an_empty_value() {
    let sandbox = Sandbox::new();
    let shell = sandbox.data.path().join("defaults.sh");
    fs::write(
        &shell,
        concat!(
            "#!/bin/sh\n",
            "# /// script\n",
            "# [tool.skit]\n",
            "# schema = 1\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"FALLBACK_ON_EMPTY\"\n",
            "# kind = \"envdefault\"\n",
            "# type = \"str\"\n",
            "# default = \"first\"\n",
            "#\n",
            "# [[tool.skit.params]]\n",
            "# name = \"EMPTY_IS_VALUE\"\n",
            "# kind = \"envdefault\"\n",
            "# type = \"str\"\n",
            "# default = \"second\"\n",
            "# ///\n",
            "printf '%s %s\\n' \"${FALLBACK_ON_EMPTY:-first}\" \"${EMPTY_IS_VALUE-second}\"\n",
        ),
    )
    .unwrap();
    sandbox
        .command()
        .args([
            "add",
            shell.to_str().unwrap(),
            "--name",
            "Defaults",
            "--no-input",
        ])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["show", "defaults", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["fields"][0]["key"], "FALLBACK_ON_EMPTY");
    assert_eq!(record["fields"][0]["delivers_empty"], false);
    assert_eq!(record["fields"][1]["key"], "EMPTY_IS_VALUE");
    assert_eq!(record["fields"][1]["delivers_empty"], true);
}

#[test]
fn run_uses_user_configured_prompt_runner_rows() {
    let sandbox = Sandbox::new();
    let prompt = sandbox.data.path().join("review.prompt.md");
    fs::write(&prompt, "Review {{subject}}.").unwrap();
    sandbox
        .command()
        .args(["add", prompt.to_str().unwrap(), "--name", "Review"])
        .assert()
        .success();
    // The row names a real program, and `printf` is not one on every host. The stand-in writes
    // its first argument with no line end after it, which is what this assertion reads, and the
    // row itself is what the test is about.
    let emit = shim::write_shim(sandbox.state.path(), "emit", shim::Shim::WriteArgumentRaw);
    sandbox
        .command()
        .args([
            "runner",
            "add",
            "custom",
            emit.to_str().unwrap(),
            "{{prompt}}",
        ])
        .assert()
        .success();
    sandbox
        .command()
        .args([
            "run",
            "review",
            "--runner",
            "custom",
            "--set",
            "subject=Rust",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("→ ")
                .and(predicate::str::contains(
                    "<rendered prompt omitted; use --dry-run to inspect it>",
                ))
                .and(predicate::str::ends_with("Review Rust.")),
        );
}

#[test]
fn raw_mode_refuses_template_artifacts_without_writing_state() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo {name}");
    sandbox
        .command()
        .args(["run", "demo", "--raw"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("does not apply"));
    assert!(!sandbox.state.path().join("values/demo.toml").exists());
}

#[test]
fn agent_explicit_to_remains_the_v040_skills_directory_contract() {
    let sandbox = Sandbox::new();
    let target = sandbox.data.path().join("skills");
    sandbox
        .command()
        .args(["agent", "install", "--to", target.to_str().unwrap()])
        .assert()
        .success();
    assert!(target.join("skit/SKILL.md").is_file());
    assert!(!target.join("skills/skit/SKILL.md").exists());
}

#[test]
fn bare_agent_install_never_creates_an_unselected_third_party_directory() {
    let sandbox = Sandbox::new();
    let home = TempDir::new().unwrap();
    sandbox
        .command()
        .env("HOME", home.path())
        .args(["agent", "install"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "name a target (claude, codex, agents) or pass --to DIR",
        ));
    for directory in [".agents", ".claude", ".codex"] {
        assert!(!home.path().join(directory).exists());
    }
}

#[test]
fn bare_agent_install_refuses_to_guess_even_one_existing_agent_directory() {
    let sandbox = Sandbox::new();
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".codex")).unwrap();
    sandbox
        .command()
        .env("HOME", home.path())
        .args(["agent", "install"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("name a target"));
    assert!(!home.path().join(".codex/skills").exists());
}

#[test]
fn agent_install_rejects_mixed_target_syntax_before_writing() {
    let sandbox = Sandbox::new();
    let target = sandbox.data.path().join("explicit");
    sandbox
        .command()
        .args([
            "agent",
            "install",
            "claude",
            "--to",
            target.to_str().unwrap(),
        ])
        .assert()
        .code(2);
    sandbox
        .command()
        .args([
            "agent",
            "install",
            "--to",
            target.to_str().unwrap(),
            "--project",
        ])
        .assert()
        .code(2);
    assert!(!target.exists());
}

#[test]
fn agents_convention_is_project_scoped_without_project_flag() {
    let sandbox = Sandbox::new();
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    sandbox
        .command()
        .env("HOME", home.path())
        .current_dir(project.path())
        .args(["agent", "install", "agents"])
        .assert()
        .success();
    assert!(
        project
            .path()
            .join(".agents/skills/skit/SKILL.md")
            .is_file()
    );
    assert!(!home.path().join(".agents").exists());
}

#[test]
fn deps_refuses_package_axes_for_kinds_without_package_management() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo ok");
    sandbox
        .command()
        .args(["deps", "demo", "--dep", "requests"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "doesn't take package dependencies",
        ));
    sandbox
        .command()
        .args(["deps", "demo", "--need", "printf"])
        .assert()
        .success();
}

/// Assert one stored timestamp keeps the exact spelling version 0.4 wrote.
///
/// Version 0.4 stamps with `datetime.now(UTC).replace(microsecond=0).isoformat()`
/// (`models.py:248`), which produces `2026-08-23T09:48:54+00:00`: whole seconds, and the offset
/// named `+00:00`. Every separator and digit position is pinned, so a stamp that keeps the
/// fractional second or renames the offset is a different string.
fn assert_v040_stamp(value: &str) {
    assert_eq!(value.len(), 25, "{value}");
    assert!(value.ends_with("+00:00"), "{value}");
    assert!(!value.contains('.'), "{value}");
    assert!(!value.contains('Z'), "{value}");
    let bytes = value.as_bytes();
    for (index, separator) in [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')] {
        assert_eq!(bytes[index], separator, "{value} at {index}");
    }
    for index in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        assert!(bytes[index].is_ascii_digit(), "{value} at {index}");
    }
}

fn quoted_field(document: &str, key: &str) -> String {
    let value = document
        .parse::<toml::Table>()
        .unwrap_or_else(|error| panic!("{document} is not a table: {error}"));
    value
        .get(key)
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("{key} is missing from {document}"))
        .to_owned()
}

#[test]
fn stored_timestamps_keep_the_v040_spelling_the_store_and_the_state_file() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo hello");

    let meta_path = sandbox.data.path().join("scripts/demo/meta.toml");
    let meta = fs::read_to_string(&meta_path).unwrap();
    let added_at = quoted_field(&meta, "added_at");
    assert_v040_stamp(&added_at);
    assert!(
        meta.contains(&format!("added_at = \"{added_at}\"")),
        "{meta}"
    );

    let shown = sandbox
        .command()
        .args(["show", "demo", "--json"])
        .output()
        .unwrap();
    let record: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(record["added_at"], Value::String(added_at));

    sandbox
        .command()
        .args(["run", "demo", "--no-input"])
        .assert()
        .success();
    let state = fs::read_to_string(sandbox.state.path().join("values/demo.toml")).unwrap();
    let document = state.parse::<toml::Table>().unwrap();
    let at = document["last_run"]["at"]
        .as_str()
        .unwrap_or_else(|| panic!("last_run.at is missing from {state}"));
    assert_v040_stamp(at);
}

/// A stamp version 0.4 wrote reads back byte-for-byte: no read path rewrites it.
#[test]
fn a_version_0_4_stamp_survives_a_read_unchanged() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Demo", "echo hello");
    let meta_path = sandbox.data.path().join("scripts/demo/meta.toml");
    let meta = fs::read_to_string(&meta_path).unwrap();
    let written = quoted_field(&meta, "added_at");
    let planted = "2019-03-04T05:06:07+00:00";
    fs::write(
        &meta_path,
        meta.replace(
            &format!("added_at = \"{written}\""),
            &format!("added_at = \"{planted}\""),
        ),
    )
    .unwrap();

    let shown = sandbox
        .command()
        .args(["show", "demo", "--json"])
        .output()
        .unwrap();
    let record: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(record["added_at"], Value::String(planted.to_owned()));
    assert_eq!(
        quoted_field(&fs::read_to_string(&meta_path).unwrap(), "added_at"),
        planted
    );
}
