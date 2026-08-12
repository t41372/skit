//! Python parity contracts from `tests/test_kindnames.py` at main@206f9ef.

use skit_i18n::{Locale, kind_label};

const EXPECTED: [(&str, &str); 13] = [
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

#[test]
fn test_kind_label_maps_each_registered_kind() {
    for (kind, label) in EXPECTED {
        assert_eq!(
            kind_label(Locale::En, kind).as_ref(),
            label,
            "Python main requires the exact English kind label for {kind}"
        );
    }
}

#[test]
fn test_every_known_kind_has_a_dedicated_label() {
    let known_kinds = [
        "python",
        "shell",
        "fish",
        "js",
        "ts",
        "powershell",
        "ruby",
        "perl",
        "lua",
        "r",
        "exe",
        "command",
        "prompt",
    ];

    assert_eq!(known_kinds.len(), EXPECTED.len());
    for kind in known_kinds {
        let expected = EXPECTED
            .iter()
            .find_map(|(candidate, label)| (*candidate == kind).then_some(*label))
            .unwrap_or_else(|| panic!("registered kind is missing an expected label: {kind}"));
        assert_eq!(
            kind_label(Locale::En, kind).as_ref(),
            expected,
            "registered kind must use its dedicated Python-main label: {kind}"
        );
    }
}

#[test]
fn test_unknown_kind_falls_through_to_its_raw_id() {
    assert_eq!(kind_label(Locale::En, "cobol").as_ref(), "cobol");
    assert_eq!(kind_label(Locale::En, "").as_ref(), "");
}
