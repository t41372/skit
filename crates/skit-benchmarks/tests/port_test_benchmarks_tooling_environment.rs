//! Frozen environment/provenance contracts from `tests/test_benchmarks_tooling.py`.

use std::{fs, path::{Path, PathBuf}, process::Command, sync::OnceLock};

use skit_benchmarks::{
    BenchmarkProfile,
    environment::{collect_meta, platform_key, pull_request_number, version_from_output},
};

fn version_probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE
        .get_or_init(|| {
            let root = std::env::temp_dir().join(format!(
                "skit-benchmark-version-probe-{}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            let source = root.join("version_probe.rs");
            fs::write(
                &source,
                r#"fn main() { println!("uv 0.11.26"); }"#,
            )
            .unwrap();
            let executable = root.join(if cfg!(windows) {
                "version-probe.exe"
            } else {
                "version-probe"
            });
            let status = Command::new("rustc")
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .status()
                .expect("run rustc for benchmark version probe");
            assert!(status.success(), "failed to compile benchmark version probe");
            executable
        })
        .as_path()
}

#[test]
fn test_platform_key() {
    assert_eq!(platform_key("Linux", "x86_64"), "linux-x86_64");
    assert_eq!(platform_key("Darwin", "arm64"), "darwin-aarch64");
    assert_eq!(platform_key("Linux", "AMD64"), "linux-x86_64");
}

#[test]
fn test_uv_version_from_output() {
    assert_eq!(
        version_from_output("uv 0.11.26 (abc 2026-01-01)"),
        "0.11.26"
    );
    assert_eq!(
        version_from_output("garbage"),
        "unknown",
        "malformed `uv --version` output must not be mistaken for a version token"
    );
}

#[test]
fn test_installed_uv_version() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-benchmarks lives under <repo>/crates/skit-benchmarks");
    let probe = version_probe();
    let with_uv = collect_meta(BenchmarkProfile::Pr, repo, probe, None, Some(probe)).unwrap();
    assert_eq!(with_uv.uv, "0.11.26");

    let without_uv = collect_meta(BenchmarkProfile::Pr, repo, probe, None, None).unwrap();
    assert_eq!(without_uv.uv, "unknown");
}

#[test]
fn test_pull_request_number() {
    for (reference, expected) in [
        ("refs/pull/29/merge", Some("29")),
        ("refs/pull/1234/head", Some("1234")),
        ("refs/heads/main", None),
        ("refs/tags/v0.4.0", None),
        ("", None),
        ("refs/pull//merge", None),
        ("refs/pull/main/merge", None),
        ("refs/pull/29", None),
    ] {
        assert_eq!(
            pull_request_number(reference).as_deref(),
            expected,
            "unexpected PR number for {reference:?}"
        );
    }
}
