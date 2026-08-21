//! Mechanical port of the Python oracle module `tests/test_uvman.py`
//! (`origin/main@206f9ef`): private-uv bootstrap — target/triple resolution, download URL
//! construction, SHA256 pinning, checksum-gated atomic install, and download consent.
//! Each `#[test]` keeps its Python `def test_*` name so it traces back to its origin, and each
//! Python "WHY" comment is preserved above it.
//!
//! Concept mapping used throughout:
//! - Python `uvman._triple()` (reads `platform.machine`/`sys.platform`/`_is_musl`) ->
//!   `UvTarget::from_parts(arch, os, musl)` (the Rust API takes the three inputs explicitly, so a
//!   monkeypatched host becomes three literal arguments). OS names are Rust's normalized form:
//!   Python `sys.platform` "win32" -> "windows", "darwin" -> "darwin"/"macos", "linux" -> "linux".
//! - Python `uvman.download_url(triple)` -> `uv_asset(&target, mirror_base).url`.
//! - Python `uvman.UV_VERSION` -> `skit_runtime::UV_VERSION` (deliberately bumped 0.11.26 -> 0.12.3).
//! - Python `uvman._UV_SHA256[triple]` -> `uv_asset(&target, None).checksum` (the private `CHECKSUMS`
//!   table surfaced through the built asset).
//! - Python `uvman._extract_uv(archive, dest)` + `_verify_checksum` -> `install_verified_uv_archive`
//!   (Rust folds checksum verification and extraction into one call).
//! - Python `uvman.ensure_uv_downloaded(quiet=True)` -> `ensure_managed_uv(data_dir, mirror_base)`.
//! - Python `UvDownloadError` / `UvDeclinedError` -> `UvBootstrapError` / `UvBootstrapError::Declined`.
//!
//! Buckets:
//! - Bucket 1 (target/URL/checksum/atomic-install byte logic): the bulk of real asserting tests
//!   below, driving the real public API.
//! - Bucket 2 (owning private/cross-crate targets): consent and mirror composition live in existing
//!   skit-cli policy, PTY, and workflow targets. Musl detection and install durability run against
//!   crate-private typed seams in `uv.rs`. The frozen exact names stay unique across those owners.
//! - Bucket 3 (semantic closures and platform gates, `#[ignore]`d): quiet bootstrap and unpinned
//!   construction map to stronger Rust owners. Directory-fsync omission remains a Windows-host gate.
//! - Bucket 4 (opt-in network liveness, `#[ignore]`d): the two `@net` tests, faithful to the Python
//!   `SKIT_NET_TESTS` skip; run them when bumping `UV_VERSION`.

use std::{collections::BTreeSet, fs, io::Write as _, net::TcpListener, time::Duration};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest as _, Sha256};
use skit_runtime::{
    UV_VERSION, UvAsset, UvBootstrapError, UvTarget, ensure_managed_uv,
    install_verified_uv_archive, managed_uv_path, uv_asset,
};
use tempfile::TempDir;

// --- The oracle's module-level TRIPLES fixture (the exact set the pin/liveness checks iterate). ---
const TRIPLES: [&str; 8] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

/// Every target `_triple()` can emit: {x86_64, aarch64} x {apple-darwin, unknown-linux-gnu,
/// unknown-linux-musl, pc-windows-msvc}. Built by exhausting the `from_parts` inputs, mirroring the
/// oracle's `_producible_triples` (which monkeypatches machine/platform/`_is_musl`).
fn all_targets() -> Vec<UvTarget> {
    let mut targets = Vec::new();
    for arch in ["x86_64", "arm64"] {
        targets.push(UvTarget::from_parts(arch, "darwin", false).unwrap());
        targets.push(UvTarget::from_parts(arch, "windows", false).unwrap());
        // linux additionally branches on libc flavor; cover both to reach the musl triples.
        targets.push(UvTarget::from_parts(arch, "linux", false).unwrap());
        targets.push(UvTarget::from_parts(arch, "linux", true).unwrap());
    }
    targets
}

fn producible_triples() -> BTreeSet<String> {
    all_targets()
        .iter()
        .map(|t| t.triple().to_owned())
        .collect()
}

/// Lowercase-hex SHA256, matching the Rust bootstrap's own `hex_digest`.
fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Oracle helper `_tar_gz_with_uv`: a real tar.gz holding a single executable member `exe_name`
/// (nested under `uv-1.0/`, like the real release layout), for the extraction/copy pipeline.
fn tar_gz_with_uv(exe_name: &str, content: &[u8]) -> Vec<u8> {
    let mut archive = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut tar = tar::Builder::new(&mut archive);
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("uv-1.0/{exe_name}"), content)
            .unwrap();
        tar.finish().unwrap();
    }
    archive.flush().unwrap();
    archive.finish().unwrap()
}

/// A real tar.gz with only an unrelated file (no `uv` member) — the oracle's "empty" archive.
fn tar_gz_readme_only() -> Vec<u8> {
    let mut archive = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut tar = tar::Builder::new(&mut archive);
        let bytes = b"nothing here\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "README.txt", bytes.as_slice())
            .unwrap();
        tar.finish().unwrap();
    }
    archive.flush().unwrap();
    archive.finish().unwrap()
}

/// An asset whose pinned checksum matches `archive`, so the checksum gate opens and extraction runs.
fn asset_for(filename: &str, executable_name: &str, archive: &[u8]) -> UvAsset {
    UvAsset {
        version: UV_VERSION.to_owned(),
        filename: filename.to_owned(),
        url: "https://example.invalid/archive".to_owned(),
        checksum: hex_digest(archive),
        executable_name: executable_name.to_owned(),
    }
}

/// A base URL whose port is closed: any fetch against it fails with a connection error. Used to
/// prove either that a fetch never happens or that a network failure surfaces as a typed error.
/// Mirrors the refused-port idiom in the crate's own `private_tests`.
fn dead_mirror() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

fn ureq_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into()
}

// ---- TRIPLES / pin coverage ----------

#[test]
fn test_triples_covers_every_pinned_and_producible_triple() {
    // TRIPLES is exactly what the two network tests below iterate over. If it ever drifts from the
    // full set of pinned / producible triples, the live cross-check would silently stop covering the
    // missing triple(s), and a bad pin could ship undetected. This test is offline but proves the
    // drift cannot happen silently: any future producible triple must also be added here.
    let produced = producible_triples();
    let declared: BTreeSet<String> = TRIPLES.iter().map(|t| (*t).to_owned()).collect();
    assert_eq!(declared, produced);
    // Every producible triple resolves to a pinned asset: `uv_asset` fails closed on a missing
    // pin, so an Ok for each target proves the producible -> pinned direction (Python's
    // `== _UV_SHA256`). The reverse (no EXTRA stale pin) cannot be checked here — `CHECKSUMS` is
    // private to skit-runtime.
    for target in all_targets() {
        assert!(uv_asset(&target, None).is_ok());
    }
}

#[test]
#[ignore = "external network gate: run with SKIT_NET_TESTS=1 and --ignored when UV_VERSION changes; target: Astral uv release assets"]
fn test_pinned_uv_release_exists() {
    let agent = ureq_agent();
    for target in all_targets() {
        let url = uv_asset(&target, None).unwrap().url;
        let response = agent
            .get(&url)
            .call()
            .expect("release asset should be reachable");
        assert_eq!(response.status().as_u16(), 200, "{url}");
    }
}

#[test]
#[ignore = "external network gate: run with SKIT_NET_TESTS=1 and --ignored when UV_VERSION changes; target: Astral uv SHA-256 sidecars"]
fn test_pinned_sha256_matches_live_sidecar() {
    // A future UV_VERSION bump that forgets to refresh the pinned table must fail loudly here: every
    // pinned hash must equal the official `.sha256` sidecar. Built from the canonical GitHub base
    // (mirror_base = None) so a configured mirror cannot skew the check.
    let agent = ureq_agent();
    for target in all_targets() {
        let asset = uv_asset(&target, None).unwrap();
        let sidecar = format!("{}.sha256", asset.url);
        let mut response = agent
            .get(&sidecar)
            .call()
            .expect("sidecar should be reachable");
        assert_eq!(response.status().as_u16(), 200, "{sidecar}");
        let body = response
            .body_mut()
            .with_config()
            .limit(4096)
            .read_to_vec()
            .expect("sidecar body should read");
        let text = String::from_utf8_lossy(&body);
        let official = text.split_whitespace().next().unwrap_or_default();
        assert_eq!(official, asset.checksum, "{}", target.triple());
    }
}

// ---- Download consent (_ask_consent) ----------

// ---- _triple: architecture / platform resolution ----------

#[test]
fn test_triple_unsupported_arch_raises() {
    let error = UvTarget::from_parts("mips", "linux", false).unwrap_err();
    assert!(matches!(
        error,
        UvBootstrapError::UnsupportedPlatform { .. }
    ));
    assert!(error.to_string().to_lowercase().contains("unsupported"));
}

#[test]
fn test_triple_darwin_aarch64() {
    assert_eq!(
        UvTarget::from_parts("arm64", "darwin", false)
            .unwrap()
            .triple(),
        "aarch64-apple-darwin",
    );
}

#[test]
fn test_triple_windows_x86_64() {
    // "AMD64" exercises the amd64 -> x86_64 alias; Rust os name "windows" stands in for sys.platform
    // "win32".
    assert_eq!(
        UvTarget::from_parts("AMD64", "windows", false)
            .unwrap()
            .triple(),
        "x86_64-pc-windows-msvc",
    );
}

#[test]
fn test_triple_linux_aarch64() {
    assert_eq!(
        UvTarget::from_parts("aarch64", "linux", false)
            .unwrap()
            .triple(),
        "aarch64-unknown-linux-gnu",
    );
}

// ---- _triple / _is_musl: musl (Alpine) detection ----------

#[test]
fn test_triple_linux_musl_x86_64() {
    // On a musl userland (e.g. Alpine), the target must be musl, not gnu — a gnu uv binary cannot
    // exec without glibc's dynamic loader.
    assert_eq!(
        UvTarget::from_parts("x86_64", "linux", true)
            .unwrap()
            .triple(),
        "x86_64-unknown-linux-musl",
    );
}

#[test]
fn test_triple_linux_musl_aarch64() {
    assert_eq!(
        UvTarget::from_parts("aarch64", "linux", true)
            .unwrap()
            .triple(),
        "aarch64-unknown-linux-musl",
    );
}

#[test]
fn test_download_url_musl_triple_targz() {
    // The musl triples are Linux, so they must still map to the tar.gz archive extension (only the
    // windows triples use .zip).
    let target = UvTarget::from_parts("x86_64", "linux", true).unwrap();
    let url = uv_asset(&target, None).unwrap().url;
    assert!(url.ends_with(".tar.gz"));
    assert!(url.contains(&format!("{UV_VERSION}/uv-x86_64-unknown-linux-musl.tar.gz")));
}

// ---- download_url ----------

#[test]
fn test_download_url_structure() {
    let gnu = UvTarget::from_parts("x86_64", "linux", false).unwrap();
    let url = uv_asset(&gnu, None).unwrap().url;
    assert!(url.contains(UV_VERSION));
    assert!(url.ends_with(".tar.gz"));
    let windows = UvTarget::from_parts("x86_64", "windows", false).unwrap();
    assert!(uv_asset(&windows, None).unwrap().url.ends_with(".zip"));
}

// ---- ensure_uv_downloaded: binary already exists ----------

#[test]
fn test_ensure_uv_already_exists() {
    // If the binary is already present, skip download and return the path immediately. A dead mirror
    // proves the network is never touched when the binary already exists.
    let data_dir = TempDir::new().unwrap();
    let exe = managed_uv_path(data_dir.path());
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::write(&exe, "existing").unwrap();
    let result = ensure_managed_uv(data_dir.path(), Some(dead_mirror().as_str())).unwrap();
    assert_eq!(result, exe);
}

// ---- _extract_uv: missing executable in archive raises ----------

#[test]
fn test_extract_uv_no_exe_in_archive_raises() {
    // An archive that contains no 'uv' executable must raise (Archive). The asset's checksum matches
    // the archive, so the checksum gate opens and extraction is actually reached.
    let archive = tar_gz_readme_only();
    let asset = asset_for("empty.tar.gz", "uv", &archive);
    let destination = TempDir::new().unwrap();
    assert!(matches!(
        install_verified_uv_archive(&archive, &asset, destination.path()),
        Err(UvBootstrapError::Archive { .. }),
    ));
}

// ---- ensure_uv_downloaded: network error wrapped as a typed error ----------

#[test]
fn test_ensure_uv_network_error_wrapped() {
    // A network failure must surface as a typed error, not a panic. A dead mirror (closed port)
    // makes the fetch fail with a connection error, wrapped as UvBootstrapError::Download.
    let data_dir = TempDir::new().unwrap();
    let error = ensure_managed_uv(data_dir.path(), Some(dead_mirror().as_str())).unwrap_err();
    assert!(matches!(error, UvBootstrapError::Download { .. }));
}

// ---- SHA256 pinning + checksum verification (no network) ----------

#[test]
fn test_uv_sha256_covers_every_producible_triple() {
    // The pinned table must key on exactly the triples `_triple()` can emit, so no reachable platform
    // is left without a hash to verify against.
    let produced = producible_triples();
    assert_eq!(produced.len(), 8);
    // Each pinned value (surfaced through the built asset) is a 64-char lowercase-hex SHA256 digest.
    for target in all_targets() {
        let checksum = uv_asset(&target, None).unwrap().checksum;
        assert_eq!(checksum.len(), 64);
        assert!(
            checksum
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }
}

#[test]
fn test_checksum_pass_proceeds_to_extraction() {
    // When the archive's SHA256 equals the pinned hash, the checksum gate opens and control reaches
    // extraction; the installed path holds exactly the verified bytes.
    let uv_bytes = b"known-good-uv-archive-bytes";
    let archive = tar_gz_with_uv("uv", uv_bytes);
    let asset = asset_for("uv-x86_64-unknown-linux-gnu.tar.gz", "uv", &archive);
    let destination = TempDir::new().unwrap();
    let installed = install_verified_uv_archive(&archive, &asset, destination.path()).unwrap();
    assert_eq!(installed, destination.path().join("uv"));
    assert_eq!(fs::read(&installed).unwrap(), uv_bytes);
}

#[test]
fn test_checksum_mismatch_raises_checksum_error_not_generic() {
    // A tampered/corrupt archive (hash != pinned) fails closed with the checksum message — NOT the
    // generic download wrapper — and extraction is never reached; both digests are surfaced.
    let data = b"tampered-bytes-from-a-hostile-mirror";
    let pinned = "0".repeat(64); // a valid-shaped but wrong digest
    let asset = UvAsset {
        version: UV_VERSION.to_owned(),
        filename: "uv-x86_64-unknown-linux-gnu.tar.gz".to_owned(),
        url: "https://example.invalid/archive".to_owned(),
        checksum: pinned.clone(),
        executable_name: "uv".to_owned(),
    };
    let destination = TempDir::new().unwrap();
    let error = install_verified_uv_archive(data, &asset, destination.path()).unwrap_err();
    let message = error.to_string();
    assert!(matches!(error, UvBootstrapError::Checksum { .. }));
    assert!(message.to_lowercase().contains("checksum")); // distinguishes it from the generic failure
    assert!(!message.contains("Failed to download"));
    assert!(message.contains(&pinned)); // expected digest surfaced
    assert!(message.contains(&hex_digest(data))); // actual digest surfaced
    assert!(!destination.path().join("uv").exists()); // a mismatched archive is never extracted
}

// ---- _extract_uv: staged-file fsync (durability across power loss) ----------
