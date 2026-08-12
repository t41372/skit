//! Runtime-plan ports from Python `tests/test_review_fixes.py` at `main@206f9ef`.
//!
//! The Python tests called `launcher.build_command`. Rust's public equivalent is the immutable
//! `LaunchPlan`: these tests inspect its real program/argv, not a helper recreated in the test.

use std::{collections::BTreeMap, path::{Path, PathBuf}};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{LaunchPaths, ProgramProbe, build_launch_preview};

#[derive(Clone, Copy, Debug)]
struct PreviewFs;

impl ProgramProbe for PreviewFs {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(name))
    }

    fn is_file(&self, _path: &Path) -> bool {
        true
    }

    fn is_dir(&self, _path: &Path) -> bool {
        true
    }

    fn exists(&self, _path: &Path) -> bool {
        true
    }

    fn is_executable(&self, _path: &Path) -> bool {
        true
    }
}

fn entry(kind: &str, name: &str, settings: EntrySettings) -> Entry {
    let kind = EntryKind::parse(kind).unwrap();
    let mut meta = EntryMeta::minimal(name, kind);
    meta.workdir = "invoke".to_owned();
    settings.write_to_meta(&mut meta);
    Entry {
        slug: Slug::from_display_name(name),
        meta,
    }
}

fn paths(script: &str) -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from(script),
        entry_dir: PathBuf::from("entry"),
        invoke_cwd: PathBuf::from("invoke"),
    }
}

fn command_string(plan: &skit_runtime::LaunchPlan) -> &str {
    plan.args
        .last()
        .expect("command entries pass one complete shell command as the last shell argument")
}

#[test]
fn test_escaped_placeholder_not_substituted() {
    let settings = EntrySettings {
        template: "echo {{name}} {name}".to_owned(),
        params: vec!["name".to_owned()],
        ..EntrySettings::default()
    };
    let entry = entry("command", "esc", settings);
    let values = BTreeMap::from([("name".to_owned(), "X".to_owned())]);
    let assembly = Assembly {
        command_values: values.clone(),
        masked_command_values: values,
        ..Assembly::default()
    };

    let plan = build_launch_preview(&entry, &paths(""), &assembly, None, None, None, &PreviewFs)
        .unwrap();

    assert_eq!(command_string(&plan), "echo {name} X");
}

#[test]
fn test_escape_unescaped_even_without_params() {
    let settings = EntrySettings {
        template: "echo {{literal}}".to_owned(),
        ..EntrySettings::default()
    };
    let entry = entry("command", "noparams", settings);

    let plan = build_launch_preview(
        &entry,
        &paths(""),
        &Assembly::default(),
        None,
        None,
        None,
        &PreviewFs,
    )
    .unwrap();

    assert_eq!(command_string(&plan), "echo {literal}");
}

#[cfg(not(windows))]
#[test]
fn test_extra_args_quoted_for_posix_shell() {
    let settings = EntrySettings {
        template: "echo hi".to_owned(),
        ..EntrySettings::default()
    };
    let entry = entry("command", "quoting", settings);
    let values = vec!["$HOME".to_owned(), "a b".to_owned(), "`whoami`".to_owned()];
    let assembly = Assembly {
        args: values.clone(),
        masked_args: values,
        ..Assembly::default()
    };

    let plan = build_launch_preview(&entry, &paths(""), &assembly, None, None, None, &PreviewFs)
        .unwrap();
    let command = command_string(&plan);

    assert!(command.contains("'$HOME'"), "{command}");
    assert!(command.contains("'a b'"), "{command}");
    assert!(command.contains("'`whoami`'"), "{command}");
}

#[test]
fn test_build_python_only_requires_python() {
    let settings = EntrySettings {
        requires_python: ">=3.11".to_owned(),
        ..EntrySettings::default()
    };
    let entry = entry("python", "only-python", settings);
    let plan = build_launch_preview(
        &entry,
        &paths("script.py"),
        &Assembly::default(),
        None,
        None,
        None,
        &PreviewFs,
    )
    .unwrap();

    assert_eq!(plan.program, PathBuf::from("uv"));
    assert_eq!(
        plan.args,
        ["run", "--no-project", "--python", ">=3.11", "--script", "script.py"]
            .map(str::to_owned)
    );
    assert!(!plan.args.iter().any(|arg| arg == "--with"));
}
