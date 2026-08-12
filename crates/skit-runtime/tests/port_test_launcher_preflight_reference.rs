//! Reference-mode missing-origin preflight contract from Python `tests/test_launcher.py` at
//! `main@206f9ef`.
//!
//! Python reports the missing referenced script itself even though its parent directory disappeared.
//! Rust's public preview/preflight seam currently resolves the workdir first; this test deliberately
//! keeps the Python error priority instead of accepting any earlier failure as equivalent.

use std::path::{Path, PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug, StorageMode};
use skit_runtime::{LaunchError, LaunchPaths, ProgramProbe, build_launch_preview};

#[derive(Debug)]
struct MissingReferenceProbe;

impl ProgramProbe for MissingReferenceProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(name))
    }

    fn is_file(&self, _path: &Path) -> bool {
        false
    }

    fn is_dir(&self, path: &Path) -> bool {
        path == Path::new("/invoke")
    }

    fn is_executable(&self, _path: &Path) -> bool {
        false
    }
}

#[test]
fn test_preflight_reference_mode_still_raises_on_missing_script_when_origin_gone() {
    let mut meta = EntryMeta::minimal("Reference", EntryKind::parse("python").unwrap());
    meta.mode = StorageMode::Reference;
    meta.source = "/gone/refdir/ref.py".to_owned();
    meta.workdir = "origin".to_owned();
    let entry = Entry {
        slug: Slug::parse("reference").unwrap(),
        meta,
    };
    let paths = LaunchPaths {
        script: PathBuf::from("/gone/refdir/ref.py"),
        entry_dir: PathBuf::from("/data/scripts/reference"),
        invoke_cwd: PathBuf::from("/invoke"),
    };

    let error = build_launch_preview(
        &entry,
        &paths,
        &Assembly::default(),
        None,
        None,
        None,
        &MissingReferenceProbe,
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            LaunchError::TargetMissing { ref path }
                if path == Path::new("/gone/refdir/ref.py")
        ),
        "Python reports the missing referenced script, not an earlier workdir error: {error}"
    );
}
