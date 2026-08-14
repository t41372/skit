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
    thread,
    time::Duration,
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
    sandbox
        .command()
        .args(["agent", "install"])
        .assert()
        .code(2);
    assert_eq!(fs::read_dir(sandbox.home.path()).unwrap().count(), 0);
}

#[test]
fn test_cli_bare_interactive_no_candidates_exits_1() {
    // Interactive (pty) but no marker directories exist: exit 1, and the message steers to --to.
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let (code, output) = run_agent_install_pty(
        home.path(),
        project.path(),
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("--to"), "{output}");
}

#[test]
fn test_cli_bare_interactive_picks_and_confirms() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".claude")).unwrap();
    fs::create_dir(project.path().join(".agents")).unwrap();
    let (code, output) = run_agent_install_pty(
        home.path(),
        project.path(),
        data.path(),
        state.path(),
        config.path(),
        &[b"2\n", b"y\n"],
    );
    assert_eq!(code, 0, "{output}");
    // The menu text is part of the contract a mouse-less user reads. `_agent_pick_target`'s exact
    // pin is white-box, so it is salvaged here as full-line assertions. The pty rewrites \n to
    // \r\n, so each line is matched on its own, never across a boundary.
    let claude_skills = home.path().join(".claude").join("skills");
    let agents_skills = project.path().join(".agents").join("skills");
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
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".claude")).unwrap();
    let (code, output) = run_agent_install_pty(
        home.path(),
        project.path(),
        data.path(),
        state.path(),
        config.path(),
        &[b"1\n", b"n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("Cancelled \u{2014} nothing was written."),
        "{output}"
    );
    assert!(
        !home.path().join(".claude").join("skills").exists(),
        "{output}"
    );
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
// Timing copied verbatim from `terminal_pty.rs::run_pty_configured` (the proven-non-flaky
// configuration): 40ms settle, 120ms per input chunk, no cursor-query answer, and the slave is
// dropped before the reader thread starts.
// --------------------------------------------------------------------------

fn run_agent_install_pty(
    home: &Path,
    cwd: &Path,
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
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
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data);
    command.env("SKIT_STATE_DIR", state);
    command.env("SKIT_CONFIG_DIR", config);
    command.env("HOME", home);
    command.env("USERPROFILE", home);
    command.cwd(cwd);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(40));
    for bytes in input {
        thread::sleep(Duration::from_millis(120));
        if writer.write_all(bytes).is_err() {
            break;
        }
        let _ = writer.flush();
    }
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    (status.exit_code(), output)
}
