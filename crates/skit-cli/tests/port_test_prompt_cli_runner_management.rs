use std::{fs, process::Output};

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn config_path(&self) -> std::path::PathBuf {
        self.config.path().join("config.toml")
    }

    fn write_config(&self, text: &str) {
        fs::create_dir_all(self.config.path()).unwrap();
        fs::write(self.config_path(), text).unwrap();
    }

    fn config_text(&self) -> String {
        fs::read_to_string(self.config_path()).unwrap_or_default()
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("stdout was not exactly JSON: {error}\n{}", combined(&output))
        })
    }

    fn add_prompt_pinned(&self, name: &str, runner: &str) {
        let source = self.home.path().join(format!("{name}.prompt.md"));
        fs::write(&source, "Do it\n").unwrap();
        let output = self.run(&[
            "add",
            source.to_str().unwrap(),
            "--name",
            name,
            "--runner",
            runner,
            "--no-input",
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(output: &Output) -> String {
    combined(output).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn runner_names(payload: &Value) -> Vec<&str> {
    payload
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect()
}

#[test]
fn test_runner_list_materializes_the_seeds() {
    let sandbox = Sandbox::new();
    assert!(!sandbox.config_path().exists());
    let output = sandbox.run(&["runner", "list"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = flat(&output);
    for name in [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
        "copilot",
        "cursor",
        "pi",
    ] {
        assert!(shown.contains(name), "seed {name} missing: {shown}");
    }
    assert!(shown.contains("amp -x"), "{shown}");
    assert!(shown.contains("does not open an interactive session"), "{shown}");
    let stored = sandbox.config_text();
    assert!(stored.contains("runners_seeded = true"), "{stored}");
}

#[test]
fn test_runner_list_json() {
    let sandbox = Sandbox::new();
    let payload = sandbox.json(&["runner", "list", "--json"]);
    let rows = payload.as_array().unwrap();
    for expected in [
        json!({"name":"claude","argv":["claude","--","{{prompt}}"]}),
        json!({"name":"opencode","argv":["opencode","--prompt={{prompt}}"]}),
        json!({"name":"copilot","argv":["copilot","--interactive={{prompt}}"]}),
        json!({"name":"cursor","argv":["cursor-agent","--","agent","{{prompt}}"]}),
        json!({"name":"pi","argv":["pi","{{prompt}}"]}),
    ] {
        assert!(rows.contains(&expected), "missing {expected} in {payload}");
    }
}

#[test]
fn test_runner_list_all_json_exposes_stable_raw_indexes_and_reasons() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \"good\", argv = [\"good\", \"{{prompt}}\"] }, ",
        "{ name = \"broken\", argv = [\"broken\"] }, ",
        "\"not-a-table\"",
        "]\n",
    ));
    let payload = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(
        payload,
        json!([
            {"row":0,"name":"good","argv":["good","{{prompt}}"],"reason":null,"descriptor":"good","valid":true},
            {"row":1,"name":"broken","argv":["broken"],"reason":"prompt-slot-count","descriptor":"broken","valid":false},
            {"row":2,"name":null,"argv":null,"reason":"row-not-table","descriptor":"not-a-table","valid":false}
        ])
    );
}

#[test]
fn test_runner_list_empty_state() {
    let sandbox = Sandbox::new();
    sandbox.write_config("[prompt]\nrunners_seeded = true\nrunners = []\n");
    for args in [vec!["runner", "list"], vec!["runner", "list", "--all"]] {
        let output = sandbox.run(&args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        let shown = flat(&output);
        assert!(shown.contains("No agents are configured"), "{shown}");
        assert!(
            shown.contains("skit runner add mycli -- mycli run {{prompt}}"),
            "{shown}"
        );
    }
}

#[test]
fn test_runner_list_without_amp_omits_the_one_shot_note() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [{ name = \"mycli\", argv = [\"mycli\", \"run\", \"{{prompt}}\"] }]\n",
    ));
    let output = sandbox.run(&["runner", "list"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("mycli"), "{shown}");
    assert!(!shown.contains("one-shot") && !shown.contains("does not open an interactive session"), "{shown}");
}

#[test]
fn test_runner_add_with_flag_bearing_argv() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[
        "runner",
        "add",
        " sonnet ",
        "claude",
        "--model",
        "sonnet",
        "{{prompt}}",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let payload = sandbox.json(&["runner", "list", "--json"]);
    assert!(
        payload.as_array().unwrap().contains(&json!({
            "name":"sonnet",
            "argv":["claude","--model","sonnet","{{prompt}}"]
        })),
        "{payload}"
    );
    assert!(!runner_names(&payload).contains(&" sonnet "));
}

#[test]
fn test_runner_add_preserves_bad_rows_and_force_repairs_matching_name() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \"typo\", argv = [\"old\"] }, ",
        "\"not-a-table\"",
        "]\n",
    ));
    let added = sandbox.run(&["runner", "add", "new", "new", "{{prompt}}"]);
    assert_eq!(added.status.code(), Some(0), "{}", combined(&added));
    let after_add = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(after_add[0]["name"], "typo");
    assert_eq!(after_add[0]["argv"], json!(["old"]));
    assert_eq!(after_add[1]["descriptor"], "not-a-table");

    let refused = sandbox.run(&["runner", "add", "typo", "fixed", "{{prompt}}"]);
    assert_eq!(refused.status.code(), Some(1), "{}", combined(&refused));
    let repaired = sandbox.run(&[
        "runner", "add", "typo", "--force", "--", "fixed", "{{prompt}}",
    ]);
    assert_eq!(repaired.status.code(), Some(0), "{}", combined(&repaired));
    let rows = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(rows[0]["name"], "typo");
    assert_eq!(rows[0]["argv"], json!(["fixed", "{{prompt}}"]));
    assert_eq!(rows[1]["descriptor"], "not-a-table");
    assert_eq!(rows[2]["name"], "new");
}

#[test]
fn test_runner_add_blank_name_is_refused_before_seeding() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["runner", "add", "   ", "x", "{{prompt}}"]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(combined(&output).contains("A name is required"), "{}", combined(&output));
    assert!(!sandbox.config_path().exists(), "blank-name validation seeded config before refusal");
}

#[test]
fn test_runner_add_validation_errors() {
    let sandbox = Sandbox::new();
    for (args, needle) in [
        (vec!["runner", "add", "noslot", "claude"], "exactly once"),
        (vec!["runner", "add", "bin", "{{prompt}}"], "first word"),
        (vec!["runner", "add", "stray", "x", "{{other}}"], "only the {{prompt}} slot"),
        (vec!["runner", "add", "bare"], "needs a command"),
    ] {
        let output = sandbox.run(&args);
        assert_eq!(output.status.code(), Some(2), "args={args:?}\n{}", combined(&output));
        assert!(combined(&output).contains(needle), "args={args:?}\n{}", combined(&output));
    }
}

#[test]
fn test_runner_add_duplicate_name_refused() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["runner", "add", "claude", "x", "{{prompt}}"]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("already exists"), "{}", combined(&output));
}

#[test]
fn test_runner_add_reports_malformed_config_container() {
    for (prompt, needle) in [
        ("prompt = \"broken\"\n", "isn't a table"),
        ("[prompt]\nrunners = \"broken\"\n", "isn't a list"),
    ] {
        let sandbox = Sandbox::new();
        sandbox.write_config(prompt);
        let before = sandbox.config_text();
        let output = sandbox.run(&["runner", "add", "new", "new", "{{prompt}}"]);
        assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
        assert!(combined(&output).contains(needle), "{}", combined(&output));
        assert_eq!(sandbox.config_text(), before, "malformed container was modified on refusal");
    }
}

#[test]
fn test_runner_remove_and_unknown() {
    let sandbox = Sandbox::new();
    let first = sandbox.run(&["runner", "remove", " amp ", "-y"]);
    assert_eq!(first.status.code(), Some(0), "{}", combined(&first));
    let payload = sandbox.json(&["runner", "list", "--json"]);
    assert!(!runner_names(&payload).contains(&"amp"));
    let second = sandbox.run(&["runner", "remove", "amp", "-y"]);
    assert_eq!(second.status.code(), Some(1), "{}", combined(&second));
    assert!(combined(&second).contains("Unknown runner"), "{}", combined(&second));
}

#[test]
fn test_runner_remove_blank_name_is_usage_error_before_seeding() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["runner", "remove", "   ", "--yes"]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(combined(&output).contains("A name is required"), "{}", combined(&output));
    assert!(!sandbox.config_path().exists(), "blank removal target seeded config before refusal");
}

#[test]
fn test_runner_remove_rejects_ambiguous_or_invalid_targets_before_writing() {
    for (args, needle) in [
        (vec![], "exactly one"),
        (vec!["amp", "--row", "0"], "exactly one"),
        (vec!["--row", "not-an-index"], "non-negative index"),
        (vec!["--row", "-1"], "non-negative index"),
    ] {
        let sandbox = Sandbox::new();
        sandbox.write_config("[prompt]\nrunners_seeded = true\nrunners = []\n");
        let before = sandbox.config_text();
        let mut full = vec!["runner", "remove"];
        full.extend(args);
        full.push("--yes");
        let output = sandbox.run(&full);
        assert_eq!(output.status.code(), Some(2), "args={full:?}\n{}", combined(&output));
        assert!(combined(&output).contains(needle), "args={full:?}\n{}", combined(&output));
        assert_eq!(sandbox.config_text(), before, "invalid target modified config");
    }
}

#[test]
fn test_removing_every_runner_stays_empty() {
    let sandbox = Sandbox::new();
    for name in [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
        "copilot",
        "cursor",
        "pi",
    ] {
        let output = sandbox.run(&["runner", "remove", name, "--yes"]);
        assert_eq!(output.status.code(), Some(0), "name={name}\n{}", combined(&output));
    }
    let payload = sandbox.json(&["runner", "list", "--json"]);
    assert_eq!(payload, json!([]), "removed seed rows resurrected: {payload}");
    let text = sandbox.config_text();
    assert!(text.contains("runners_seeded = true"), "seed marker was lost: {text}");
}

#[test]
fn test_runner_remove_warns_and_preserves_affected_prompt_pins() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt_pinned("p", "amp");
    let output = sandbox.run(&["runner", "remove", "amp", "--yes"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(combined(&output).contains("1 prompt pins this runner"), "{}", combined(&output));
    let shown = sandbox.json(&["show", "p", "--json"]);
    assert_eq!(shown["runner"], "amp", "runner removal silently rewrote prompt pin");
    assert!(!runner_names(&sandbox.json(&["runner", "list", "--json"])).contains(&"amp"));
}

#[test]
fn test_runner_remove_raw_row_is_targeted_and_requires_yes_noninteractively() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \"good\", argv = [\"good\", \"{{prompt}}\"] }, ",
        "{ name = \"broken\", argv = [\"broken\"] }, ",
        "\"untouched\"",
        "]\n",
    ));
    let refused = sandbox.run(&["runner", "remove", "--row", "1", "--no-input"]);
    assert_eq!(refused.status.code(), Some(2), "{}", combined(&refused));
    assert!(combined(&refused).contains("pass --yes"), "{}", combined(&refused));
    assert_eq!(sandbox.json(&["runner", "list", "--all", "--json"]).as_array().unwrap().len(), 3);

    let removed = sandbox.run(&["runner", "remove", "--row", "1", "--yes"]);
    assert_eq!(removed.status.code(), Some(0), "{}", combined(&removed));
    let shown = combined(&removed);
    assert!(shown.contains("Malformed runner row 1 removed"), "{shown}");
    assert!(!shown.contains("Runner broken removed"), "{shown}");
    let rows = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["name"], "good");
    assert_eq!(rows[1]["descriptor"], "untouched");

    let unknown = sandbox.run(&["runner", "remove", "--row", "9", "--yes"]);
    assert_eq!(unknown.status.code(), Some(1), "{}", combined(&unknown));
    assert!(combined(&unknown).contains("runner list --all"), "{}", combined(&unknown));
}

#[test]
fn test_runner_remove_raw_duplicate_has_no_false_pin_warning_or_key_removed_claim() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \"same\", argv = [\"first\", \"{{prompt}}\"] }, ",
        "{ name = \"same\", argv = [\"second\", \"{{prompt}}\"] }",
        "]\n",
    ));
    sandbox.add_prompt_pinned("p", "same");
    let output = sandbox.run(&["runner", "remove", "--row", "1", "--yes"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(!shown.contains("pins this runner"), "{shown}");
    assert!(!shown.contains("Runner same removed"), "{shown}");
    assert!(shown.contains("Malformed runner row 1 removed"), "{shown}");
    let runners = sandbox.json(&["runner", "list", "--json"]);
    assert_eq!(runners, json!([{"name":"same","argv":["first","{{prompt}}"]}]));
    assert_eq!(sandbox.json(&["show", "p", "--json"])["runner"], "same");
}

#[test]
fn test_runner_remove_raw_valid_row_requires_stable_name_path() {
    let sandbox = Sandbox::new();
    let original = concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \"same\", argv = [\"first\", \"{{prompt}}\"] }, ",
        "{ name = \"same\", argv = [\"second\", \"{{prompt}}\"] }",
        "]\n",
    );
    sandbox.write_config(original);
    let output = sandbox.run(&["runner", "remove", "--row", "0", "--yes"]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("skit runner remove \"same\""), "{}", flat(&output));
    assert_eq!(sandbox.config_text(), original, "valid row removal changed config despite refusal");
}

#[test]
fn test_runner_remove_container_repairs_only_targeted_prompt_value() {
    let sandbox = Sandbox::new();
    sandbox.write_config("language = \"zh-TW\"\nprompt = \"garbage\"\n");
    let inspected = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert!(inspected[0]["row"].is_null());
    assert_eq!(inspected[0]["reason"], "prompt-section-not-table");
    let output = sandbox.run(&["runner", "remove", "--row", "container", "--yes"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("Malformed prompt runner container removed"), "{shown}");
    assert!(!shown.contains("Runner container removed"), "{shown}");
    let text = sandbox.config_text();
    assert!(text.contains("language = \"zh-TW\""), "unrelated config was lost: {text}");
    assert!(text.contains("runners_seeded = true"), "repaired prompt marker missing: {text}");
    assert!(text.contains("runners = []"), "repaired prompt runner list not empty: {text}");
}
