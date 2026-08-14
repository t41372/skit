//! Exact runtime-boundary ports from Python v0.4 `tests/test_flows.py`.
//!
//! Python kept planning, failure classification, transparency command construction, and process
//! execution in `flows.execute`. Rust splits those responsibilities into public runtime operations;
//! these tests keep the frozen observable contracts at those public boundaries.

use std::{collections::BTreeMap, io, path::{Path, PathBuf}};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_runtime::{
    LaunchError, LaunchPaths, LaunchPlan, ProgramProbe, PromptRunner,
    build_launch_plan, build_launch_preview, execute_launch,
};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|item| item == path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|item| item == path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        self.executable.iter().any(|item| item == path)
    }
}

fn entry(kind: &str) -> Entry {
    let mut entry = Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    };
    entry.meta.workdir = "invoke".to_owned();
    entry
}

fn paths(script: &str, cwd: &str) -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from(script),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from(cwd),
    }
}

fn probe_with_cwd(cwd: &str) -> Probe {
    Probe {
        dirs: vec![PathBuf::from(cwd), PathBuf::from("/data/scripts/demo")],
        ..Probe::default()
    }
}

#[cfg(unix)]
#[test]
fn test_execute_runs_and_returns_the_scripts_exit_code() {
    let cwd = TempDir::new().unwrap();
    let plan = LaunchPlan {
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_owned(), "exit 7".to_owned()],
        env: BTreeMap::new(),
        cwd: cwd.path().to_path_buf(),
        display: "/bin/sh -c 'exit 7'".to_owned(),
        warnings: Vec::new(),
    };
    assert_eq!(execute_launch(&plan).unwrap(), 7);
}

#[test]
fn test_command_template_secret_does_not_get_prompt_agent_warning() {
    let mut command = entry("command");
    let settings = EntrySettings {
        template: "send {api_key}".to_owned(),
        params: vec!["api_key".to_owned()],
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut command.meta);
    let assembly = Assembly {
        command_values: BTreeMap::from([("api_key".to_owned(), "hunter2".to_owned())]),
        masked_command_values: BTreeMap::from([("api_key".to_owned(), "•••".to_owned())]),
        ..Assembly::default()
    };
    let plan = build_launch_preview(
        &command,
        &paths("", "/invoke"),
        &assembly,
        None,
        None,
        None,
        &probe_with_cwd("/invoke"),
    )
    .unwrap();
    assert!(plan.warnings.is_empty(), "a command placeholder must never acquire prompt-agent warnings: {:?}", plan.warnings);
}

#[test]
fn test_execute_classifies_missing_target() {
    let shell = entry("shell");
    let mut probe = probe_with_cwd("/invoke");
    probe.programs.insert("bash".to_owned(), PathBuf::from("/bin/bash"));
    let error = build_launch_plan(
        &shell,
        &paths("/gone/script.sh", "/invoke"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(matches!(error, LaunchError::TargetMissing { ref path } if path == Path::new("/gone/script.sh")));
    assert_eq!(error.exit_code(), 127);
    assert!(error.to_string().contains("/gone/script.sh"), "{error}");
}

#[test]
fn test_execute_classifies_not_executable() {
    let mut executable = entry("exe");
    executable.meta.mode = StorageMode::Reference;
    executable.meta.source = "/work/not-x".to_owned();
    let mut probe = probe_with_cwd("/invoke");
    probe.files.push(PathBuf::from("/work/not-x"));
    let error = build_launch_plan(
        &executable,
        &paths("/work/not-x", "/invoke"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap_err();
    assert!(matches!(error, LaunchError::TargetNotExecutable { ref path } if path == Path::new("/work/not-x")));
    assert_eq!(error.exit_code(), 126);
}

#[test]
fn test_transparency_command_source_shows_filled_template() {
    let mut command = entry("command");
    let settings = EntrySettings {
        template: "echo {msg}".to_owned(),
        params: vec!["msg".to_owned()],
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut command.meta);
    let assembly = Assembly {
        command_values: BTreeMap::from([("msg".to_owned(), "hello".to_owned())]),
        masked_command_values: BTreeMap::from([("msg".to_owned(), "hello".to_owned())]),
        ..Assembly::default()
    };
    let plan = build_launch_preview(
        &command,
        &paths("", "/invoke"),
        &assembly,
        None,
        None,
        None,
        &probe_with_cwd("/invoke"),
    )
    .unwrap();
    assert!(plan.display.contains("echo hello"), "{}", plan.display);
}

#[test]
fn test_transparency_command_source_masks_secret_placeholder() {
    let mut command = entry("command");
    let settings = EntrySettings {
        template: "curl -H \"Authorization: Bearer {api_key}\" https://api.example.com".to_owned(),
        params: vec!["api_key".to_owned()],
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut command.meta);
    let assembly = Assembly {
        command_values: BTreeMap::from([("api_key".to_owned(), "sk-SUPERSECRET-123".to_owned())]),
        masked_command_values: BTreeMap::from([("api_key".to_owned(), "•••".to_owned())]),
        ..Assembly::default()
    };
    let plan = build_launch_preview(
        &command,
        &paths("", "/invoke"),
        &assembly,
        None,
        None,
        None,
        &probe_with_cwd("/invoke"),
    )
    .unwrap();
    assert!(!plan.display.contains("sk-SUPERSECRET-123"), "secret leaked into display: {}", plan.display);
    assert!(plan.display.contains("•••"), "masked placeholder disappeared: {}", plan.display);
    assert!(plan.args.iter().any(|arg| arg.contains("sk-SUPERSECRET-123")), "real process argv lost the secret value: {:?}", plan.args);
}

#[test]
fn test_normal_prompt_transparency_is_compact_and_never_reads_the_body() {
    let prompt = entry("prompt");
    let body = "DO-NOT-COPY-THIS-BODY plaintext\n";
    let runner = PromptRunner {
        name: "agent".to_owned(),
        argv: vec!["agent".to_owned(), "--".to_owned(), "{{prompt}}".to_owned()],
    };
    let assembly = Assembly {
        args: vec!["--model".to_owned(), "fast".to_owned()],
        masked_args: vec!["--model".to_owned(), "fast".to_owned()],
        ..Assembly::default()
    };
    let mut probe = probe_with_cwd("/invoke");
    probe.programs.insert("agent".to_owned(), PathBuf::from("/bin/agent"));
    let plan = build_launch_plan(
        &prompt,
        &paths("/deleted/review.prompt.md", "/invoke"),
        &assembly,
        Some(body),
        Some(&runner),
        &probe,
    )
    .unwrap();
    assert!(plan.display.contains("agent"), "{}", plan.display);
    assert!(plan.display.contains("--model fast"), "{}", plan.display);
    assert!(plan.display.contains("rendered prompt omitted"), "{}", plan.display);
    assert!(!plan.display.contains(body.trim()), "prompt body leaked into normal transparency: {}", plan.display);
    assert!(!plan.display.contains("plaintext"), "prompt value leaked into normal transparency: {}", plan.display);
}

#[test]
fn test_execute_not_executable_message_carries_the_error() {
    let error = LaunchError::TargetNotExecutable { path: PathBuf::from("chmod +x it") };
    assert!(error.to_string().contains("chmod +x it"), "{error}");
}

#[test]
fn test_execute_launch_error_message_carries_the_error() {
    let error = LaunchError::Process {
        operation: "run",
        source: io::Error::other("workdir gone"),
    };
    assert_eq!(error.exit_code(), 125);
    assert!(error.to_string().contains("workdir gone"), "{error}");
}

#[test]
fn test_execute_forwards_invoke_cwd() {
    let shell = entry("shell");
    let mut probe = probe_with_cwd("/run-here");
    probe.files.push(PathBuf::from("/copy/script.sh"));
    probe.programs.insert("bash".to_owned(), PathBuf::from("/bin/bash"));
    let plan = build_launch_plan(
        &shell,
        &paths("/copy/script.sh", "/run-here"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.cwd, PathBuf::from("/run-here"));
}
