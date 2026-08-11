//! Public-surface behavioral ports from `origin/main@206f9ef:tests/test_launcher.py`.
//!
//! Private Python helpers map to the public Rust launch planner/workdir resolver. This branch does
//! not patch runtime code when an oracle assertion fails.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_runtime::{
    LaunchError, LaunchPaths, ProgramProbe, build_launch_plan, resolve_launch_workdir,
};

#[derive(Debug, Default)]
struct FakeProbe {
    programs: BTreeMap<String, PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    executable: Vec<PathBuf>,
}

impl ProgramProbe for FakeProbe {
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
        self.executable.iter().any(|candidate| candidate == path)
    }
}

fn entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta: EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap()),
    }
}

fn paths(script: impl Into<PathBuf>, invoke_cwd: impl Into<PathBuf>) -> LaunchPaths {
    LaunchPaths {
        script: script.into(),
        entry_dir: PathBuf::from("/data/scripts/demo"),
        invoke_cwd: invoke_cwd.into(),
    }
}

fn assembly(args: &[&str]) -> Assembly {
    let args = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    Assembly {
        args: args.clone(),
        masked_args: args,
        ..Assembly::default()
    }
}

#[test]
fn test_python_command_uses_uv_run_script() {
    let mut python = entry("python");
    python.meta.workdir = "invoke".to_owned();
    let script = PathBuf::from("/data/scripts/demo/script.py");
    let probe = FakeProbe {
        programs: BTreeMap::from([("uv".to_owned(), PathBuf::from("/fake/uv"))]),
        files: vec![script.clone()],
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    let plan = build_launch_plan(
        &python,
        &paths(&script, "/invoke"),
        &assembly(&["--x", "1"]),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("/fake/uv"));
    assert_eq!(
        plan.args,
        [
            "run",
            "--no-project",
            "--script",
            "/data/scripts/demo/script.py",
            "--x",
            "1",
        ]
    );
}

#[test]
fn test_python_with_deps_and_python_version() {
    let mut python = entry("python");
    python.meta.workdir = "invoke".to_owned();
    let mut settings = EntrySettings::default();
    settings.requires_python = ">=3.11".to_owned();
    settings.dependencies = vec!["requests".to_owned(), "rich".to_owned()];
    settings.write_to_meta(&mut python.meta);
    let script = PathBuf::from("/data/scripts/demo/script.py");
    let probe = FakeProbe {
        programs: BTreeMap::from([("uv".to_owned(), PathBuf::from("/uv"))]),
        files: vec![script.clone()],
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    let plan = build_launch_plan(
        &python,
        &paths(script, "/invoke"),
        &Assembly::default(),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert!(
        plan.args
            .windows(2)
            .any(|pair| pair == ["--python", ">=3.11"])
    );
    assert_eq!(
        plan.args
            .iter()
            .filter(|arg| arg.as_str() == "--with")
            .count(),
        2
    );
    assert!(
        plan.args
            .windows(2)
            .any(|pair| pair == ["--with", "requests"])
    );
    assert!(plan.args.windows(2).any(|pair| pair == ["--with", "rich"]));
}

#[test]
fn test_workdir_origin_is_source_parent() {
    let mut python = entry("python");
    python.meta.mode = StorageMode::Reference;
    python.meta.source = "/origin/project/s.py".to_owned();
    python.meta.workdir = "origin".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/origin/project")],
        ..FakeProbe::default()
    };

    assert_eq!(
        resolve_launch_workdir(&python, &paths("/unused", "/invoke"), &probe).unwrap(),
        PathBuf::from("/origin/project")
    );
}

#[test]
fn test_workdir_store_and_invoke() {
    let mut python = entry("python");
    let probe = FakeProbe {
        dirs: vec![
            PathBuf::from("/data/scripts/demo"),
            PathBuf::from("/invoke"),
        ],
        ..FakeProbe::default()
    };

    python.meta.workdir = "store".to_owned();
    assert_eq!(
        resolve_launch_workdir(&python, &paths("/unused", "/invoke"), &probe).unwrap(),
        PathBuf::from("/data/scripts/demo")
    );

    python.meta.workdir = "invoke".to_owned();
    assert_eq!(
        resolve_launch_workdir(&python, &paths("/unused", "/invoke"), &probe).unwrap(),
        PathBuf::from("/invoke")
    );
}

#[test]
fn test_workdir_origin_no_source_falls_back_to_cwd() {
    let mut python = entry("python");
    python.meta.source.clear();
    python.meta.workdir = "origin".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    assert_eq!(
        resolve_launch_workdir(&python, &paths("/unused", "/invoke"), &probe).unwrap(),
        PathBuf::from("/invoke")
    );
}

#[test]
fn test_workdir_absolute_path_used_directly() {
    let mut python = entry("python");
    python.meta.workdir = "/custom/work".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/custom/work")],
        ..FakeProbe::default()
    };

    assert_eq!(
        resolve_launch_workdir(&python, &paths("/unused", "/invoke"), &probe).unwrap(),
        PathBuf::from("/custom/work")
    );
}

#[test]
fn test_run_entry_missing_workdir_raises() {
    let mut python = entry("python");
    python.meta.workdir = "/nonexistent/path/that/does/not/exist".to_owned();

    assert!(matches!(
        resolve_launch_workdir(&python, &paths("/unused", "/invoke"), &FakeProbe::default(),)
            .unwrap_err(),
        LaunchError::WorkdirMissing { .. }
    ));
}

#[test]
fn test_exe_missing_source_raises() {
    let mut exe = entry("exe");
    exe.meta.source = "/missing/tool".to_owned();
    exe.meta.workdir = "invoke".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    assert!(matches!(
        build_launch_plan(
            &exe,
            &paths("/unused", "/invoke"),
            &Assembly::default(),
            None,
            None,
            &probe,
        )
        .unwrap_err(),
        LaunchError::TargetMissing { .. }
    ));
}

#[test]
fn test_exe_directory_source_refused_as_not_executable() {
    let mut exe = entry("exe");
    exe.meta.source = "/Bundle.app".to_owned();
    exe.meta.workdir = "invoke".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke"), PathBuf::from("/Bundle.app")],
        ..FakeProbe::default()
    };

    assert!(matches!(
        build_launch_plan(
            &exe,
            &paths("/unused", "/invoke"),
            &Assembly::default(),
            None,
            None,
            &probe,
        )
        .unwrap_err(),
        LaunchError::TargetNotExecutable { .. }
    ));
}

#[test]
fn test_build_command_unknown_kind_raises() {
    let mut unknown = entry("future-kind");
    unknown.meta.workdir = "invoke".to_owned();
    let probe = FakeProbe {
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    assert!(matches!(
        build_launch_plan(
            &unknown,
            &paths("/unused", "/invoke"),
            &Assembly::default(),
            None,
            None,
            &probe,
        )
        .unwrap_err(),
        LaunchError::UnknownKind { .. }
    ));
}

#[test]
fn test_command_template_appends_extra_args() {
    let mut command = entry("command");
    command.meta.workdir = "invoke".to_owned();
    let mut settings = EntrySettings::default();
    settings.template = "echo hello".to_owned();
    settings.write_to_meta(&mut command.meta);
    #[cfg(windows)]
    let programs = BTreeMap::from([("cmd.exe".to_owned(), PathBuf::from("cmd.exe"))]);
    #[cfg(not(windows))]
    let programs = BTreeMap::from([("sh".to_owned(), PathBuf::from("/bin/sh"))]);
    let probe = FakeProbe {
        programs,
        dirs: vec![PathBuf::from("/invoke")],
        ..FakeProbe::default()
    };

    let plan = build_launch_plan(
        &command,
        &paths("/unused", "/invoke"),
        &assembly(&["world"]),
        None,
        None,
        &probe,
    )
    .unwrap();

    assert!(plan.args.iter().any(|arg| arg.contains("hello")));
    assert!(plan.args.iter().any(|arg| arg.contains("world")));
}
