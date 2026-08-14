use std::{fs, path::PathBuf};

use skit_domain::{Entry, EntryKind, EntryMeta, Slug, StorageMode};
use skit_runtime::{LaunchPaths, SystemProbe, resolve_launch_workdir};

fn entry() -> Entry {
    Entry {
        slug: Slug::parse("job").expect("slug"),
        meta: EntryMeta::minimal("job", EntryKind::parse("python").expect("python kind")),
    }
}

fn paths(root: &std::path::Path, script: PathBuf, invoke_cwd: PathBuf) -> LaunchPaths {
    LaunchPaths {
        script,
        entry_dir: root.join("store/job"),
        invoke_cwd,
    }
}

#[test]
fn test_for_entry_resolves_the_entry_workdir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("job.py");
    fs::write(&source, b"print('hi')\n").expect("source");
    let invoke_cwd = std::env::current_dir().expect("cwd");
    let mut entry = entry();
    entry.meta.source = source.to_string_lossy().into_owned();
    entry.meta.workdir = temp.path().to_string_lossy().into_owned();
    let launch_paths = paths(temp.path(), source, invoke_cwd.clone());

    assert_eq!(
        resolve_launch_workdir(&entry, &launch_paths, &SystemProbe).expect("explicit workdir"),
        temp.path()
    );
    assert_eq!(
        launch_paths.invoke_cwd, invoke_cwd,
        "the TUI context must retain the process invocation directory beside the resolved workdir"
    );
}

#[test]
fn test_for_entry_reference_entry_roots_at_its_origin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let origin = temp.path().join("proj");
    fs::create_dir(&origin).expect("origin");
    let source = origin.join("job.py");
    fs::write(&source, b"print('hi')\n").expect("source");
    let mut entry = entry();
    entry.meta.mode = StorageMode::Reference;
    entry.meta.source = source.to_string_lossy().into_owned();
    entry.meta.workdir = "origin".to_owned();
    let launch_paths = paths(temp.path(), source, temp.path().to_path_buf());

    assert_eq!(
        resolve_launch_workdir(&entry, &launch_paths, &SystemProbe).expect("reference origin"),
        origin
    );
}

#[test]
fn test_vanished_origin_reference_entry_degrades() {
    let temp = tempfile::tempdir().expect("tempdir");
    let parent = temp.path().join("proj");
    let origin = parent.join("deep");
    fs::create_dir_all(&origin).expect("origin");
    let source = origin.join("job.py");
    fs::write(&source, b"print('hi')\n").expect("source");
    let mut entry = entry();
    entry.meta.mode = StorageMode::Reference;
    entry.meta.source = source.to_string_lossy().into_owned();
    entry.meta.workdir = "origin".to_owned();
    let launch_paths = paths(temp.path(), source.clone(), temp.path().to_path_buf());

    fs::remove_file(&source).expect("remove source");
    fs::remove_dir(&origin).expect("remove vanished origin");

    assert_eq!(
        resolve_launch_workdir(&entry, &launch_paths, &SystemProbe)
            .expect("a vanished reference origin must remain available to the path TUI as a degraded root"),
        origin,
        "reference mode does not silently recover its semantic workdir to invoke_cwd"
    );
    assert!(parent.is_dir(), "the picker has a nearest existing ancestor to fall back to");
}
