//! Exact executable accounting for completed small Python behavior modules at `main@206f9ef`.
//!
//! These checks are deliberately separate from behavioral coverage. They parse the already-ported
//! Rust sources and require the functions whose own attributes contain `#[test]` to be exactly the
//! frozen Python contract names. A helper, comment, or stray test attribute cannot satisfy them.

use std::{fs, path::Path};

use syn::{Attribute, Item};

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn executable_names(relative: &str) -> Vec<String> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let path = repo.join(relative);
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("{} is not valid Rust: {error}", path.display()))
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn assert_exact(relative: &str, expected: &[&str]) {
    let actual = executable_names(relative);
    let expected = expected
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "{relative} must contain exactly the frozen Python executable tests, in order"
    );
}

#[test]
fn callmatch_has_all_9_frozen_python_tests() {
    assert_exact(
        "crates/skit-language/tests/port_test_callmatch.rs",
        &[
            "test_equal_count_duplicate_prompts_bind_in_positional_order",
            "test_duplicate_prompt_gone_from_source_falls_back_to_position_ambiguous",
            "test_promptless_entry_cannot_recover_a_site_the_multiset_pass_claimed",
            "test_unique_prompt_after_a_multiset_match_still_resolves",
            "test_single_shared_prompt_resolves_by_uniqueness_not_multiset",
            "test_no_recorded_prompt_falls_back_to_position_silently",
            "test_renamed_prompt_with_a_call_still_at_position_is_flagged_ambiguous",
            "test_missing_when_neither_prompt_nor_position_resolves",
            "test_current_site_with_dynamic_prompt_is_ignored_for_prompt_matching",
        ],
    );
}

#[test]
fn analyzer_signals_has_all_9_frozen_python_tests() {
    assert_exact(
        "crates/skit-language/tests/port_test_analyzer_signals.rs",
        &[
            "test_accumulator_is_demoted",
            "test_clean_constant_is_not_demoted",
            "test_reassignment_inside_while_loop_demotes",
            "test_augassign_outside_loop_still_demotes",
            "test_uses_argv_detected",
            "test_filename_literal_hint_found",
            "test_no_hint_for_named_constant_usage",
            "test_hint_excludes_non_filenames",
            "test_hint_dedupes_and_caps_at_three",
        ],
    );
}

#[test]
fn rewrite_has_both_frozen_python_tests() {
    assert_exact(
        "crates/skit-language/tests/port_test_rewrite.rs",
        &[
            "test_detect_newline_prefers_crlf_then_lone_cr_then_lf",
            "test_restore_newline_is_a_no_op_for_lf_and_exact_otherwise",
        ],
    );
}

#[test]
fn argv_text_has_the_frozen_python_test() {
    assert_exact(
        "crates/skit-application/tests/port_test_argv_text.rs",
        &["test_windows_split_ignores_separator_only_tail"],
    );
}

#[test]
fn corpus_has_all_11_frozen_python_tests() {
    assert_exact(
        "crates/skit-language/tests/port_test_corpus.rs",
        &[
            "test_analyzer_never_raises",
            "test_metawriter_byte_fidelity",
            "test_block_roundtrip_preserves_shebang",
            "test_shim_no_values_is_identity",
            "test_shim_full_injection_compiles",
            "test_shell_inject_no_values_writes_nothing",
            "test_shell_full_injection_reparses",
            "test_js_analyzer_never_raises",
            "test_js_block_byte_fidelity",
            "test_js_inject_no_values_is_identity",
            "test_js_full_injection_reparses",
        ],
    );
}

#[test]
fn kindnames_has_all_5_frozen_python_tests_across_i18n_and_tui() {
    assert_exact(
        "crates/skit-i18n/tests/port_test_kindnames.rs",
        &[
            "test_kind_label_maps_each_registered_kind",
            "test_every_known_kind_has_a_dedicated_label",
            "test_unknown_kind_falls_through_to_its_raw_id",
        ],
    );
    assert_exact(
        "crates/skit-tui/tests/port_test_kindnames.rs",
        &[
            "test_kind_choices_exact_options_and_order",
            "test_kind_choices_offer_exe_false_drops_only_exe",
        ],
    );
}

#[test]
fn tui_nav_has_all_5_positive_pilots() {
    assert_exact(
        "crates/skit-tui/tests/port_test_tui_nav.rs",
        &["test_run_form_boots_typeable_and_arrows_walk_the_fields"],
    );
    assert_exact(
        "crates/skit-tui/tests/port_test_tui_nav_add.rs",
        &[
            "test_add_source_arrows_walk_path_template_name",
            "test_add_review_boots_on_name_and_arrows_move",
        ],
    );
    assert_exact(
        "crates/skit-tui/tests/port_test_tui_nav_preferences_settings.rs",
        &[
            "test_prefs_boots_on_language_and_arrows_move",
            "test_settings_boots_on_name_and_arrows_move",
        ],
    );
}
