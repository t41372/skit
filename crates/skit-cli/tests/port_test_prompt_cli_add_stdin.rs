use std::{fs, path::PathBuf, process::Output};

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

    fn run_stdin(&self, args: &[&str], input: &str) -> Output {
        self.command()
            .args(args)
            .write_stdin(input)
            .output()
            .expect("run skit with stdin")
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).expect("write source");
        path
    }

    fn show_json(&self, selector: &str) -> Value {
        let output = self.run(&["show", selector, "--json"]);
        assert!(output.status.success(), "{}", combined(&output));
        serde_json::from_slice(&output.stdout).expect("show json")
    }

    fn meta(&self, slug: &str) -> String {
        fs::read_to_string(self.data.path().join("scripts").join(slug).join("meta.toml"))
            .expect("meta.toml")
    }

    fn prompt_bytes(&self, slug: &str) -> Vec<u8> {
        fs::read(self.data.path().join("scripts").join(slug).join("prompt.md"))
            .expect("prompt body")
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", combined(output));
}

fn assert_success(output: &Output) {
    assert_code(output, 0);
}

#[test]
fn test_add_prompt_file_no_input_manages_everything() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "p.prompt.md",
        "# Review\n\nCheck {{target}} for {{focus}}\n",
    );
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--no-input"]);
    assert_success(&output);
    let shown = combined(&output);
    let view = sandbox.show_json("p");
    assert_eq!(view["kind"], "prompt");
    assert_eq!(
        view["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["target", "focus"]
    );
    assert!(view["runner"].is_null(), "{view}");
    assert!(shown.contains("Managed parameters: target, focus"), "{shown}");
}

#[test]
fn test_add_prompt_secret_summary_states_both_sides_of_boundary() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("p.prompt.md", "Use {{api_key}}\n");
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--no-input"]);
    assert_success(&output);
    let shown = combined(&output);
    assert!(shown.contains("never saved by skit: api_key"), "{shown}");
    assert!(
        shown.contains("selected agent receives those values as plaintext"),
        "{shown}"
    );
    assert!(shown.contains("may log or sync them"), "{shown}");
}

#[test]
fn test_add_prompt_interactive_tick_subset_and_runner_pick() {
    // The frozen test explicitly pins the non-TTY fallback despite its historical name:
    // non-interactive add manages every prompt placeholder.
    let sandbox = Sandbox::new();
    let source = sandbox.source("p.prompt.md", "{{a}} {{b}} {{c}}\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "-n",
        "picky",
        "--no-input",
    ]);
    assert_success(&output);
    let view = sandbox.show_json("picky");
    assert_eq!(
        view["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
}

#[test]
fn test_add_prompt_runner_flag_non_interactive() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.state.path()).unwrap();
    fs::write(
        sandbox.state.path().join("prompt.toml"),
        "last_runner = \"opencode\"\n",
    )
    .unwrap();
    let source = sandbox.source("p.prompt.md", "{{a}}\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "-n",
        "auto",
        "--runner",
        " claude ",
        "--no-input",
    ]);
    assert_success(&output);
    let view = sandbox.show_json("auto");
    assert_eq!(view["runner"], "claude");
    // An add-time pin is configuration on the entry, not a picker action. The unrelated
    // prompt-selection state must not be rewritten as a side effect.
    assert_eq!(
        fs::read_to_string(sandbox.state.path().join("prompt.toml")).unwrap(),
        "last_runner = \"opencode\"\n"
    );
}

#[test]
fn test_add_prompt_from_stdin_needs_a_name() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(&["add", "-", "--prompt"], "body {{x}}\n");
    assert_code(&output, 2);
    assert!(combined(&output).contains("--name"), "{}", combined(&output));
    assert!(!sandbox.data.path().join("scripts").exists());
}

#[test]
fn test_add_prompt_from_stdin() {
    let sandbox = Sandbox::new();
    let body = "Summarize {{url}} briefly.\n";
    let output = sandbox.run_stdin(
        &["add", "-", "--prompt", "-n", "clip", "--runner", "amp"],
        body,
    );
    assert_success(&output);
    let view = sandbox.show_json("clip");
    assert_eq!(view["kind"], "prompt");
    assert_eq!(view["runner"], "amp");
    assert_eq!(
        view["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["url"]
    );
    assert_eq!(sandbox.prompt_bytes("clip"), body.as_bytes());
}

#[test]
fn test_add_kind_prompt_from_stdin_uses_the_prompt_contract() {
    let sandbox = Sandbox::new();
    let body = "Keep {{url}} literal.\r\n";
    let output = sandbox.run_stdin(
        &[
            "add",
            "-",
            "--kind",
            "prompt",
            "-n",
            "kind-clip",
            "--runner",
            "amp",
            "--no-interpolate",
        ],
        body,
    );
    assert_success(&output);
    let view = sandbox.show_json("kind-clip");
    assert_eq!(view["kind"], "prompt");
    assert_eq!(view["runner"], "amp");
    assert_eq!(view["interpolate"], false);
    assert_eq!(view["workdir"], "invoke");
    assert_eq!(view["fields"], serde_json::json!([]));
    assert_eq!(sandbox.prompt_bytes("kind-clip"), body.as_bytes());
}

#[test]
fn test_add_prompt_from_stdin_empty_body() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(&["add", "-", "--prompt", "-n", "e"], "  \n");
    assert_code(&output, 1);
    assert!(
        combined(&output).contains("Nothing arrived on stdin"),
        "{}",
        combined(&output)
    );
    assert!(!sandbox.data.path().join("scripts/e").exists());
}

#[test]
fn test_add_prompt_editor_lane_routes_to_stdin_when_not_interactive() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &["add", "--prompt", "-n", "drafted"],
        "Draft {{a}}\n",
    );
    assert_success(&output);
    let view = sandbox.show_json("drafted");
    assert_eq!(
        view["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a"]
    );
}

#[test]
fn test_add_prompt_ref_mode_keeps_original_and_pins_invoke() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("p.prompt.md", "Ref {{x}}\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--ref",
        "--no-input",
    ]);
    assert_success(&output);
    let meta = sandbox.meta("p");
    assert!(meta.contains("mode = \"reference\""), "{meta}");
    assert!(meta.contains("workdir = \"invoke\""), "{meta}");
    assert!(meta.contains(&source.display().to_string()), "{meta}");
    assert!(!sandbox.data.path().join("scripts/p/prompt.md").exists());
}

#[test]
fn test_add_prompt_no_path_with_ref_is_refused() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &["add", "--prompt", "--ref", "-n", "x"],
        "body\n",
    );
    assert_code(&output, 2);
    assert!(!sandbox.data.path().join("scripts/x").exists());
}
