//! CLI and interactive ports from Python `tests/test_agent_install.py` at `main@206f9ef`.
//!
//! The one Python-only packaging-fault test is deliberately absent here: Rust embeds the skill with
//! `include_bytes!`, so a missing packaged resource is a compile-time error rather than a runtime
//! branch. The completeness guard records that architecture-closed contract separately. Every other
//! CLI/picker contract below crosses the real `skit` process boundary.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Output,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

const SKILL_MARKER: &str = "---\nname: skit\n";

struct Sandbox {
    _root: TempDir,
    home: PathBuf,
    cwd: PathBuf,
    data: PathBuf,
    state: PathBuf,
    config: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("project");
        let data = root.path().join("data");
        let state = root.path().join("state");
        let config = root.path().join("config");
        for path in [&home, &cwd, &data, &state, &config] {
            fs::create_dir_all(path).unwrap();
        }
        Self {
            _root: root,
            home,
            cwd,
            data,
            state,
            config,
        }
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_LANG", "en")
            .env("SKIT_DATA_DIR", &self.data)
            .env("SKIT_STATE_DIR", &self.state)
            .env("SKIT_CONFIG_DIR", &self.config)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .current_dir(&self.cwd);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn pty_command(sandbox: &Sandbox, args: &[&str]) -> CommandBuilder {
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    for arg in args {
        command.arg(arg);
    }
    command.cwd(&sandbox.cwd);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", &sandbox.data);
    command.env("SKIT_STATE_DIR", &sandbox.state);
    command.env("SKIT_CONFIG_DIR", &sandbox.config);
    command.env("HOME", &sandbox.home);
    command.env("USERPROFILE", &sandbox.home);
    command.env("XDG_CONFIG_HOME", sandbox.home.join("xdg-config"));
    command.env("XDG_DATA_HOME", sandbox.home.join("xdg-data"));
    command.env("XDG_STATE_HOME", sandbox.home.join("xdg-state"));
    command
}

fn run_pty(sandbox: &Sandbox, args: &[&str], input: &[u8]) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 140,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut child = pair
        .slave
        .spawn_command(pty_command(sandbox, args))
        .unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(120));
    if !input.is_empty() {
        writer.write_all(input).unwrap();
        writer.flush().unwrap();
    }

    let status = child.wait().unwrap();
    drop(writer);
    let text = String::from_utf8_lossy(&drain.join().unwrap())
        .replace("\r\n", "\n")
        .replace('\r', "");
    (status.exit_code(), text)
}

fn installed(destination: &Path) -> PathBuf {
    destination.join("skit/SKILL.md")
}

#[test]
fn test_skill_text_is_the_bundled_skill() {
    let sandbox = Sandbox::new();
    let destination = sandbox.cwd.join("skills");
    let output = sandbox.run(&[
        "agent",
        "install",
        "--to",
        destination.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", output_text(&output));

    let text = fs::read_to_string(installed(&destination)).unwrap();
    assert!(text.starts_with(SKILL_MARKER));
}

#[test]
fn test_cli_install_to_explicit_dir() {
    let sandbox = Sandbox::new();
    let destination = sandbox.cwd.join("anywhere");
    let output = sandbox.run(&[
        "agent",
        "install",
        "--to",
        destination.to_str().unwrap(),
    ]);

    assert_eq!(code(&output), 0, "{}", output_text(&output));
    let target = installed(&destination);
    assert_eq!(
        fs::read(&target).unwrap(),
        include_bytes!("../../../skills/skit/SKILL.md")
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == format!("Installed the skit Agent Skill: {}", target.display())),
        "explicit install lost the Python user-visible success contract:\n{}",
        output_text(&output)
    );
}

#[test]
fn test_cli_install_to_a_file_fails_cleanly() {
    let sandbox = Sandbox::new();
    let blocker = sandbox.cwd.join("afile");
    fs::write(&blocker, "not a directory").unwrap();

    let output = sandbox.run(&[
        "agent",
        "install",
        "--to",
        blocker.to_str().unwrap(),
    ]);
    let text = output_text(&output);

    assert_eq!(code(&output), 1, "{text}");
    assert!(
        text.lines()
            .any(|line| line.starts_with("Could not write the skill there: ")),
        "{text}"
    );
    assert!(!text.contains("Traceback"), "{text}");
    assert_eq!(fs::read(&blocker).unwrap(), b"not a directory");
}

#[test]
fn test_cli_install_to_with_project_is_a_conflict() {
    let sandbox = Sandbox::new();
    let destination = sandbox.cwd.join("x");
    let output = sandbox.run(&[
        "agent",
        "install",
        "--to",
        destination.to_str().unwrap(),
        "--project",
    ]);

    assert_eq!(code(&output), 2, "{}", output_text(&output));
    assert!(!destination.exists());
}

#[test]
fn test_cli_install_to_expands_tilde() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["agent", "install", "--to", "~/myskills"]);

    assert_eq!(code(&output), 0, "{}", output_text(&output));
    assert!(sandbox.home.join("myskills/skit/SKILL.md").is_file());
}

#[test]
fn test_cli_install_named_target_user_scope() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["agent", "install", "claude"]);

    assert_eq!(code(&output), 0, "{}", output_text(&output));
    assert!(
        sandbox
            .home
            .join(".claude/skills/skit/SKILL.md")
            .is_file()
    );
    assert!(!sandbox.cwd.join(".claude/skills/skit/SKILL.md").exists());
}

#[test]
fn test_cli_install_named_target_project_scope() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["agent", "install", "codex", "--project"]);

    assert_eq!(code(&output), 0, "{}", output_text(&output));
    assert!(sandbox.cwd.join(".codex/skills/skit/SKILL.md").is_file());
    assert!(!sandbox.home.join(".codex/skills/skit/SKILL.md").exists());
}

#[test]
fn test_cli_install_unknown_target_exits_2() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["agent", "install", "cursor"]);
    let text = output_text(&output);

    assert_eq!(code(&output), 2, "{text}");
    assert!(text.contains("cursor"), "{text}");
    assert!(!sandbox.home.join(".claude").exists());
    assert!(!sandbox.home.join(".codex").exists());
    assert!(!sandbox.cwd.join(".agents").exists());
}

#[test]
fn test_cli_install_target_and_to_conflict_exits_2() {
    let sandbox = Sandbox::new();
    let destination = sandbox.cwd.join("x");
    let output = sandbox.run(&[
        "agent",
        "install",
        "claude",
        "--to",
        destination.to_str().unwrap(),
    ]);

    assert_eq!(code(&output), 2, "{}", output_text(&output));
    assert!(!destination.exists());
    assert!(!sandbox.home.join(".claude/skills").exists());
}

#[test]
fn test_cli_bare_non_interactive_refuses() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["agent", "install"]);

    assert_eq!(code(&output), 2, "{}", output_text(&output));
    assert!(fs::read_dir(&sandbox.home).unwrap().next().is_none());
    assert!(!sandbox.cwd.join(".claude").exists());
    assert!(!sandbox.cwd.join(".codex").exists());
    assert!(!sandbox.cwd.join(".agents").exists());
}

#[test]
fn test_cli_bare_interactive_no_candidates_exits_1() {
    let sandbox = Sandbox::new();
    let (exit, text) = run_pty(&sandbox, &["agent", "install"], b"");

    assert_eq!(exit, 1, "{text}");
    assert!(text.contains("--to"), "{text}");
    assert!(fs::read_dir(&sandbox.home).unwrap().next().is_none());
}

#[test]
fn test_cli_bare_interactive_picks_and_confirms() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.home.join(".claude")).unwrap();
    fs::create_dir_all(sandbox.cwd.join(".agents")).unwrap();

    let (exit, text) = run_pty(&sandbox, &["agent", "install"], b"2\ny\n");

    assert_eq!(exit, 0, "{text}");
    assert!(sandbox.cwd.join(".agents/skills/skit/SKILL.md").is_file());
    assert!(!sandbox.home.join(".claude/skills").exists());
}

#[test]
fn test_cli_bare_interactive_backing_out_writes_nothing() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.home.join(".claude")).unwrap();

    let (exit, text) = run_pty(&sandbox, &["agent", "install"], b"\nn\n");

    assert_eq!(exit, 0, "{text}");
    assert!(text.contains("Cancelled — nothing was written."), "{text}");
    assert!(!sandbox.home.join(".claude/skills").exists());
}

#[test]
fn test_agent_pick_target_renders_the_menu_exactly() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.home.join(".claude")).unwrap();
    fs::create_dir_all(sandbox.cwd.join(".agents")).unwrap();

    let (exit, text) = run_pty(&sandbox, &["agent", "install"], b"2\nn\n");

    assert_eq!(exit, 0, "{text}");
    let expected_menu = format!(
        "Agent directories on this machine:\n  1. claude (user)  →  {}\n  2. agents (project)  →  {}\n",
        sandbox.home.join(".claude/skills").display(),
        sandbox.cwd.join(".agents/skills").display(),
    );
    assert!(
        text.contains(&expected_menu),
        "interactive target menu drifted:\nexpected contiguous block:\n{expected_menu}\nactual:\n{text}"
    );
    assert!(text.contains("Install where? [1-2] (1): "), "{text}");
    assert!(
        text.contains(&format!(
            "Write the skill into {}? [Y/n] ",
            sandbox.cwd.join(".agents/skills").display()
        )),
        "{text}"
    );
}

#[test]
fn test_agent_pick_target_backing_out_returns_none() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.home.join(".claude")).unwrap();
    fs::create_dir_all(sandbox.cwd.join(".agents")).unwrap();

    let (exit, text) = run_pty(&sandbox, &["agent", "install"], b"2\nn\n");

    assert_eq!(exit, 0, "{text}");
    assert!(text.contains("Cancelled — nothing was written."), "{text}");
    assert!(!sandbox.home.join(".claude/skills").exists());
    assert!(!sandbox.cwd.join(".agents/skills").exists());
}
