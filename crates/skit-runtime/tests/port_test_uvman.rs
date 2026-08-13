//! Public-surface ports for the Rust-representable contracts in Python `tests/test_uvman.py` at
//! `main@206f9ef`. The companion manifest classifies private Python fault-injection seams that have
//! no public Rust equivalent. Opt-in network checks keep the oracle's `SKIT_NET_TESTS` gate.

use std::{env, fs, net::TcpListener};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest as _, Sha256};
use skit_runtime::{
    UV_VERSION, UvAsset, UvBootstrapError, UvTarget, ensure_managed_uv,
    install_verified_uv_archive, managed_uv_path, uv_asset,
};
use tempfile::TempDir;

fn target(arch: &str, os: &str, musl: bool) -> UvTarget {
    UvTarget::from_parts(arch, os, musl).unwrap()
}

fn all_targets() -> Vec<UvTarget> {
    vec![
        target("x86_64", "linux", false),
        target("aarch64", "linux", false),
        target("x86_64", "linux", true),
        target("aarch64", "linux", true),
        target("x86_64", "darwin", false),
        target("aarch64", "darwin", false),
        target("x86_64", "windows", false),
        target("aarch64", "windows", false),
    ]
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tar_gz(member_name: &str, bytes: &[u8]) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    archive
        .append_data(
            &mut header,
            format!("uv-test/{member_name}"),
            bytes,
        )
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap()
}

fn tar_asset(archive: &[u8], executable_name: &str) -> UvAsset {
    UvAsset {
        version: "test".to_owned(),
        filename: "uv-test.tar.gz".to_owned(),
        url: "https://example.invalid/uv-test.tar.gz".to_owned(),
        checksum: digest(archive),
        executable_name: executable_name.to_owned(),
    }
}

fn network_enabled() -> bool {
    env::var_os("SKIT_NET_TESTS").is_some()
}

fn assert_release_exists(target: &UvTarget) {
    if !network_enabled() {
        return;
    }
    let asset = uv_asset(target, None);
    let response = ureq::head(&asset.url)
        .call()
        .unwrap_or_else(|error| panic!("HEAD {} failed: {error}", asset.url));
    assert_eq!(
        response.status().as_u16(),
        200,
        "pinned uv release URL is not live: {}",
        asset.url
    );
}

fn assert_sidecar_matches(target: &UvTarget) {
    if !network_enabled() {
        return;
    }
    let asset = uv_asset(target, None);
    let sidecar = format!("{}.sha256", asset.url);
    let mut response = ureq::get(&sidecar)
        .call()
        .unwrap_or_else(|error| panic!("GET {sidecar} failed: {error}"));
    let body = response
        .body_mut()
        .read_to_string()
        .unwrap_or_else(|error| panic!("could not read {sidecar}: {error}"));
    let official = body
        .split_whitespace()
        .next()
        .expect("the official sidecar contains one digest");
    assert_eq!(
        official, asset.checksum,
        "the pinned checksum drifted from {sidecar}"
    );
}

#[test]
fn test_pinned_uv_release_exists() {
    for target in all_targets() {
        assert_release_exists(&target);
    }
}

#[test]
fn rust_additive_pinned_uv_release_exists_x86_64_linux_gnu() {
    assert_release_exists(&target("x86_64", "linux", false));
}

#[test]
fn rust_additive_pinned_uv_release_exists_aarch64_linux_gnu() {
    assert_release_exists(&target("aarch64", "linux", false));
}

#[test]
fn rust_additive_pinned_uv_release_exists_x86_64_linux_musl() {
    assert_release_exists(&target("x86_64", "linux", true));
}

#[test]
fn rust_additive_pinned_uv_release_exists_aarch64_linux_musl() {
    assert_release_exists(&target("aarch64", "linux", true));
}

#[test]
fn rust_additive_pinned_uv_release_exists_x86_64_macos() {
    assert_release_exists(&target("x86_64", "darwin", false));
}

#[test]
fn rust_additive_pinned_uv_release_exists_aarch64_macos() {
    assert_release_exists(&target("aarch64", "darwin", false));
}

#[test]
fn rust_additive_pinned_uv_release_exists_x86_64_windows() {
    assert_release_exists(&target("x86_64", "windows", false));
}

#[test]
fn rust_additive_pinned_uv_release_exists_aarch64_windows() {
    assert_release_exists(&target("aarch64", "windows", false));
}

#[test]
fn test_pinned_sha256_matches_live_sidecar() {
    for target in all_targets() {
        assert_sidecar_matches(&target);
    }
}

#[test]
fn rust_additive_pinned_sha256_matches_x86_64_linux_gnu() {
    assert_sidecar_matches(&target("x86_64", "linux", false));
}

#[test]
fn rust_additive_pinned_sha256_matches_aarch64_linux_gnu() {
    assert_sidecar_matches(&target("aarch64", "linux", false));
}

#[test]
fn rust_additive_pinned_sha256_matches_x86_64_linux_musl() {
    assert_sidecar_matches(&target("x86_64", "linux", true));
}

#[test]
fn rust_additive_pinned_sha256_matches_aarch64_linux_musl() {
    assert_sidecar_matches(&target("aarch64", "linux", true));
}

#[test]
fn rust_additive_pinned_sha256_matches_x86_64_macos() {
    assert_sidecar_matches(&target("x86_64", "darwin", false));
}

#[test]
fn rust_additive_pinned_sha256_matches_aarch64_macos() {
    assert_sidecar_matches(&target("aarch64", "darwin", false));
}

#[test]
fn rust_additive_pinned_sha256_matches_x86_64_windows() {
    assert_sidecar_matches(&target("x86_64", "windows", false));
}

#[test]
fn rust_additive_pinned_sha256_matches_aarch64_windows() {
    assert_sidecar_matches(&target("aarch64", "windows", false));
}

#[test]
fn test_triple_unsupported_arch_raises() {
    let error = UvTarget::from_parts("mips", "linux", false).unwrap_err();
    assert!(
        matches!(&error, UvBootstrapError::UnsupportedPlatform { .. }),
        "unexpected error: {error:?}"
    );
    assert!(error.to_string().to_lowercase().contains("unsupported"));
}

#[test]
fn test_triple_darwin_aarch64() {
    assert_eq!(
        target("arm64", "darwin", false).triple(),
        "aarch64-apple-darwin"
    );
}

#[test]
fn test_triple_windows_x86_64() {
    assert_eq!(
        target("AMD64", "windows", false).triple(),
        "x86_64-pc-windows-msvc"
    );
}

#[test]
fn test_triple_linux_aarch64() {
    assert_eq!(
        target("aarch64", "linux", false).triple(),
        "aarch64-unknown-linux-gnu"
    );
}

#[test]
fn test_triple_linux_musl_x86_64() {
    assert_eq!(
        target("x86_64", "linux", true).triple(),
        "x86_64-unknown-linux-musl"
    );
}

#[test]
fn test_triple_linux_musl_aarch64() {
    assert_eq!(
        target("aarch64", "linux", true).triple(),
        "aarch64-unknown-linux-musl"
    );
}

#[test]
fn test_download_url_musl_triple_targz() {
    let target = target("x86_64", "linux", true);
    let asset = uv_asset(&target, None);
    assert!(asset.url.ends_with(".tar.gz"), "{}", asset.url);
    assert!(
        asset.url.contains(&format!(
            "{UV_VERSION}/uv-x86_64-unknown-linux-musl.tar.gz"
        )),
        "{}",
        asset.url
    );
}

#[test]
fn test_download_url_structure() {
    let linux = uv_asset(&target("x86_64", "linux", false), None);
    assert!(linux.url.contains(UV_VERSION), "{}", linux.url);
    assert!(linux.url.ends_with(".tar.gz"), "{}", linux.url);

    let windows = uv_asset(&target("x86_64", "windows", false), None);
    assert!(windows.url.ends_with(".zip"), "{}", windows.url);
}

#[test]
fn test_ensure_uv_already_exists() {
    let root = TempDir::new().unwrap();
    let executable = managed_uv_path(root.path());
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, b"already installed").unwrap();

    let resolved = ensure_managed_uv(root.path(), Some("http://127.0.0.1:1"))
        .expect("an existing private uv must bypass all download work");
    assert_eq!(resolved, executable);
    assert_eq!(fs::read(resolved).unwrap(), b"already installed");
}

#[test]
fn test_extract_uv_no_exe_in_archive_raises() {
    let archive = tar_gz("README.txt", b"nothing here\n");
    let asset = tar_asset(&archive, "uv");
    let root = TempDir::new().unwrap();

    let error = install_verified_uv_archive(&archive, &asset, root.path()).unwrap_err();
    assert!(
        matches!(&error, UvBootstrapError::Archive { .. }),
        "unexpected error: {error:?}"
    );
    assert!(error.to_string().contains("uv"), "{error}");
    assert!(!root.path().join("uv").exists());
}

#[test]
fn test_ensure_uv_network_error_wrapped() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mirror = format!("http://{address}");
    let root = TempDir::new().unwrap();

    let error = ensure_managed_uv(root.path(), Some(&mirror)).unwrap_err();
    let UvBootstrapError::Download { url, reason } = error else {
        panic!("network failure was not wrapped as UvBootstrapError::Download");
    };
    assert!(url.starts_with(&mirror), "{url}");
    assert!(!reason.trim().is_empty());
    assert!(!managed_uv_path(root.path()).exists());
}

#[test]
fn test_checksum_pass_proceeds_to_extraction() {
    let archive = tar_gz("uv", b"known-good-uv-archive-bytes");
    let asset = tar_asset(&archive, "uv");
    let root = TempDir::new().unwrap();

    let installed = install_verified_uv_archive(&archive, &asset, root.path()).unwrap();
    assert_eq!(installed, root.path().join("uv"));
    assert_eq!(fs::read(installed).unwrap(), b"known-good-uv-archive-bytes");
}

#[test]
fn test_checksum_mismatch_raises_checksum_error_not_generic() {
    let archive = tar_gz("uv", b"tampered-bytes-from-a-hostile-mirror");
    let actual = digest(&archive);
    let pinned = "00".repeat(32);
    let mut asset = tar_asset(&archive, "uv");
    asset.checksum = pinned.clone();
    let root = TempDir::new().unwrap();

    let error = install_verified_uv_archive(&archive, &asset, root.path()).unwrap_err();
    assert!(matches!(&error, UvBootstrapError::Checksum));
    let message = error.to_string();
    assert!(message.to_lowercase().contains("checksum"), "{message}");
    assert!(
        !message.contains("could not download uv"),
        "checksum mismatch was hidden by a generic download error: {message}"
    );
    // Python v0.4 surfaces both values so a bad pin can be diagnosed without reproducing the
    // download. Keep the exact diagnostic contract even if the current Rust error is deliberately
    // red here.
    assert!(message.contains(&pinned), "missing expected digest: {message}");
    assert!(message.contains(&actual), "missing actual digest: {message}");
    assert!(!root.path().join("uv").exists());
}
