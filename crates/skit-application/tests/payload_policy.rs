use std::path::Path;

use skit_application::{add_workdir, payload_stored_name, supports_storage_modes};
use skit_domain::{EntryKind, StorageMode};

fn kind(value: &str) -> EntryKind {
    EntryKind::parse(value).unwrap()
}

#[test]
fn add_workdir_keeps_script_prompt_executable_and_command_lanes_distinct() {
    assert_eq!(add_workdir(&kind("python"), StorageMode::Copy), "invoke");
    assert_eq!(
        add_workdir(&kind("python"), StorageMode::Reference),
        "origin"
    );
    assert_eq!(add_workdir(&kind("prompt"), StorageMode::Copy), "invoke");
    assert_eq!(
        add_workdir(&kind("prompt"), StorageMode::Reference),
        "invoke"
    );
    assert_eq!(add_workdir(&kind("exe"), StorageMode::Reference), "origin");
    assert_eq!(
        add_workdir(&kind("command"), StorageMode::Reference),
        "invoke"
    );
}

#[test]
fn payload_names_use_the_library_layout_and_preserve_module_extensions() {
    let cases = [
        ("python", "custom.py", "script.py"),
        ("shell", "deploy.zsh", "script.sh"),
        ("fish", "deploy.fish", "script.fish"),
        ("powershell", "deploy.ps1", "script.ps1"),
        ("ruby", "deploy.rb", "script.rb"),
        ("perl", "deploy.pl", "script.pl"),
        ("lua", "deploy.lua", "script.lua"),
        ("r", "deploy.R", "script.r"),
        ("prompt", "review.prompt.md", "prompt.md"),
        ("js", "module.mjs", "script.mjs"),
        ("js", "module.cjs", "script.cjs"),
        ("ts", "module.mts", "script.mts"),
        ("ts", "module.cts", "script.cts"),
        ("js", "module.txt", "script.js"),
        ("exe", "my-tool", "my-tool"),
        ("new-kind", "source.xyz", "payload"),
    ];
    for (entry_kind, source, expected) in cases {
        assert_eq!(
            payload_stored_name(&kind(entry_kind), Path::new(source)),
            expected,
            "{entry_kind} from {source}"
        );
    }
}

#[test]
fn storage_mode_capability_matches_the_latest_main_language_registry() {
    for entry_kind in [
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
        "prompt",
    ] {
        assert!(supports_storage_modes(&kind(entry_kind)), "{entry_kind}");
    }
    for entry_kind in ["exe", "command", "future-kind"] {
        assert!(!supports_storage_modes(&kind(entry_kind)), "{entry_kind}");
    }
}
