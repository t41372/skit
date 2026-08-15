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

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_prompt(&self, name: &str, body: &str, runner: Option<&str>) {
        let source = self.source(&format!("{name}.prompt.md"), body);
        let mut args = vec![
            "add",
            source.to_str().unwrap(),
            "--name",
            name,
            "--no-input",
        ];
        if let Some(runner) = runner {
            args.extend(["--runner", runner]);
        }
        let output = self.run(&args);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn empty_runners(&self) {
        fs::create_dir_all(self.config.path()).unwrap();
        fs::write(
            self.config.path().join("config.toml"),
            "[prompt]\nrunners_seeded = true\nrunners = []\n",
        )
        .unwrap();
    }

    fn set_last_runner(&self, name: &str) {
        fs::create_dir_all(self.state.path()).unwrap();
        fs::write(
            self.state.path().join("prompt.toml"),
            format!("last_runner = {name:?}\n"),
        )
        .unwrap();
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

#[test]
fn test_run_prompt_no_input_without_pin_is_126() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "Do {{a}}\n", None);
    let output = sandbox.run(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(output.status.code(), Some(126), "{}", combined(&output));
    assert!(combined(&output).contains("No runner selected"), "{}", combined(&output));
}

#[test]
fn test_run_no_input_is_provably_unaffected_by_last_picked_state() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "Do {{a}}\n", None);
    sandbox.set_last_runner("claude");
    let output = sandbox.run(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(output.status.code(), Some(126), "{}", combined(&output));
    assert!(combined(&output).contains("No runner selected"), "{}", combined(&output));
}

#[test]
fn test_run_prompt_unknown_runner_is_126_listing_names() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "Do {{a}}\n", None);
    let output = sandbox.run(&[
        "run",
        "p",
        "--runner",
        "ghost",
        "--set",
        "a=1",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(126), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("ghost"), "{shown}");
    assert!(shown.contains("claude"), "{shown}");
}

#[test]
fn test_run_prompt_pinned_but_removed_runner_is_126() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "Do {{a}}\n", Some("claude"));
    sandbox.empty_runners();
    let output = sandbox.run(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(output.status.code(), Some(126), "{}", combined(&output));
    assert!(combined(&output).contains("claude"), "{}", combined(&output));
}

#[test]
fn test_run_unpinned_prompt_with_empty_runner_list_teaches_a_copyable_recovery() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "Do {{a}}\n", None);
    sandbox.empty_runners();
    let output = sandbox.run(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(output.status.code(), Some(126), "{}", combined(&output));
    let shown = flat(&output);
    assert!(shown.contains("No agents are configured"), "{shown}");
    assert!(
        shown.contains("skit runner add mycli -- mycli run {{prompt}}"),
        "{shown}"
    );
}

#[test]
fn test_run_runner_flag_on_non_prompt_is_usage_error() {
    let sandbox = Sandbox::new();
    let added = sandbox.run(&["add", "--cmd", "echo {m}", "--name", "cmd"]);
    assert_eq!(added.status.code(), Some(0), "{}", combined(&added));
    let output = sandbox.run(&[
        "run",
        "cmd",
        "--runner",
        "claude",
        "--set",
        "m=1",
        "--no-input",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("--runner only applies to prompt entries"),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_run_prompt_dry_run_prints_the_resolved_argv() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "Say {{a}}!\n", None);
    let output = sandbox.run(&[
        "run",
        "p",
        "--runner",
        "claude",
        "--set",
        "a=hello world",
        "--no-input",
        "--dry-run",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("claude"), "{shown}");
    assert!(shown.contains("hello world"), "{shown}");
}

#[test]
fn test_run_prompt_dry_run_missing_body_is_127_before_output() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "Say it\n", Some("claude"));
    fs::remove_file(sandbox.data.path().join("scripts/p/prompt.md")).unwrap();
    let output = sandbox.run(&["run", "p", "--no-input", "--dry-run"]);
    assert_eq!(output.status.code(), Some(127), "{}", combined(&output));
    let shown = combined(&output);
    assert!(shown.contains("doesn't exist"), "{shown}");
    assert!(!shown.contains('→'), "missing body leaked a launch line: {shown}");
}

#[test]
fn test_overlong_prompt_refuses_before_normal_transparency() {
    let sandbox = Sandbox::new();
    let marker = "MUST-NOT-REACH-SCROLLBACK";
    sandbox.add_prompt("p", &format!("{marker}{}", "x".repeat(100_100)), Some("claude"));
    let output = sandbox.run(&["run", "p", "--no-input"]);
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    let shown = combined(&output);
    assert!(!shown.contains(marker), "overlong prompt leaked into output: {shown}");
    assert!(shown.contains("over this platform"), "{shown}");
}

#[test]
fn test_dry_run_refuses_nul_without_looking_up_agent_binary() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "before\0after", Some("claude"));
    let output = sandbox
        .command()
        .env("PATH", "")
        .args(["run", "p", "--no-input", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    assert!(combined(&output).contains("NUL"), "{}", combined(&output));
}

#[test]
fn test_dry_run_refuses_overlong_prompt_without_printing_it() {
    let sandbox = Sandbox::new();
    let marker = "DRY-RUN-TOO-LONG";
    sandbox.add_prompt("p", &format!("{marker}{}", "x".repeat(100_100)), Some("claude"));
    let output = sandbox.run(&["run", "p", "--no-input", "--dry-run"]);
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    let shown = combined(&output);
    assert!(!shown.contains(marker), "overlong dry-run leaked prompt: {shown}");
    assert!(shown.contains("over this platform"), "{shown}");
}

#[test]
fn test_prompt_extra_agent_args_do_not_fill_required_placeholders() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("p", "Do {{a}}\n", Some("claude"));
    let output = sandbox.run(&["run", "p", "--no-input", "--", "--model", "opus"]);
    assert_eq!(output.status.code(), Some(125), "{}", combined(&output));
    assert!(combined(&output).contains("a is required"), "{}", combined(&output));
}

#[test]
fn test_run_prompt_secret_placeholder_masked_in_dry_run() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("sec", "Use {{api_key}}\n", None);
    let output = sandbox.run(&[
        "run",
        "sec",
        "--runner",
        "claude",
        "--set",
        "api_key=hunter2",
        "--no-input",
        "--dry-run",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(!shown.contains("hunter2"), "secret leaked in dry run: {shown}");
    assert!(shown.contains("•••"), "{shown}");
    assert!(!shown.contains("receives"), "dry-run emitted delivery warning: {shown}");
}
