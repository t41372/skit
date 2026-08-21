use std::fs;

use skit_application::library_detail::{LibraryDetailRepository as _, LibraryTargetState};
use skit_domain::Slug;
use skit_store::FileStore;
use tempfile::TempDir;

fn write_meta(root: &TempDir, slug: &str, body: &str) {
    let directory = root.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("meta.toml"), body).unwrap();
}

#[test]
fn detail_snapshots_keep_source_bytes_and_storage_states_adapter_owned() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "copy",
        concat!(
            "schema = 1\nname = \"Copy\"\nkind = \"shell\"\nmode = \"copy\"\n",
            "source = \"/missing/original.sh\"\nworkdir = \"invoke\"\ndescription = \"\"\n",
        ),
    );
    let source = b"printf '\xff'\r\n";
    fs::write(root.path().join("scripts/copy/script.sh"), source).unwrap();

    write_meta(
        &root,
        "missing",
        concat!(
            "schema = 1\nname = \"Missing\"\nkind = \"shell\"\nmode = \"reference\"\n",
            "source = \"/definitely/missing/skit-library.sh\"\nworkdir = \"origin\"\n",
            "description = \"\"\n",
        ),
    );
    write_meta(
        &root,
        "future",
        concat!(
            "schema = 1\nname = \"Future\"\nkind = \"martian\"\nmode = \"reference\"\n",
            "source = \"/definitely/missing/future\"\nworkdir = \"origin\"\ndescription = \"\"\n",
        ),
    );

    let snapshots = FileStore::new(root.path()).detail_snapshots().unwrap();
    let by_slug = |slug: &str| {
        snapshots
            .iter()
            .find(|snapshot| snapshot.entry.slug == Slug::parse(slug).unwrap())
            .unwrap()
    };

    let copy = by_slug("copy");
    assert_eq!(copy.source.as_deref(), Some(source.as_slice()));
    assert_eq!(copy.target, LibraryTargetState::Present);
    assert!(!copy.original_source_exists);

    let missing = by_slug("missing");
    assert!(missing.source.is_none());
    assert_eq!(
        missing.target,
        LibraryTargetState::Missing("/definitely/missing/skit-library.sh".into())
    );

    let future = by_slug("future");
    assert_eq!(future.target, LibraryTargetState::NotApplicable);
}
