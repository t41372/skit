use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug};
use skit_runtime::{
    LaunchError, LaunchPaths, LaunchWarning, ProgramProbe, PromptRunner, build_launch_plan,
    build_launch_preview,
};

#[derive(Debug)]
struct Probe;

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        matches!(
            Path::new(name)
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("agent" | "pi" | "pi.cmd" | "pi.exe" | "pi.ps1")
        )
        .then(|| PathBuf::from(name))
    }

    fn is_file(&self, _path: &Path) -> bool {
        true
    }

    fn is_dir(&self, _path: &Path) -> bool {
        true
    }

    fn is_executable(&self, _path: &Path) -> bool {
        true
    }
}

fn prompt() -> Entry {
    let mut meta = EntryMeta::minimal("Prompt", EntryKind::parse("prompt").unwrap());
    meta.workdir = "invoke".to_owned();
    Entry {
        slug: Slug::parse("prompt").unwrap(),
        meta,
    }
}

fn paths() -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from("/data/prompt.md"),
        entry_dir: PathBuf::from("/data"),
        invoke_cwd: PathBuf::from("/work"),
    }
}

#[test]
fn a_prompt_token_can_fill_one_argv_token_without_a_shell() {
    let runner = PromptRunner {
        name: "agent".to_owned(),
        argv: vec![
            "agent".to_owned(),
            "--prompt={{prompt}}".to_owned(),
            "--mode=interactive".to_owned(),
        ],
    };
    let plan = build_launch_plan(
        &prompt(),
        &paths(),
        &Assembly::default(),
        Some("hello world"),
        Some(&runner),
        &Probe,
    )
    .unwrap();

    assert_eq!(plan.args, ["--prompt=hello world", "--mode=interactive"]);
}

#[test]
fn prompt_extra_flags_are_inserted_before_the_first_option_delimiter() {
    let runner = PromptRunner {
        name: "agent".to_owned(),
        argv: vec![
            "agent".to_owned(),
            "--prompt".to_owned(),
            "{{prompt}}".to_owned(),
            "--".to_owned(),
            "literal".to_owned(),
        ],
    };
    let assembly = Assembly {
        args: vec!["--model".to_owned(), "opus".to_owned()],
        masked_args: vec!["--model".to_owned(), "opus".to_owned()],
        ..Assembly::default()
    };

    let plan = build_launch_plan(
        &prompt(),
        &paths(),
        &assembly,
        Some("review"),
        Some(&runner),
        &Probe,
    )
    .unwrap();

    assert_eq!(
        plan.args,
        ["--prompt", "review", "--model", "opus", "--", "literal"]
    );
}

#[derive(Debug)]
struct OfflineProbe;

impl ProgramProbe for OfflineProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        panic!("preview must not look up {name} on PATH")
    }

    fn is_file(&self, _path: &Path) -> bool {
        true
    }

    fn is_dir(&self, _path: &Path) -> bool {
        true
    }

    fn is_executable(&self, _path: &Path) -> bool {
        true
    }
}

#[test]
fn preview_is_offline_and_displays_the_complete_masked_prompt_argv() {
    let runner = PromptRunner {
        name: "agent".to_owned(),
        argv: vec!["agent".to_owned(), "--".to_owned(), "{{prompt}}".to_owned()],
    };
    let assembly = Assembly {
        args: vec!["--model".to_owned(), "opus".to_owned()],
        masked_args: vec!["--model".to_owned(), "opus".to_owned()],
        ..Assembly::default()
    };

    let plan = build_launch_preview(
        &prompt(),
        &paths(),
        &assembly,
        Some("token=actual"),
        Some("token=***"),
        Some(&runner),
        &OfflineProbe,
    )
    .unwrap();

    assert_eq!(plan.args, ["--model", "opus", "--", "token=actual"]);
    assert!(plan.display.contains("token=***"));
    assert!(!plan.display.contains("token=actual"));
    assert!(!plan.display.contains("<prompt>"));
}

#[test]
fn the_prompt_marker_must_appear_exactly_once_and_never_in_the_program_token() {
    for argv in [
        vec!["agent".to_owned(), "run".to_owned()],
        vec![
            "agent".to_owned(),
            "{{prompt}}".to_owned(),
            "again={{prompt}}".to_owned(),
        ],
        vec!["agent-{{prompt}}".to_owned(), "run".to_owned()],
    ] {
        let runner = PromptRunner {
            name: "bad".to_owned(),
            argv,
        };
        assert!(matches!(
            build_launch_plan(
                &prompt(),
                &paths(),
                &Assembly::default(),
                Some("body"),
                Some(&runner),
                &Probe,
            ),
            Err(LaunchError::InvalidPromptRunner { .. })
        ));
    }
}

#[test]
fn prompt_runner_environment_is_only_the_assembly_environment() {
    let runner = PromptRunner {
        name: "agent".to_owned(),
        argv: vec!["agent".to_owned(), "{{prompt}}".to_owned()],
    };
    let assembly = Assembly {
        env_values: BTreeMap::from([("MODE".to_owned(), "safe".to_owned())]),
        masked_env: BTreeMap::from([("MODE".to_owned(), "safe".to_owned())]),
        ..Assembly::default()
    };
    let plan = build_launch_plan(
        &prompt(),
        &paths(),
        &assembly,
        Some("body"),
        Some(&runner),
        &Probe,
    )
    .unwrap();
    assert_eq!(
        plan.env,
        BTreeMap::from([("MODE".to_owned(), "safe".to_owned())])
    );
}

#[test]
fn prompt_argv_refuses_nul_and_an_oversized_posix_command_line() {
    let runner = PromptRunner {
        name: "agent".to_owned(),
        argv: vec!["agent".to_owned(), "{{prompt}}".to_owned()],
    };
    assert!(matches!(
        build_launch_plan(
            &prompt(),
            &paths(),
            &Assembly::default(),
            Some("before\0after"),
            Some(&runner),
            &Probe,
        ),
        Err(LaunchError::PromptContainsNul)
    ));

    let oversized = "界".repeat(34_000);
    assert!(matches!(
        build_launch_plan(
            &prompt(),
            &paths(),
            &Assembly::default(),
            Some(&oversized),
            Some(&runner),
            &Probe,
        ),
        Err(LaunchError::PromptArgvTooLong { size, limit, unit })
            if size > limit && limit == 100_000 && unit == "bytes"
    ));
}

#[test]
fn pi_runner_protects_ambiguous_opening_text_and_reports_the_change() {
    for body in [
        "--help",
        "@notes.txt",
        "config",
        "install",
        "list",
        "remove",
        "uninstall",
        "update",
    ] {
        let runner = PromptRunner {
            name: "renamed-runner".to_owned(),
            argv: vec!["/tools/pi.EXE".to_owned(), "{{prompt}}".to_owned()],
        };
        let plan = build_launch_plan(
            &prompt(),
            &paths(),
            &Assembly::default(),
            Some(body),
            Some(&runner),
            &Probe,
        )
        .unwrap();
        assert_eq!(plan.args, [format!("\n{body}")]);
        assert_eq!(plan.warnings, [LaunchWarning::PiPromptProtected]);
    }

    let runner = PromptRunner {
        name: "pi".to_owned(),
        argv: vec!["pi".to_owned(), "{{prompt}}".to_owned()],
    };
    let plan = build_launch_plan(
        &prompt(),
        &paths(),
        &Assembly::default(),
        Some("normal text"),
        Some(&runner),
        &Probe,
    )
    .unwrap();
    assert_eq!(plan.args, ["normal text"]);
    assert!(plan.warnings.is_empty());
}
