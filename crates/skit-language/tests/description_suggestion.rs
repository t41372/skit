use skit_language::suggest_description;

#[test]
fn python_description_comes_only_from_the_parsed_module_docstring() {
    assert_eq!(
        suggest_description(
            "python",
            b"#!/usr/bin/env python3\n\"\"\" Ship releases.\n\nMore detail.\"\"\"\nprint(1)\n",
        ),
        "Ship releases."
    );
    assert_eq!(
        suggest_description("python", b"print(1)\n\"\"\"not a module docstring\"\"\"\n"),
        ""
    );
    assert_eq!(suggest_description("python", b"\"\"\"unterminated\n"), "");
}

#[test]
fn comment_languages_share_the_leading_comment_block_contract() {
    for kind in ["shell", "fish", "powershell", "ruby", "perl", "r"] {
        assert_eq!(
            suggest_description(
                kind,
                b"#!/usr/bin/env tool\n\n# /// script\n#\n# Ship the current build\n# more\ncode\n",
            ),
            "Ship the current build",
            "{kind}"
        );
    }
    for kind in ["js", "ts"] {
        assert_eq!(
            suggest_description(kind, b"// Run the worker\nconsole.log('ok')\n"),
            "Run the worker",
            "{kind}"
        );
    }
    assert_eq!(
        suggest_description("lua", b"-- Resize things\nprint('ok')\n"),
        "Resize things"
    );
    assert_eq!(suggest_description("ruby", b"VALUE = 1\n# too late\n"), "");
}

#[test]
fn prompt_description_uses_one_unicode_safe_capped_heading_line() {
    assert_eq!(
        suggest_description("prompt", b"\n\n## A title ##\nbody\n"),
        "A title ##"
    );
    let exact = format!("{}{}", "\u{754c}".repeat(119), "\u{1f642}");
    assert_eq!(
        suggest_description("prompt", format!("# {exact}\nbody").as_bytes()),
        exact
    );
    let over = format!("{exact}\u{5c3e}");
    let expected = format!("{}\u{2026}", "\u{754c}".repeat(119));
    assert_eq!(suggest_description("prompt", over.as_bytes()), expected);
}

#[test]
fn invalid_interpreted_bytes_use_the_latest_main_replacement_view() {
    assert_eq!(
        suggest_description("python", b"\"\"\"valid \xff tail\"\"\"\n"),
        "valid \u{fffd} tail"
    );
    assert_eq!(
        suggest_description("shell", b"# valid \xff\xfe tail\necho ok\n"),
        "valid \u{fffd}\u{fffd} tail"
    );
    assert_eq!(
        suggest_description("lua", b"-- mixed \xe2(\xa1 bytes\n"),
        "mixed \u{fffd}(\u{fffd} bytes"
    );
}

#[test]
fn non_source_kinds_do_not_invent_descriptions() {
    assert_eq!(suggest_description("exe", b"# not source metadata\n"), "");
    assert_eq!(suggest_description("command", b"echo hi\n"), "");
    assert_eq!(suggest_description("newer-kind", b"# future\n"), "");
}
