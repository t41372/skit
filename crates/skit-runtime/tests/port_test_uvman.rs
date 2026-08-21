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
//! - Bucket 2 (cross-crate / white-box, `#[ignore]`d): interactive consent (`_ask_consent`) lives in
//!   skit-cli's `TerminalUvConsent` (`crates/skit-cli/src/run/command.rs`); musl detection is the
//!   private, `/lib`-hardcoded `host_uses_musl` with no injectable seam; the fsync spy tests target
//!   internals with no public seam; the blank-`uv_binary` fallback is resolved in skit-cli/skit-store.
//! - Bucket 3 (divergences, `#[ignore]`d): three oracle behaviors the Rust surface does not match —
//!   the checksum error drops the expected/actual digests, a directory-fsync failure is propagated
//!   rather than swallowed, and an unpinned triple is a construction-time `expect` panic rather than
//!   a typed fail-closed error. See the module notes in the port ledger.
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
    // drift can't happen silently: any future producible triple must also be added here.
    let produced = producible_triples();
    let declared: BTreeSet<String> = TRIPLES.iter().map(|t| (*t).to_owned()).collect();
    assert_eq!(declared, produced);
    // Every producible triple resolves to a pinned asset: `uv_asset` fails closed on a missing
    // pin, so an Ok for each target proves the producible -> pinned direction (Python's
    // `== _UV_SHA256`). The reverse (no EXTRA stale pin) can't be checked here — `CHECKSUMS` is
    // private to skit-runtime.
    for target in all_targets() {
        assert!(uv_asset(&target, None).is_ok());
    }
}

#[test]
#[ignore = "network liveness (opt-in): run when bumping UV_VERSION, like the oracle's SKIT_NET_TESTS gate"]
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
#[ignore = "network liveness (opt-in): cross-checks each pinned hash against Astral's live .sha256 sidecar"]
fn test_pinned_sha256_matches_live_sidecar() {
    // A future UV_VERSION bump that forgets to refresh the pinned table must fail loudly here: every
    // pinned hash must equal the official `.sha256` sidecar. Built from the canonical GitHub base
    // (mirror_base = None) so a configured mirror can't skew the check.
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

#[test]
#[ignore = "CROSS-CRATE: quiet=True consent-bypass (uvman.py:251) is skit-cli's job — ensure_managed_uv takes no consent argument; consent is checked before it in crates/skit-cli/src/run/command.rs:733."]
fn test_quiet_skips_consent() {
    // Oracle: quiet=True (programmatic call) bypasses consent entirely; _ask_consent is never called.
}

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
#[ignore = "CROSS-CRATE (white-box): the private in-crate helper host_uses_musl (crates/skit-runtime/src/uv.rs:441-450) is cfg-gated to linux and hardcodes /lib with no injectable seam. Its logic (presence of /lib/ld-musl-*.so.1) matches the oracle, but the public surface takes `musl: bool` explicitly (UvTarget::from_parts), so the filesystem-probe assertion can't be driven from an integration test."]
fn test_is_musl_true_when_ld_musl_present() {
    // Oracle: a fake /lib containing ld-musl-x86_64.so.1 -> _is_musl() is True.
}

#[test]
#[ignore = "CROSS-CRATE (white-box): private host_uses_musl (crates/skit-runtime/src/uv.rs:441-450), hardcoded /lib, no injectable seam."]
fn test_is_musl_false_when_ld_musl_absent() {
    // Oracle: an empty /lib (no ld-musl-*.so.1) -> _is_musl() is False.
}

#[test]
#[ignore = "CROSS-CRATE (white-box): private host_uses_musl (crates/skit-runtime/src/uv.rs:441-450); read_dir on a missing /lib returns false, not an error, matching the oracle."]
fn test_is_musl_false_when_lib_dir_missing() {
    // Oracle: a missing /lib (minimal container) must not raise — just means "not musl".
}

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

// ---- _extract_uv: atomic install (no partial binary survives a mid-install failure) ----------
//
// Rust's atomic install has no `shutil.copy2` seam to monkeypatch. These ports force the failure by
// occupying the final `uv` path with a directory: `fs::rename(staged, target)` onto a directory
// fails (EISDIR), which is the same failure class (the install cannot complete) the oracle simulates
// with a mid-copy OSError. The contract under test is unchanged: no torn binary at the final path,
// no stray staged `.tmp`, and a later attempt succeeds once the obstruction is gone.

#[test]
fn test_extract_uv_failed_copy_leaves_no_partial_binary() {
    let archive = tar_gz_with_uv("uv", b"genuine-uv-bytes");
    let asset = asset_for("uv.tar.gz", "uv", &archive);
    let destination = TempDir::new().unwrap();
    fs::create_dir(destination.path().join("uv")).unwrap(); // block the final rename

    assert!(matches!(
        install_verified_uv_archive(&archive, &asset, destination.path()),
        Err(UvBootstrapError::Io { .. }),
    ));

    // The staged tmp file was cleaned up — nothing poisoned.
    assert!(fs::read_dir(destination.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
    // The final `uv` path holds no torn binary (still the obstruction, never a partial file).
    assert!(destination.path().join("uv").is_dir());
}

#[test]
fn test_extract_uv_self_heals_after_interrupted_install() {
    // After a failed install leaves no binary at dest, a fresh (unobstructed) extraction attempt must
    // succeed cleanly — proving the failure didn't poison the destination for next time.
    let archive = tar_gz_with_uv("uv", b"the-real-uv-binary");
    let asset = asset_for("uv.tar.gz", "uv", &archive);
    let destination = TempDir::new().unwrap();
    let obstruction = destination.path().join("uv");
    fs::create_dir(&obstruction).unwrap();
    assert!(install_verified_uv_archive(&archive, &asset, destination.path()).is_err());

    fs::remove_dir(&obstruction).unwrap(); // clear the obstruction (restore a clean dest)
    let installed = install_verified_uv_archive(&archive, &asset, destination.path()).unwrap();
    assert_eq!(installed, destination.path().join("uv"));
    assert_eq!(fs::read(&installed).unwrap(), b"the-real-uv-binary");
}

// ---- _extract_uv: staged-file fsync (durability across power loss) ----------

#[test]
#[ignore = "CROSS-CRATE (white-box): the oracle spies on os.fsync vs os.replace call order. Rust's install does file.sync_all() before fs::rename in one private closure (crates/skit-runtime/src/uv.rs:335-337) with no seam to observe the ordering from an integration test. The ordering is correct in code."]
fn test_extract_uv_fsyncs_staged_file_before_replace() {
    // Oracle: os.fsync of the staged file must run before os.replace commits the rename.
}

#[test]
#[ignore = "CROSS-CRATE (white-box): the directory-fsync swallow is implemented — install_verified_uv_archive wraps the post-rename sync_directory in `let _ =` (crates/skit-runtime/src/uv.rs), matching uvman.py:210-212 contextlib.suppress — but no public API can force sync_directory to fail, so the asserting body is not drivable from an integration test."]
fn test_extract_uv_dir_fsync_failure_is_swallowed() {
    // Oracle: the post-replace directory fsync is best-effort; a failure there must not fail the
    // install (dest's content durability was already secured by the staged-file fsync).
}

#[test]
#[ignore = "CROSS-CRATE (white-box): a staged-file fsync failure must propagate AND compose with the cleanup-on-failure. Rust does exactly this — file.sync_all()? propagates and the closure's `if result.is_err() { remove_file(staged) }` cleans up (crates/skit-runtime/src/uv.rs:335-343) — but there is no seam to force sync_all to fail from an integration test."]
fn test_extract_uv_staged_fsync_failure_triggers_existing_cleanup() {
    // Oracle: a failure fsync'ing the staged file's data propagates and the staged tmp file is
    // unlinked; dest_dir is left as if the install never started.
}

#[test]
#[ignore = "CROSS-CRATE (white-box + platform): directory fsync must not even be attempted on Windows. Rust cfg-gates it to a no-op — sync_directory is `#[cfg(not(unix))] -> Ok(())` (crates/skit-runtime/src/uv.rs:435-438) — matching the oracle, but there is no fsync spy seam and this can't be observed from a POSIX runner."]
fn test_extract_uv_skips_dir_fsync_on_windows() {
    // Oracle: on win32 only the staged file is fsync'd, never the directory.
}

#[test]
#[ignore = "CROSS-CRATE (white-box): the end-to-end self-heal needs a fake-served archive to reach install. The public path (ensure_managed_uv) fetches from a real URL and verifies against the real pinned checksum for the current triple, so a fake archive can't pass the gate; the injectable fetch (ensure_managed_uv_from_asset) is private. The self-heal contract is covered at install level (test_extract_uv_self_heals_after_interrupted_install) and by in-crate private_tests (crates/skit-runtime/src/uv.rs:531-554)."]
fn test_ensure_uv_downloaded_atomic_install_self_heals() {
    // Oracle: a flaky first install raises but leaves no binary; the next call re-downloads and
    // installs successfully.
}

#[test]
#[ignore = "CROSS-CRATE (white-box): the fail-closed path is implemented — uv_asset returns UvBootstrapError::NoPinnedChecksum naming the triple (uvman.py:229-233 parity) — but every producible UvTarget triple is pinned, so the branch is reachable only by constructing a martian triple in-crate. Covered by private_tests::unpinned_triple_fails_closed_with_a_typed_error (crates/skit-runtime/src/uv.rs)."]
fn test_checksum_fail_closed_when_triple_unpinned() {
    // Oracle: a triple with no pinned hash raises UvDownloadError naming the triple, never extracts.
}
