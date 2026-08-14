//! Mechanical port of the Python oracle module `tests/test_prompt_utf8.py`
//! (`origin/main@206f9ef`): "Prompt payloads have one strict UTF-8 boundary across every
//! product surface." Each `#[test]` keeps its Python `def test_*` name and its WHY comment,
//! so it traces back to its origin. Parametrized oracle defs split into one Rust test per
//! parameter (suffix `_copy` / `_reference`).
//!
//! WHY `skit-cli-rs` (crate_hint `skit-i18n` REJECTED): the oracle's boundary lives in
//! `skit.langs.prompt.text.decode/read`, which raises `PromptEncodingError(path, offset)`.
//! In the Rust rewrite that primitive is NOT a shared function — `skit-i18n` owns only the
//! catalog string ("Prompt {} isn't valid UTF-8 (invalid byte at offset {})."), and the
//! decode is re-inlined per surface (`cli.rs` show/add, `run/command.rs` launch). The one
//! self-contained home that reaches every surface the oracle spans (store add, launcher,
//! flows, healthcheck, cli, tui) is the composition-root crate, driven black-box through the
//! real `skit` binary — the same disposition and `Sandbox` shape as the sibling
//! `port_test_prompt_cli.rs`.
//!
//! OBSERVABLE MAPPING (black-box binary, all three `SKIT_*_DIR` pinned to a per-test `TempDir`
//! and `SKIT_LANG=en` on every invocation):
//! - `store.add_prompt(path, mode)` -> `skit add <path> --prompt [--ref] --no-input`.
//! - `store.list_entries()` -> the registered entry dirs under `<data>/scripts`.
//! - `launcher.build_command(entry, runner)` (the rendered argv) -> a fake runner on `PATH`
//!   whose binary copies `$1` byte-for-byte into `$SKIT_CAP` (`printf %s`), read back
//!   out of band from skit's own stdout (which omits the body).
//! - `launcher.preflight(entry, runner)` -> `skit run <name> --runner … --no-input`.
//! - `healthcheck.collect([entry]).launch_blocked` -> `skit doctor --json` `launch_blocked`.
//! - `healthcheck.entry_drifted(entry)` -> `skit doctor --json` `drift`.
//! - `cli.app` via `CliRunner` -> the binary; `runner.output` merges stdout+stderr, so
//!   substring checks run against the CONCATENATION (`combined`); `--json` purity keeps STDOUT
//!   as exactly one JSON document.
//! - `prompt.text.read` byte fidelity -> the stored body file bytes under the entry dir.
//!
//! Buckets:
//! - REAL asserting `#[test]` (API EXISTS, behavior reachable black-box and matching the oracle).
//! - FAILING CONTRACT (divergence): the full asserting body is kept intact and `#[ignore]`d with
//!   the OBSERVED-vs-oracle evidence; deleting the `#[ignore]` after the impl is fixed turns it
//!   green. Never softened to match Rust output. The Rust rewrite does NOT enforce the oracle's
//!   single strict boundary uniformly on the remaining launch, health, params, and edit surfaces.
//! - UNMAPPED (cross-crate): a Python-private store seam (`store._add_entry`, `store.add_script`,
//!   the `Path.open`/mid-add TOCTOU monkeypatches) or a Textual screen — not reachable from a
//!   non-tty binary without a forbidden dependency edit. Compiling `#[ignore]` stub naming the
//!   owning tier.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;

// The oracle's `_invalid_prompt` fixture body: a CRLF line then a lone `\xc3` lead byte.
const INVALID_PROMPT: &[u8] = b"Review {{target}}\r\ninvalid:\xc3(\n";
// The oracle's invalid stdin body: `\xff` at index 7.
const INVALID_STDIN: &[u8] = b"Review \xff now\n";

/// The oracle's `data.index(b"\xc3")` / `exc.start` — the first invalid byte's index.
fn offset_of(bytes: &[u8], invalid: u8) -> usize {
    bytes
        .iter()
        .position(|byte| *byte == invalid)
        .expect("the fixture carries the invalid byte")
}

/// The oracle's `" ".join(output.split())` — collapse whitespace runs so a rich line wrap cannot
/// straddle an "offset N" phrase.
fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    /// The oracle's `runner.invoke(cli.app, …)`: the real `skit` binary, all three roots pinned
    /// under the sandbox, locale fixed to English.
    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// (exit code, stdout+stderr concatenated) — the merged-stream view CliRunner.output gives.
    fn out(&self, args: &[&str]) -> (i32, String) {
        let output = self.command().args(args).output().unwrap();
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        (output.status.code().unwrap_or(-1), combined)
    }

    /// Assert success and return the merged output.
    fn ok(&self, args: &[&str]) -> String {
        let (code, combined) = self.out(args);
        assert_eq!(code, 0, "args={args:?}\n{combined}");
        combined
    }

    /// Whole-STDOUT-as-one-JSON — the `--json` purity contract.
    fn json(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("args={args:?}: stdout is not one JSON doc: {error}"))
    }

    /// Write a byte-exact input file under the data dir; return its absolute path as a string.
    fn write_file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.data.path().join(name);
        fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn scripts_dir(&self) -> PathBuf {
        self.data.path().join("scripts")
    }

    fn entry_dir(&self, slug: &str) -> PathBuf {
        self.scripts_dir().join(slug)
    }

    /// The oracle's `store.list_entries()` reduced to registered slugs — one entry per subdir of
    /// `<data>/scripts` that carries a `meta.toml`.
    fn entry_slugs(&self) -> Vec<String> {
        let mut slugs = Vec::new();
        if let Ok(items) = fs::read_dir(self.scripts_dir()) {
            for item in items.flatten() {
                if item.path().join("meta.toml").is_file() {
                    slugs.push(item.file_name().to_string_lossy().into_owned());
                }
            }
        }
        slugs.sort();
        slugs
    }

    /// The stored body file bytes (the single non-`meta.toml` file in the entry dir).
    fn body_bytes(&self, slug: &str) -> Option<Vec<u8>> {
        let dir = self.entry_dir(slug);
        for entry in fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            if entry.file_name() != std::ffi::OsStr::new("meta.toml") && entry.path().is_file() {
                return fs::read(entry.path()).ok();
            }
        }
        None
    }

    fn body_path(&self, slug: &str) -> PathBuf {
        let dir = self.entry_dir(slug);
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() != std::ffi::OsStr::new("meta.toml") && entry.path().is_file() {
                return entry.path();
            }
        }
        panic!("no stored body under {}", dir.display());
    }

    /// The oracle's `<data>/drafts` — the only draft directory skit owns.
    fn drafts_dir(&self) -> PathBuf {
        self.data.path().join("drafts")
    }
}

/// A directory holding a fake agent binary that copies its prompt argument (`$1`) byte-for-byte
/// into `$SKIT_CAP`. `printf %s` appends no newline, so the capture equals the rendered prompt
/// exactly — the black-box witness for `launcher.build_command`'s argv.
fn capturing_agent() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let agent = dir.path().join("agent");
    fs::write(
        &agent,
        "#!/bin/sh\nif [ -n \"$SKIT_CAP\" ]; then printf '%s' \"$1\" > \"$SKIT_CAP\"; fi\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).unwrap();
    let cap = dir.path().join("cap.bin");
    (dir, cap)
}

/// A `$EDITOR` script that overwrites the file it is given with `bytes` — the black-box stand-in
/// for the oracle's `open_in_editor` monkeypatch.
fn replacing_editor(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let editor = dir.join(name);
    let octal = bytes
        .iter()
        .map(|byte| format!("\\{byte:03o}"))
        .collect::<String>();
    fs::write(&editor, format!("#!/bin/sh\nprintf '{octal}' > \"$1\"\n")).unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    editor
}

// ==========================================================================
// store.add_prompt — the strict boundary before any entry write
// ==========================================================================

#[test]
fn test_store_rejects_invalid_prompt_before_any_entry_write_copy() {
    let sandbox = Sandbox::new();
    let source = sandbox.write_file("bad.prompt.md", INVALID_PROMPT);
    let offset = offset_of(INVALID_PROMPT, 0xc3);
    let (code, combined) = sandbox.out(&["add", &source, "--prompt", "--no-input"]);
    assert_ne!(code, 0, "{combined}");
    assert!(combined.contains(&source), "{combined}");
    assert!(
        flatten(&combined).contains(&format!("offset {offset}")),
        "{combined}"
    );
    assert!(!sandbox.scripts_dir().exists(), "{combined}");
    assert!(
        sandbox.entry_slugs().is_empty(),
        "{:?}",
        sandbox.entry_slugs()
    );
}

#[test]
fn test_store_rejects_invalid_prompt_before_any_entry_write_reference() {
    let sandbox = Sandbox::new();
    let source = sandbox.write_file("bad.prompt.md", INVALID_PROMPT);
    let offset = offset_of(INVALID_PROMPT, 0xc3);
    let (code, combined) = sandbox.out(&["add", &source, "--prompt", "--ref", "--no-input"]);
    assert_ne!(code, 0, "{combined}");
    assert!(combined.contains(&source), "{combined}");
    assert!(
        flatten(&combined).contains(&format!("offset {offset}")),
        "{combined}"
    );
    assert!(!sandbox.scripts_dir().exists(), "{combined}");
    assert!(
        sandbox.entry_slugs().is_empty(),
        "{:?}",
        sandbox.entry_slugs()
    );
}

#[test]
fn test_valid_utf8_crlf_cjk_and_emoji_stays_byte_exact_in_store_and_argv() {
    // A CRLF body of CJK plus a ZWJ emoji cluster must survive the store copy AND the rendered
    // argv byte-for-byte — the oracle's `entry.script_path.read_bytes() == raw` and
    // `build_command == ["/bin/agent", body]`. The `{{目標}}` placeholder stays literal (no
    // value bound), so the delivered prompt equals the whole original body.
    const BODY: &[u8] = &[
        0xe5, 0xaf, 0xa9, 0xe6, 0x9f, 0xbb, 0x20, 0x7b, 0x7b, 0xe7, 0x9b, 0xae, 0xe6, 0xa8, 0x99,
        0x7d, 0x7d, 0x20, 0xf0, 0x9f, 0x91, 0xa9, 0xf0, 0x9f, 0x8f, 0xbd, 0xe2, 0x80, 0x8d, 0xf0,
        0x9f, 0x92, 0xbb, 0x0d, 0x0a, 0xe7, 0xac, 0xac, 0xe4, 0xba, 0x8c, 0xe8, 0xa1, 0x8c, 0x0d,
        0x0a,
    ];
    let sandbox = Sandbox::new();
    let source = sandbox.write_file("exact.prompt.md", BODY);
    // The oracle's `config.PromptRunner("agent", ("agent", "{{prompt}}"))`.
    sandbox.ok(&["runner", "add", "agent", "--", "agent", "{{prompt}}"]);
    sandbox.ok(&[
        "add",
        &source,
        "-n",
        "exact",
        "--prompt",
        "--no-interpolate",
        "--no-input",
        "--runner",
        "agent",
    ]);

    // Stored copy is byte-exact.
    assert_eq!(sandbox.body_bytes("exact").as_deref(), Some(BODY));

    // Rendered argv is byte-exact: run through the capturing agent, read the capture out of band.
    let (agent_dir, cap) = capturing_agent();
    let existing_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{existing_path}", agent_dir.path().display());
    let output = sandbox
        .command()
        .env("PATH", path)
        .env("SKIT_CAP", &cap)
        .args(["run", "exact", "--no-input"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&cap).unwrap(), BODY);
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the oracle monkeypatches `store._add_entry` to swap the source mid-add, proving the COPY stores the analyzed/hashed snapshot rather than the live replacement. That intra-add TOCTOU seam is only observable at the skit-store atomic-mutation tier (crates/skit-store/src/mutations/atomic.rs); a black-box binary cannot inject a write between skit's own analyze and copy steps."]
fn test_copy_add_stores_the_same_snapshot_it_analyzed_and_hashed() {}

#[test]
#[ignore = "FAILING CONTRACT (divergence): reference add records one snapshot (description 'Original', params ['first'], source_hash of the ORIGINAL) — Rust matches all three (meta.toml/show --json). The divergence is the preflight of the now-invalid LIVE body: oracle `launcher.preflight` raises 'offset 19'; Rust `run` refuses (exit 125) with the offset-less 'is not valid UTF-8'. Sequential source swap after add is behaviorally equivalent to the oracle's mid-add race for this observable."]
fn test_reference_add_records_one_snapshot_then_preflight_reads_the_live_body() {
    let sandbox = Sandbox::new();
    let original = b"# Original\nHello {{first}}\n";
    let source = sandbox.write_file("live.prompt.md", original);
    sandbox.ok(&["runner", "add", "agent", "--", "agent", "{{prompt}}"]);
    sandbox.ok(&[
        "add",
        &source,
        "-n",
        "live",
        "--ref",
        "--no-input",
        "--runner",
        "agent",
    ]);

    // The single recorded snapshot is the original body (read before the live swap).
    let show = sandbox.json(&["show", "live", "--json"]);
    assert_eq!(show["description"], "Original");
    let fields: Vec<&str> = show["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["first"]);
    let digest = sha256_hex(original);
    assert_eq!(show["source_hash"], format!("sha256:{digest}"));

    // Now the live body goes invalid; preflight reads it and refuses at the first bad byte.
    let replacement = b"invalid live body: \xff\n";
    fs::write(&source, replacement).unwrap();
    let offset = offset_of(replacement, 0xff);
    let (code, combined) = sandbox.out(&["run", "live", "--runner", "agent", "--no-input"]);
    assert_eq!(code, 125, "{combined}");
    assert!(
        flatten(&combined).contains(&format!("offset {offset}")),
        "{combined}"
    );
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the oracle drives Python-private store seams — a monkeypatched `Path.open('rb')` raising PermissionError (a store-fault the binary cannot inject) and two `store._add_entry` ValueErrors ('mutually exclusive' payload+payload_bytes, 'payload_mode requires payload_bytes'). In Rust those guards are UNREPRESENTABLE: skit-application's `EntryPayload { bytes, permissions }` already bundles bytes with their mode, so a caller cannot pass a path AND bytes, nor a mode without bytes. Owning tier: skit-application/skit-store mutations."]
fn test_prompt_read_error_and_ambiguous_payload_leave_no_store_writes() {}

#[test]
fn test_prompt_copy_preserves_private_source_permissions() {
    // A copy add must carry the source's 0o600 mode onto the stored body (POSIX permission bits).
    let sandbox = Sandbox::new();
    let source = sandbox.write_file("private.prompt.md", b"Private {{topic}}\n");
    fs::set_permissions(Path::new(&source), fs::Permissions::from_mode(0o600)).unwrap();
    sandbox.ok(&["add", &source, "-n", "priv", "--prompt", "--no-input"]);

    let stored = sandbox.body_path("priv");
    assert_eq!(
        fs::metadata(&stored).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(&stored).unwrap(), fs::read(&source).unwrap());
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the oracle asserts the GENERIC `store.add_script(path, kind='prompt')` raises StoreUsageError naming 'add_prompt', forcing prompt onboarding through the dedicated seam. The Rust rewrite has no generic-vs-prompt split at the store tier — a single composition-root `add` lane onboards prompts directly — so there is no bypassable generic API to guard. Owning tier: skit-store."]
fn test_store_generic_script_api_refuses_prompt_onboarding_bypass() {}

// ==========================================================================
// stdin — the decode boundary before a draft is allocated
// ==========================================================================

#[test]
fn test_invalid_utf8_prompt_stdin_fails_before_allocating_a_draft() {
    let sandbox = Sandbox::new();
    let offset = offset_of(INVALID_STDIN, 0xff);
    let output = sandbox
        .command()
        .args(["add", "-", "--kind", "prompt", "--name", "bad-pipe"])
        .write_stdin(INVALID_STDIN)
        .output()
        .unwrap();
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.code(), Some(1), "{combined}");
    assert!(combined.contains("<stdin>"), "{combined}");
    assert!(
        flatten(&combined).contains(&format!("offset {offset}")),
        "{combined}"
    );
    assert!(!combined.contains("Traceback"), "{combined}");
    assert!(
        sandbox.entry_slugs().is_empty(),
        "{:?}",
        sandbox.entry_slugs()
    );
    let drafts = sandbox.drafts_dir();
    assert!(
        !drafts.exists() || fs::read_dir(&drafts).unwrap().next().is_none(),
        "{}",
        drafts.display()
    );
}

#[test]
fn test_invalid_utf8_prompt_stdin_cli_boundary_maps_decode_error_to_clean_exit() {
    let sandbox = Sandbox::new();
    let offset = offset_of(INVALID_STDIN, 0xff);
    let output = sandbox
        .command()
        .args(["add", "-", "--kind", "prompt", "--name", "bad-in-process"])
        .write_stdin(INVALID_STDIN)
        .output()
        .unwrap();
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.code(), Some(1), "{combined}");
    assert!(combined.contains("<stdin>"), "{combined}");
    assert!(
        flatten(&combined).contains(&format!("offset {offset}")),
        "{combined}"
    );
    assert!(!combined.contains("Traceback"), "{combined}");
    assert!(
        sandbox.entry_slugs().is_empty(),
        "{:?}",
        sandbox.entry_slugs()
    );
    assert!(
        !sandbox.drafts_dir().exists(),
        "{}",
        sandbox.drafts_dir().display()
    );
}

#[test]
fn test_add_entry_raw_byte_payload_without_explicit_mode_remains_supported() {
    // The generic raw-snapshot seam still persists arbitrary body bytes and one registry row.
    // Black-box twin of `store._add_entry(payload_bytes=raw)`: the stdin lane feeds raw bytes
    // through the same default-mode payload path. CRLF is preserved and the entry resolves.
    let sandbox = Sandbox::new();
    let raw = b"Review {{target}}\r\n";
    sandbox
        .command()
        .args([
            "add",
            "-",
            "--kind",
            "prompt",
            "-n",
            "raw-snapshot",
            "--no-input",
        ])
        .write_stdin(raw.as_slice())
        .assert()
        .success();
    assert_eq!(
        sandbox.body_bytes("raw-snapshot").as_deref(),
        Some(raw.as_slice())
    );
    // Resolvable back to the same stored entry.
    assert_eq!(
        sandbox.json(&["show", "raw-snapshot", "--json"])["slug"],
        "raw-snapshot"
    );
}

// ==========================================================================
// launch + health — a changed body is blocked and reported the same way
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): a stored prompt gone invalid must be launch-blocked AND health-reported with the SAME 'offset 7'. Rust drift stays false (matches `entry_drifted is False`), but the launch refusal (`run`, exit 125) prints the offset-less 'is not valid UTF-8', and `doctor --json` `launch_blocked` is EMPTY (oracle: contains 'offset 7'). `flows.plan_for_entry` (source 'none', empty text) maps to the skit-ui form-plan tier and has no black-box observable here."]
fn test_changed_prompt_is_launch_blocked_and_health_reports_the_same_error() {
    let sandbox = Sandbox::new();
    let source = sandbox.write_file("changed.prompt.md", b"Review {{target}}\n");
    sandbox.ok(&["runner", "add", "agent", "--", "agent", "{{prompt}}"]);
    sandbox.ok(&[
        "add",
        &source,
        "-n",
        "changed",
        "--prompt",
        "--no-input",
        "--runner",
        "agent",
    ]);
    let bad = b"Review \xff now\n";
    fs::write(sandbox.body_path("changed"), bad).unwrap();
    let offset = offset_of(bad, 0xff);

    let (code, combined) = sandbox.out(&["run", "changed", "--runner", "agent", "--no-input"]);
    assert_eq!(code, 125, "{combined}");
    assert!(
        flatten(&combined).contains(&format!("offset {offset}")),
        "{combined}"
    );

    let doctor = sandbox.json(&["doctor", "--json"]);
    let blocked = doctor["launch_blocked"]["changed"]
        .as_str()
        .unwrap_or_default();
    assert!(
        blocked.contains(&format!("offset {offset}")),
        "launch_blocked={blocked:?}"
    );
    // Health does not call this drift — an unreadable body is not a schema drift.
    let drift = doctor["drift"].as_array().cloned().unwrap_or_default();
    assert!(
        !drift.iter().any(|item| item == "changed"),
        "drift={drift:?}"
    );
}

// ==========================================================================
// cli edit — refuse invalid bytes, keep them for a corrective re-edit
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle `skit edit` on an editor that writes invalid bytes REFUSES (exit 1, 'offset 7', no 'Saved'), keeps the authored bytes for a corrective edit, and a repairing re-edit prints 'Saved'. Rust `edit` does NOT validate prompt UTF-8: it ACCEPTS the invalid bytes (exit 0, 'Edited: clip (clip)'), and the repair path prints 'Edited', never 'Saved'. Copy-mode source stays untouched (matches)."]
fn test_cli_edit_refuses_invalid_prompt_bytes_and_the_next_edit_can_repair_them_copy() {
    let sandbox = Sandbox::new();
    let source = sandbox.write_file("cli-copy.prompt.md", b"Review {{target}}\n");
    sandbox.ok(&["add", &source, "-n", "clip", "--prompt", "--no-input"]);
    let target = sandbox.body_path("clip");
    let scratch = TempDir::new().unwrap();
    let invalid = b"edited:\xff\n";
    let offset = offset_of(invalid, 0xff);
    let editor = replacing_editor(scratch.path(), "bad.sh", invalid);

    let refused = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["edit", "clip"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert_eq!(refused.status.code(), Some(1), "{combined}");
    assert!(
        flatten(&combined).contains(&format!("offset {offset}")),
        "{combined}"
    );
    assert!(!combined.contains("Saved"), "{combined}");
    assert_eq!(fs::read(&target).unwrap(), invalid); // authored bytes are kept
    assert_eq!(fs::read_to_string(&source).unwrap(), "Review {{target}}\n"); // copy source untouched

    let repaired = b"Repaired {{target}}\n";
    let good = replacing_editor(scratch.path(), "good.sh", repaired);
    let accepted = sandbox
        .command()
        .env("EDITOR", &good)
        .args(["edit", "clip"])
        .output()
        .unwrap();
    let accepted_out = format!(
        "{}{}",
        String::from_utf8_lossy(&accepted.stdout),
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert_eq!(accepted.status.code(), Some(0), "{accepted_out}");
    assert!(accepted_out.contains("Saved"), "{accepted_out}");
    assert_eq!(fs::read(&target).unwrap(), repaired);
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the reference-mode twin. Oracle `skit edit` refuses invalid bytes (exit 1, 'offset 7', no 'Saved') and a repair prints 'Saved'. Rust `edit` accepts the invalid bytes (exit 0, 'Edited'), never validating prompt UTF-8, and never prints 'Saved'."]
fn test_cli_edit_refuses_invalid_prompt_bytes_and_the_next_edit_can_repair_them_reference() {
    let sandbox = Sandbox::new();
    let source = sandbox.write_file("cli-reference.prompt.md", b"Review {{target}}\n");
    sandbox.ok(&[
        "add",
        &source,
        "-n",
        "clip",
        "--prompt",
        "--ref",
        "--no-input",
    ]);
    // Reference mode edits the referenced source in place.
    let target = PathBuf::from(&source);
    let scratch = TempDir::new().unwrap();
    let invalid = b"edited:\xff\n";
    let offset = offset_of(invalid, 0xff);
    let editor = replacing_editor(scratch.path(), "bad.sh", invalid);

    let refused = sandbox
        .command()
        .env("EDITOR", &editor)
        .args(["edit", "clip"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert_eq!(refused.status.code(), Some(1), "{combined}");
    assert!(
        flatten(&combined).contains(&format!("offset {offset}")),
        "{combined}"
    );
    assert!(!combined.contains("Saved"), "{combined}");
    assert_eq!(fs::read(&target).unwrap(), invalid); // authored bytes are kept

    let repaired = b"Repaired {{target}}\n";
    let good = replacing_editor(scratch.path(), "good.sh", repaired);
    let accepted = sandbox
        .command()
        .env("EDITOR", &good)
        .args(["edit", "clip"])
        .output()
        .unwrap();
    let accepted_out = format!(
        "{}{}",
        String::from_utf8_lossy(&accepted.stdout),
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert_eq!(accepted.status.code(), Some(0), "{accepted_out}");
    assert!(accepted_out.contains("Saved"), "{accepted_out}");
    assert_eq!(fs::read(&target).unwrap(), repaired);
}

// ==========================================================================
// tui edit / review / settings — the Textual screens
// ==========================================================================

#[test]
#[ignore = "UNMAPPED (cross-crate): oracle drives `tui.MenuApp.action_edit` with a monkeypatched editor writing invalid bytes and reads the '#status' Static ('Error: … offset 7'), then a repair ('Edited …'). The Ratatui MenuApp/library-edit reducer lives in skit-tui (its own TestBackend port), not reachable from a non-tty binary."]
fn test_library_edit_refuses_invalid_prompt_bytes_and_recovers_on_reedit_copy() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the reference-mode twin of the library-edit invalid-bytes flow via `tui.MenuApp`. The Ratatui reducer + '#status' surface belong to skit-tui's own TestBackend port; a non-tty binary cannot drive them."]
fn test_library_edit_refuses_invalid_prompt_bytes_and_recovers_on_reedit_reference() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): oracle pushes `tui_add.PromptReviewScreen(bad_path)` and asserts the '#pv-text-error' Static shows 'offset N' with no U+FFFD, that accept is refused while the body is invalid, and cancel writes nothing. The prompt-review screen is a skit-tui Ratatui screen with no black-box binary seam."]
fn test_tui_review_refuses_invalid_initial_body_without_replacement() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): oracle drives `PromptReviewScreen.action_edit_source` (new invalid bytes -> '#pv-text-error' 'offset 8', schema kept from the valid text) and `tui_settings.ScriptSettingsScreen` ('#st-prompt-text-error' 'offset 9'). Both are skit-tui Ratatui screens, driven by that crate's own TestBackend port, not a non-tty binary."]
fn test_tui_review_rescan_and_settings_handle_new_invalid_bytes() {}

// ==========================================================================
// cli sweep — add / show / params / run / doctor refuse a corrupt prompt cleanly
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): one oracle def sweeping five surfaces; add and show/show --json now converge (source path/offset, no half-commit or U+FFFD). Remaining divergences: (1) `params --json` exits 0 reading the declared params (oracle: exit 1, 'offset 7'); (2) `run --runner codex` exits 125 with the offset-less 'is not valid UTF-8' (oracle: 'offset 7'); (3) `doctor --json` `launch_blocked` is empty (oracle: contains 'offset 7')."]
fn test_cli_add_params_run_and_doctor_refuse_corrupt_prompt_cleanly() {
    let sandbox = Sandbox::new();
    let bad = sandbox.write_file("bad.prompt.md", INVALID_PROMPT);
    let offset = offset_of(INVALID_PROMPT, 0xc3);
    let (add_code, add_out) = sandbox.out(&["add", &bad, "--prompt", "--no-input"]);
    assert_eq!(add_code, 1, "{add_out}");
    assert!(add_out.replace('\n', "").contains(&bad), "{add_out}");
    assert!(
        flatten(&add_out).contains(&format!("offset {offset}")),
        "{add_out}"
    );
    assert!(!add_out.contains('\u{FFFD}'), "{add_out}");
    assert!(
        sandbox.entry_slugs().is_empty(),
        "{:?}",
        sandbox.entry_slugs()
    );

    // A valid entry whose stored body is corrupted after the fact.
    let valid = sandbox.write_file("ok.prompt.md", b"Review {{target}}\n");
    sandbox.ok(&[
        "add",
        &valid,
        "-n",
        "corrupt",
        "--prompt",
        "--no-input",
        "--runner",
        "codex",
    ]);
    fs::write(sandbox.body_path("corrupt"), b"broken:\xff\n").unwrap();

    for args in [vec!["show", "corrupt"], vec!["show", "corrupt", "--json"]] {
        let (code, combined) = sandbox.out(&args);
        assert_eq!(code, 1, "{args:?}: {combined}");
        assert!(
            flatten(&combined).contains("offset 7"),
            "{args:?}: {combined}"
        );
        assert!(!combined.contains("fields"), "{args:?}: {combined}");
        assert!(!combined.contains("No form fields"), "{args:?}: {combined}");
        assert!(!combined.contains('\u{FFFD}'), "{args:?}: {combined}");
    }

    let (params_code, params_out) = sandbox.out(&["params", "corrupt", "--json"]);
    assert_eq!(params_code, 1, "{params_out}");
    assert!(params_out.contains("offset 7"), "{params_out}");
    assert!(!params_out.contains('\u{FFFD}'), "{params_out}");

    let (run_code, run_out) = sandbox.out(&["run", "corrupt", "--runner", "codex", "--no-input"]);
    assert_eq!(run_code, 125, "{run_out}");
    assert!(run_out.contains("offset 7"), "{run_out}");
    assert!(!run_out.contains('\u{FFFD}'), "{run_out}");

    let doctor = sandbox.json(&["doctor", "--json"]);
    let blocked = doctor["launch_blocked"]["corrupt"]
        .as_str()
        .unwrap_or_default();
    assert!(blocked.contains("offset 7"), "launch_blocked={blocked:?}");
}

/// Lowercase hex SHA-256 — the oracle's `hashlib.sha256(data).hexdigest()`.
fn sha256_hex(data: &[u8]) -> String {
    // Minimal, dependency-free SHA-256 so the test stays self-contained.
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in w.iter_mut().take(16).enumerate() {
            let base = index * 4;
            *word = u32::from_be_bytes([
                chunk[base],
                chunk[base + 1],
                chunk[base + 2],
                chunk[base + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for index in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (state, value) in h.iter_mut().zip(v) {
            *state = state.wrapping_add(value);
        }
    }
    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}
