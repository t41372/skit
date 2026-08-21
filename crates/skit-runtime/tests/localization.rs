//! Every runtime error must present a complete message in each supported locale.

use std::{io, path::PathBuf};

use skit_i18n::{Locale, Localize};
use skit_runtime::{
    DependencyError, LaunchError, UvBootstrapError, javascript_dependency_install_announcement,
};

/// Check that English text does not drift and that each locale keeps the values.
fn assert_localized(error: &(impl Localize + std::fmt::Display), values: &[&str]) {
    let message = error.message();
    assert_eq!(error.to_string(), message.localize(Locale::En));
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let text = message.localize(locale);
        let template = message.template();
        assert!(!text.trim().is_empty(), "{template} is empty");
        assert!(!text.contains("{}"), "{template} kept an empty hole");
        for value in values {
            assert!(text.contains(value), "{text} lost the value {value}");
        }
    }
}

fn io_failure() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "permission denied")
}

#[test]
fn every_dependency_error_localizes_and_keeps_its_values() {
    assert_localized(&DependencyError::CopyStorageRequired, &[]);
    assert_localized(
        &DependencyError::InvalidPackage {
            value: "@@bad".to_owned(),
        },
        &["@@bad"],
    );
    assert_localized(
        &DependencyError::UnsupportedRuntime {
            runtime: "deno".to_owned(),
        },
        &["deno"],
    );
    assert_localized(
        &DependencyError::InstallerNotFound {
            name: "npm".to_owned(),
        },
        &["npm"],
    );
    assert_localized(
        &DependencyError::Io {
            operation: "create",
            path: "/entries/demo".to_owned(),
            reason: "permission denied".to_owned(),
        },
        &["/entries/demo", "permission denied"],
    );
    assert_localized(
        &DependencyError::ClearFailed {
            item: "package.json".to_owned(),
            reason: "permission denied".to_owned(),
        },
        &["package.json", "permission denied"],
    );
    assert_localized(
        &DependencyError::InstallFailed {
            installer: "npm".to_owned(),
            exit_code: Some(23),
            detail: "package missing".to_owned(),
        },
        &["npm", "package missing"],
    );
    assert_localized(
        &DependencyError::InstallerStartFailed {
            installer: "npm".to_owned(),
            reason: "permission denied".to_owned(),
        },
        &["npm", "permission denied"],
    );
    assert_localized(
        &DependencyError::Rollback {
            path: "/entries/demo".to_owned(),
            primary: Box::new(DependencyError::Io {
                operation: "rename",
                path: "/entries/demo".to_owned(),
                reason: "device is busy".to_owned(),
            }),
            rollback: Box::new(DependencyError::Io {
                operation: "remove",
                path: "/entries/demo".to_owned(),
                reason: "permission denied".to_owned(),
            }),
        },
        &["/entries/demo", "device is busy", "permission denied"],
    );
}

#[test]
fn dependency_install_receipts_match_v040_in_every_locale() {
    let announcement = javascript_dependency_install_announcement("npm");
    let start = DependencyError::InstallerStartFailed {
        installer: "npm".to_owned(),
        reason: "Exec format error".to_owned(),
    };
    let failed = DependencyError::InstallFailed {
        installer: "npm".to_owned(),
        exit_code: Some(1),
        detail: "npm error it failed".to_owned(),
    };
    for (locale, expected_announcement, expected_start, expected_failed) in [
        (
            Locale::En,
            "Installing dependencies (npm)…",
            "Couldn't run npm: Exec format error",
            "Installing dependencies failed (npm): npm error it failed",
        ),
        (
            Locale::ZhCn,
            "正在安装依赖(npm)…",
            "无法运行 npm:Exec format error",
            "依赖安装失败(npm):npm error it failed",
        ),
        (
            Locale::ZhTw,
            "正在安裝依賴(npm)…",
            "無法執行 npm:Exec format error",
            "依賴安裝失敗(npm):npm error it failed",
        ),
    ] {
        assert_eq!(announcement.localize(locale), expected_announcement);
        assert_eq!(start.message().localize(locale), expected_start);
        assert_eq!(failed.message().localize(locale), expected_failed);
    }
}

#[test]
fn dependency_cleanup_refusal_matches_v040_in_every_locale() {
    let error = DependencyError::ClearFailed {
        item: "package.json".to_owned(),
        reason: "held by another process".to_owned(),
    };
    let message = error.message();
    assert_eq!(
        message.localize(Locale::En),
        "Couldn't clear the old dependency environment: package.json: held by another process"
    );
    assert_eq!(
        message.localize(Locale::ZhCn),
        "无法清除旧的依赖环境:package.json: held by another process"
    );
    assert_eq!(
        message.localize(Locale::ZhTw),
        "無法清除舊的依賴環境:package.json: held by another process"
    );
}

#[test]
fn every_launch_error_localizes_and_keeps_its_values() {
    assert_localized(
        &LaunchError::UnknownKind {
            kind: "brainfuck".to_owned(),
        },
        &["brainfuck"],
    );
    assert_localized(
        &LaunchError::TargetMissing {
            path: PathBuf::from("/gone/script.py"),
        },
        &["/gone/script.py"],
    );
    assert_localized(
        &LaunchError::TargetNotExecutable {
            path: PathBuf::from("/data/tool"),
        },
        &["/data/tool"],
    );
    assert_localized(
        &LaunchError::ProgramNotFound {
            name: "python3".to_owned(),
        },
        &["python3"],
    );
    assert_localized(
        &LaunchError::MissingNeed {
            name: "rsync".to_owned(),
        },
        &["rsync"],
    );
    assert_localized(
        &LaunchError::WorkdirMissing {
            path: PathBuf::from("/gone"),
        },
        &["/gone"],
    );
    assert_localized(
        &LaunchError::InvalidWorkdir {
            value: "relative/path".to_owned(),
        },
        &["relative/path"],
    );
    assert_localized(
        &LaunchError::MissingTemplateValue {
            name: "target".to_owned(),
        },
        &["target"],
    );
    assert_localized(
        &LaunchError::UnsafeTemplatePlaceholder {
            name: "target".to_owned(),
        },
        &[],
    );
    assert_localized(&LaunchError::PromptRunnerRequired, &[]);
    assert_localized(&LaunchError::PromptBodyRequired, &[]);
    assert_localized(
        &LaunchError::InvalidPromptRunner {
            name: "claude".to_owned(),
        },
        &["claude"],
    );
    assert_localized(&LaunchError::PromptContainsNul, &[]);
    assert_localized(
        &LaunchError::PromptArgvTooLong {
            size: 300_000,
            limit: 131_072,
            unit: "bytes",
        },
        &["300000", "131072", "bytes"],
    );
    assert_localized(
        &LaunchError::Process {
            operation: "start",
            source: io_failure(),
        },
        &["permission denied"],
    );
}

#[test]
fn every_uv_bootstrap_error_localizes_and_keeps_its_values() {
    assert_localized(
        &UvBootstrapError::UnsupportedPlatform {
            platform: "plan9-riscv".to_owned(),
        },
        &["plan9-riscv"],
    );
    assert_localized(
        &UvBootstrapError::Download {
            url: "https://example.invalid/uv.tar.gz".to_owned(),
            reason: "connection refused".to_owned(),
        },
        &["https://example.invalid/uv.tar.gz", "connection refused"],
    );
    assert_localized(
        &UvBootstrapError::Checksum {
            expected: "aaaa".to_owned(),
            actual: "bbbb".to_owned(),
        },
        &["aaaa", "bbbb"],
    );
    assert_localized(
        &UvBootstrapError::NoPinnedChecksum {
            triple: "riscv64-unknown-linux-gnu".to_owned(),
        },
        &["riscv64-unknown-linux-gnu"],
    );
    assert_localized(
        &UvBootstrapError::Archive {
            reason: "the executable is absent".to_owned(),
        },
        &["the executable is absent"],
    );
    assert_localized(
        &UvBootstrapError::Io {
            operation: "rename",
            path: "/data/uv".to_owned(),
            source: io_failure(),
        },
        &["/data/uv", "permission denied"],
    );
}

#[test]
fn a_multi_value_message_places_every_value_in_its_own_hole() {
    // `contains` cannot see a swapped hole order, so pin the whole rendering.
    let error = DependencyError::Io {
        operation: "create",
        path: "/entries/demo".to_owned(),
        reason: "permission denied".to_owned(),
    };

    assert_eq!(
        error.message().localize(Locale::En),
        "could not create JavaScript dependencies at /entries/demo: permission denied"
    );
    assert_eq!(
        error.message().localize(Locale::ZhCn),
        "无法创建 /entries/demo 处的 JavaScript 依赖项：permission denied"
    );

    let uv = UvBootstrapError::Io {
        operation: "rename",
        path: "/data/bin/uv".to_owned(),
        source: io_failure(),
    };
    assert_eq!(
        uv.message().localize(Locale::ZhTw),
        "無法重新命名 /data/bin/uv 處的專用 uv：permission denied"
    );
}

#[test]
fn a_prompt_runner_refusal_keeps_its_literal_marker() {
    let error = LaunchError::InvalidPromptRunner {
        name: "claude".to_owned(),
    };
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        assert!(error.message().localize(locale).contains("{{prompt}}"));
    }
}
