use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug};
use skit_runtime::{LaunchError, LaunchPaths, ProgramProbe, PromptRunner, build_launch_plan};

#[derive(Debug)]
struct Probe;

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        (name == "agent").then(|| PathBuf::from("/bin/agent"))
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
