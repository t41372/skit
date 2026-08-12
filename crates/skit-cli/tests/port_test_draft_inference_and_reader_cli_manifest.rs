//! Mechanical completeness guard for Python `tests/test_draft_inference_and_reader_cli.py` at
//! `main@206f9ef`. Every frozen Python test has one executable Rust oracle in the dedicated target;
//! this module has no architecture exceptions.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGET: &str = "crates/skit-cli/tests/port_test_draft_inference_and_reader_cli.rs";

const EXPECTED: &[&str] = &[
    "test_python_version_pin_rows",
    "test_kind_for_draft_shebang_first",
    "test_is_draft_needs_both_dir_and_prefix",
    "test_reader_fields_predicate_rows",
    "test_cli_add_bash_shebang_draft_lands_as_shell_and_unlinks",
    "test_cli_add_awk_shebang_draft_is_unknown_kept_with_kind_escape",
    "test_cli_add_no_shebang_draft_falls_back_to_python",
    "test_cli_add_bash_shebang_py_outside_drafts_stays_python",
    "test_cli_add_parked_user_file_in_drafts_dir_is_not_unlinked",
    "test_stdin_python2_shebang_is_refused",
    "test_path_add_python2_extensionless_is_refused",
    "test_stdin_versioned_shebang_pins_requires_python_and_announces",
    "test_explicit_python_beats_the_shebang_pin_silently",
    "test_existing_pep723_block_beats_the_shebang_pin_silently",
    "test_dep_flag_present_still_pins_from_the_shebang",
    "test_suggested_deps_noninteractive_pins_from_the_shebang",
    "test_onboard_script_params_returns_empty_for_analyzerless_kind",
    "test_docopt_python_read_view_offers_manage",
    "test_docopt_python_manage_prints_no_flip_note",
    "test_dynamic_getopts_read_view_offers_manage",
    "test_dynamic_getopts_manage_prints_no_flip_note",
    "test_reference_getopts_read_view_has_no_manage_advice",
    "test_reference_constants_read_view_names_unmanaged_with_teaching",
    "test_reference_reader_add_prints_the_read_notice",
    "test_reference_constants_add_prints_the_skip_line",
    "test_one_field_getopts_add_says_singular",
    "test_multi_field_getopts_add_says_plural",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_draft_inference_and_reader_cli_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXPECTED.len(), 27);
    assert_eq!(
        EXPECTED.iter().copied().collect::<BTreeSet<_>>().len(),
        27,
        "duplicate expected names could hide a missing Python contract"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source = fs::read_to_string(repo.join(TARGET)).unwrap();
    let syntax = syn::parse_file(&source).unwrap();
    let actual = syntax
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let expected = EXPECTED
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        actual, expected,
        "draft-inference/reader contract target gained, lost, renamed, or merged a frozen oracle"
    );
}
