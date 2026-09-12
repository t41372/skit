//! Runtime target port from Python v0.4 `tests/test_store.py`.

use std::path::{Path, PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug, StorageMode};
use skit_runtime::{LaunchError, LaunchPaths, ProgramProbe, build_launch_plan};

#[derive(Debug)]
struct Probe {
    entry_dir: PathBuf,
}

impl ProgramProbe for Probe {
    fn find_program(&self, _name: &str) -> Option<PathBuf> {
        None
    }

    fn is_file(&self, _path: &Path) -> bool {
        false
    }

    fn is_dir(&self, path: &Path) -> bool {
        path == self.entry_dir
    }

    fn is_executable(&self, _path: &Path) -> bool {
        false
    }
}

#[test]
fn test_a_copy_mode_exe_meta_still_reports_its_gone_binary() {
    let source = PathBuf::from("/gone/tool");
    let entry_dir = PathBuf::from("/data/scripts/binary");
    let mut meta = EntryMeta::minimal("binary", EntryKind::parse("exe").unwrap());
    meta.mode = StorageMode::Copy; // the same hand-edited malformed shape as the Python oracle
    meta.source = source.display().to_string();
    meta.workdir = "store".to_owned();
    let entry = Entry {
        slug: Slug::parse("binary").unwrap(),
        meta,
    };
    let paths = LaunchPaths {
        // A mode-derived implementation might incorrectly trust this existing entry directory.
        script: entry_dir.clone(),
        entry_dir: entry_dir.clone(),
        invoke_cwd: PathBuf::from("/invoke"),
    };
    let error = build_launch_plan(
        &entry,
        &paths,
        &Assembly::default(),
        None,
        None,
        &Probe { entry_dir },
    )
    .unwrap_err();
    assert!(
        matches!(&error, LaunchError::TargetMissing { path } if path == &source),
        "direct executable target was derived from mode/script_path instead of source: {error:?}"
    );
}
