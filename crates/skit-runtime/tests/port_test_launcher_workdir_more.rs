//! Remaining work-directory and preview ports from Python `tests/test_launcher.py` at
//! `main@206f9ef`.
//!
//! Python separates `_resolve_workdir` (policy selection) from preflight existence checks. Rust's
//! public resolver currently combines selection and validation. These tests keep the Python outcome
//! as the oracle: if the combined Rust seam rejects earlier, the test stays red rather than treating
//! "some error" as equivalent.

use std::path::{Path, PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug, StorageMode};
use skit_runtime::{
    LaunchPaths, ProgramProbe, build_launch_preview, resolve_launch_workdir,
};

#[derive(Debug, Default)]
struct Probe {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(name))
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|candidate| candidate == path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|candidate| candidate == path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        self.is_file(path)
    }
}

fn python(mode: StorageMode, source: &str, workdir: &str) -> Entry {
    let mut meta = EntryMeta::minimal("Demo", EntryKind::parse("python").unwrap());
    meta.mode = mode;
    meta.source = source.to_owned();
    meta.workdir = workdir.to_owned();
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta,
    }
}

fn paths(script: impl Into<PathBuf>, invoke: impl Into<PathBuf>) -> LaunchPaths {
    LaunchPaths {
        script: script.into(),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: invoke.into(),
    }
}

#[test]
fn test_resolve_workdir_copy_mode_falls_back_when_origin_gone() {
    let entry = python(StorageMode::Copy, "/deleted/origin/s.py", "origin");
    let probe = Probe {
        dirs: vec![PathBuf::from("/invoke")],
        ..Probe::default()
    };

    assert_eq!(
        resolve_launch_workdir(&entry, &paths("/data/scripts/demo/script.py", "/invoke"), &probe)
            .unwrap(),
        PathBuf::from("/invoke")
    );
}

#[test]
fn test_resolve_workdir_reference_mode_not_masked_when_origin_gone() {
    let entry = python(StorageMode::Reference, "/gone/refdir/ref.py", "origin");
    let probe = Probe {
        dirs: vec![PathBuf::from("/invoke")],
        ..Probe::default()
    };

    // Python `_resolve_workdir` still chooses the reference source's own parent. The later
    // preflight target check owns the refusal. Rust currently validates the directory inside its
    // resolver; if that rejects here, this deliberately-red test records the parity difference.
    assert_eq!(
        resolve_launch_workdir(&entry, &paths("/gone/refdir/ref.py", "/invoke"), &probe).unwrap(),
        PathBuf::from("/gone/refdir")
    );
}

#[test]
fn test_describe_command_isolates_like_build_command() {
    let mut entry = python(StorageMode::Copy, "/origin/s.py", "invoke");
    entry.meta.workdir = "invoke".to_owned();
    let script = PathBuf::from("/data/scripts/demo/script.py");
    let probe = Probe {
        files: vec![script.clone()],
        dirs: vec![PathBuf::from("/invoke")],
    };

    let preview = build_launch_preview(
        &entry,
        &paths(&script, "/invoke"),
        &Assembly {
            args: vec!["--x".to_owned()],
            masked_args: vec!["--x".to_owned()],
            ..Assembly::default()
        },
        None,
        None,
        None,
        &probe,
    )
    .unwrap();

    assert!(preview.display.contains("--no-project"), "{}", preview.display);
    assert!(preview.display.contains("--script"), "{}", preview.display);
    assert!(
        preview.display.contains("/data/scripts/demo/script.py"),
        "{}",
        preview.display
    );
    assert!(preview.display.contains("--x"), "{}", preview.display);
}
