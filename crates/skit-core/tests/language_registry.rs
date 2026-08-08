use skit_core::{
    DepsFlavor, Family, kind_for_extension, kind_for_shebang_text, known_kinds, python_version_pin,
    shebang_program_from_line, spec_for, stored_name,
};

fn registered(kind: &str) -> &'static skit_core::LanguageSpec {
    match spec_for(kind) {
        Some(spec) => spec,
        None => panic!("missing registered kind: {kind}"),
    }
}

#[test]
fn registry_contains_the_current_python_kind_set() {
    assert_eq!(
        known_kinds(),
        &[
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
        ]
    );
}

#[test]
fn core_traits_match_the_existing_language_registry() {
    let python = registered("python");
    assert_eq!(python.family, Family::Interpreted);
    assert_eq!(python.glyph, "⬡");
    assert_eq!(python.extensions, &[".py"]);
    assert_eq!(python.shebangs, &["python", "python3"]);
    assert_eq!(python.stored_name, "script.py");
    assert!(python.supports_modes);
    assert_eq!(python.deps_flavor, DepsFlavor::Uv);

    let shell = registered("shell");
    assert_eq!(shell.default_interpreter, "bash");
    assert_eq!(shell.stored_name, "script.sh");
    assert_eq!(shell.extensions, &[".sh", ".bash", ".zsh"]);

    let js = registered("js");
    assert_eq!(js.glyph, "✦");
    assert_eq!(js.stored_name, "script.js");
    assert_eq!(js.deps_flavor, DepsFlavor::Npm);

    let ts = registered("ts");
    assert_eq!(ts.glyph, "✧");
    assert_eq!(ts.stored_name, "script.ts");
    assert_eq!(ts.deps_flavor, DepsFlavor::Npm);

    let command = registered("command");
    assert_eq!(command.family, Family::Template);
    assert!(!command.takes_argv);
    assert!(command.placeholder_params);
    assert_eq!(command.stored_name, "");

    let prompt = registered("prompt");
    assert_eq!(prompt.family, Family::Interpreted);
    assert_eq!(prompt.extensions, &[".prompt.md", ".prompt"]);
    assert_eq!(prompt.stored_name, "prompt.md");
    assert!(prompt.supports_modes);
    assert!(!prompt.takes_argv);
    assert!(prompt.placeholder_params);
}

#[test]
fn every_copyable_interpreted_kind_keeps_its_historical_stored_name() {
    let expected = [
        ("python", "script.py"),
        ("shell", "script.sh"),
        ("fish", "script.fish"),
        ("js", "script.js"),
        ("ts", "script.ts"),
        ("powershell", "script.ps1"),
        ("ruby", "script.rb"),
        ("perl", "script.pl"),
        ("lua", "script.lua"),
        ("r", "script.r"),
        ("prompt", "prompt.md"),
    ];
    for (kind, name) in expected {
        assert_eq!(stored_name(kind), name, "stored name for {kind}");
    }
    assert_eq!(stored_name("future-kind"), "payload");
}

#[test]
fn extensions_include_compound_prompt_names_and_are_case_insensitive() {
    assert_eq!(kind_for_extension("tool.PY"), Some("python"));
    assert_eq!(kind_for_extension("script.ZSH"), Some("shell"));
    assert_eq!(kind_for_extension("review.prompt.md"), Some("prompt"));
    assert_eq!(kind_for_extension("review.PROMPT.MD"), Some("prompt"));
    assert_eq!(kind_for_extension("notes.md"), None);
}

#[test]
fn shebang_parser_handles_direct_and_env_indirection() {
    assert_eq!(shebang_program_from_line("#!/bin/bash -e"), Some("bash"));
    assert_eq!(
        shebang_program_from_line("#!/usr/bin/env -S deno run --allow-net"),
        Some("deno")
    );
    assert_eq!(
        shebang_program_from_line("#!/usr/bin/env python3.13"),
        Some("python3.13")
    );
    assert_eq!(shebang_program_from_line("#!/usr/bin/env -S"), None);
    assert_eq!(shebang_program_from_line("echo no"), None);
}

#[test]
fn shebang_kind_mapping_accepts_versioned_python_but_not_python2() {
    assert_eq!(
        kind_for_shebang_text("#!/usr/bin/env python3.13\nprint(1)"),
        Some("python")
    );
    assert_eq!(kind_for_shebang_text("#!/bin/zsh\necho ok"), Some("shell"));
    assert_eq!(
        kind_for_shebang_text("#!/usr/bin/fish\necho ok"),
        Some("fish")
    );
    assert_eq!(kind_for_shebang_text("#!/usr/bin/env node\n"), Some("js"));
    assert_eq!(kind_for_shebang_text("#!/usr/bin/env python2\n"), None);
    assert_eq!(kind_for_shebang_text("#!/usr/bin/env unknown\n"), None);
}

#[test]
fn versioned_python_shebangs_keep_their_requires_python_pin() {
    assert_eq!(python_version_pin(Some("python3.12")), ">=3.12,<3.13");
    assert_eq!(python_version_pin(Some("python3.12.1")), ">=3.12.1,<3.13");
    assert_eq!(python_version_pin(Some("python3")), "");
    assert_eq!(python_version_pin(Some("python")), "");
    assert_eq!(python_version_pin(Some("python2.7")), "");
    assert_eq!(python_version_pin(None), "");
}

#[test]
fn unknown_kinds_remain_open() {
    assert!(spec_for("future-kind").is_none());
}
