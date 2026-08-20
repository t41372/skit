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
//! Buckets (22 tests; 18 active, 4 ignored):
//! - Active asserting `#[test]`: 16 contracts from this oracle plus
//!   `test_deps_need_sets_the_list` and `test_deps_clear_needs`, rehomed here from the runtime
//!   target because the real binary/store write door is owned by skit-cli-rs. The two-flags draft
//!   boundary is the active multi-flag ordering owner.
//! - CLOSURE — 4 ignored exact names: the kind-exe draft twin names the stronger canonical owner;
//!   the Python public-store refusal cases (7, 9) have no
//!   equivalent Rust public store seam, and their closest CLI mappings duplicate stronger
//!   executable owners; the language `registry.spec_for(...).deps_flavor` premise (20) is a
//!   cross-crate compiling stub because the Rust rewrite disperses that surface.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

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
        self.skit_locale("en")
    }

    fn skit_locale(&self, locale: &str) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", locale);
        command
    }

    fn deps_json(&self, selector: &str) -> serde_json::Value {
        let output = self
            .skit()
            .args(["deps", selector, "--json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", combine(&output));
        serde_json::from_slice(&output.stdout).unwrap()
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

    fn payload_bytes(&self, slug: &str, name: &str) -> Vec<u8> {
        std::fs::read(self.entry_dir(slug).join(name)).unwrap()
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

fn assert_human_output(output: &std::process::Output, expected: &str) {
    assert_eq!(output.status.code(), Some(0), "{}", combine(output));
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    assert!(output.stderr.is_empty(), "{}", combine(output));
}

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, path: &Path, rows: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, rows);
            } else {
                rows.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut rows = BTreeMap::new();
    collect(root, root, &mut rows);
    rows
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

/// Add the oracle's shell copy entry. Needs apply without enabling package-dependency axes.
fn add_shell(sandbox: &Sandbox, name: &str) {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("d.sh");
    std::fs::write(&source, "#!/bin/sh\necho hi\n").unwrap();
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
fn test_two_flags_together_are_both_named_and_joined() {
    // --ref AND --exe together -> both are named, joined with "/" in passing order
    // ("Drop --ref/--exe."). --kind (never passed) stays out of the message.
    let expected = [
        (
            "en",
            "skit-new-both.py is one of skit's own kept drafts — a resumed draft is always added as a copy (and consumed on success), which a reference or program entry can't be. Drop --ref/--exe.",
        ),
        (
            "zh-CN",
            "skit-new-both.py 是 skit 自己保留的草稿——恢复草稿一律以副本加入(成功后即消耗),而 reference 或程序项目做不到这点。请去掉 --ref/--exe。",
        ),
        (
            "zh-TW",
            "skit-new-both.py 是 skit 自己保留的草稿——恢復草稿一律以副本加入(成功後即消耗),而 reference 或程式項目做不到這點。請拿掉 --ref/--exe。",
        ),
    ];
    for (locale, expected) in expected {
        let sandbox = Sandbox::new();
        let draft = sandbox.draft("skit-new-both.py", "print('x')\n");
        let data_before = tree_bytes(sandbox.data.path());
        let state_before = tree_bytes(sandbox.state.path());
        let config_before = tree_bytes(sandbox.config.path());
        let output = sandbox
            .skit_locale(locale)
            .arg("add")
            .arg(&draft)
            .args(["-n", "both", "--ref", "--exe", "--no-input"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "locale={locale}");
        let flat = flatten(&combine(&output));
        assert!(flat.contains(expected), "locale={locale}: {flat}");
        assert!(!flat.contains("--kind"));
        assert_eq!(tree_bytes(sandbox.data.path()), data_before);
        assert_eq!(tree_bytes(sandbox.state.path()), state_before);
        assert_eq!(tree_bytes(sandbox.config.path()), config_before);
        assert!(!sandbox.entry_dir("both").exists());
    }
}

#[test]
#[ignore = "SEMANTIC DUPLICATE (owned-draft root): the stronger canonical single-flag/no-write owner is port_test_add_validation_contracts::test_kind_exe_on_a_kept_draft_is_refused_naming_only_kind_exe. Keep this frozen body for oracle accounting."]
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
    let view = sandbox.deps_json("a");
    assert_eq!(view["requires_python"], ">=3.11");
    assert_eq!(view["dependencies"], serde_json::json!([]));
    let flat = flatten(&combine(output));
    assert_eq!(flat, "Python constraint of a updated: >=3.11");
    assert!(!flat.contains("Dependencies")); // the edit that didn't happen isn't reported
}

#[test]
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
    let view = sandbox.deps_json("a");
    assert_eq!(view["requires_python"], "");
    assert_eq!(view["dependencies"], serde_json::json!(["requests"]));
    let flat = flatten(&combine(output));
    assert_eq!(flat, "Python constraint of a updated: —");
    assert!(!flat.contains("Dependencies"));
}

#[test]
fn test_deps_dep_only_prints_the_deps_line() {
    // --dep alone edited only the dependency axis, so the confirmation is the deps line — the
    // constraint line is absent because its gate (`python is not None`) never fired.
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    let before_config = tree_bytes(sandbox.config.path());
    let before_state = tree_bytes(sandbox.state.path());
    let assert = sandbox
        .skit()
        .args(["deps", "a", "--dep", "requests"])
        .assert();
    let output = assert.get_output();
    assert_human_output(output, "Dependencies of a updated: requests\n");
    assert_eq!(
        sandbox.deps_json("a"),
        serde_json::json!({
            "dependencies": ["requests"],
            "requires_python": "",
            "needs": [],
        })
    );
    assert!(sandbox.stored_copy("a").contains("requests"));
    assert_eq!(tree_bytes(sandbox.config.path()), before_config);
    assert_eq!(tree_bytes(sandbox.state.path()), before_state);
}

#[test]
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
    assert_human_output(
        output,
        concat!(
            "Dependencies of a updated: rich\n",
            "Python constraint of a updated: >=3.12\n",
        ),
    );
    assert_eq!(
        sandbox.deps_json("a"),
        serde_json::json!({
            "dependencies": ["rich"],
            "requires_python": ">=3.12",
            "needs": [],
        })
    );

    // All requested axes report in stable Dependencies -> Python -> Needs order. Repeating the
    // same request localizes the receipt but does not rewrite metadata or the source payload.
    add_py(&sandbox, "ordered");
    let args = [
        "deps", "ordered", "--dep", "httpx", "--python", ">=3.12", "--need", "jq",
    ];
    let english = sandbox.skit().args(args).output().unwrap();
    assert_human_output(
        &english,
        concat!(
            "Dependencies of ordered updated: httpx\n",
            "Python constraint of ordered updated: >=3.12\n",
            "Needs of ordered updated: jq\n",
        ),
    );
    assert_eq!(
        sandbox.deps_json("ordered"),
        serde_json::json!({
            "dependencies": ["httpx"],
            "requires_python": ">=3.12",
            "needs": ["jq"],
        })
    );
    let meta = sandbox.meta_toml("ordered");
    let source = sandbox.stored_copy("ordered");
    let config = tree_bytes(sandbox.config.path());
    let state = tree_bytes(sandbox.state.path());
    for (locale, expected) in [
        (
            "zh-CN",
            concat!(
                "ordered 的依赖已更新:httpx\n",
                "ordered 的 Python 版本约束已更新:>=3.12\n",
                "ordered 所需的命令已更新：jq\n",
            ),
        ),
        (
            "zh-TW",
            concat!(
                "ordered 的依賴已更新:httpx\n",
                "ordered 的 Python 版本約束已更新:>=3.12\n",
                "ordered 所需的命令已更新：jq\n",
            ),
        ),
    ] {
        let output = sandbox.skit_locale(locale).args(args).output().unwrap();
        assert_human_output(&output, expected);
        assert_eq!(sandbox.meta_toml("ordered"), meta);
        assert_eq!(sandbox.stored_copy("ordered"), source);
        assert_eq!(tree_bytes(sandbox.config.path()), config);
        assert_eq!(tree_bytes(sandbox.state.path()), state);
    }
}

#[test]
fn test_deps_clear_prints_the_deps_line() {
    // --clear is a dependency edit (to empty), so it takes the deps line too — and with no
    // --python given, the constraint line's own gate (`python is not None`) stays silent.
    let sandbox = Sandbox::new();
    add_py(&sandbox, "a");
    sandbox
        .skit()
        .args(["deps", "a", "--dep", "requests", "--python", ">=3.11"])
        .assert()
        .success();
    let seeded = sandbox.stored_copy("a");
    let before_config = tree_bytes(sandbox.config.path());
    let before_state = tree_bytes(sandbox.state.path());
    let assert = sandbox.skit().args(["deps", "a", "--clear"]).assert();
    let output = assert.get_output();
    assert_human_output(output, "Dependencies of a updated: —\n");
    assert_eq!(
        sandbox.deps_json("a"),
        serde_json::json!({
            "dependencies": [],
            "requires_python": ">=3.11",
            "needs": [],
        })
    );
    let cleared = sandbox.stored_copy("a");
    assert_ne!(cleared, seeded);
    assert!(!cleared.contains("requests"));
    assert!(cleared.contains("requires-python = \">=3.11\""));
    assert_eq!(tree_bytes(sandbox.config.path()), before_config);
    assert_eq!(tree_bytes(sandbox.state.path()), before_state);
}

// ==========================================================================
// 5. Interpreter-oracle needs receipts — rehomed from the wrong runtime crate
// ==========================================================================

#[test]
fn test_deps_need_sets_the_list() {
    let sandbox = Sandbox::new();
    add_shell(&sandbox, "d");
    let payload = sandbox.payload_bytes("d", "script.sh");
    let before_config = tree_bytes(sandbox.config.path());
    let before_state = tree_bytes(sandbox.state.path());

    let output = sandbox
        .skit()
        .args(["deps", "d", "--need", "jq", "--need", "ffmpeg"])
        .output()
        .unwrap();

    assert_human_output(&output, "Needs of d updated: jq, ffmpeg\n");
    assert_eq!(
        sandbox.deps_json("d"),
        serde_json::json!({
            "dependencies": [],
            "requires_python": "",
            "needs": ["jq", "ffmpeg"],
        })
    );
    let meta = toml::from_str::<toml::Table>(&sandbox.meta_toml("d")).unwrap();
    assert_eq!(
        meta["needs"],
        toml::Value::Array(vec!["jq".into(), "ffmpeg".into()])
    );
    assert_eq!(sandbox.payload_bytes("d", "script.sh"), payload);
    assert_eq!(tree_bytes(sandbox.config.path()), before_config);
    assert_eq!(tree_bytes(sandbox.state.path()), before_state);
}

#[test]
fn test_deps_clear_needs() {
    let sandbox = Sandbox::new();
    add_shell(&sandbox, "d");
    sandbox
        .skit()
        .args(["deps", "d", "--need", "jq"])
        .assert()
        .success();
    let payload = sandbox.payload_bytes("d", "script.sh");
    let before_config = tree_bytes(sandbox.config.path());
    let before_state = tree_bytes(sandbox.state.path());

    let output = sandbox
        .skit()
        .args(["deps", "d", "--clear-needs"])
        .output()
        .unwrap();

    assert_human_output(&output, "Needs of d updated: —\n");
    assert_eq!(
        sandbox.deps_json("d"),
        serde_json::json!({
            "dependencies": [],
            "requires_python": "",
            "needs": [],
        })
    );
    assert!(!sandbox.meta_toml("d").contains("needs"));
    assert_eq!(sandbox.payload_bytes("d", "script.sh"), payload);
    assert_eq!(tree_bytes(sandbox.config.path()), before_config);
    assert_eq!(tree_bytes(sandbox.state.path()), before_state);

    // A requested clear still reports its axis, but a successful no-op rewrites no bytes.
    let meta = sandbox.meta_toml("d");
    let output = sandbox
        .skit()
        .args(["deps", "d", "--clear-needs"])
        .output()
        .unwrap();
    assert_human_output(&output, "Needs of d updated: —\n");
    assert_eq!(sandbox.meta_toml("d"), meta);
    assert_eq!(sandbox.payload_bytes("d", "script.sh"), payload);
}

// ==========================================================================
// 6. registry sanity — js is the npm flavor this whole gate keys on
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (language registry): the oracle's registry.spec_for('js').deps_flavor == 'npm' (and python != 'npm') has no single public Rust surface. The Rust rewrite disperses the LangSpec; the kind->flavor map is the private const fn dependency_flavor (skit-cli/src/cli.rs:5338) plus skit-ui's DependencySurface, neither reachable as a public spec_for. Owner: language registry. Python ref registry.spec_for, test_dependency_command_contracts.py:306-314."]
fn test_js_is_npm_flavor_and_python_is_not() {
    // The premise the uv_flavor gate rests on, pinned directly: js is deps_flavor 'npm'
    // (so its --python is inapplicable) and python is not.
}
