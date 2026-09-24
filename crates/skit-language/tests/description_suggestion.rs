use skit_language::{suggest_description, suggest_prompt_name};

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

/// C12 of the 2026-11 audit: a new prompt draft takes its name from its first words, not from its
/// temporary file name.
#[test]
fn a_prompt_name_comes_from_the_first_words_of_the_first_line() {
    assert_eq!(
        suggest_prompt_name(b"# Review the pull request\n\nCheck {{diff}}.\n").as_deref(),
        Some("Review the pull request")
    );
    // At most four words, and no sentence mark at the end.
    assert_eq!(
        suggest_prompt_name(b"Summarize these meeting notes for the team, please.\n").as_deref(),
        Some("Summarize these meeting notes")
    );
    assert_eq!(
        suggest_prompt_name(b"Explain this error:\n{{error}}\n").as_deref(),
        Some("Explain this error")
    );
    // A placeholder keeps its name without the braces, and emphasis marks go.
    assert_eq!(
        suggest_prompt_name(b"Translate {{text}} into **{{language}}**\n").as_deref(),
        Some("Translate text into language")
    );
    // A line without a letter or a digit names nothing, so the next line does.
    assert_eq!(
        suggest_prompt_name(b"---\n\n> Write a haiku\n").as_deref(),
        Some("Write a haiku")
    );
    // A word that would pass thirty-two characters ends the name.
    assert_eq!(
        suggest_prompt_name(b"Internationalization considerations everywhere\n").as_deref(),
        Some("Internationalization")
    );
    // Text without spaces stops at thirty-two characters.
    let long = "\u{5199}".repeat(50);
    assert_eq!(
        suggest_prompt_name(long.as_bytes()),
        Some("\u{5199}".repeat(32))
    );
    // Nothing usable leaves the name to the caller.
    assert_eq!(suggest_prompt_name(b" \n\t\n"), None);
    assert_eq!(suggest_prompt_name(b"--- ***\n"), None);
}

/// skit writes a starter line into each new prompt draft. That line names no prompt, so the name
/// and the description come from the first line that the person wrote.
#[test]
fn the_new_prompt_starter_line_is_not_a_name_or_a_description() {
    for starter in [
        "# New prompt",
        "# \u{65b0}\u{63d0}\u{793a}\u{8bcd}",
        "# \u{65b0}\u{63d0}\u{793a}\u{8a5e}",
    ] {
        let draft = format!("{starter}\n\nReview the pull request for bugs\n");
        assert_eq!(
            suggest_prompt_name(draft.as_bytes()).as_deref(),
            Some("Review the pull request"),
            "{starter}"
        );
        assert_eq!(
            suggest_description("prompt", draft.as_bytes()),
            "Review the pull request for bugs",
            "{starter}"
        );
    }
    assert_eq!(suggest_prompt_name(b"# New prompt\n\n"), None);
    // The same words in a heading that skit did not write stay a description.
    assert_eq!(
        suggest_description("prompt", b"# New prompt ideas\n"),
        "New prompt ideas"
    );
}
