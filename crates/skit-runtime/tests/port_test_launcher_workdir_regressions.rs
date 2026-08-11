//! Public launcher regressions from Python v0.4 `tests/test_launcher.py` that are not duplicates of
//! the general launch-plan coverage.
//!
//! These exercise the public workdir resolver and launch display. They deliberately distinguish a
//! copied entry whose historical origin disappeared (fallback is safe) from a reference entry whose
//! origin disappeared (fallback would mask a broken reference).

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug, StorageMode};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, build_launch_plan, resolve_launch_workdir,
};

#[derive(Debug, Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
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

fn entry(mode: StorageMode, source: &str) -> Entry {
    let mut meta = EntryMeta::minimal("Demo", EntryKind::parse("python").unwrap());
    meta.mode = mode;
    meta.source = source.to_owned();
    meta.workdir = "origin".to_owned();
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta,
    }
}

fn paths(script: impl Into<PathBuf>) -> LaunchPaths {
    LaunchPaths {
        script: script.into(),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

#[test]
fn test_resolve_workdir_copy_mode_falls_back_when_origin_gone() {
    let copied = entry(StorageMode::Copy, "/gone/project/script.py");
    let probe = Probe {
        dirs: vec![PathBuf::from("/invoke")],
        ..Probe::default()
    };

    assert_eq!(
        resolve_launch_workdir(&copied, &paths("/data/scripts/demo/script.py"), &probe).unwrap(),
        PathBuf::from("/invoke")
    );
}

#[test]
fn test_resolve_workdir_reference_mode_does_not_mask_missing_origin() {
    let referenced = entry(StorageMode::Reference, "/gone/project/script.py");
    let probe = Probe {
        dirs: vec![PathBuf::from("/invoke")],
        ..Probe::default()
    };

    let error =
        resolve_launch_workdir(&referenced, &paths("/gone/project/script.py"), &probe).unwrap_err();
    assert!(matches!(
        error,
        LaunchError::WorkdirMissing { ref path } if path == Path::new("/gone/project")
    ));
}

#[test]
fn test_describe_command_isolates_project_like_build_command() {
    let mut copied = entry(StorageMode::Copy, "/origin/project/script.py");
    copied.meta.workdir = "invoke".to_owned();
    let script = PathBuf::from("/data/scripts/demo/script.py");
    let probe = Probe {
        programs: BTreeMap::from([("uv".to_owned(), PathBuf::from("/fake/uv"))]),
        files: vec![script.clone()],
        dirs: vec![PathBuf::from("/invoke")],
    };

    let plan = build_launch_plan(
        &copied,
        &paths(&script),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert_eq!(
        plan.args,
        [
            "run",
            "--no-project",
            "--script",
            "/data/scripts/demo/script.py",
        ]
    );
    assert!(
        plan.display.contains("run --no-project --script"),
        "{}",
        plan.display
    );
}
