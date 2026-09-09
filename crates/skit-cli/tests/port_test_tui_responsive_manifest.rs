//! Completeness guard for Python `tests/test_tui_responsive.py` at `main@206f9ef`.
//!
//! Seventeen contracts have executable terminal-geometry equivalents. Three Rust-only contracts
//! cover frontend adapter invariants. Two Python contracts assert widget structures that do not
//! exist in the Ratatui frontend and stay architecture-closed rather than being represented by a
//! weaker test of a different widget.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGET: &str = "crates/skit-tui/tests/port_test_tui_responsive.rs";

const EXECUTABLE: &[&str] = &[
    "test_breakpoint_tiers_are_the_documented_contract",
    "test_chip_glues_every_blank_so_the_pill_is_one_word",
    "test_nav_chip_is_exactly_the_two_key_only_pills",
    "test_width_tier_boundary_flips_side_by_side_to_stacked",
    "test_narrow_short_hides_detail_and_tab_pin_survives_resizes",
    "test_tiny_narrow_tab_still_brings_the_pane_back",
    "test_tab_walks_the_pin_states_on_a_wide_terminal_too",
    "test_height_tier_boundaries_flatten_search_then_drop_the_global_row",
    "test_flattened_search_still_filters",
    "test_footer_wraps_between_pills_and_wrapped_chips_stay_clickable",
    "test_portrait_stacks_the_detail_pane_and_uncaps_the_footer",
    "test_short_tier_caps_visible_lines_but_keeps_chips_scroll_reachable",
    "test_prefs_mirror_rows_are_horizontal_until_narrow_and_sentences_always_stack",
    "test_help_overlay_caps_to_a_tiny_screen_and_scrolls_by_key",
    "test_confirm_remove_shrinks_for_a_long_name_on_a_narrow_screen",
    "test_env_picker_fits_input_and_esc_chip_across_the_tiers",
    "test_add_source_fields_stay_reachable_on_short_terminals",
];

const RUST_ADDITIVE: &[&str] = &[
    "test_growing_across_height_tiers_never_shrinks_the_primary_viewport",
    "test_footer_minimum_structure_is_monotonic_and_keeps_status_out_of_hits",
    "test_root_hit_rectangles_stay_inside_every_boundary_viewport",
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_run_form_stacks_preset_row_and_choices_when_narrow",
        "Python combines two Textual widget-shape promises: #preset-row is a caption plus RadioSet that changes from horizontal to vertical, and the parameter choice RadioSet stacks at the same CSS width tier. Rust intentionally presents saved presets as a compact typed Select rather than a caption+RadioSet row, so a same-named test of only parameter-choice packing would drop half the oracle.",
    ),
    (
        "test_inline_form_gets_width_tiers_but_no_height_tiers",
        "Python has a distinct _InlineFormApp that sizes itself to content and deliberately omits terminal-height CSS classes. Rust collect_form/collect_run_form use the same Ratatui alternate-screen frontend as the workbench and expose no separate inline screen or height-tier class surface.",
    ),
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn every_executable_responsive_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 17);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 2);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 19);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let source = fs::read_to_string(repo.join(TARGET)).unwrap();
    let names = test_names(&source);
    assert_eq!(
        names.len(),
        EXECUTABLE.len() + RUST_ADDITIVE.len(),
        "responsive target added or lost a declared test: {names:#?}"
    );
    let actual = names.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        actual.len(),
        names.len(),
        "duplicate responsive Python-name mappings were collapsed: {names:#?}"
    );
    let expected = EXECUTABLE
        .iter()
        .chain(RUST_ADDITIVE)
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "responsive test inventory drifted");
}

#[test]
fn textual_only_responsive_contracts_are_not_impersonated() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let source = fs::read_to_string(repo.join(TARGET)).unwrap();
    let actual = test_names(&source).into_iter().collect::<BTreeSet<_>>();

    for (name, reason) in ARCHITECTURE_CLOSED {
        assert!(
            !actual.contains(*name),
            "{name} is architecture-closed ({reason}); do not add a weaker same-named stand-in"
        );
        assert!(!reason.trim().is_empty());
    }
}
