use std::str::FromStr as _;

use skit_benchmarks::{BenchmarkProfile, pipeline::prepare_datasets};
use tempfile::TempDir;

#[test]
fn profile_parser_accepts_only_the_public_profile_names() {
    assert_eq!(
        BenchmarkProfile::from_str("pr").unwrap(),
        BenchmarkProfile::Pr
    );
    assert_eq!(
        BenchmarkProfile::from_str("compare").unwrap(),
        BenchmarkProfile::Compare
    );
    assert!(BenchmarkProfile::from_str("quick").is_err());
}

#[test]
fn dataset_preparation_reuses_verified_manifests() {
    let root = TempDir::new().unwrap();
    let first = prepare_datasets(root.path(), &[0, 2]).unwrap();
    let second = prepare_datasets(root.path(), &[0, 2]).unwrap();
    assert_eq!(first[&2].slugs, second[&2].slugs);
    assert_eq!(first[&0].root, root.path().join("datasets/n0"));
}

#[cfg(unix)]
#[test]
fn dataset_preparation_refuses_a_symlinked_dataset_root() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    symlink(outside.path(), root.path().join("datasets")).unwrap();

    assert!(prepare_datasets(root.path(), &[0]).is_err());
    assert!(!outside.path().join("n0").exists());
}
