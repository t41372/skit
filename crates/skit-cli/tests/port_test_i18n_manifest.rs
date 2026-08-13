//! Completeness guard for Python `tests/test_i18n.py` at `main@206f9ef`.
//!
//! Rust's static catalog and public locale/config seams execute every observable contract they can
//! represent. Babel extraction, Textual sink scanning, gettext plural semantics, the private Python
//! fallback-chain object, and synthetic `.mo` injection are architecture-closed and may not be
//! impersonated by unrelated same-named Rust tests.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGETS: &[&str] = &[
    "crates/skit-i18n/tests/port_test_i18n.rs",
    "crates/skit-cli/tests/port_test_i18n_config.rs",
];

const EXECUTABLE: &[&str] = &[
    "test_locales_shipped",
    "test_parity_with_pot",
    "test_locale_is_fully_translated",
    "test_exact_match",
    "test_alias_hant",
    "test_alias_hans_and_bare_zh",
    "test_posix_style_tag",
    "test_unknown_falls_back_to_en",
    "test_zh_region_with_no_script_hint_defaults_to_simplified",
    "test_conflicting_script_and_region_lets_script_win",
    "test_conflicting_script_and_region_hk_mo_variants",
    "test_plain_tags_unaffected_by_script_over_region_precedence",
    "test_zh_tw_message",
    "test_zh_cn_message",
    "test_entry_library_copy_does_not_narrow_the_mixed_library_to_scripts",
    "test_missing_id_returns_source",
    "test_pseudo_locale",
    "test_pseudo_preserves_placeholder",
    "test_missing_zh_tw_msgid_falls_back_to_english_not_simplified",
    "test_missing_zh_hk_msgid_falls_back_to_english_not_simplified",
    "test_conflicting_script_and_region_tag_end_to_end",
    "test_env_override_wins",
    "test_lang_env",
    "test_c_locale_ignored",
    "test_set_language_persists",
    "test_backs_up_corrupt_config_instead_of_wiping_it",
    "test_warns_when_corrupt_config_cannot_even_be_backed_up",
    "test_valid_config_is_unaffected",
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_pot_covers_all_source_gettext_msgids",
        "Python runs Babel over .py gettext/ngettext literals and compares them with a committed POT. Rust has a compiled static catalog and no Babel/POT extraction artifact or equivalent public extractor seam.",
    ),
    (
        "test_no_unwrapped_ui_strings",
        "The oracle invokes scripts/i18n_coverage.py over Textual Static/Label/RadioButton/notify/help/title sinks. Those Python/Textual AST sink types do not exist in the Ratatui implementation; a different Rust sink scanner would be a new tooling contract, not this test.",
    ),
    (
        "test_no_dynamic_gettext",
        "The oracle detects gettext()/ngettext() calls on non-literal Python expressions and proves the detector with gettext(_LABELS[k]). Rust exposes text/format_text/Message over a static catalog and has no gettext/ngettext call grammar or Python detector seam.",
    ),
    (
        "test_chain_always_ends_with_en",
        "Python negotiate() publicly returns an ordered fallback-chain list. Rust detect_locale returns only one Locale and static-catalog lookup falls directly to the source string; no chain object exists to inspect.",
    ),
    (
        "test_traditional_chain_excludes_simplified",
        "This asserts the exact private/public Python negotiate() chain [zh-TW, en]. Rust has no fallback-chain representation; the observable no-Simplified leakage is covered separately by the executable missing-msgid tests.",
    ),
    (
        "test_simplified_chain_still_resolves_to_zh_cn",
        "This asserts the exact Python chain [zh-CN, en]. Rust has no chain representation; primary Simplified-family resolution is covered by executable detector tests.",
    ),
    (
        "test_en_plural",
        "Python gettext exposes ngettext singular/plural selection. Rust v0.5 uses caller-selected static catalog rows and has no ngettext/plural API to execute this runtime contract.",
    ),
    (
        "test_zh_plural_single_form",
        "Python gettext catalog plural rules are evaluated by ngettext. Rust has no plural-rule engine or ngettext seam; callers choose a concrete catalog row.",
    ),
    (
        "test_variable_substitution",
        "The oracle is specifically Python percent-mapping substitution on a gettext-returned %(file)s template. Rust format_text uses typed {} values and does not own Python percent formatting.",
    ),
    (
        "test_zh_cn_still_gets_its_own_translation",
        "The oracle injects a synthetic zh_CN .mo containing a new msgid absent from the real catalog. Rust's catalog is compile-time static and has no injectable catalog directory, so reproducing this with an existing msgid would not exercise the same branch.",
    ),
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn parity_test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                let name = function.sig.ident.to_string();
                (!name.starts_with("rust_additive_")).then_some(name)
            }
            _ => None,
        })
        .collect()
}

fn all_parity_test_names(repo: &Path) -> Vec<String> {
    TARGETS
        .iter()
        .flat_map(|target| {
            let source = fs::read_to_string(repo.join(target)).unwrap();
            parity_test_names(&source)
        })
        .collect()
}

#[test]
fn every_executable_i18n_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 28);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 10);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 38);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let names = all_parity_test_names(repo);
    assert_eq!(
        names.len(),
        EXECUTABLE.len(),
        "an i18n Python oracle was duplicated or an unexpected parity test was added: {names:#?}"
    );
    let actual = names.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        actual.len(),
        names.len(),
        "two Rust targets claim the same i18n Python oracle: {names:#?}"
    );
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected, "i18n executable mapping drifted");
}

#[test]
fn python_only_i18n_contracts_are_not_impersonated() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let actual = all_parity_test_names(repo)
        .into_iter()
        .collect::<BTreeSet<_>>();

    for (name, reason) in ARCHITECTURE_CLOSED {
        assert!(
            !actual.contains(*name),
            "{name} is architecture-closed ({reason}); do not add a weaker same-named stand-in"
        );
        assert!(!reason.trim().is_empty());
    }
}
