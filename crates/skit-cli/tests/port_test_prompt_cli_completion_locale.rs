use std::{fs, process::Output};

use assert_cmd::Command;
use serde_json::Value;
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

    fn command(&self, lang: &str) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", lang)
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path());
        command
    }

    fn write_config(&self, text: &str) {
        fs::create_dir_all(self.config.path()).unwrap();
        fs::write(self.config.path().join("config.toml"), text).unwrap();
    }

    fn complete_runner(&self, prefix: &str) -> Output {
        self.command("en")
            .env("COMPLETE", "fish")
            .args(["--", "skit", "run", "ghost", "--runner", prefix])
            .output()
            .unwrap()
    }
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn test_complete_runner_names() {
    let sandbox = Sandbox::new();
    let matches = sandbox.complete_runner("cl");
    assert_eq!(matches.status.code(), Some(0), "{}", combined(&matches));
    let stdout = String::from_utf8(matches.stdout).unwrap();
    assert!(stdout.lines().any(|line| line.split('\t').next() == Some("claude")), "{stdout}");

    let none = sandbox.complete_runner("zz");
    assert_eq!(none.status.code(), Some(0), "{}", combined(&none));
    assert!(none.stdout.is_empty(), "unknown prefix produced completion: {}", String::from_utf8_lossy(&none.stdout));

    sandbox.write_config("prompt = \"malformed\"\n");
    let broken = sandbox.complete_runner("");
    assert_eq!(broken.status.code(), Some(0), "completion crashed on malformed config: {}", combined(&broken));
    assert!(broken.stdout.is_empty(), "malformed config must degrade to no runner candidates");
}

#[test]
fn test_runner_list_all_preserves_anonymous_argv_and_localizes_human_status() {
    let sandbox = Sandbox::new();
    sandbox.write_config(concat!(
        "[prompt]\n",
        "runners_seeded = true\n",
        "runners = [",
        "{ name = \"   \", argv = [\"valuable-agent\", \"--model\", \"x\", \"{{prompt}}\"] }, ",
        "{ name = \"broken\", argv = [\"broken\"] }",
        "]\n",
    ));

    let json_output = sandbox.command("en").args(["runner", "list", "--all", "--json"]).output().unwrap();
    assert_eq!(json_output.status.code(), Some(0), "{}", combined(&json_output));
    let payload: Value = serde_json::from_slice(&json_output.stdout).unwrap();
    assert!(payload[0]["name"].is_null(), "{payload}");
    assert_eq!(payload[0]["argv"], serde_json::json!(["valuable-agent", "--model", "x", "{{prompt}}"]));
    assert_eq!(payload[0]["reason"], "name");

    let human = sandbox.command("x-pseudo").args(["runner", "list", "--all"]).output().unwrap();
    assert_eq!(human.status.code(), Some(0), "{}", combined(&human));
    let flat = combined(&human).split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("valuable-agent"), "{flat}");
    assert!(flat.contains("--model x"), "{flat}");
    assert!(flat.contains("{{prompt}}"), "{flat}");
    assert!(flat.contains('⟦'), "pseudo locale did not reach human status: {flat}");
    assert!(!flat.contains("prompt-slot-count"), "machine reason leaked into human status: {flat}");
}
