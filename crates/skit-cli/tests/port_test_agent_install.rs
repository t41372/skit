//! Mechanical port of the Python oracle module `tests/test_agent_install.py`
//! (`origin/main@206f9ef`): "`skit agent install` — consent-first installer for the bundled
//! Agent Skill." Each `#[test]` keeps its Python `def test_*` name and the Python "WHY" comment
//! above it, and drives the real public Rust surface.
//!
//! Concept mapping used throughout:
//! - Python `agentskill.skill_text()` -> the bundled `skills/skit/SKILL.md`, which the CLI embeds
//!   with `include_bytes!` (`crates/skit-cli/src/cli.rs:5171`). `include_str!` reads the identical
//!   byte source here, so the marker assertion pins the same artifact.
//! - Python `agentskill.detect_targets(home, cwd)` -> `skit_application::detect_agent_targets`.
//! - Python `agentskill.named_target(name, project, home, cwd)` -> the PRIVATE
//!   `skit_application::agent_skill::named_target`, folded into the public `plan_agent_install`.
//!   `AgentInstallPlan::Ready` exposes only `skills_dir`, which encodes both the name (marker dir)
//!   and the scope (home vs cwd root); so the oracle's separate name/scope assertions are folded
//!   into the `skills_dir` equality. An unknown name returns `AgentInstallError::UnknownTarget`
//!   where the oracle returns `None`.
//! - Python `agentskill.install_into(skills_dir, text)` -> `skit_store::FileAgentSkillStore::install`.
//! - Python CliRunner over `cli.app ["agent","install",…]` -> the real `skit` binary, driven with
//!   `assert_cmd` for the non-interactive lanes and a `portable-pty` for the interactive picker
//!   (the CLI decides interactivity with `is_terminal()` on both standard streams, so a real pty is
//!   the only place the bare-mode picker is reachable — mirroring `terminal_pty.rs`).
//!
//! Buckets:
//! - Bucket 1 (real asserting tests): the headless helpers plus every non-interactive CLI lane and
//!   the three pty-driven bare-mode lanes.
//! - Bucket 2 (white-box / unmapped): `test_cli_install_broken_package_fails_loudly` and the two
//!   `test_agent_pick_target_*` tests are `#[ignore]`d compiling stubs — they pin Python-private
//!   internals (a monkeypatched `skill_text`, the Rich `Prompt`/`Confirm` internals of the private
//!   `_agent_pick_target`) that have no observable Rust equivalent. Their WHY comments name the
//!   real Rust seam and the sibling test that covers the observable behavior.
//! - Divergences: `test_cli_install_to_explicit_dir` and `test_cli_install_to_a_file_fails_cleanly`
//!   keep their full asserting bodies but are `#[ignore]`d as FAILING CONTRACTs — the Rust success
//!   and write-error messages diverge from the oracle (see each attribute for the evidence).

use std::{
    collections::BTreeSet,
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Output,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::{
    AgentInstallError, AgentInstallPlan, AgentInstallRequest, AgentRoots, AgentScope,
    detect_agent_targets, plan_agent_install,
};
use skit_store::FileAgentSkillStore;
use tempfile::TempDir;

/// Python `SKILL_MARKER`.
const SKILL_MARKER: &str = "---\nname: skit\n";

/// The bundled Agent Skill the CLI embeds — the Rust twin of `agentskill.skill_text()`.
const BUNDLED_SKILL: &str = include_str!("../../../skills/skit/SKILL.md");

// --------------------------------------------------------------------------
// Sandbox for the non-interactive CLI lanes (real `skit` binary via assert_cmd)
// --------------------------------------------------------------------------

/// Fresh, isolated environment for one `skit agent install` invocation.
///
/// HOME and USERPROFILE are pinned to a temp so a named/user-scope install can never touch the
/// real `~/.claude`, matching the oracle's `fake_home` fixture; the three `SKIT_*_DIR` variables
/// keep every write inside the sandbox; `project` is the working directory (`fake_cwd`).
struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    project: TempDir,
    scratch: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            project: TempDir::new().unwrap(),
            scratch: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .current_dir(self.project.path());
        command
    }
}

/// Python `result.output` is the merged stdout+stderr stream; build the same for line assertions.
fn combined(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

fn tree_snapshot(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn visit(root: &Path, directory: &Path, output: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if path.is_dir() {
                output.push((relative, None));
                visit(root, &path, output);
            } else {
                output.push((relative, Some(fs::read(path).unwrap())));
            }
        }
    }

    let mut output = Vec::new();
    visit(root, root, &mut output);
    output.sort_by(|left, right| left.0.cmp(&right.0));
    output
}

fn sandbox_snapshot(sandbox: &Sandbox) -> [Vec<(PathBuf, Option<Vec<u8>>)>; 5] {
    [
        tree_snapshot(sandbox.data.path()),
        tree_snapshot(sandbox.state.path()),
        tree_snapshot(sandbox.config.path()),
        tree_snapshot(sandbox.home.path()),
        tree_snapshot(sandbox.project.path()),
    ]
}

/// Resolve one named target through the public plan and return its `skills_dir`.
///
/// The Rust `named_target` is private, so `plan_agent_install` is the seam; a named target always
/// resolves to `AgentInstallPlan::Ready`.
fn ready_skills_dir(name: &str, project: bool, roots: &AgentRoots) -> PathBuf {
    match plan_agent_install(
        &AgentInstallRequest {
            target: Some(name.to_owned()),
            directory: None,
            project,
            interactive: false,
        },
        roots,
        |_| false,
    )
    .unwrap()
    {
        AgentInstallPlan::Ready { skills_dir } => skills_dir,
        AgentInstallPlan::Choose { .. } => unreachable!("a named target resolves to Ready"),
    }
}

// --------------------------------------------------------------------------
// headless helpers (agentskill.py)
// --------------------------------------------------------------------------

#[test]
fn test_skill_text_is_the_bundled_skill() {
    assert!(BUNDLED_SKILL.starts_with(SKILL_MARKER));
}

#[test]
fn test_detect_targets_reports_only_existing_marker_dirs() {
    let home = PathBuf::from("/tmp/h");
    let cwd = PathBuf::from("/tmp/c");
    let existing: BTreeSet<PathBuf> = [home.join(".claude"), cwd.join(".agents")]
        .into_iter()
        .collect();
    let roots = AgentRoots {
        home: Some(home.clone()),
        cwd: cwd.clone(),
    };
    let found = detect_agent_targets(&roots, |path| existing.contains(path));
    assert_eq!(
        found
            .iter()
            .map(|target| (target.name.as_str(), target.scope))
            .collect::<Vec<_>>(),
        [
            ("claude", AgentScope::User),
            ("agents", AgentScope::Project)
        ]
    );
    assert_eq!(found[0].skills_dir(), home.join(".claude").join("skills"));
    assert_eq!(found[1].skills_dir(), cwd.join(".agents").join("skills"));
}

#[test]
fn test_detect_targets_empty_when_nothing_exists() {
    let roots = AgentRoots {
        home: Some(PathBuf::from("/tmp/h")),
        cwd: PathBuf::from("/tmp/c"),
    };
    assert!(detect_agent_targets(&roots, |_| false).is_empty());
}

#[test]
fn test_named_target_user_and_project_scopes() {
    // Oracle asserts name+scope+skills_dir; the Rust helper is private, so name and scope are
    // folded into the skills_dir equality (see the file header's mapping note).
    let roots = AgentRoots {
        home: Some(PathBuf::from("/tmp/h")),
        cwd: PathBuf::from("/tmp/c"),
    };
    assert_eq!(
        ready_skills_dir("claude", false, &roots),
        PathBuf::from("/tmp/h/.claude/skills")
    );
    assert_eq!(
        ready_skills_dir("codex", true, &roots),
        PathBuf::from("/tmp/c/.codex/skills")
    );
}

#[test]
fn test_named_target_agents_is_always_project_scoped() {
    let roots = AgentRoots {
        home: Some(PathBuf::from("/tmp/h")),
        cwd: PathBuf::from("/tmp/c"),
    };
    for project in [false, true] {
        // `agents` is a project-level convention, so it resolves to the project scope regardless
        // of the --project flag.
        assert_eq!(
            ready_skills_dir("agents", project, &roots),
            PathBuf::from("/tmp/c/.agents/skills")
        );
    }
}

#[test]
fn test_named_target_unknown_is_none() {
    // Oracle: named_target("cursor", …) is None for both scopes. Rust folds the None into
    // plan_agent_install returning AgentInstallError::UnknownTarget.
    let roots = AgentRoots {
        home: Some(PathBuf::from("/tmp/h")),
        cwd: PathBuf::from("/tmp/c"),
    };
    for project in [false, true] {
        let error = plan_agent_install(
            &AgentInstallRequest {
                target: Some("cursor".to_owned()),
                directory: None,
                project,
                interactive: false,
            },
            &roots,
            |_| false,
        )
        .unwrap_err();
        assert_eq!(
            error,
            AgentInstallError::UnknownTarget {
                name: "cursor".to_owned()
            }
        );
    }
}

#[test]
fn test_install_into_writes_and_upgrades() {
    let scratch = TempDir::new().unwrap();
    let skills_dir = scratch.path().join("skills");
    let out = FileAgentSkillStore
        .install(&skills_dir, BUNDLED_SKILL.as_bytes())
        .unwrap();
    assert_eq!(out, skills_dir.join("skit").join("SKILL.md"));
    assert_eq!(fs::read_to_string(&out).unwrap(), BUNDLED_SKILL);
    fs::write(&out, "stale").unwrap();
    let again = FileAgentSkillStore
        .install(&skills_dir, BUNDLED_SKILL.as_bytes())
        .unwrap();
    assert_eq!(again, out);
    assert_eq!(fs::read_to_string(&out).unwrap(), BUNDLED_SKILL); // reinstall = upgrade
}

// --------------------------------------------------------------------------
// CLI: explicit consent paths
// --------------------------------------------------------------------------

#[test]
fn test_cli_install_to_explicit_dir() {
    let sandbox = Sandbox::new();
    let dest = sandbox.scratch.path().join("anywhere");
    let assert = sandbox
        .command()
        .args(["agent", "install", "--to"])
        .arg(&dest)
        .assert()
        .success();
    let installed = dest.join("skit").join("SKILL.md");
    assert_eq!(fs::read_to_string(&installed).unwrap(), BUNDLED_SKILL);
    let expected = format!("Installed the skit Agent Skill: {}", installed.display());
    assert!(
        combined(assert.get_output())
            .lines()
            .any(|line| line == expected.as_str())
    );
}

#[test]
#[ignore = "UNMAPPED (bucket 2): the oracle monkeypatches agentskill.skill_text to raise FileNotFoundError and asserts the CLI surfaces it loudly, not as a write error (test_agent_install.py:126-138). The Rust CLI embeds skills/skit/SKILL.md with include_bytes! at compile time (cli.rs:5171), so there is no runtime skill_text() to fail — a strictly stronger guarantee. Nothing to observe."]
fn test_cli_install_broken_package_fails_loudly() {
    // A bundled skill missing from the package is a packaging bug. In Rust it is a compile-time
    // impossibility (the bytes are embedded), so this white-box failure mode has no analogue.
}

#[test]
fn test_cli_install_to_a_file_fails_cleanly() {
    let sandbox = Sandbox::new();
    let blocker = sandbox.scratch.path().join("afile");
    fs::write(&blocker, "not a directory").unwrap();
    let assert = sandbox
        .command()
        .args(["agent", "install", "--to"])
        .arg(&blocker)
        .assert()
        .code(1);
    let text = combined(assert.get_output());
    assert!(
        text.lines()
            .any(|line| line.starts_with("Could not write the skill there: "))
    );
    assert!(!text.contains("Traceback"));
}

#[test]
fn test_cli_install_to_with_project_is_a_conflict() {
    let sandbox = Sandbox::new();
    let dest = sandbox.scratch.path().join("x");
    sandbox
        .command()
        .args(["agent", "install", "--to"])
        .arg(&dest)
        .arg("--project")
        .assert()
        .code(2);
    assert!(!dest.exists());
}

#[test]
fn test_cli_install_to_expands_tilde() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["agent", "install", "--to", "~/myskills"])
        .assert()
        .success();
    assert!(sandbox.home.path().join("myskills/skit/SKILL.md").is_file());
}

#[test]
fn test_cli_install_named_target_user_scope() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["agent", "install", "claude"])
        .assert()
        .success();
    assert!(
        sandbox
            .home
            .path()
            .join(".claude/skills/skit/SKILL.md")
            .is_file()
    );
}

#[test]
fn test_cli_install_named_target_project_scope() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["agent", "install", "codex", "--project"])
        .assert()
        .success();
    assert!(
        sandbox
            .project
            .path()
            .join(".codex/skills/skit/SKILL.md")
            .is_file()
    );
}

#[test]
fn test_cli_install_unknown_target_exits_2() {
    let sandbox = Sandbox::new();
    let assert = sandbox
        .command()
        .args(["agent", "install", "cursor"])
        .assert()
        .code(2);
    assert!(combined(assert.get_output()).contains("cursor"));
    assert!(!sandbox.home.path().join(".claude").exists());
}

#[test]
fn test_cli_install_target_and_to_conflict_exits_2() {
    let sandbox = Sandbox::new();
    let dest = sandbox.scratch.path().join("x");
    sandbox
        .command()
        .args(["agent", "install", "claude", "--to"])
        .arg(&dest)
        .assert()
        .code(2);
    assert!(!dest.exists());
}

// --------------------------------------------------------------------------
// CLI: bare mode — never guess
// --------------------------------------------------------------------------

#[test]
fn test_cli_bare_non_interactive_refuses() {
    // assert_cmd's stdin is not a tty, so bare mode is non-interactive: it refuses (exit 2) rather
    // than guessing, and writes nothing anywhere.
    let sandbox = Sandbox::new();
    let before = sandbox_snapshot(&sandbox);
    sandbox
        .command()
        .args(["agent", "install"])
        .assert()
        .code(2);
    assert_eq!(sandbox_snapshot(&sandbox), before);
}

#[test]
fn test_cli_bare_interactive_no_candidates_exits_1() {
    // Interactive (pty) but no marker directories exist: exit 1, and the message steers to --to.
    let sandbox = Sandbox::new();
    let before = sandbox_snapshot(&sandbox);
    let (code, output) = run_agent_install_pty(&sandbox, "en", &[]);
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("--to"), "{output}");
    assert_eq!(sandbox_snapshot(&sandbox), before);
}

#[test]
fn test_cli_bare_interactive_picks_and_confirms() {
    let sandbox = Sandbox::new();
    fs::create_dir(sandbox.home.path().join(".claude")).unwrap();
    fs::create_dir(sandbox.project.path().join(".agents")).unwrap();
    let (code, output) = run_agent_install_pty(
        &sandbox,
        "en",
        &[("Install where?", b"2\n"), ("Write the skill into", b"y\n")],
    );
    assert_eq!(code, 0, "{output}");
    // The menu text is part of the contract a mouse-less user reads. `_agent_pick_target`'s exact
    // pin is white-box, so it is salvaged here as full-line assertions. The pty rewrites \n to
    // \r\n, so each line is matched on its own, never across a boundary.
    let claude_skills = sandbox.home.path().join(".claude").join("skills");
    let agents_skills = sandbox.project.path().join(".agents").join("skills");
    assert!(
        output.contains("Agent directories on this machine:"),
        "{output}"
    );
    let line_one = format!("  1. claude (user)  \u{2192}  {}", claude_skills.display());
    let line_two = format!(
        "  2. agents (project)  \u{2192}  {}",
        agents_skills.display()
    );
    assert!(output.contains(line_one.as_str()), "{output}");
    assert!(output.contains(line_two.as_str()), "{output}");
    assert!(output.contains("Install where?"), "{output}");
    let confirm = format!("Write the skill into {}?", agents_skills.display());
    assert!(output.contains(confirm.as_str()), "{output}");
    assert!(
        agents_skills.join("skit").join("SKILL.md").is_file(),
        "{output}"
    );
    assert!(!claude_skills.exists(), "{output}"); // only the picked one
}

#[test]
fn test_cli_bare_interactive_backing_out_writes_nothing() {
    let sandbox = Sandbox::new();
    fs::create_dir(sandbox.home.path().join(".claude")).unwrap();
    let before = sandbox_snapshot(&sandbox);
    let (code, output) = run_agent_install_pty(
        &sandbox,
        "en",
        &[("Install where?", b"1\n"), ("Write the skill into", b"n\n")],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("Cancelled \u{2014} nothing was written."),
        "{output}"
    );
    assert!(
        !sandbox.home.path().join(".claude").join("skills").exists(),
        "{output}"
    );
    assert_eq!(sandbox_snapshot(&sandbox), before);
}

#[test]
fn test_cli_bare_interactive_default_reprompt_and_eof_are_localized() {
    for (locale, install_prompt, invalid, confirm, cancelled, aborted) in [
        (
            "en",
            "Install where?",
            "Choose a number from 1 to 2.",
            "Write the skill into",
            "Cancelled — nothing was written.",
            "operation cancelled",
        ),
        (
            "zh-CN",
            "安装到哪里？",
            "请选择 1 到 2 之间的数字。",
            "将 Skill 写入",
            "已取消，未写入任何内容。",
            "操作已取消",
        ),
        (
            "zh-TW",
            "要安裝到哪裡？",
            "請選擇 1 到 2 之間的數字。",
            "要將 Skill 寫入",
            "已取消，未寫入任何內容。",
            "操作已取消",
        ),
    ] {
        let single = Sandbox::new();
        fs::create_dir(single.home.path().join(".claude")).unwrap();
        let data_before = tree_snapshot(single.data.path());
        let state_before = tree_snapshot(single.state.path());
        let config_before = tree_snapshot(single.config.path());
        let project_before = tree_snapshot(single.project.path());
        let (code, output) = run_agent_install_pty(
            &single,
            locale,
            &[(install_prompt, b"\n"), (confirm, b"y\n")],
        );
        assert_eq!(code, 0, "{locale}: {output}");
        assert!(output.contains("[1-1]"), "{locale}: {output}");
        assert_eq!(tree_snapshot(single.data.path()), data_before);
        assert_eq!(tree_snapshot(single.state.path()), state_before);
        assert_eq!(tree_snapshot(single.config.path()), config_before);
        assert_eq!(tree_snapshot(single.project.path()), project_before);
        assert_eq!(
            fs::read(single.home.path().join(".claude/skills/skit/SKILL.md")).unwrap(),
            BUNDLED_SKILL.as_bytes()
        );

        let eof = Sandbox::new();
        fs::create_dir(eof.home.path().join(".claude")).unwrap();
        let before = sandbox_snapshot(&eof);
        let (code, output) = run_agent_install_pty(&eof, locale, &[(install_prompt, b"\x04")]);
        assert_eq!(code, 130, "{locale}: {output}");
        assert!(output.contains(aborted), "{locale}: {output}");
        assert_eq!(sandbox_snapshot(&eof), before);

        let reprompt = Sandbox::new();
        fs::create_dir(reprompt.home.path().join(".claude")).unwrap();
        fs::create_dir(reprompt.project.path().join(".agents")).unwrap();
        let before = sandbox_snapshot(&reprompt);
        let (code, output) = run_agent_install_pty(
            &reprompt,
            locale,
            &[
                (install_prompt, b"9\n"),
                (invalid, b""),
                (install_prompt, b"2\n"),
                (confirm, b"n\n"),
            ],
        );
        assert_eq!(code, 0, "{locale}: {output}");
        assert!(output.contains(invalid), "{locale}: {output}");
        assert!(output.contains(cancelled), "{locale}: {output}");
        assert_eq!(sandbox_snapshot(&reprompt), before);
    }
}

#[test]
#[ignore = "UNMAPPED (bucket 2): white-box test of the Python-private helper cli._agent_pick_target. The Rust pick_agent_target is a private fn in the skit-cli binary (cli.rs:5178), and the Rich Prompt/Confirm internals it pins (choices list, default \"1\", confirm default True, that it talks through skit's own console) have no line-prompt equivalent. Its observable menu text is pinned by test_cli_bare_interactive_picks_and_confirms."]
fn test_agent_pick_target_renders_the_menu_exactly() {
    // Oracle test_agent_install.py:231-268: captures cli.console output and the Rich Prompt/Confirm
    // arguments to pin the numbered picker menu exactly. No Rust analogue (private fn, line prompt).
}

#[test]
#[ignore = "UNMAPPED (bucket 2): white-box test of the Python-private helper cli._agent_pick_target (returns None on a declined confirmation, test_agent_install.py:271-276). pick_agent_target is private to the skit-cli binary; the observable back-out behavior is pinned by test_cli_bare_interactive_backing_out_writes_nothing."]
fn test_agent_pick_target_backing_out_returns_none() {
    // No Rust analogue: the private picker cannot be observed apart from the CLI it serves.
}

// --------------------------------------------------------------------------
// pty harness for the interactive bare-mode lanes
//
// Each answer waits for the prompt that owns it. This keeps invalid-input reprompts and EOF
// behavior deterministic without guessing how long the child takes to start.
// --------------------------------------------------------------------------

fn run_agent_install_pty(
    sandbox: &Sandbox,
    locale: &str,
    input: &[(&str, &[u8])],
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(["agent", "install"]);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", locale);
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("HOME", sandbox.home.path());
    command.env("USERPROFILE", sandbox.home.path());
    command.cwd(sandbox.project.path());
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let output = Arc::new(Mutex::new(Vec::new()));
    let reader_output = Arc::clone(&output);
    let drain = thread::spawn(move || {
        let mut bytes = [0_u8; 1024];
        loop {
            match reader.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(read) => reader_output
                    .lock()
                    .unwrap()
                    .extend_from_slice(&bytes[..read]),
            }
        }
    });
    let mut writer = pair.master.take_writer().unwrap();
    let mut checkpoint = 0;
    for (prompt, answer) in input {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let bytes = output.lock().unwrap();
            let position = bytes[checkpoint..]
                .windows(prompt.len())
                .position(|window| window == prompt.as_bytes());
            let shown = String::from_utf8_lossy(&bytes).into_owned();
            drop(bytes);
            if let Some(position) = position {
                checkpoint += position + prompt.len();
                break;
            }
            assert!(Instant::now() < deadline, "did not see {prompt:?}: {shown}");
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before {prompt:?}: {shown}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        if !answer.is_empty() {
            writer.write_all(answer).unwrap();
            writer.flush().unwrap();
        }
    }
    let status = child.wait().unwrap();
    drop(writer);
    drain.join().unwrap();
    let output = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    (status.exit_code(), output)
}
