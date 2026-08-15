//! Live exact-name audit for frozen `tests/test_prompt_kind.py`.
//!
//! This guard is intentionally **not** attached to the master behavior inventory until its
//! missing/extra sets are empty. It parses the preserved Python test file in the repository and
//! requires every frozen name to be either an executable Rust owner or one of a fixed, narrow
//! Python-only seam closures. `rust_additive_*` never counts.

use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};
use syn::{Attribute, Item};

const CLOSED: &[(&str, &str)] = &[
    (
        "test_check_argv_length_accepts_surrogateescaped_os_bytes",
        "Python str can carry surrogateescape code points that round-trip arbitrary POSIX argv bytes. Rust prompt bodies and runner argv are valid-UTF-8 String values before the public launch-plan boundary, so this exact surrogateescape representation has no Rust value/injection seam. UTF-8 byte counting and real child argv remain executable for representable strings.",
    ),
    (
        "test_check_argv_length_refuses_unencodable_surrogate_cleanly",
        "Python can construct an isolated surrogate code point and specifically assert its UnicodeEncodeError cause. Rust String cannot contain Unicode surrogate scalar values, so there is no equivalent public input value to feed the prompt argv boundary. Ordinary encoding/length/NUL refusals remain executable.",
    ),
    (
        "test_surrogateescaped_value_reaches_a_real_child_as_the_original_byte",
        "Python os.fsdecode/os.fsencode deliberately round-trip an arbitrary non-UTF-8 POSIX argv byte through surrogateescape. Rust's prompt value and argv contracts use String, not OsString/OsStr payload values, so the exact 0xff round-trip is not representable at this public boundary. Real child spawning and byte-exact UTF-8 prompt delivery remain executable.",
    ),
    (
        "test_build_script_override_reads_the_override",
        "Python PromptLaunch exposes a private script_override path parameter and directly tests that private loader. Rust's public prompt launch-plan API receives an already captured rendered prompt and the CLI has no prompt-body script-override lane. Public stored/reference source selection and prepared-snapshot launch races remain executable.",
    ),
    (
        "test_describe_with_no_pin_and_no_runner_never_reads_config",
        "Python monkeypatches the private config loader and asserts a zero-call optimization inside PromptLaunch.describe. Rust exposes public description/preview behavior but no injectable config-reader callback for this call-count-only seam. No-runner display/refusal behavior remains executable.",
    ),
];

const OWNERS: &[&str] = &[
    "crates/skit-language/tests/port_test_prompt_kind_analyzer.rs",
    "crates/skit-language/tests/port_test_prompt_kind_corpus_render.rs",
    "crates/skit-language/tests/port_test_prompt_kind_description.rs",
    "crates/skit-runtime/tests/port_test_prompt_kind_runner_argv.rs",
    "crates/skit-runtime/tests/port_test_prompt_kind_windows_argv.rs",
    "crates/skit-runtime/tests/port_test_prompt_kind_launch.rs",
    "crates/skit-runtime/tests/port_test_prompt_kind_interpolate.rs",
    "crates/skit-runtime/tests/port_test_prompt_kind_describe_degrade.rs",
    "crates/skit-application/tests/port_test_prompt_kind_policy.rs",
    "crates/skit-application/tests/port_test_prompt_kind_runner_validation.rs",
    "crates/skit-application/tests/port_test_prompt_kind_explicit_workdir.rs",
    "crates/skit-application/tests/port_test_prompt_kind_atomic_update.rs",
    "crates/skit-store/tests/port_test_prompt_kind_runner_config.rs",
    "crates/skit-store/tests/port_test_prompt_kind_runner_cas.rs",
    "crates/skit-store/tests/port_test_prompt_kind_runner_remaining.rs",
    "crates/skit-store/tests/port_test_prompt_kind_state_meta.rs",
    "crates/skit-ui/tests/port_test_prompt_kind_interpolate_flood.rs",
    "crates/skit-ui/tests/port_test_prompt_kind_strict_selection.rs",
    "crates/skit-form/tests/port_test_prompt_kind_plan.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_unmanaged.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_store_public.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_missing_value.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_runner_resolution.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_config_process_lock.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_entry_lock.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_entry_lock_frozen.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_unreadable_plan.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_unreadable_build.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_prompt_only_writes.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_real_spawn.rs",
    "crates/skit-cli/tests/port_test_prompt_kind_preflight.rs",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn rust_tests(path: &Path) -> Vec<String> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let file = syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("could not parse {}: {error}", path.display()));
    file.items.iter().filter_map(|item| match item {
        Item::Fn(function) if has_test(&function.attrs) => {
            let name = function.sig.ident.to_string();
            name.starts_with("test_").then_some(name)
        }
        _ => None,
    }).collect()
}

fn frozen_python_names(source: &str) -> Vec<String> {
    source.lines().filter_map(|line| {
        let line = line.trim_start();
        let rest = line.strip_prefix("def test_")?;
        let tail = rest.split_once('(')?.0;
        Some(format!("test_{tail}"))
    }).collect()
}

#[test]
fn prompt_kind_frozen_name_audit_is_complete() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().and_then(Path::parent)
        .expect("skit-cli lives at <repo>/crates/skit-cli");
    let python = fs::read_to_string(repo.join("tests/test_prompt_kind.py")).expect("preserved Python behavior suite");
    let frozen_list = frozen_python_names(&python);
    let frozen = frozen_list.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(frozen_list.len(), 115, "frozen Prompt-Kind denominator changed");
    assert_eq!(frozen.len(), 115, "duplicate frozen Prompt-Kind test name");
    for sentinel in [
        "test_placeholder_names_accept_unicode_identifiers_and_reject_non_names",
        "test_check_argv_length_measures_windows_quoted_utf16",
        "test_runner_targeted_transactions_do_not_lose_concurrent_distinct_adds",
        "test_prompt_meta_setters_preserve_concurrent_distinct_fields",
    ] {
        assert!(frozen.contains(sentinel), "preserved Prompt-Kind source lost sentinel {sentinel}");
    }

    let closed = CLOSED.iter().map(|(name, _)| *name).collect::<BTreeSet<_>>();
    assert_eq!(closed.len(), 5, "Prompt-Kind architecture-closure allowlist must stay fixed at five");
    assert_eq!(closed.len(), CLOSED.len(), "duplicate Prompt-Kind closure name");
    assert!(CLOSED.iter().all(|(_, reason)| !reason.trim().is_empty()));
    assert!(closed.is_subset(&frozen), "Prompt-Kind closure list contains a non-frozen name");

    let mut owners = BTreeMap::<String, String>::new();
    let mut duplicates = Vec::new();
    for relative in OWNERS {
        for name in rust_tests(&repo.join(relative)) {
            if let Some(previous) = owners.insert(name.clone(), (*relative).to_owned()) {
                duplicates.push(format!("{name}: {previous} and {relative}"));
            }
        }
    }
    assert!(duplicates.is_empty(), "duplicate Prompt-Kind owners:\n{}", duplicates.join("\n"));

    let expected = frozen.difference(&closed).copied().collect::<BTreeSet<_>>();
    let actual = owners.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
    let extras = actual.difference(&expected).copied().collect::<Vec<_>>();
    assert_eq!(expected.len(), 110, "Prompt-Kind executable partition must stay 110/115");
    assert_eq!(actual.len(), 110, "Prompt-Kind owner count must stay exactly 110 executable frozen names");
    assert!(
        missing.is_empty() && extras.is_empty(),
        "Prompt-Kind exact-name audit still incomplete: executable={}/{} closed={} missing={missing:?} extras={extras:?}",
        actual.len(), frozen.len(), closed.len(),
    );
}
