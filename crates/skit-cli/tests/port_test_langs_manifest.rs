//! Completeness guard for Python `tests/test_langs.py` at `main@206f9ef`.
//!
//! Ten contracts have executable Rust public/CLI equivalents. Eleven describe Python's unified
//! `LangSpec`/`LazyCapabilities` object, Python import-module behavior, or monkeypatchable launcher
//! namespaces that the Rust architecture does not expose. They stay architecture-closed instead of
//! being represented by weaker same-named tests.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGET: &str = "crates/skit-cli/tests/port_test_langs.rs";

const EXECUTABLE: &[&str] = &[
    "test_stored_name_unknown_kind_falls_back_to_payload",
    "test_unknown_kind_build_command_raises_clean_launch_error",
    "test_unknown_kind_run_entry_raises_before_spawning",
    "test_unknown_kind_never_reports_missing",
    "test_unknown_kind_preflight_still_checks_workdir",
    "test_unknown_kind_script_path_uses_payload_fallback",
    "test_params_exe_prints_plain_message_without_manage_dead_end",
    "test_doctor_missing_uv_pure_exe_library_exits_zero",
    "test_doctor_missing_uv_with_python_entry_exits_one",
    "test_doctor_json_missing_uv_pure_exe_library_exits_zero",
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_every_known_kind_resolves_to_a_complete_spec",
        "Python exposes one public LangSpec registry whose rows jointly own glyph, family, launch strategy, stored name, and capabilities. Rust intentionally splits those responsibilities across domain/application/language/runtime/frontend APIs and has no single public spec object whose completeness can be asserted without inventing a test-only registry.",
    ),
    (
        "test_python_spec_capabilities_and_pinned_store_name",
        "The oracle inspects Python LangSpec fields such as extensions, comment syntax, analyzer, cli_reader, params_io, supports_deps, takes_argv, editable, and has_original_file. Rust has no unified public LangSpec field set. The observable pinned `script.py` stored-name part is covered by the executable stored-name contract; that does not justify impersonating the rest of this structural test.",
    ),
    (
        "test_exe_and_command_specs_have_no_analysis_capabilities",
        "Python asserts absent analyzer/cli_reader/params_io fields and family/takes_argv flags on two LangSpec instances. Rust models these rules in separate typed surfaces rather than nullable public capability fields, so a same-named test over unrelated functions would not prove the Python structural contract.",
    ),
    (
        "test_resolving_a_spec_does_not_import_a_language_parser",
        "The contract observes Python sys.modules after importing registry specs. Rust parser crates are statically linked and there is no runtime module-import graph equivalent to inspect.",
    ),
    (
        "test_asking_for_a_capability_resolves_it_once",
        "Python constructs LazyCapabilities around a counted builder and proves one-time lazy resolution. Rust exposes no public lazy-capability builder/cache object; recreating one only in a test would test invented code.",
    ),
    (
        "test_without_drops_exactly_the_named_capabilities",
        "Python LangSpec.without() produces a degraded nullable-capability view. Rust has no public equivalent that can selectively remove analyzer/injector while retaining the same spec identity.",
    ),
    (
        "test_capabilities_do_not_decide_spec_identity",
        "This is Python dataclass equality with capabilities declared compare=False. Rust has no corresponding public LangSpec equality object, so translating it to equality of different Rust values would test a different invariant.",
    ),
    (
        "test_spec_for_unknown_kind_is_none_and_cached",
        "Python exposes a cached spec_for() lookup and the contract observes two calls returning None. Rust keeps EntryKind open-ended and has no public cached spec lookup; unknown-kind public degradation is exercised end to end by the executable tests instead.",
    ),
    (
        "test_unknown_kind_describe_returns_template_and_never_raises",
        "Python exposes launcher.describe_command(entry), a total side-effect-free helper whose unknown-kind fallback is the metadata template. Rust exposes no public launcher description helper for an unknown kind; `show` or `--dry-run` would exercise different behavior and cannot stand in for it.",
    ),
    (
        "test_launcher_uv_delegates_follow_patches_on_the_canonical_module",
        "The oracle monkeypatches Python module namespace skit.langs.launch and proves legacy launcher aliases delegate dynamically. Rust has no module monkeypatch namespace or alias-indirection contract.",
    ),
    (
        "test_plan_without_cli_reader_degrades_to_none_plan",
        "Python removes only cli_reader from a Python LangSpec, monkeypatches flows.spec_for, and observes plan_for_entry degradation. Rust has no injectable public capability registry that can represent the same partly-disabled spec without changing production code.",
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
                let name = function.sig.ident.to_string();
                (!name.starts_with("rust_additive_")).then_some(name)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn every_executable_langs_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 10);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 11);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 21);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let source = fs::read_to_string(repo.join(TARGET)).unwrap();
    let actual = test_names(&source);
    let unique = actual.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        actual.len(),
        unique.len(),
        "a Python langs contract has more than one exact-name Rust oracle: {actual:?}"
    );
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(unique, expected, "langs executable mapping drifted");
}

#[test]
fn python_only_langs_contracts_are_not_impersonated() {
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
