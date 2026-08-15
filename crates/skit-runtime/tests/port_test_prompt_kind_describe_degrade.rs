use std::path::PathBuf;

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, Slug};
use skit_runtime::{LaunchPaths, ProgramProbe, PromptRunner, build_launch_preview};

#[derive(Debug)]
struct Probe;

impl ProgramProbe for Probe {
    fn find_program(&self, _name: &str) -> Option<PathBuf> {
        None
    }

    fn is_file(&self, path: &std::path::Path) -> bool {
        path == std::path::Path::new("/data/scripts/p/prompt.md")
    }

    fn is_dir(&self, path: &std::path::Path) -> bool {
        matches!(path.to_str(), Some("/invoke" | "/data/scripts/p"))
    }

    fn is_executable(&self, _path: &std::path::Path) -> bool {
        true
    }
}

fn entry() -> Entry {
    let mut meta = EntryMeta::minimal("p", EntryKind::parse("prompt").unwrap());
    meta.workdir = "invoke".to_owned();
    Entry {
        slug: Slug::parse("p").unwrap(),
        meta,
    }
}

fn paths() -> LaunchPaths {
    LaunchPaths {
        script: PathBuf::from("/data/scripts/p/prompt.md"),
        entry_dir: PathBuf::from("/data/scripts/p"),
        invoke_cwd: PathBuf::from("/invoke"),
    }
}

#[test]
fn test_describe_degrades_on_missing_body_and_missing_values() {
    let runner = PromptRunner {
        name: "rec".to_owned(),
        argv: vec![
            "rec-bin".to_owned(),
            "--prompt={{prompt}}".to_owned(),
        ],
    };

    // Python's describe path catches either a missing managed value or an unreadable body and
    // deliberately falls back to the configured runner template. Rust's public preview boundary
    // receives an already prepared body rather than those private exceptions, so `None` is the
    // exact equivalent state: no rendered body is available to describe. It must still be stable
    // and useful instead of turning a harmless description into a launch failure.
    let preview = build_launch_preview(
        &entry(),
        &paths(),
        &Assembly::default(),
        None,
        None,
        Some(&runner),
        &Probe,
    )
    .expect("describe/preview must degrade to the runner template when no rendered body is available");

    assert!(preview.display.contains("rec-bin"), "{}", preview.display);
    assert!(
        preview.display.contains("{{prompt}}"),
        "the stable fallback must show the unresolved prompt slot: {}",
        preview.display
    );
}
