use std::fs;

use skit_i18n::Locale;
use skit_store::{FileStore, RegistryRebuildProblem};
use tempfile::TempDir;

fn write_meta(root: &TempDir, slug: &str, body: &str) {
    let directory = root.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("meta.toml"), body).unwrap();
}

#[test]
fn rebuild_reports_each_skipped_directory_and_missing_reference_in_scan_order() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("scripts/a-missing")).unwrap();
    write_meta(&root, "b-corrupt", "name = [not valid");
    write_meta(
        &root,
        "c-reference",
        concat!(
            "name = \"Reference\"\n",
            "kind = \"python\"\n",
            "mode = \"reference\"\n",
            "source = \"/definitely/missing/source.py\"\n",
        ),
    );
    let store = FileStore::new(root.path());

    let report = store.rebuild_registry_report().unwrap();

    assert_eq!(report.entry_count, 1);
    assert!(matches!(
        &report.problems[0],
        RegistryRebuildProblem::MissingMetadata { slug } if slug == "a-missing"
    ));
    assert!(matches!(
        &report.problems[1],
        RegistryRebuildProblem::CorruptMetadata { slug, .. } if slug == "b-corrupt"
    ));
    assert_eq!(
        report.problems[2],
        RegistryRebuildProblem::MissingReferenceSource {
            slug: "c-reference".to_owned(),
            path: "/definitely/missing/source.py".to_owned(),
        }
    );
    assert_eq!(
        report.problems[0].message().localize(Locale::En),
        "a-missing: meta.toml is missing; skipped"
    );
    assert!(
        report.problems[1]
            .message()
            .localize(Locale::En)
            .starts_with("b-corrupt: meta.toml is corrupt (")
    );
    assert_eq!(
        report.problems[2].message().localize(Locale::En),
        "c-reference: the referenced source file is gone: /definitely/missing/source.py"
    );
}

#[test]
fn count_only_rebuild_keeps_its_existing_api() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "valid",
        "name = \"Valid\"\nkind = \"shell\"\nmode = \"copy\"\n",
    );
    let store = FileStore::new(root.path());

    assert_eq!(store.rebuild_registry().unwrap(), 1);
}
