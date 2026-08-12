//! Executable completeness guard for Python `tests/test_uv_metadata_views.py` at `main@206f9ef`.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

struct Mapping {
    python: &'static str,
    path: &'static str,
    rust: &'static str,
}

const CLI: &str = "crates/skit-cli/tests/port_test_effective_uv_metadata.rs";
const TUI: &str = "crates/skit-cli/tests/port_test_uv_metadata_views_tui.rs";
const RACE: &str = "crates/skit-cli/tests/port_test_uv_metadata_views_race.rs";

const MAPPINGS: &[Mapping] = &[
    Mapping { python: "test_show_human_block_only_prints_effective_deps_and_constraint", path: CLI, rust: "test_show_human_block_only_prints_effective_deps_and_constraint" },
    Mapping { python: "test_show_human_meta_carried_deps_unchanged", path: CLI, rust: "test_show_human_meta_carried_deps_unchanged" },
    Mapping { python: "test_show_human_no_uv_metadata_prints_neither_line", path: CLI, rust: "test_show_human_no_uv_metadata_prints_neither_line" },
    Mapping { python: "test_detail_pane_block_only_shows_effective_depends_on", path: TUI, rust: "test_detail_pane_block_only_shows_effective_depends_on" },
    Mapping { python: "test_detail_pane_no_deps_omits_the_depends_on_line", path: TUI, rust: "test_detail_pane_no_deps_omits_the_depends_on_line" },
    Mapping { python: "test_settings_save_diffs_against_compose_time_baseline_not_a_re_read", path: RACE, rust: "test_settings_save_diffs_against_compose_time_baseline_not_a_re_read" },
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_uv_metadata_view_python_test_has_a_real_rust_test() {
    assert_eq!(MAPPINGS.len(), 6, "frozen Python UV-view oracle count changed");
    assert_eq!(
        MAPPINGS.iter().map(|mapping| mapping.python).collect::<BTreeSet<_>>().len(),
        6,
        "duplicate Python mappings make UV-view accounting dishonest"
    );
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let mut failures = Vec::new();
    for mapping in MAPPINGS {
        let path = repo.join(mapping.path);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        let file = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("{} is not valid Rust: {error}", path.display()));
        let executable = file.items.iter().any(|item| match item {
            Item::Fn(function) => {
                function.sig.ident == mapping.rust && has_test_attribute(&function.attrs)
            }
            _ => false,
        });
        if !executable {
            failures.push(format!(
                "{} -> {}::{} is missing or not #[test]",
                mapping.python, mapping.path, mapping.rust
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "UV-view parity manifest contains fake/non-executable mappings:\n{}",
        failures.join("\n")
    );
}
