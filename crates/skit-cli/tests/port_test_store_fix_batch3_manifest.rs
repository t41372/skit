//! Exact and unique accounting for the 13 Batch 3 `test_store_fix.py` owners.

use std::{collections::BTreeMap, fs, path::Path};

const EXPECTED: [&str; 13] = [
    "test_add_python_copy_mode_defaults_workdir_to_invoke",
    "test_add_python_reference_mode_still_defaults_workdir_to_origin",
    "test_add_python_non_utf8_source_skips_injection_keeps_deps_in_meta",
    "test_add_python_utf8_source_still_injects_normally",
    "test_update_dependencies_copy_non_utf8_leaves_stored_copy_byte_identical",
    "test_update_dependencies_copy_utf8_syncs_block_and_stays_utf8",
    "test_update_dependencies_copy_sync_swallows_read_oserror",
    "test_update_dependencies_refuses_when_a_non_utf8_copy_carries_its_own_block",
    "test_update_dependencies_python_unpin_is_refused_for_the_same_copy",
    "test_update_dependencies_untouched_axes_never_reach_the_refusal",
    "test_deps_edit_on_a_crlf_copy_keeps_one_block_and_its_params",
    "test_add_with_deps_does_not_double_block_a_crlf_script",
    "test_add_keeps_an_lf_script_lf_when_injecting_a_block",
];

fn scan(path: &Path, found: &mut BTreeMap<String, Vec<(String, bool)>>) {
    for item in fs::read_dir(path).unwrap() {
        let item = item.unwrap();
        let path = item.path();
        if item.file_type().unwrap().is_dir() {
            scan(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = fs::read_to_string(&path).unwrap();
            let mut ignored = false;
            for line in source.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("#[ignore") {
                    ignored = true;
                }
                if let Some((name, _)) = trimmed
                    .strip_prefix("fn ")
                    .and_then(|rest| rest.split_once('('))
                {
                    if EXPECTED.contains(&name) {
                        found
                            .entry(name.to_owned())
                            .or_default()
                            .push((path.display().to_string(), ignored));
                    }
                    ignored = false;
                }
            }
        }
    }
}

#[test]
fn test_store_fix_batch3_names_are_exactly_and_uniquely_owned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = BTreeMap::new();
    scan(&root.join("crates"), &mut found);
    let expected = EXPECTED
        .into_iter()
        .map(|name| (name.to_owned(), 1_usize))
        .collect::<BTreeMap<_, _>>();
    let actual = found
        .iter()
        .map(|(name, owners)| (name.clone(), owners.len()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual, expected, "Batch 3 exact owners drifted: {found:#?}");
    assert!(
        found.values().flatten().all(|(_, ignored)| !ignored),
        "Batch 3 owners must stay executable: {found:#?}"
    );
}
