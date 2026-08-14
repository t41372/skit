//! Exact frozen-name accounting for `main@206f9ef:tests/test_path_tui.py`.
//!
//! Executable owners may fail against the current Rust implementation. That is a parity finding,
//! not a reason to weaken or close them. The five closed names below are limited to Python-private
//! helper seams whose observable behavior is separately pinned through real Rust TUI/filesystem
//! boundaries: three injected `os.scandir`/`DirEntry` fault seams and two direct `looks_pathy()`
//! helper probes. This allowlist is intentionally fixed and cannot silently grow.

use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

use syn::{Attribute, Item};

const FROZEN: &[&str] = &[
    "test_path_field_completes_bare_prefix_at_workdir",
    "test_str_field_needs_pathy_text",
    "test_secretless_activation_never_guesses_beyond_prefix",
    "test_hidden_entries_only_behind_a_dot_prefix",
    "test_cwd_token_completes_at_invoke_cwd_not_workdir",
    "test_unset_env_token_is_silence_not_a_traceback",
    "test_relative_env_token_falls_back_to_the_workdir_rule",
    "test_home_prefix_completes_inside_home",
    "test_missing_workdir_silences_bare_completion",
    "test_missing_workdir_silences_relative_token_lookup",
    "test_shlexy_field_completes_only_the_trailing_piece",
    "test_scan_cap_stops_the_scan_exactly",
    "test_scan_degrades_on_oserror",
    "test_unstatable_entry_is_treated_as_a_file",
    "test_for_entry_resolves_the_entry_workdir",
    "test_for_entry_reference_entry_roots_at_its_origin",
    "test_vanished_origin_reference_entry_degrades",
    "test_picker_start_last_resort_is_the_invoke_cwd",
    "test_picker_start_degrades_to_nearest_existing_ancestor",
    "test_value_for_is_relative_inside_the_root_and_posix_everywhere",
    "test_picker_enter_descends_then_picks_and_filter_clears",
    "test_picker_use_this_directory_row_by_real_keys",
    "test_picker_arrows_steer_highlight_without_leaving_the_filter",
    "test_picker_prefix_matches_outrank_substring_hits",
    "test_picker_filter_is_case_insensitive_substring",
    "test_picker_row_click_is_the_mouse_path",
    "test_picker_zero_match_enter_is_a_noop",
    "test_picker_filtering_hides_the_pinned_row",
    "test_picker_backspace_ascends_only_on_empty_filter",
    "test_picker_backspace_noops_at_the_filesystem_root",
    "test_picker_esc_cancels_and_up_chip_is_clickable",
    "test_picker_missing_workdir_opens_at_ancestor_with_notice",
    "test_path_fields_render_hint_and_suggester",
    "test_token_menu_puts_file_row_first_on_path_fields_and_picker_replaces",
    "test_picker_appends_quoted_to_the_extra_args_row",
    "test_picker_appends_quoted_to_a_multiple_field",
    "test_token_rows_still_insert_at_cursor",
    "test_browse_link_renders_on_text_fields_only",
    "test_browse_link_opens_the_picker_directly_and_replaces",
    "test_browse_without_a_key_uses_the_focused_field_and_its_dialect",
    "test_browse_refuses_numeric_secret_and_unknown_rows",
    "test_fieldrow_browsable_needs_a_context",
    "test_fieldrow_shlexy_and_insert_mode_all_branches",
    "test_insert_picked_shapes",
    "test_insert_picked_escapes_glob_metacharacters",
    "test_secret_field_never_gets_a_suggester",
    "test_token_menu_without_context_has_no_file_row",
    "test_looks_pathy_windows_recognition",
    "test_looks_pathy_token_and_separator_spellings",
    "test_suggester_is_case_sensitive_query_not_casefolded",
    "test_suggester_does_not_cache_stale_results",
    "test_brace_escapes_on_a_normal_field_halves_doubled_braces",
    "test_brace_escapes_off_on_a_placeholder_field_keeps_doubled_braces",
    "test_shlexy_trailing_piece_refuses_either_quote",
    "test_bare_token_prefix_without_separator_is_silent",
    "test_list_filtered_reveals_hidden_only_behind_a_dot_filter",
    "test_list_filtered_dir_sorts_before_an_earlier_file_within_a_rank",
    "test_list_filtered_tiebreak_is_case_insensitive",
    "test_picker_pinned_row_shows_its_label",
    "test_picker_empty_directory_highlights_the_pinned_row",
    "test_picker_ascend_repopulates_the_parent_listing",
];

const CLOSED: &[&str] = &[
    // Python shrinks a module constant and injects a fake scandir iterator to prove the exact
    // `scanned >= SCAN_CAP` private-loop boundary. Rust delegates enumeration to
    // ratatui-interact::FileExplorerState and exposes no scan iterator/cap injection seam.
    "test_scan_cap_stops_the_scan_exactly",
    // Python monkeypatches os.scandir to throw before yielding. Rust's explorer owns that I/O and
    // has no injectable reader; missing/degraded roots remain executable elsewhere in this module.
    "test_scan_degrades_on_oserror",
    // Python supplies a fake DirEntry whose is_dir() throws after enumeration. The Rust explorer
    // does not expose a DirEntry metadata hook; real picker/selection behavior remains executable.
    "test_unstatable_entry_is_treated_as_a_file",
    // Direct unit probe of the private Python activation helper. Observable plain-string path
    // activation is pinned by `test_str_field_needs_pathy_text` plus token/root completion tests.
    "test_looks_pathy_windows_recognition",
    // Same private-helper seam: slash/token activation and the bare-word negative are exercised
    // through executable visible-completion contracts rather than a re-invented Rust classifier.
    "test_looks_pathy_token_and_separator_spellings",
];

const OWNER_FILES: &[&str] = &[
    "crates/skit-tui/tests/port_test_path_tui_picker.rs",
    "crates/skit-application/tests/port_test_path_tui_insertion.rs",
    "crates/skit-ui/tests/port_test_path_tui_run.rs",
    "crates/skit-tui/tests/port_test_path_tui_modal_roots.rs",
    "crates/skit-tui/tests/port_test_path_tui_suggestions.rs",
    "crates/skit-tui/tests/port_test_path_tui_suggestion_edges.rs",
    "crates/skit-tui/tests/port_test_path_tui_interactions.rs",
    "crates/skit-runtime/tests/port_test_path_tui_workdir.rs",
    "crates/skit-ui/tests/port_test_path_tui_output.rs",
    "crates/skit-ui/tests/port_test_path_tui_contextless_token.rs",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn parity_tests(path: &Path) -> Vec<String> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let file = syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("could not parse {} as Rust: {error}", path.display()));
    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                let name = function.sig.ident.to_string();
                name.starts_with("test_").then_some(name)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn frozen_path_tui_partition_is_exact() {
    let frozen = FROZEN.iter().copied().collect::<BTreeSet<_>>();
    let closed = CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(FROZEN.len(), 61, "frozen test_path_tui.py denominator drifted");
    assert_eq!(frozen.len(), 61, "duplicate frozen path-TUI name");
    assert_eq!(CLOSED.len(), 5, "architecture closure allowlist may not expand or shrink silently");
    assert_eq!(closed.len(), 5, "duplicate architecture-closed path-TUI name");
    assert!(closed.is_subset(&frozen), "closed names must come from the frozen Python surface");

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives at <repo>/crates/skit-cli");
    let mut owners = BTreeMap::<String, String>::new();
    let mut duplicates = Vec::new();
    for relative in OWNER_FILES {
        let path = repo.join(relative);
        for name in parity_tests(&path) {
            if let Some(previous) = owners.insert(name.clone(), (*relative).to_owned()) {
                duplicates.push(format!("{name}: {previous} and {relative}"));
            }
        }
    }
    assert!(duplicates.is_empty(), "duplicate path-TUI parity owners:\n{}", duplicates.join("\n"));

    let expected = frozen
        .difference(&closed)
        .copied()
        .collect::<BTreeSet<_>>();
    let actual = owners.keys().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 56, "executable path-TUI partition must stay 56/61");
    assert_eq!(actual.len(), 56, "canonical owner files must contain exactly 56 parity tests");

    let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
    let extras = actual.difference(&expected).copied().collect::<Vec<_>>();
    assert!(
        missing.is_empty() && extras.is_empty(),
        "path-TUI exact-name mismatch; missing={missing:?}, extras={extras:?}"
    );
    assert!(
        closed.iter().all(|name| !actual.contains(name)),
        "an architecture-closed name must not also claim an executable owner"
    );
}
