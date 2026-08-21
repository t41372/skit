//! Final ownership manifest for Python 0.4 `tests/test_uvman.py`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const ORACLE_NAMES: [&str; 36] = [
    "test_triples_covers_every_pinned_and_producible_triple",
    "test_pinned_uv_release_exists",
    "test_pinned_sha256_matches_live_sidecar",
    "test_consent_non_interactive_auto_yes",
    "test_consent_interactive_answers",
    "test_consent_eof_is_yes",
    "test_declined_raises_with_guidance",
    "test_quiet_skips_consent",
    "test_triple_unsupported_arch_raises",
    "test_triple_darwin_aarch64",
    "test_triple_windows_x86_64",
    "test_triple_linux_aarch64",
    "test_is_musl_true_when_ld_musl_present",
    "test_is_musl_false_when_ld_musl_absent",
    "test_is_musl_false_when_lib_dir_missing",
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
    "test_uv_sha256_covers_every_producible_triple",
    "test_checksum_pass_proceeds_to_extraction",
    "test_checksum_mismatch_raises_checksum_error_not_generic",
    "test_extract_uv_failed_copy_leaves_no_partial_binary",
    "test_extract_uv_self_heals_after_interrupted_install",
    "test_extract_uv_fsyncs_staged_file_before_replace",
    "test_extract_uv_dir_fsync_failure_is_swallowed",
    "test_extract_uv_staged_fsync_failure_triggers_existing_cleanup",
    "test_extract_uv_skips_dir_fsync_on_windows",
    "test_ensure_uv_downloaded_atomic_install_self_heals",
    "test_checksum_fail_closed_when_triple_unpinned",
];

const EXECUTABLE: [&str; 31] = [
    "test_triples_covers_every_pinned_and_producible_triple",
    "test_consent_non_interactive_auto_yes",
    "test_consent_interactive_answers",
    "test_consent_eof_is_yes",
    "test_declined_raises_with_guidance",
    "test_triple_unsupported_arch_raises",
    "test_triple_darwin_aarch64",
    "test_triple_windows_x86_64",
    "test_triple_linux_aarch64",
    "test_is_musl_true_when_ld_musl_present",
    "test_is_musl_false_when_ld_musl_absent",
    "test_is_musl_false_when_lib_dir_missing",
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
    "test_uv_sha256_covers_every_producible_triple",
    "test_checksum_pass_proceeds_to_extraction",
    "test_checksum_mismatch_raises_checksum_error_not_generic",
    "test_extract_uv_failed_copy_leaves_no_partial_binary",
    "test_extract_uv_self_heals_after_interrupted_install",
    "test_extract_uv_fsyncs_staged_file_before_replace",
    "test_extract_uv_dir_fsync_failure_is_swallowed",
    "test_extract_uv_staged_fsync_failure_triggers_existing_cleanup",
    "test_ensure_uv_downloaded_atomic_install_self_heals",
];

struct Gate {
    name: &'static str,
    reason: &'static str,
    owner_file: &'static str,
    run_condition: &'static str,
    target: &'static str,
    requires_windows_cfg: bool,
}

const GATES: [Gate; 3] = [
    Gate {
        name: "test_pinned_uv_release_exists",
        reason: "This opt-in liveness check contacts every pinned Astral release asset.",
        owner_file: "crates/skit-runtime/tests/port_test_uvman.rs",
        run_condition: "SKIT_NET_TESTS=1",
        target: "Astral uv release assets",
        requires_windows_cfg: false,
    },
    Gate {
        name: "test_pinned_sha256_matches_live_sidecar",
        reason: "This opt-in liveness check compares every pin with Astral's live sidecar.",
        owner_file: "crates/skit-runtime/tests/port_test_uvman.rs",
        run_condition: "SKIT_NET_TESTS=1",
        target: "Astral uv SHA-256 sidecars",
        requires_windows_cfg: false,
    },
    Gate {
        name: "test_extract_uv_skips_dir_fsync_on_windows",
        reason: "Only a native Windows build can execute the no-directory-open implementation.",
        owner_file: "crates/skit-runtime/src/uv.rs",
        run_condition: "--ignored",
        target: "x86_64-pc-windows-msvc",
        requires_windows_cfg: true,
    },
];

struct Closure {
    name: &'static str,
    reason: &'static str,
    strong_owner: &'static str,
}

const CLOSURES: [Closure; 2] = [
    Closure {
        name: "test_quiet_skips_consent",
        reason: "Rust separates the always-quiet runtime installer from the CLI consent policy instead of passing a quiet flag through one Python-style function.",
        strong_owner: "crates/skit-cli/src/run/command.rs::bootstrap_tests::a_completed_bootstrap_pins_the_installed_uv_in_settings_and_metadata + crates/skit-runtime/src/uv.rs::ensure_managed_uv",
    },
    Closure {
        name: "test_checksum_fail_closed_when_triple_unpinned",
        reason: "Every producible target is pinned, so the unsupported-table branch needs a private constructed target rather than a weaker public surrogate.",
        strong_owner: "crates/skit-runtime/src/uv.rs::private_tests::unpinned_triple_fails_closed_with_a_typed_error",
    },
];

const KNOWN_OWNER_FILES: [&str; 5] = [
    "crates/skit-runtime/tests/port_test_uvman.rs",
    "crates/skit-runtime/src/uv.rs",
    "crates/skit-cli/tests/edge_workflows.rs",
    "crates/skit-cli/src/run/command.rs",
    "crates/skit-cli/tests/terminal_pty.rs",
];

#[derive(Debug)]
struct Occurrence {
    name: String,
    file: String,
    ignore_reason: Option<String>,
    windows_cfg: bool,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(directory: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}

fn test_name(line: &str) -> Option<&str> {
    let declaration = line.trim_start().strip_prefix("fn ")?.split('(').next()?;
    declaration.starts_with("test_").then_some(declaration)
}

fn ignore_reason(line: &str) -> Option<String> {
    let line = line.trim_start();
    if !line.starts_with("#[ignore") {
        return None;
    }
    let start = line.find('"')? + 1;
    let end = line.rfind('"')?;
    (end >= start).then(|| line[start..end].to_owned())
}

fn occurrences() -> Vec<Occurrence> {
    let root = repository_root();
    let expected: BTreeSet<&str> = ORACLE_NAMES.into_iter().collect();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut found = Vec::new();

    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        let file = path.strip_prefix(&root).unwrap().display().to_string();
        let mut pending_ignore = None;
        let mut pending_windows_cfg = false;
        for line in source.lines() {
            if let Some(reason) = ignore_reason(line) {
                pending_ignore = Some(reason);
            }
            if line.trim_start() == "#[cfg(windows)]" {
                pending_windows_cfg = true;
            }
            if let Some(name) = test_name(line) {
                if expected.contains(name) {
                    found.push(Occurrence {
                        name: name.to_owned(),
                        file: file.clone(),
                        ignore_reason: pending_ignore.take(),
                        windows_cfg: pending_windows_cfg,
                    });
                }
                pending_ignore = None;
                pending_windows_cfg = false;
            }
        }
    }
    found
}

fn unique<'a>(
    label: &str,
    names: impl IntoIterator<Item = &'a str>,
    expected_len: usize,
) -> BTreeSet<&'a str> {
    let names: Vec<&str> = names.into_iter().collect();
    assert_eq!(names.len(), expected_len, "{label} raw count changed");
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(
        unique.len(),
        expected_len,
        "{label} contains a duplicate: {names:?}"
    );
    assert!(
        unique.iter().all(|name| !name.is_empty()),
        "{label} contains an empty name"
    );
    unique
}

#[test]
fn uvman_oracle_has_31_active_three_gates_two_closures_and_exactly_36_names() {
    let oracle = unique("oracle", ORACLE_NAMES, 36);
    let executable = unique("executable", EXECUTABLE, 31);
    let gates = unique("gates", GATES.iter().map(|gate| gate.name), 3);
    let closures = unique("closures", CLOSURES.iter().map(|closure| closure.name), 2);
    let categorized = unique(
        "categorized union",
        EXECUTABLE
            .into_iter()
            .chain(GATES.iter().map(|gate| gate.name))
            .chain(CLOSURES.iter().map(|closure| closure.name)),
        36,
    );

    assert_eq!(categorized, oracle);
    assert!(executable.is_disjoint(&gates));
    assert!(executable.is_disjoint(&closures));
    assert!(gates.is_disjoint(&closures));
    for gate in &GATES {
        assert!(!gate.reason.is_empty());
        assert!(!gate.owner_file.is_empty());
        assert!(!gate.run_condition.is_empty());
        assert!(!gate.target.is_empty());
    }
    for closure in &CLOSURES {
        assert!(!closure.reason.is_empty());
        assert!(!closure.strong_owner.is_empty());
    }
}

#[test]
fn uvman_source_has_34_unique_function_occurrences_with_honest_activity_and_gates() {
    let occurrences = occurrences();
    let mut counts = BTreeMap::<&str, Vec<&str>>::new();
    for occurrence in &occurrences {
        counts
            .entry(&occurrence.name)
            .or_default()
            .push(&occurrence.file);
    }
    assert_eq!(
        occurrences.len(),
        34,
        "31 executable and 3 gate functions must occur once each: {counts:#?}"
    );
    let actual: BTreeSet<&str> = occurrences
        .iter()
        .map(|occurrence| occurrence.name.as_str())
        .collect();
    assert_eq!(
        actual.len(),
        34,
        "an exact owner is duplicated: {counts:#?}"
    );
    let expected: BTreeSet<&str> = EXECUTABLE
        .into_iter()
        .chain(GATES.iter().map(|gate| gate.name))
        .collect();
    assert_eq!(
        actual, expected,
        "an executable or gate owner is missing or extra"
    );

    let known: BTreeSet<&str> = KNOWN_OWNER_FILES.into_iter().collect();
    let actual_files: BTreeSet<&str> = occurrences
        .iter()
        .map(|occurrence| occurrence.file.as_str())
        .collect();
    assert_eq!(
        actual_files, known,
        "uvman ownership moved outside the audited files"
    );

    for name in EXECUTABLE {
        let occurrence = occurrences
            .iter()
            .find(|occurrence| occurrence.name == name)
            .unwrap();
        assert!(
            occurrence.ignore_reason.is_none(),
            "executable owner {name} is ignored in {}",
            occurrence.file
        );
    }
    for gate in &GATES {
        let occurrence = occurrences
            .iter()
            .find(|occurrence| occurrence.name == gate.name)
            .unwrap();
        assert_eq!(occurrence.file, gate.owner_file);
        let reason = occurrence
            .ignore_reason
            .as_deref()
            .expect("every gate must keep a nonempty ignore reason");
        assert!(!reason.is_empty());
        assert!(reason.contains(gate.run_condition), "{reason}");
        assert!(reason.contains(gate.target), "{reason}");
        assert_eq!(occurrence.windows_cfg, gate.requires_windows_cfg);
    }
    for closure in &CLOSURES {
        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.name != closure.name),
            "closure {} must not retain an empty ignored function",
            closure.name
        );
    }
}
