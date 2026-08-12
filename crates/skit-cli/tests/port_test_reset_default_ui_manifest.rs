//! Completeness guard for Python `tests/test_reset_default_ui.py` at `main@206f9ef`.
//!
//! Thirteen contracts have executable Rust equivalents across the mature Ratatui run form and the
//! real CLI/Settings surfaces. Python's separate `promptform.collect` dumb-terminal line form has no
//! Rust equivalent: `collect_run_form` is the same Ratatui alternate-screen frontend, so repeating a
//! normal TUI hint test would be a weaker stand-in rather than coverage of a second surface.

use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

use syn::{Attribute, Item};

struct Mapping {
    python: &'static str,
    path: &'static str,
}

const TUI: &str = "crates/skit-tui/tests/port_test_reset_default_ui.rs";
const CLI: &str = "crates/skit-cli/tests/port_test_reset_default_ui_cli.rs";

const EXECUTABLE: &[Mapping] = &[
    Mapping { python: "test_ctrl_o_from_focused_field_restores_default_over_remembered_value", path: TUI },
    Mapping { python: "test_reset_field_by_key_restores_text_bool_and_choice_defaults", path: TUI },
    Mapping { python: "test_reset_chip_mouse_click_restores_the_default", path: TUI },
    Mapping { python: "test_reset_chip_present_for_default_absent_for_secret_and_no_default", path: TUI },
    Mapping { python: "test_choice_default_outside_its_choices_gets_no_chip_and_no_ctrl_o", path: TUI },
    Mapping { python: "test_footer_advertises_ctrl_o_only_when_some_field_is_resettable", path: TUI },
    Mapping { python: "test_ctrl_o_on_field_without_default_leaves_value_unchanged", path: TUI },
    Mapping { python: "test_ctrl_o_with_focus_outside_any_field_row_is_a_no_op", path: TUI },
    Mapping { python: "test_input_binding_field_renders_the_ask_in_terminal_hint", path: TUI },
    Mapping { python: "test_plain_const_field_renders_no_input_binding_hint", path: TUI },
    Mapping { python: "test_params_default_column_shows_the_sources_live_value", path: CLI },
    Mapping { python: "test_show_json_delivers_empty_true_for_str_const_false_for_int", path: CLI },
    Mapping { python: "test_settings_param_row_shows_the_sources_live_default", path: CLI },
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_promptform_prints_input_binding_hint",
        "Python has an independent line-oriented promptform.collect fallback for --plain/dumb terminals. Rust collect_run_form still enters the same Ratatui alternate-screen frontend, so there is no second line-form renderer whose hint can be tested without duplicating the ordinary TUI contract.",
    ),
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn test_names(source: &str) -> BTreeSet<String> {
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
fn every_executable_reset_default_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 13);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 1);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 14);
    assert_eq!(
        EXECUTABLE
            .iter()
            .map(|mapping| mapping.python)
            .chain(ARCHITECTURE_CLOSED.iter().map(|(name, _)| *name))
            .collect::<BTreeSet<_>>()
            .len(),
        14,
        "duplicate accounting could hide a frozen reset/default contract"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let targets = [TUI, CLI]
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(repo.join(path)).unwrap();
            (path, test_names(&source))
        })
        .collect::<BTreeMap<_, _>>();
    let expected = EXECUTABLE
        .iter()
        .map(|mapping| mapping.python.to_owned())
        .collect::<BTreeSet<_>>();
    let actual = targets
        .values()
        .flat_map(|names| names.iter().cloned())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "reset/default executable mapping drifted");

    for mapping in EXECUTABLE {
        assert!(
            targets[mapping.path].contains(mapping.python),
            "{} moved away from its audited public seam {}",
            mapping.python,
            mapping.path
        );
        let count = targets
            .values()
            .filter(|names| names.contains(mapping.python))
            .count();
        assert_eq!(count, 1, "{} is mapped {count} times", mapping.python);
    }
}

#[test]
fn line_form_only_contract_is_not_impersonated_by_the_ratatui_tests() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let executable = [TUI, CLI]
        .into_iter()
        .flat_map(|path| test_names(&fs::read_to_string(repo.join(path)).unwrap()))
        .collect::<BTreeSet<_>>();

    for (name, reason) in ARCHITECTURE_CLOSED {
        assert!(!reason.trim().is_empty());
        assert!(
            !executable.contains(*name),
            "{name} is architecture-closed ({reason}); do not add a same-named Ratatui stand-in"
        );
    }
}
