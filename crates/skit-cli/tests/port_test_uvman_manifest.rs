//! Completeness guard for Python `tests/test_uvman.py` at `main@206f9ef`.
//!
//! Twenty-two contracts have executable Rust public/CLI equivalents. Fourteen depend on Python-only
//! private table mutation or precise monkeypatchable copy/fsync/musl fault seams that Rust does not
//! expose. They stay architecture-closed rather than being represented by weaker success-path tests.

use std::{collections::BTreeSet, fs, path::Path};

use syn::{Attribute, Item};

const TARGETS: &[&str] = &[
    "crates/skit-runtime/tests/port_test_uvman.rs",
    "crates/skit-cli/tests/port_test_uvman_cli.rs",
];

const EXECUTABLE: &[&str] = &[
    "test_pinned_uv_release_exists",
    "test_pinned_sha256_matches_live_sidecar",
    "test_consent_non_interactive_auto_yes",
    "test_consent_interactive_answers",
    "test_consent_eof_is_yes",
    "test_declined_raises_with_guidance",
    "test_triple_unsupported_arch_raises",
    "test_triple_darwin_aarch64",
    "test_triple_windows_x86_64",
    "test_triple_linux_aarch64",
    "test_triple_linux_musl_x86_64",
    "test_triple_linux_musl_aarch64",
    "test_download_url_musl_triple_targz",
    "test_download_url_structure",
    "test_ensure_uv_already_exists",
    "test_extract_uv_no_exe_in_archive_raises",
    "test_ensure_uv_network_error_wrapped",
    "test_download_url_uses_configured_mirror",
    "test_download_url_defaults_to_github_without_mirror",
    "test_download_url_github_when_uv_binary_blank",
    "test_checksum_pass_proceeds_to_extraction",
    "test_checksum_mismatch_raises_checksum_error_not_generic",
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    (
        "test_triples_covers_every_pinned_and_producible_triple",
        "Python compares the exact private _UV_SHA256 key set with every private _triple() output. Rust exposes UvTarget construction and each selected asset checksum, but its CHECKSUMS table is private; parsing source text or merely proving the eight public targets work would not prove the no-extra-private-key half of this contract.",
    ),
    (
        "test_quiet_skips_consent",
        "Python's ensure_uv_downloaded(quiet=True) is a public programmatic bypass. Rust has no quiet bootstrap argument; terminal consent is owned by the private CLI composition root, so inventing an equivalent flag in a test would test behavior the Rust product does not expose.",
    ),
    (
        "test_is_musl_true_when_ld_musl_present",
        "Python monkeypatches the private _MUSL_LD_DIR filesystem root. Rust host_uses_musl is private and reads the real /lib; UvTarget::from_parts can exercise the resulting musl target but cannot inject the detector root itself.",
    ),
    (
        "test_is_musl_false_when_ld_musl_absent",
        "The oracle injects an empty private musl-linker directory. Rust has no public detector-root seam, so a synthetic UvTarget::from_parts(..., musl=false) test would skip the detector this contract is about.",
    ),
    (
        "test_is_musl_false_when_lib_dir_missing",
        "The oracle injects a missing private musl-linker directory and proves the detector does not raise. Rust's fixed /lib detector is private and cannot be redirected without changing production code.",
    ),
    (
        "test_uv_sha256_covers_every_producible_triple",
        "Python inspects every private checksum key and validates every digest's lowercase-hex shape. Rust's CHECKSUMS table is private; uv_asset exposes only the checksum selected for a public target, so it cannot prove that no unreachable extra table rows exist.",
    ),
    (
        "test_extract_uv_failed_copy_leaves_no_partial_binary",
        "Python monkeypatches shutil.copy2 to fail after entering the staged-copy path. Rust's public installer performs direct staged writes and exposes no write-fault injection seam; making the destination unwritable would fail before the same boundary and would be a weaker test.",
    ),
    (
        "test_extract_uv_self_heals_after_interrupted_install",
        "The oracle forces the first private copy operation to fail and then retries the same extraction unpatched. Rust exposes the verified installer but not the staged-write fault point, so this exact fail-then-retry sequence cannot be induced without production changes.",
    ),
    (
        "test_extract_uv_fsyncs_staged_file_before_replace",
        "Python spies on private os.fsync and os.replace call order. Rust's File::sync_all and fs::rename calls are internal implementation steps with no public observer or injected filesystem port.",
    ),
    (
        "test_extract_uv_dir_fsync_failure_is_swallowed",
        "Python injects a failure specifically for the post-replace directory fsync. Rust exposes no directory-sync fault seam, so a normal successful install cannot prove whether that exact failure is swallowed.",
    ),
    (
        "test_extract_uv_staged_fsync_failure_triggers_existing_cleanup",
        "Python injects EIO specifically at staged-file fsync and inspects cleanup. Rust's staged File::sync_all is private and not replaceable from an integration test.",
    ),
    (
        "test_extract_uv_skips_dir_fsync_on_windows",
        "Python monkeypatches sys.platform and records each private _fsync_path call. A Rust integration test can run the installer on Windows, but success alone cannot prove that the directory fsync call was skipped rather than attempted successfully.",
    ),
    (
        "test_ensure_uv_downloaded_atomic_install_self_heals",
        "Python combines an injected first-copy ENOSPC with mocked network bytes and then a successful second call. Rust's public ensure_managed_uv has a real downloader and no injected staged-write failure port, so reproducing only the second successful install would omit the atomic-failure contract.",
    ),
    (
        "test_checksum_fail_closed_when_triple_unpinned",
        "Python temporarily replaces the private checksum table with an empty dict. Rust's uv_asset chooses from a private compile-time CHECKSUMS table and exposes no way to construct a supported target while removing its pin; source parsing or a fabricated UvAsset would bypass the pin-selection branch under test.",
    ),
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn parity_test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                let name = function.sig.ident.to_string();
                (!name.starts_with("rust_additive_")).then_some(name)
            }
            _ => None,
        })
        .collect()
}

fn actual_names(repo: &Path) -> Vec<String> {
    TARGETS
        .iter()
        .flat_map(|target| {
            let source = fs::read_to_string(repo.join(target)).unwrap();
            parity_test_names(&source)
        })
        .collect()
}

#[test]
fn every_executable_uvman_contract_has_exactly_one_rust_oracle() {
    assert_eq!(EXECUTABLE.len(), 22);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 14);
    assert_eq!(EXECUTABLE.len() + ARCHITECTURE_CLOSED.len(), 36);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let actual = actual_names(repo);
    let unique = actual.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        actual.len(),
        unique.len(),
        "a Python uvman contract has more than one exact-name Rust oracle: {actual:?}"
    );
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(unique, expected, "uvman executable mapping drifted");
}

#[test]
fn private_uvman_contracts_are_not_impersonated() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives under <repo>/crates/skit-cli");
    let actual = actual_names(repo).into_iter().collect::<BTreeSet<_>>();

    for (name, reason) in ARCHITECTURE_CLOSED {
        assert!(
            !actual.contains(*name),
            "{name} is architecture-closed ({reason}); do not add a weaker same-named stand-in"
        );
        assert!(!reason.trim().is_empty());
    }
}
