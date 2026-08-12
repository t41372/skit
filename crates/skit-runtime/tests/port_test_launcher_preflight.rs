//! Preflight-equivalent ports from Python `tests/test_launcher.py` at `main@206f9ef`.
//!
//! Python exposes a dedicated `launcher.preflight()` helper. Rust deliberately folds that boundary
//! into `build_launch_preview`: preview validates source/executable/workdir facts while replacing
//! program lookup with a non-launching preview probe. These tests make that architectural mapping
//! explicit and keep every Python refusal/pass contract executable. A red assertion is a parity
//! finding; do not weaken it to match the current Rust implementation.

use std::path::{Path, PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_runtime::{LaunchError, LaunchPaths, ProgramProbe, build_launch_preview};

#[derive(Debug, Default)]
struct PreflightProbe {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
}

impl ProgramProbe for PreflightProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        panic!("preflight/preview must not consult the local program lookup: {name}")
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|candidate| candidate == path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|candidate| candidate == path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        self.executable.iter().any(|candidate| candidate == path)
    }
}

fn entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    }
}

fn paths(script: impl Into<PathBuf>) -> LaunchPaths {
    LaunchPaths {
        script: script.into(),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

fn preview(entry: &Entry, paths: &LaunchPaths, probe: &PreflightProbe) -> Result<(), LaunchError> {
    build_launch_preview(entry, paths, &Assembly::default(), None, None, None, probe).map(drop)
}

#[test]
fn test_preflight_refuses_exe_directory_source() {
    let mut executable = entry("exe");
    executable.meta.mode = StorageMode::Reference;
    executable.meta.source = "/Bundle.app".to_owned();
    executable.meta.workdir = "invoke".to_owned();
    let probe = PreflightProbe {
        dirs: vec![PathBuf::from("/invoke"), PathBuf::from("/Bundle.app")],
        ..PreflightProbe::default()
    };

    let error = preview(&executable, &paths("/Bundle.app"), &probe).unwrap_err();
    assert!(
        matches!(
            error,
            LaunchError::TargetNotExecutable { ref path }
                if path == Path::new("/Bundle.app")
        ),
        "directory-shaped executable was not refused as not-executable: {error}"
    );
}

#[test]
fn test_preflight_passes_for_healthy_entry() {
    let mut python = entry("python");
    python.meta.workdir = "invoke".to_owned();
    let script = PathBuf::from("/data/scripts/demo/script.py");
    let probe = PreflightProbe {
        files: vec![script.clone()],
        dirs: vec![PathBuf::from("/invoke")],
        ..PreflightProbe::default()
    };

    preview(&python, &paths(script), &probe).unwrap();
}

#[test]
fn test_preflight_raises_for_missing_python_script() {
    let mut python = entry("python");
    python.meta.workdir = "invoke".to_owned();
    let missing = PathBuf::from("/data/scripts/demo/script.py");
    let probe = PreflightProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..PreflightProbe::default()
    };

    let error = preview(&python, &paths(&missing), &probe).unwrap_err();
    assert!(
        matches!(
            error,
            LaunchError::TargetMissing { ref path } if path == &missing
        ),
        "missing Python source reached a later launch stage: {error}"
    );
}

#[test]
fn test_preflight_raises_for_missing_exe() {
    let mut executable = entry("exe");
    executable.meta.mode = StorageMode::Reference;
    executable.meta.source = "/missing/tool".to_owned();
    executable.meta.workdir = "invoke".to_owned();
    let probe = PreflightProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..PreflightProbe::default()
    };

    let error = preview(&executable, &paths("/missing/tool"), &probe).unwrap_err();
    assert!(
        matches!(
            error,
            LaunchError::TargetMissing { ref path } if path == Path::new("/missing/tool")
        ),
        "missing executable was not rejected before launch: {error}"
    );
}

#[test]
fn test_preflight_raises_for_missing_workdir() {
    let mut python = entry("python");
    python.meta.workdir = "/nonexistent/path/that/does/not/exist".to_owned();
    let script = PathBuf::from("/data/scripts/demo/script.py");
    let probe = PreflightProbe {
        files: vec![script.clone()],
        ..PreflightProbe::default()
    };

    let error = preview(&python, &paths(script), &probe).unwrap_err();
    assert!(
        matches!(
            error,
            LaunchError::WorkdirMissing { ref path }
                if path == Path::new("/nonexistent/path/that/does/not/exist")
        ),
        "missing workdir was not rejected by preflight: {error}"
    );
}

#[test]
fn test_preflight_does_not_invoke_uv() {
    let mut python = entry("python");
    python.meta.workdir = "invoke".to_owned();
    let script = PathBuf::from("/data/scripts/demo/script.py");
    let probe = PreflightProbe {
        files: vec![script.clone()],
        dirs: vec![PathBuf::from("/invoke")],
        ..PreflightProbe::default()
    };

    // `PreflightProbe::find_program` panics unconditionally. Reaching this assertion therefore
    // proves preview/preflight did not look for or download uv while still validating the source.
    let plan = build_launch_preview(
        &python,
        &paths(script),
        &Assembly::default(),
        None,
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(plan.program, PathBuf::from("uv"));
    assert!(
        plan.args
            .starts_with(&["run".to_owned(), "--no-project".to_owned()])
    );
}

#[test]
fn test_preflight_passes_for_command_entry_without_workdir_or_target_issues() {
    let mut command = entry("command");
    command.meta.workdir = "invoke".to_owned();
    EntrySettings {
        template: "echo hi".to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut command.meta);
    let probe = PreflightProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..PreflightProbe::default()
    };

    preview(&command, &paths(PathBuf::new()), &probe).unwrap();
}

#[test]
fn test_preflight_succeeds_for_copy_mode_entry_with_deleted_origin() {
    let mut python = entry("python");
    python.meta.mode = StorageMode::Copy;
    python.meta.source = "/deleted/origin/s.py".to_owned();
    python.meta.workdir = "origin".to_owned();
    let stored = PathBuf::from("/data/scripts/demo/script.py");
    let probe = PreflightProbe {
        files: vec![stored.clone()],
        dirs: vec![PathBuf::from("/invoke")],
        ..PreflightProbe::default()
    };

    let plan = build_launch_preview(
        &python,
        &paths(stored),
        &Assembly::default(),
        None,
        None,
        None,
        &probe,
    )
    .unwrap();
    assert_eq!(
        plan.cwd,
        PathBuf::from("/invoke"),
        "copy-mode preflight must survive a deleted original by falling back to invoke cwd"
    );
}
