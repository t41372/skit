use std::path::Path;

use skit_application::{
    ExecutableDialect, ExecutableSourceFacts, ForcedAddKind, add_workdir, payload_stored_name,
    source_is_executable, supports_storage_modes,
};
use skit_domain::{EntryKind, StorageMode};

fn kind(value: &str) -> EntryKind {
    EntryKind::parse(value).unwrap()
}

#[test]
fn forced_add_kinds_are_closed_without_closing_stored_entry_kinds() {
    assert_eq!(
        ForcedAddKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>(),
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
            "ts",
            "exe",
        ]
    );
    assert_eq!(ForcedAddKind::parse("exe"), Some(ForcedAddKind::Executable));
    for value in ["prompt", "command", "unknown"] {
        assert_eq!(ForcedAddKind::parse(value), None, "{value}");
        assert_eq!(kind(value).as_str(), value, "stored kinds stay open");
    }
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

#[test]
fn test_infer_kind_windows_uses_pathext_not_execute_bit() {
    let windows = ExecutableDialect::Windows {
        pathext: Some(".COM;.EXE;.BAT;.CMD"),
    };
    assert!(source_is_executable(ExecutableSourceFacts {
        path: Path::new("tool.exe"),
        is_file: true,
        unix_mode: None,
        dialect: windows,
    }));
    assert!(!source_is_executable(ExecutableSourceFacts {
        path: Path::new("run.BAT"),
        is_file: false,
        unix_mode: None,
        dialect: windows,
    }));
    assert!(source_is_executable(ExecutableSourceFacts {
        path: Path::new("run.BAT"),
        is_file: true,
        unix_mode: None,
        dialect: windows,
    }));
    assert!(!source_is_executable(ExecutableSourceFacts {
        path: Path::new("notes.txt"),
        is_file: true,
        unix_mode: Some(0o777),
        dialect: windows,
    }));
}

#[test]
fn test_infer_kind_windows_reads_pathext_env() {
    let windows = ExecutableDialect::Windows {
        pathext: Some(".PY1;.fOo"),
    };
    assert!(source_is_executable(ExecutableSourceFacts {
        path: Path::new("thing.FOO"),
        is_file: true,
        unix_mode: None,
        dialect: windows,
    }));
    assert!(!source_is_executable(ExecutableSourceFacts {
        path: Path::new("thing.exe"),
        is_file: true,
        unix_mode: None,
        dialect: windows,
    }));
}

#[test]
fn test_infer_kind_windows_falls_back_to_default_pathext() {
    for pathext in [None, Some("")] {
        assert!(source_is_executable(ExecutableSourceFacts {
            path: Path::new("go.bat"),
            is_file: true,
            unix_mode: None,
            dialect: ExecutableDialect::Windows { pathext },
        }));
    }
}

#[test]
fn posix_executable_inference_uses_only_a_real_files_execute_bits() {
    for (is_file, unix_mode, expected) in [
        (true, Some(0o755), true),
        (true, Some(0o644), false),
        (false, Some(0o755), false),
        (true, None, false),
    ] {
        assert_eq!(
            source_is_executable(ExecutableSourceFacts {
                path: Path::new("extensionless"),
                is_file,
                unix_mode,
                dialect: ExecutableDialect::Posix,
            }),
            expected
        );
    }
}
