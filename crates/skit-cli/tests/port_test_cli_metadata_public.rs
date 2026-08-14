use std::{fs, path::PathBuf, process::Output};

use assert_cmd::Command;
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
            data: TempDir::new().expect("data"),
            state: TempDir::new().expect("state"),
            config: TempDir::new().expect("config"),
            home: TempDir::new().expect("home"),
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
        self.command().args(args).output().expect("run skit")
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).expect("source");
        path
    }

    fn deps_json(&self, selector: &str) -> serde_json::Value {
        let output = self.run(&["deps", selector, "--json"]);
        assert_success(&output);
        serde_json::from_slice(&output.stdout).expect("deps json")
    }

    fn show_json(&self, selector: &str) -> serde_json::Value {
        let output = self.run(&["show", selector, "--json"]);
        assert_success(&output);
        serde_json::from_slice(&output.stdout).expect("show json")
    }

    fn write_state(&self, slug: &str, body: &str) {
        let root = self.state.path().join("values");
        fs::create_dir_all(&root).expect("values dir");
        fs::write(root.join(format!("{slug}.toml")), body).expect("state");
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", combined(output));
}

#[test]
fn test_resolve_metadata_existing_block_not_asked() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "owned.py",
        concat!(
            "# /// script\n",
            "# dependencies = [\"requests\"]\n",
            "# ///\n",
            "print(1)\n",
        ),
    );
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--name", "owned"]);
    assert_success(&output);
    assert_eq!(
        sandbox.deps_json("owned")["dependencies"],
        serde_json::json!(["requests"])
    );
}

#[test]
fn test_resolve_metadata_explicit_opts() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "job",
        "--dep",
        "requests",
        "--dep",
        "rich",
        "--python",
        ">=3.11",
        "--no-input",
    ]);
    assert_success(&output);
    let deps = sandbox.deps_json("job");
    assert_eq!(deps["dependencies"], serde_json::json!(["requests", "rich"]));
    assert_eq!(deps["requires_python"], ">=3.11");
}

#[test]
fn test_resolve_metadata_explicit_opts_strips_and_drops_empties() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "job",
        "--dep",
        "",
        "--dep",
        "  requests  ",
        "--dep",
        "   ",
        "--python",
        "   ",
        "--no-input",
    ]);
    assert_success(&output);
    let deps = sandbox.deps_json("job");
    assert_eq!(deps["dependencies"], serde_json::json!(["requests"]));
    assert_eq!(deps["requires_python"], "");
}

#[test]
fn test_resolve_metadata_no_suggestions() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("plain.py", "print(1)\n");
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--name", "plain"]);
    assert_success(&output);
    assert_eq!(sandbox.deps_json("plain")["dependencies"], serde_json::json!([]));
}

#[test]
fn test_resolve_metadata_non_interactive_uses_suggestions() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("suggest.py", "import requests\nprint(requests)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "suggest",
        "--no-input",
    ]);
    assert_success(&output);
    assert_eq!(
        sandbox.deps_json("suggest")["dependencies"],
        serde_json::json!(["requests"])
    );
}

#[test]
fn test_prompt_identity_non_interactive_passes_through() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("worker.py", "print(1)\n");
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--no-input"]);
    assert_success(&output);
    let shown = sandbox.show_json("worker");
    assert_eq!(shown["name"], "worker");
    assert_eq!(shown["description"], "");
}

#[test]
fn test_prompt_identity_explicit_values_skip_prompts() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("s.py", "print(1)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "given",
        "--description",
        "a desc",
    ]);
    assert_success(&output);
    let shown = sandbox.show_json("given");
    assert_eq!(shown["name"], "given");
    assert_eq!(shown["description"], "a desc");
}

#[test]
fn test_onboard_params_no_candidates() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("x.py", "print(1)\n");
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--name", "x"]);
    assert_success(&output);
    let stored = fs::read_to_string(sandbox.data.path().join("scripts/x/script.py")).expect("stored");
    assert!(!stored.contains("[tool.skit]"), "{stored}");
}

#[test]
fn test_onboard_params_non_interactive_returns_empty() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("x.py", "CITY = \"Taipei\"\nprint(CITY)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "x",
        "--no-input",
    ]);
    assert_success(&output);
    let stored = fs::read_to_string(sandbox.data.path().join("scripts/x/script.py")).expect("stored");
    assert!(!stored.contains("[tool.skit]"), "{stored}");
}

#[test]
fn test_command_placeholders_prefill_from_last() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.run(&["add", "--cmd", "echo {msg}", "--name", "e"]));
    sandbox.write_state("e", "[values]\nmsg = \"remembered\"\n");
    let output = sandbox.run(&["run", "e", "--no-input"]);
    assert_success(&output);
    assert!(combined(&output).contains("remembered"), "{}", combined(&output));
}

#[test]
fn test_command_without_placeholders_has_no_fields() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.run(&["add", "--cmd", "echo hi", "--name", "e"]));
    let params = sandbox.run(&["params", "e", "--json"]);
    assert_success(&params);
    let json: serde_json::Value = serde_json::from_slice(&params.stdout).expect("params json");
    assert_eq!(json["parameters"], serde_json::json!([]));
    let run = sandbox.run(&["run", "e", "--no-input"]);
    assert_success(&run);
    assert!(combined(&run).contains("hi"), "{}", combined(&run));
}
