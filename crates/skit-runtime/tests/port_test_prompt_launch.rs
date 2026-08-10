//! Public runtime ports of Python v0.4 prompt-launch argv contracts.
//!
//! These tests stop at `build_launch_plan`: no private helper is exposed for the port. A red
//! assertion is a behavior mismatch to keep, not a request to patch runtime production code.

use std::{collections::BTreeMap, path::{Path, PathBuf}};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, LaunchWarning, ProgramProbe, PromptRunner, build_launch_plan,
};

#[derive(Debug, Default)]
struct FakeProbe {
    programs: BTreeMap<String, PathBuf>,
    dirs: Vec<PathBuf>,
}

impl ProgramProbe for FakeProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, _path: &Path) -> bool {
        false
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|candidate| candidate == path)
    }

    fn is_executable(&self, _path: &Path) -> bool {
        false
    }
}

fn entry() -> Entry {
    let mut meta = EntryMeta::minimal("Prompt", EntryKind::parse("prompt").unwrap());
    meta.workdir = "invoke".to_owned();
    Entry {
        slug: Slug::parse("prompt").unwrap(),
        meta,
    }
}

fn paths() -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from("/data/scripts/prompt/prompt.md"),
        entry_dir: PathBuf::from("/data/scripts/prompt"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

fn assembly(extra: &[&str]) -> Assembly {
    let args = extra.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>();
    Assembly {
        args: args.clone(),
        masked_args: args,
        ..Assembly::default()
    }
}

fn runner(name: &str, argv: &[&str]) -> PromptRunner {
    PromptRunner {
        name: name.to_owned(),
        argv: argv.iter().map(|token| (*token).to_owned()).collect(),
    }
}

fn probe(programs: &[&str]) -> FakeProbe {
    FakeProbe {
        programs: programs
            .iter()
            .map(|name| ((*name).to_owned(), PathBuf::from(format!("/bin/{name}"))))
            .collect(),
        dirs: vec![PathBuf::from("/invoke")],
    }
}

#[test]
fn test_build_renders_two_stages_and_appends_extra() {
    let plan = build_launch_plan(
        &entry(),
        &paths(),
        &assembly(&["--model", "opus"]),
        Some("Do X\n"),
        Some(&runner("rec", &["rec-bin", "{{prompt}}"])),
        &probe(&["rec-bin"]),
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("/bin/rec-bin"));
    assert_eq!(plan.args, ["Do X\n", "--model", "opus"]);
}

#[test]
fn test_seeded_positional_runner_protects_dash_prefixed_prompt_and_keeps_extra() {
    let plan = build_launch_plan(
        &entry(),
        &paths(),
        &assembly(&["--model", "opus"]),
        Some("--help"),
        Some(&runner("claude", &["claude", "--", "{{prompt}}"])),
        &probe(&["claude"]),
    )
    .unwrap();

    assert_eq!(plan.args, ["--model", "opus", "--", "--help"]);
}

#[test]
fn test_seeded_opencode_binds_dash_prefixed_prompt_and_keeps_extra() {
    let plan = build_launch_plan(
        &entry(),
        &paths(),
        &assembly(&["--model", "provider/model"]),
        Some("--version"),
        Some(&runner("opencode", &["opencode", "--prompt={{prompt}}"])),
        &probe(&["opencode"]),
    )
    .unwrap();

    assert_eq!(
        plan.args,
        ["--prompt=--version", "--model", "provider/model"]
    );
}

#[test]
fn test_seeded_copilot_binds_dash_prefixed_prompt_and_keeps_extra() {
    let plan = build_launch_plan(
        &entry(),
        &paths(),
        &assembly(&["--model", "gpt-5"]),
        Some("--version"),
        Some(&runner(
            "copilot",
            &["copilot", "--interactive={{prompt}}"],
        )),
        &probe(&["copilot"]),
    )
    .unwrap();

    assert_eq!(plan.args, ["--interactive=--version", "--model", "gpt-5"]);
}

#[test]
fn test_seeded_cursor_selects_agent_before_passing_prompt() {
    for body in ["--help\nsecond line", "status"] {
        let plan = build_launch_plan(
            &entry(),
            &paths(),
            &assembly(&["--model", "gpt-5"]),
            Some(body),
            Some(&runner(
                "cursor",
                &["cursor-agent", "--", "agent", "{{prompt}}"],
            )),
            &probe(&["cursor-agent"]),
        )
        .unwrap();

        assert_eq!(
            plan.args,
            ["--model", "gpt-5", "--", "agent", body]
        );
    }
}

#[test]
fn test_seeded_pi_warns_and_prefixes_newline_for_parser_ambiguous_prompt() {
    for body in [
        "--help\nsecond line",
        "-v",
        "@README.md",
        "config",
        "install",
        "list",
        "remove",
        "uninstall",
        "update",
    ] {
        let plan = build_launch_plan(
            &entry(),
            &paths(),
            &assembly(&["--model", "fast"]),
            Some(body),
            Some(&runner("pi", &["pi", "{{prompt}}"])),
            &probe(&["pi"]),
        )
        .unwrap();

        assert_eq!(plan.args, [format!("\n{body}"), "--model".to_owned(), "fast".to_owned()]);
        assert_eq!(plan.warnings, [LaunchWarning::PiPromptProtected]);
    }
}

#[test]
fn test_seeded_pi_keeps_unambiguous_prompt_byte_exact() {
    for body in ["ordinary prompt", "first line\nsecond line", " install", "help"] {
        let plan = build_launch_plan(
            &entry(),
            &paths(),
            &Assembly::default(),
            Some(body),
            Some(&runner("pi", &["pi", "{{prompt}}"])),
            &probe(&["pi"]),
        )
        .unwrap();

        assert_eq!(plan.args, [body]);
        assert!(plan.warnings.is_empty());
    }
}

#[test]
fn test_build_refuses_nul_in_prompt_as_launch_error() {
    assert!(matches!(
        build_launch_plan(
            &entry(),
            &paths(),
            &Assembly::default(),
            Some("bad\0prompt"),
            Some(&runner("rec", &["rec-bin", "{{prompt}}"])),
            &probe(&["rec-bin"]),
        )
        .unwrap_err(),
        LaunchError::PromptContainsNul
    ));
}

#[test]
fn test_build_without_runner_is_exit_126() {
    let error = build_launch_plan(
        &entry(),
        &paths(),
        &Assembly::default(),
        Some("body"),
        None,
        &probe(&[]),
    )
    .unwrap_err();

    assert!(matches!(error, LaunchError::PromptRunnerRequired));
    assert_eq!(error.exit_code(), 126);
}

#[test]
fn test_build_missing_body_is_a_clean_pre_spawn_error() {
    let error = build_launch_plan(
        &entry(),
        &paths(),
        &Assembly::default(),
        None,
        Some(&runner("rec", &["rec-bin", "{{prompt}}"])),
        &probe(&["rec-bin"]),
    )
    .unwrap_err();

    assert!(matches!(error, LaunchError::PromptBodyRequired));
}

#[test]
fn test_build_missing_binary_is_exit_126() {
    let error = build_launch_plan(
        &entry(),
        &paths(),
        &Assembly::default(),
        Some("body"),
        Some(&runner("rec", &["rec-bin", "{{prompt}}"])),
        &probe(&[]),
    )
    .unwrap_err();

    assert!(matches!(error, LaunchError::ProgramNotFound { .. }));
    assert_eq!(error.exit_code(), 126);
}

#[test]
fn test_invalid_runner_requires_exactly_one_prompt_marker_outside_program() {
    for invalid in [
        runner("empty", &[]),
        runner("missing", &["agent"]),
        runner("program", &["agent-{{prompt}}", "x"]),
        runner("double", &["agent", "{{prompt}}", "{{prompt}}"]),
    ] {
        assert!(matches!(
            build_launch_plan(
                &entry(),
                &paths(),
                &Assembly::default(),
                Some("body"),
                Some(&invalid),
                &probe(&["agent"]),
            )
            .unwrap_err(),
            LaunchError::InvalidPromptRunner { .. }
        ));
    }
}

#[test]
fn test_build_over_long_render_is_a_clean_launch_error() {
    let body = "x".repeat(100_100);
    let error = build_launch_plan(
        &entry(),
        &paths(),
        &Assembly::default(),
        Some(&body),
        Some(&runner("rec", &["rec-bin", "{{prompt}}"])),
        &probe(&["rec-bin"]),
    )
    .unwrap_err();

    assert!(matches!(error, LaunchError::PromptArgvTooLong { .. }));
}

#[test]
fn test_builtin_amp_plan_reports_one_shot_warning() {
    let plan = build_launch_plan(
        &entry(),
        &paths(),
        &Assembly::default(),
        Some("body"),
        Some(&runner("amp", &["amp", "-x", "{{prompt}}"])),
        &probe(&["amp"]),
    )
    .unwrap();

    assert!(plan.warnings.contains(&LaunchWarning::AmpOneShot));
}
