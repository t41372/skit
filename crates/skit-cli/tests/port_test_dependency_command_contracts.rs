//! Mechanical port of the Python oracle module `tests/test_dependency_command_contracts.py`
//! (`origin/main@206f9ef`): "Dependency-command contracts (exit codes, exact refusal/
//! confirmation copy, meta + stored PEP 723 text, the store chokepoints in isolation)."
//! Each `#[test]` keeps its Python `def test_*` name so it traces back to its origin, and
//! each Python "WHY" comment is preserved above it.
//!
//! WHY skit-cli: the oracle drives `cli.app` (a Typer `CliRunner`), the `store` module
//! (`add_python` / `add_script` / `update_dependencies` / `resolve`), and the language
//! `registry`. The Rust rewrite DISPERSES that surface — there is no public
//! `store.update_dependencies` / `store.add_python` function; the dependency and add
//! contracts live only behind the `deps` and `add` commands at the composition root. Only
//! `skit-cli-rs` can spawn the real `skit` binary AND depends on every crate the port needs,
//! so the port lives here and drives each contract through the binary (the sibling
//! `port_test_js_deps.rs` sets the same precedent for store-level tests).
//!
//! Concept mapping used throughout:
//! - Python `cli.app` (`CliRunner.invoke`) -> the real `skit` binary via `assert_cmd`;
//!   `result.output` (stdout + stderr) -> `combine(&Output)`; `_flat(text)` -> `flatten(&str)`.
//! - Python `store.add_python(src, name=…)` -> `skit add <s.py> -n <name> --no-input`.
//! - Python `store.add_script(js, kind="js", name=…)` -> `skit add <t.js> -n <name>
//!   --no-input` (the `.js` extension infers kind js — the probed outcome of `kind="js"`).
//! - Python `store.update_dependencies(name, deps, requires_python=…)` -> `skit deps <name>
//!   [--dep …] [--clear] [--python …]`; a raised `StoreUsageError` -> a usage refusal (exit 2).
//! - Python `store.resolve(name).meta` -> `skit deps <name> --json` (dependencies /
//!   requires_python) and the entry's `meta.toml` (test 14's `dependencies is None`).
//! - Python `drafts_dir()` -> `<SKIT_DATA_DIR>/drafts`.
//!
//! Buckets (recorded per test in the structured result):
//! - REAL asserting `#[test]` (API exists, behavior agrees): the npm refusal spellings (3–5),
//!   the uv-flavor '-'/'none' normalization (6, 8), the deps-only npm edit (10), and the
//!   add_python belt's validation, strip-and-drop, and no-deps transparency (11–14). 10 tests.
//! - FAILING CONTRACT (divergence) — full oracle-faithful body, `#[ignore]`d because the
//!   Rust behavior diverges (verified against the built binary): the wholly-unimplemented
//!   drafts-boundary guard (1, 2) and the deps confirmation-line shape (15–19). 7 tests.
//! - CLOSURE — 3 ignored exact names: the Python public-store refusal cases (7, 9) have no
//!   equivalent Rust public store seam, and their closest CLI mappings duplicate stronger
//!   executable owners; the language `registry.spec_for(...).deps_flavor` premise (20) is a
//!   cross-crate compiling stub because the Rust rewrite disperses that surface.

use std::path::PathBuf;

use tempfile::TempDir;

// ============================================================================
// Self-contained fixtures (no shared helper is edited or imported).
// ============================================================================

/// A fresh three-directory sandbox plus the real `skit` binary. Every invocation
/// re-points SKIT_DATA_DIR / SKIT_STATE_DIR / SKIT_CONFIG_DIR at the temp roots, so skit
/// writes only inside the sandbox (the oracle's `monkeypatch.setenv` autouse fixture).
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

    fn skit(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// The `scripts/<slug>` entry directory (the oracle's `entry.dir`).
    fn entry_dir(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug)
    }

    /// The stored python copy's text (the oracle's `_stored_block`: `entry.dir / "script.py"`).
    fn stored_copy(&self, slug: &str) -> String {
        std::fs::read_to_string(self.entry_dir(slug).join("script.py")).unwrap()
    }

    /// The entry's raw `meta.toml` (for test 14's `meta.dependencies is None`).
    fn meta_toml(&self, slug: &str) -> String {
        std::fs::read_to_string(self.entry_dir(slug).join("meta.toml")).unwrap()
    }

    /// Write a file into skit's OWN drafts home (`drafts_dir()`), returning its path.
    fn draft(&self, name: &str, body: &str) -> PathBuf {
        let dir = self.data.path().join("drafts");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }
}

/// Combine stdout and stderr, mirroring Typer's `CliRunner.output`.
fn combine(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

/// Python `_flat(text) = " ".join(text.split())` — collapse every run of whitespace.
fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Add a python copy entry named `name` from a fresh `print(1)` source (the oracle's `_py` +
/// `store.add_python`). The temp source may drop at once: copy mode copies its bytes now.
fn add_py(sandbox: &Sandbox, name: &str) {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("s.py");
    std::fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .skit()
        .arg("add")
        .arg(&source)
        .args(["-n", name, "--no-input"])
        .assert()
        .success();
}

/// Add a js copy entry named `name` (the oracle's `_js` + `store.add_script(kind="js")`).
fn add_js(sandbox: &Sandbox, name: &str) {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("t.js");
    std::fs::write(&source, "console.log(1)\n").unwrap();
    sandbox
        .skit()
        .arg("add")
        .arg(&source)
        .args(["-n", name, "--no-input"])
        .assert()
        .success();
}

// ==========================================================================
// 1. The drafts-boundary refusal names ONLY the flags actually typed
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the drafts-boundary guard (oracle cli.py:1894-1933) is wholly unimplemented. `skit add <draft> --ref --exe --no-input` should exit 2 naming 'one of skit's own kept drafts' and 'Drop --ref/--exe.'; Rust adds successfully (exit 0, entry created, draft kept). Verified against the built binary."]
fn test_two_flags_together_are_both_named_and_joined() {
    // --ref AND --exe together -> both are named, joined with "/" in passing order
    // ("Drop --ref/--exe."). --kind (never passed) stays out of the message.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-both.py", "print('x')\n");
    let assert = sandbox
        .skit()
        .arg("add")
        .arg(&draft)
        .args(["-n", "both", "--ref", "--exe", "--no-input"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    let flat = flatten(&combine(output));
    assert!(flat.contains("one of skit's own kept drafts"));
    assert!(flat.contains("Drop --ref/--exe.")); // both named, joined in passing order
    assert!(!flat.contains("--kind")); // never passed — never named
    assert!(draft.exists()); // a refused add consumes nothing
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the drafts-boundary guard (oracle cli.py:1894-1933) is wholly unimplemented. `skit add <draft> --kind exe --no-input` should exit 2 naming 'Drop --kind exe.'; Rust adds successfully (exit 0, entry created, draft kept). Verified against the built binary."]
fn test_kind_exe_alone_names_only_kind_exe() {
    // --kind exe alone -> the refusal names ONLY "--kind exe"; the "--ref"/"--exe" flag
    // literals (neither passed) are absent — the honest-naming rule end to end.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-kindonly.py", "print('x')\n");
    let assert = sandbox
        .skit()
        .arg("add")
        .arg(&draft)
        .args(["-n", "kindonly", "--kind", "exe", "--no-input"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    let flat = flatten(&combine(output));
    assert!(flat.contains("Drop --kind exe."));
    assert!(!flat.contains("--ref"));
    assert!(!flat.contains("--exe")); // "--kind exe" is not the bare "--exe" flag literal
    assert!(draft.exists());
}

// ==========================================================================
// 2. '-'/'none' → automatic is gated on uv flavor
// ==========================================================================

#[test]
fn test_js_deps_python_dash_is_refused_as_inapplicable() {
    // `skit deps <js> --python -` is REFUSED (exit 2, "doesn't apply"), NOT silently accepted:
    // normalizing '-' to "" first would make a kind-inapplicable flag succeed for some spellings
    // (— / none) and fail for others (>=3.11) — value-dependent acceptance.
    let sandbox = Sandbox::new();
    add_js(&sandbox, "jsx");
    let assert = sandbox
        .skit()
        .args(["deps", "jsx", "--python", "-"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    assert!(flatten(&combine(output)).contains("A Python constraint doesn't apply to js scripts."));
}

#[test]
fn test_js_deps_python_none_is_refused_as_inapplicable() {
    // The other automatic token behaves identically: '-' and 'none' are NOT special-cased
    // into acceptance on an npm entry.
    let sandbox = Sandbox::new();
    add_js(&sandbox, "jsx");
    let assert = sandbox
        .skit()
        .args(["deps", "jsx", "--python", "none"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    assert!(flatten(&combine(output)).contains("A Python constraint doesn't apply to js scripts."));
}

#[test]
fn test_js_deps_python_empty_string_is_refused_as_inapplicable() {
    // The missing spelling: `--python ''` (empty) is a spelling too, and is REFUSED identically
    // to '-'/'none'/a real constraint (exit 2, "doesn't apply") — nothing is written. The npm
    // predicate keys on `requires_python is not None`, not truthiness, so the empty string can't
    // slip through as a green "Python constraint updated: —"; add and deps answer it the same.
    let sandbox = Sandbox::new();
    add_js(&sandbox, "jsx");
    // a dep to prove the refusal disturbs nothing
    sandbox
        .skit()
        .args(["deps", "jsx", "--dep", "chalk"])
        .assert()
        .success();
    let assert = sandbox
        .skit()
        .args(["deps", "jsx", "--python", ""])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    assert!(flatten(&combine(output)).contains("A Python constraint doesn't apply to js scripts."));
    let view = sandbox.skit().args(["deps", "jsx", "--json"]).assert();
    let json = combine(view.get_output());
    assert!(json.contains("\"requires_python\":\"\"")); // untouched
    assert!(json.contains("\"dependencies\":[\"chalk\"]")); // the refusal wrote nothing
}

#[test]
fn test_python_deps_python_dash_is_still_automatic() {
    // The regression: on a uv-flavor (python) entry, '-' STILL normalizes to automatic — the
    // gate narrows the normalization to uv entries, it does not remove it.
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    sandbox
        .skit()
        .args(["deps", "a", "--dep", "requests", "--python", ">=3.11"])
        .assert()
        .success();
    let assert = sandbox
        .skit()
        .args(["deps", "a", "--python", "-", "--json"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(0));
    assert!(combine(output).contains("\"requires_python\":\"\"")); // cleared to automatic
}

#[test]
#[ignore = "ARCHITECTURE-CLOSED / SEMANTIC DUPLICATE: the Python oracle calls public store.update_dependencies and observes StoreUsageError, but Rust has no public store dependency-update seam. This closest Rust body drives the CLI and is strictly duplicated by the stronger executable owner test_js_deps_python_dash_is_refused_as_inapplicable, which pins the exact message. Keep this exact name as a closure; do not count it as REAL."]
fn test_store_npm_spec_plus_dash_reaches_the_npm_refusal() {
    // The store unit: an npm-flavor entry + '-' is NOT normalized before the npm branch, so it
    // reaches the 'doesn't apply' refusal (StoreUsageError) instead of a silent accept.
    let sandbox = Sandbox::new();
    add_js(&sandbox, "jsx");
    let assert = sandbox
        .skit()
        .args(["deps", "jsx", "--python", "-"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    assert!(combine(output).contains("doesn't apply"));
}

#[test]
fn test_store_uv_spec_plus_dash_normalizes() {
    // The complement unit: a uv-flavor entry + '-' IS normalized to "" (the gate's True
    // branch) — meta records automatic, no error.
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    let assert = sandbox
        .skit()
        .args([
            "deps", "a", "--dep", "requests", "--python", "none", "--json",
        ])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(0));
    assert!(combine(output).contains("\"requires_python\":\"\""));
}

#[test]
#[ignore = "ARCHITECTURE-CLOSED / SEMANTIC DUPLICATE: the Python oracle calls public store.update_dependencies and observes StoreUsageError, but Rust has no public store dependency-update seam. This closest Rust body drives the CLI and is strictly duplicated by the stronger executable owner test_js_deps_python_empty_string_is_refused_as_inapplicable, which pins the exact message and unchanged state. Keep this exact name as a closure; do not count it as REAL."]
fn test_store_npm_spec_plus_empty_string_reaches_the_npm_refusal() {
    // The empty branch of the npm predicate `requires_python is not None`: `""` (not None) is a
    // Python constraint spelling and raises StoreUsageError on an npm entry — the branch that used
    // to slip through when the predicate was bare truthiness.
    let sandbox = Sandbox::new();
    add_js(&sandbox, "jsx");
    let assert = sandbox
        .skit()
        .args(["deps", "jsx", "--python", ""])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    assert!(combine(output).contains("doesn't apply"));
}

#[test]
fn test_store_npm_spec_plus_none_deps_edit_is_not_refused() {
    // The None branch of the same predicate: a deps-only edit (requires_python is None) on an
    // npm entry is NOT a Python edit, so the constraint refusal is skipped and the dependency
    // lands — the predicate must refuse the empty string WITHOUT catching a plain deps write.
    let sandbox = Sandbox::new();
    add_js(&sandbox, "jsx");
    let assert = sandbox
        .skit()
        .args(["deps", "jsx", "--dep", "chalk", "--json"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(0));
    assert!(combine(output).contains("\"dependencies\":[\"chalk\"]"));
}

// ==========================================================================
// 3. add_python's strip-and-drop + validate-before-build belt
// ==========================================================================

#[test]
fn test_add_python_belt_rejects_a_bad_dep_before_any_entry_exists() {
    // A direct store.add_python with an unparseable dependency raises at the belt — BEFORE the
    // source is read or a meta/entry dir is built, so no half-made entry is registered.
    let sandbox = Sandbox::new();
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("s.py");
    std::fs::write(&source, "print(1)\n").unwrap();
    let assert = sandbox
        .skit()
        .arg("add")
        .arg(&source)
        .args(["-n", "belt", "--dep", "@@@", "--no-input"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    assert!(combine(output).contains("isn't a package requirement"));
    // nothing was created
    let view = sandbox.skit().args(["deps", "belt", "--json"]).assert();
    assert_eq!(view.get_output().status.code(), Some(1));
}

#[test]
fn test_add_python_belt_rejects_a_bad_python_before_any_entry_exists() {
    // The constraint half of the belt: an unparseable requires-python is refused the same way.
    let sandbox = Sandbox::new();
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("s.py");
    std::fs::write(&source, "print(1)\n").unwrap();
    let assert = sandbox
        .skit()
        .arg("add")
        .arg(&source)
        .args(["-n", "belt", "--python", "not-a-version", "--no-input"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(2));
    assert!(combine(output).contains("isn't a Python version constraint"));
    let view = sandbox.skit().args(["deps", "belt", "--json"]).assert();
    assert_eq!(view.get_output().status.code(), Some(1));
}

#[test]
fn test_add_python_belt_drops_a_whitespace_dep_from_the_block() {
    // The strip-and-drop half: a whitespace-only entry alongside a real one is dropped — the
    // stored block declares only the real dependency, never the "" that would brick every run.
    let sandbox = Sandbox::new();
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("s.py");
    std::fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .skit()
        .arg("add")
        .arg(&source)
        .args(["-n", "belt2", "--dep", "  ", "--dep", "rich", "--no-input"])
        .assert()
        .success();
    let block = sandbox.stored_copy("belt2");
    assert!(block.contains("rich"));
    assert!(!block.contains("\"  \"")); // the whitespace entry never reached the PEP 723 block
}

#[test]
fn test_add_python_belt_with_no_deps_is_unchanged() {
    // The None branch of the belt (`dependencies` falsy → stays None): a plain add still writes
    // no block and records no dependencies — the belt is transparent when there is nothing to
    // filter.
    let sandbox = Sandbox::new();
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("s.py");
    std::fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .skit()
        .arg("add")
        .arg(&source)
        .args(["-n", "plain", "--no-input"])
        .assert()
        .success();
    // meta.dependencies is None: meta.toml records no dependencies key.
    assert!(!sandbox.meta_toml("plain").contains("dependencies"));
    // and no PEP 723 block was injected into the stored copy.
    assert!(!sandbox.stored_copy("plain").contains("# /// script"));
}

// ==========================================================================
// 4. `skit deps` confirmation-line honesty
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the constraint edit lands (exit 0), but Rust prints the unconditional three-line view 'Dependencies: … / Python constraint: >=3.11 / Required commands: …' — never the per-axis 'Python constraint of a updated: >=3.11', and it always prints a 'Dependencies' line (cli.py:4980-4991). Verified against the built binary."]
fn test_deps_python_only_prints_the_constraint_line_not_the_deps_line() {
    // --python alone edited only the constraint, so the confirmation says so — and does NOT
    // claim "Dependencies … updated", which would describe an edit that never happened.
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    let assert = sandbox
        .skit()
        .args(["deps", "a", "--python", ">=3.11"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(0));
    let flat = flatten(&combine(output));
    assert!(flat.contains("Python constraint of a updated: >=3.11"));
    assert!(!flat.contains("Dependencies")); // the edit that didn't happen isn't reported
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the clear lands (exit 0), but Rust prints 'Python constraint: ' (empty) in its unconditional view — never the per-axis 'Python constraint of a updated: —', and it always prints a 'Dependencies' line (cli.py:4980-4991). Verified against the built binary."]
fn test_deps_python_only_dash_reports_the_dash_placeholder() {
    // --python - clears to automatic; the constraint line shows the em-dash placeholder for
    // "no constraint recorded" (the `escape(...) or '—'` fallback).
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    sandbox
        .skit()
        .args(["deps", "a", "--dep", "requests", "--python", ">=3.11"])
        .assert()
        .success();
    let assert = sandbox.skit().args(["deps", "a", "--python", "-"]).assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(0));
    let flat = flatten(&combine(output));
    assert!(flat.contains("Python constraint of a updated: —"));
    assert!(!flat.contains("Dependencies"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the deps edit lands (exit 0), but Rust prints the unconditional three-line view — never the per-axis 'Dependencies of a updated: requests', and it always prints a 'Python constraint' line (cli.py:4980-4991). Verified against the built binary."]
fn test_deps_dep_only_prints_the_deps_line() {
    // --dep alone edited only the dependency axis, so the confirmation is the deps line — the
    // constraint line is absent because its gate (`python is not None`) never fired.
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    let assert = sandbox
        .skit()
        .args(["deps", "a", "--dep", "requests"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(0));
    let flat = flatten(&combine(output));
    assert!(flat.contains("Dependencies of a updated: requests"));
    assert!(!flat.contains("Python constraint of"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): both axes land (the constraint reaches meta, `deps a --json` shows requires_python >=3.12), but Rust prints the unconditional three-line view — never the per-axis 'Dependencies of a updated: rich' / 'Python constraint of a updated: >=3.12' (cli.py:4980-4991). Verified against the built binary."]
fn test_deps_dep_and_python_together_prints_both_axis_lines() {
    // --dep AND --python moved BOTH axes, so BOTH confirmation lines print — each naming its own
    // axis. The per-axis rule (deps line when `dep is not None or clear`, constraint line when
    // `python is not None`) refuses the old silence about a constraint that DID move: previously the
    // second axis rode along mutely in the stored block, and only the deps line was shown.
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    let assert = sandbox
        .skit()
        .args(["deps", "a", "--dep", "rich", "--python", ">=3.12"])
        .assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(0));
    let flat = flatten(&combine(output));
    assert!(flat.contains("Dependencies of a updated: rich")); // the deps axis moved
    assert!(flat.contains("Python constraint of a updated: >=3.12")); // and so did the constraint axis
    let view = sandbox.skit().args(["deps", "a", "--json"]).assert();
    assert!(combine(view.get_output()).contains("\"requires_python\":\">=3.12\"")); // the constraint landed
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the clear lands (exit 0), but Rust prints the unconditional three-line view — never the per-axis 'Dependencies of a updated: —', and it always prints a 'Python constraint' line (cli.py:4980-4991). Verified against the built binary."]
fn test_deps_clear_prints_the_deps_line() {
    // --clear is a dependency edit (to empty), so it takes the deps line too — and with no
    // --python given, the constraint line's own gate (`python is not None`) stays silent.
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    sandbox
        .skit()
        .args(["deps", "a", "--dep", "requests"])
        .assert()
        .success();
    let assert = sandbox.skit().args(["deps", "a", "--clear"]).assert();
    let output = assert.get_output();
    assert_eq!(output.status.code(), Some(0));
    let flat = flatten(&combine(output));
    assert!(flat.contains("Dependencies of a updated: —"));
    assert!(!flat.contains("Python constraint of"));
}

// ==========================================================================
// 5. registry sanity — js is the npm flavor this whole gate keys on
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (language registry): the oracle's registry.spec_for('js').deps_flavor == 'npm' (and python != 'npm') has no single public Rust surface. The Rust rewrite disperses the LangSpec; the kind->flavor map is the private const fn dependency_flavor (skit-cli/src/cli.rs:5338) plus skit-ui's DependencySurface, neither reachable as a public spec_for. Owner: language registry. Python ref registry.spec_for, test_dependency_command_contracts.py:306-314."]
fn test_js_is_npm_flavor_and_python_is_not() {
    // The premise the uv_flavor gate rests on, pinned directly: js is deps_flavor 'npm'
    // (so its --python is inapplicable) and python is not.
}
