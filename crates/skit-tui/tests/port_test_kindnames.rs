//! Mechanical port of the Python oracle module `tests/test_kindnames.py`
//! (`origin/main@206f9ef`): "the ONE translated kind-name map (src/skit/kindnames.py),
//! shared by the Library badge and the KindPickModal." Each `#[test]` keeps its Python
//! `def test_*` name so it traces back to its origin, and each Python "WHY" comment is
//! preserved above it.
//!
//! Crate choice (the `skit-language` hint is overridden): the oracle module spans two
//! Rust tiers. `kindnames.kind_label` lives in `skit-i18n` (`kind_label`), and the
//! `kind_choices` id/order surface lives in `skit-ui` (`KnownEntryKind::picker_choices`).
//! Only `skit-tui` reaches both (both are its dependencies) and is where production
//! actually composes them into the labeled kind picker (`kind_rows`, add.rs:915-920), so
//! the two `kind_choices` tests keep a real, compiling body here instead of demoting to
//! cross-crate stubs.
//!
//! Concept mapping used throughout:
//! - Python `kindnames.kind_label(kind)` -> `skit_i18n::kind_label(Locale::En, kind)`.
//! - Python autouse `_english` fixture (`SKIT_LANG=en`) -> passing `Locale::En` explicitly.
//! - Python `kindnames.kind_choices(offer_exe=...)` (a `list[(kind, label)]`) -> the Rust
//!   composition `KnownEntryKind::picker_choices(offer_exe)` (the ids/order) each mapped
//!   through `kind_choice_label` (the labels) -- exactly what production `kind_rows` renders.
//! - Python `skit.langs.registry.KNOWN_KINDS` / `spec_for` / `LangSpec.family`: NO Rust
//!   registry equivalent (`EntryKind` is an open string, and there is no enumerable
//!   family/spec surface). The label contract is still fully asserted; see the notes below.
//!
//! Buckets:
//! - Bucket 1 (all real, passing): the three kind_label tests and the two `kind_choices`
//!   tests. exe and prompt carry their dedicated descriptive choice wording ("A program
//!   (run it directly)" / "A prompt for an AI agent") through `kind_choice_label`.

use skit_i18n::{Locale, kind_choice_label, kind_label};
use skit_ui::KnownEntryKind;

/// The English label each registered kind renders as (the oracle's module-level EXPECTED;
/// msgids ARE the English source). Includes "command" -- the 13th registered oracle kind.
const EXPECTED: &[(&str, &str)] = &[
    ("python", "Python"),
    ("shell", "Shell"),
    ("fish", "fish"),
    ("js", "JavaScript"),
    ("ts", "TypeScript"),
    ("powershell", "PowerShell"),
    ("ruby", "Ruby"),
    ("perl", "Perl"),
    ("lua", "Lua"),
    ("r", "R"),
    ("exe", "Program"),
    ("command", "Command"),
    ("prompt", "Prompt"),
];

/// The oracle's EXPECTED[kind] lookup (None when a kind has no dedicated label).
fn expected_label(kind: &str) -> Option<&'static str> {
    EXPECTED
        .iter()
        .find(|(candidate, _)| *candidate == kind)
        .map(|(_, label)| *label)
}

/// The Rust composition of the oracle's `kind_choices`: the picker ids/order, each mapped
/// through `kind_choice_label` -- byte-for-byte what production `kind_rows` (add.rs)
/// renders (interpreted kinds keep `kind_label`; exe and prompt get their descriptive
/// choice labels). Labels live only on this side; the oracle's expected literals never
/// leak into the construction, so a mismatch is a real finding, not a tautology.
fn labeled_choices(offer_exe: bool) -> Vec<(String, String)> {
    KnownEntryKind::picker_choices(offer_exe)
        .iter()
        .map(|kind| {
            (
                kind.as_str().to_owned(),
                kind_choice_label(Locale::En, kind.as_str()).into_owned(),
            )
        })
        .collect()
}

// Oracle `@pytest.mark.parametrize(("kind", "label"), sorted(EXPECTED.items()))` -> one
// `#[test]` looping the 13 pairs (the parametrization split is faithful as one def).
#[test]
fn test_kind_label_maps_each_registered_kind() {
    for (kind, label) in EXPECTED {
        assert_eq!(&*kind_label(Locale::En, kind), *label, "kind {kind}");
    }
}

#[test]
fn test_every_known_kind_has_a_dedicated_label() {
    // No registered kind may fall through to the raw-id branch -- a kind rendering as its
    // bare id in the Library badge is an untranslated leak (the map is the i18n contract).
    // 'fish' is the one kind whose label is intentionally its own id.
    //
    // Rust has no KNOWN_KINDS registry to enumerate (EntryKind is an open string). The
    // enumerable registered set is the KnownEntryKind picker (12), plus "command" -- the
    // oracle's 13th registered kind, which the Rust enum omits because command is never
    // offered in the unclassified-file picker (the oracle also excludes it from
    // kind_choices). Together they reconstruct the oracle's 13-kind KNOWN_KINDS.
    let mut registered: Vec<String> = KnownEntryKind::picker_choices(true)
        .iter()
        .map(|kind| kind.as_str().to_owned())
        .collect();
    registered.push("command".to_owned());
    for kind in &registered {
        let label = expected_label(kind)
            .unwrap_or_else(|| panic!("registered kind missing an expected label: {kind}"));
        // A mapped kind never returns via the `.get(kind, kind)` fallthrough -- its label is
        // the literal above (which, for 'fish', happens to equal the id -- still a real hit).
        assert_eq!(&*kind_label(Locale::En, kind), label);
    }
}

#[test]
fn test_unknown_kind_falls_through_to_its_raw_id() {
    // A meta written by a newer skit (an unknown kind) renders honestly as its raw id,
    // never a crash or a blank -- the `.get(kind, kind)` fallthrough.
    assert_eq!(&*kind_label(Locale::En, "cobol"), "cobol");
    assert_eq!(&*kind_label(Locale::En, ""), "");
}

#[test]
fn test_kind_choices_exact_options_and_order() {
    // The ONE option list both ask faces render: sorted interpreted kinds (prompt excluded --
    // it gets its own dedicated wording), then exe (gated), then prompt. Exact ids and labels --
    // the twins' contract.
    let full = labeled_choices(true);
    let interp: Vec<&str> = full[..full.len() - 2]
        .iter()
        .map(|(kind, _)| kind.as_str())
        .collect();
    // The oracle computes this from the registry (family == "interpreted", minus prompt,
    // sorted). Rust has no registry to compute from, so the sorted value is inlined -- already
    // verified equal to `picker_choices` order.
    assert_eq!(
        interp,
        [
            "fish",
            "js",
            "lua",
            "perl",
            "powershell",
            "python",
            "r",
            "ruby",
            "shell",
            "ts"
        ]
    );
    assert!(!interp.is_empty()); // the registry's interpreted kinds actually made it in
    // Tautological here by construction (these labels ARE kind_label) -- kept to document the
    // oracle contract that every interpreted choice's label comes from kind_label.
    assert!(
        full[..full.len() - 2]
            .iter()
            .all(|(kind, label)| label.as_str() == &*kind_label(Locale::En, kind.as_str()))
    );
    assert_eq!(
        full[full.len() - 2],
        ("exe".to_owned(), "A program (run it directly)".to_owned())
    );
    assert_eq!(
        full[full.len() - 1],
        ("prompt".to_owned(), "A prompt for an AI agent".to_owned())
    );
}

#[test]
fn test_kind_choices_offer_exe_false_drops_only_exe() {
    let full = labeled_choices(true);
    let gated = labeled_choices(false);
    let dropped_exe: Vec<(String, String)> = full
        .iter()
        .filter(|entry| entry.0.as_str() != "exe")
        .cloned()
        .collect();
    assert_eq!(gated, dropped_exe);
    assert_eq!(
        gated[gated.len() - 1],
        ("prompt".to_owned(), "A prompt for an AI agent".to_owned())
    );
}
