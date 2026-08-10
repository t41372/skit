//! Mechanical port of the Python oracle module `tests/test_config_cmd.py`
//! (`origin/main@206f9ef`): "`skit config` (git-config grammar: bare = list, KEY = read,
//! KEY VALUE = write), the first-run mirror offer, and the mirror wizard it still uses."
//! Each `#[test]` keeps its Python `def test_*` name, and each Python "WHY" comment is kept
//! above it, so it traces back to its origin.
//!
//! WHY `skit-cli`: the oracle drives `config` through Typer's `CliRunner`. Only the
//! composition-root crate runs the real `skit config` binary end to end (resolve config dir ->
//! read/normalize/write config.toml -> print), so this is a black-box `assert_cmd` port.
//!
//! Concept mapping:
//! - Python `runner.invoke(cli.app, ["config", ...])` -> a real `skit config ...` invocation with
//!   `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` pinned to fresh temp dirs and
//!   `SKIT_LANG=en` (the oracle's `isolated` fixture, which also pins the output locale).
//! - Python `config.load_mirror()` axis reads -> the `config mirror.<axis>` READ path (a preset
//!   name / URL / "off" / "custom"), the `config mirror --json` master token ("on"/"off"), and
//!   direct `config.toml` inspection where an exact stored URL is the claim.
//! - Python `config.load_config()["language"]` / `config.load_form()` / `config.load_after_run()`
//!   / `config.load_bash_path()` / `config.load_js_runner()` -> the corresponding `config KEY`
//!   READ path (raw machine token), or `config KEY --json`.
//! - Python `config.save_config({...})` / `config.save_mirror(...)` (hand-writing a corrupt or
//!   underivable document) -> writing `config.toml` bytes directly before the invocation.
//! - Python `cli._lang_override()` (a private helper) -> the `config lang --json` raw value, which
//!   is exactly `_config_raw_value("lang") == _lang_override()`.
//!
//! Buckets:
//! - Bucket 1 (API EXISTS, asserting): the bulk below — every `config` read/write/list path.
//! - Bucket 2 (DIVERGENCE, `#[ignore]` with the full asserting body): ten tests where the Rust CLI
//!   genuinely differs from the oracle — it has no localized human display sentinels
//!   ("default ($VISUAL / $EDITOR)", "auto (…)"), prints no paused "switched off" stderr notice on
//!   an axis write, gives no `mirror.pypi` axis pointer in the master-switch errors, names no
//!   choices in the `js.runner` error, and lists settings as `key=value` (not padded columns). The
//!   body is kept intact; deleting the `#[ignore]` after an impl fix must go green.
//! - Bucket 3 (CROSS-CRATE, `#[ignore]` stub): the mirror-wizard and first-run tests. Both drive
//!   the private, interactive helpers `mirror_wizard(store, &dyn FirstRunPrompt)` and
//!   `first_run_mirror_offer(store, &dyn NetworkProbe, &dyn FirstRunPrompt, interactive)` in
//!   `crates/skit-cli/src/cli.rs`, which capture prompt defaults/choices. A non-`pub` interactive
//!   path is not reachable or observable from a black-box binary integration test; it is exercised
//!   white-box inside the crate.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

// --- Preset URLs the oracle's constant tables expand to (config.py:31-52, 75-76). ---
const PYPI_PRESETS: &[(&str, &str)] = &[
    ("tsinghua", "https://pypi.tuna.tsinghua.edu.cn/simple"),
    ("aliyun", "https://mirrors.aliyun.com/pypi/simple"),
    ("ustc", "https://pypi.mirrors.ustc.edu.cn/simple"),
];
const NPM_REGISTRY_MIRROR: &str = "https://registry.npmmirror.com";
const PYTHON_INSTALL_MIRROR: &str =
    "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/";
const UV_BINARY_MIRROR: &str = "https://mirror.nju.edu.cn/github-release/astral-sh/uv";
const JS_RUNNERS: &[&str] = &["deno", "bun", "node"];

/// One captured invocation: its stdout, stderr, and process exit code.
#[derive(Debug)]
struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

impl Out {
    /// The oracle's `result.output` mixes stdout and stderr; this reproduces that combined view.
    fn both(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// The oracle's `isolated` fixture: fresh config/data/state roots plus `SKIT_LANG=en`.
#[derive(Debug)]
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

    fn command(&self, lang: &str) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", lang);
        command
    }

    /// `runner.invoke(cli.app, args)` under the English output locale.
    fn run(&self, args: &[&str]) -> Out {
        self.run_lang("en", args)
    }

    /// `runner.invoke(cli.app, args)` under an explicit output locale.
    fn run_lang(&self, lang: &str, args: &[&str]) -> Out {
        let output = self.command(lang).args(args).output().unwrap();
        Out {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
        }
    }

    fn config_path(&self) -> PathBuf {
        self.config.path().join("config.toml")
    }

    /// The oracle's `config.save_config(...)` / `config.save_mirror(...)` when it hand-writes a
    /// document — here we drop the bytes straight onto disk before the invocation.
    fn write_config(&self, contents: &str) {
        fs::write(self.config_path(), contents).unwrap();
    }

    /// The raw stored `config.toml` bytes ("" when absent) — for an exact stored-URL claim.
    fn read_config(&self) -> String {
        fs::read_to_string(self.config_path()).unwrap_or_default()
    }
}

/// The value a `config KEY` READ path prints, trimmed of the trailing newline.
fn read_key(sandbox: &Sandbox, key: &str) -> String {
    sandbox.run(&["config", key]).stdout.trim().to_owned()
}

/// Parse the flat `{"key":"value",...}` object the CLI emits (a string->string map, no nesting;
/// values here never contain a quote or comma). This local parser keeps the file self-contained
/// (no `serde_json` dev-dependency).
fn parse_flat_json(text: &str) -> BTreeMap<String, String> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'))
        .unwrap_or_else(|| panic!("not a JSON object: {text:?}"));
    let mut map = BTreeMap::new();
    if body.is_empty() {
        return map;
    }
    for pair in body.split(',') {
        let (key, value) = pair.split_once(':').unwrap();
        map.insert(
            key.trim().trim_matches('"').to_owned(),
            value.trim().trim_matches('"').to_owned(),
        );
    }
    map
}

// --- bare `skit config`: list everything ---

#[test]
fn test_bare_config_lists_all_keys() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config"]);
    assert_eq!(result.code, 0);
    for key in ["lang", "editor", "mirror", "form", "after_run"] {
        assert!(result.stdout.contains(key), "{key} in {:?}", result.stdout);
    }
    assert!(result.stdout.contains("off")); // mirror default
    assert!(result.stdout.contains("tui")); // form default
}

#[test]
fn test_bare_config_json() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "--json"]);
    assert_eq!(result.code, 0);
    let doc = parse_flat_json(&result.stdout);
    let keys = doc
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [
        "lang",
        "editor",
        "mirror",
        "mirror.pypi",
        "mirror.github",
        "mirror.npm",
        "form",
        "after_run",
        "shell.bash_path",
        "js.runner",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(keys, expected);
    assert_eq!(doc["mirror"], "off");
    assert_eq!(doc["mirror.pypi"], "off");
    assert_eq!(doc["mirror.github"], "off");
    assert_eq!(doc["mirror.npm"], "off");
    assert_eq!(doc["form"], "tui");
    assert_eq!(doc["after_run"], "exit"); // the launcher default
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle config LIST shows localized display sentinels (cli.py:5301-5326); Rust prints raw empty values, no human display column"]
fn test_config_json_emits_raw_values_never_a_localized_sentinel() {
    // `config --json` is a STABLE machine contract: it emits the RAW stored values, never the
    // localized DISPLAY sentinels. Under a non-English locale the payload must be byte-identical
    // to the English one — an agent must tell unset ("") apart from a value, regardless of the
    // user's language. (The Rust --json satisfies this; the diverging claim is the LAST block:
    // the HUMAN read must keep the localized sentinels, which the Rust CLI does not implement.)
    let sandbox = Sandbox::new();
    let doc = parse_flat_json(&sandbox.run_lang("zh-TW", &["config", "--json"]).stdout);
    assert_eq!(doc["editor"], ""); // unset editor is raw "", not the localized "default (…)"
    assert_eq!(doc["js.runner"], ""); // auto is raw ""
    assert_eq!(doc["lang"], ""); // unset language override
    assert_eq!(doc["mirror"], "off"); // a literal, never localized
    assert_eq!(doc["shell.bash_path"], "");
    // The whole-config --json payload is byte-identical across locales.
    let english = sandbox.run_lang("en", &["config", "--json"]).stdout;
    let localized = sandbox.run_lang("zh-TW", &["config", "--json"]).stdout;
    assert_eq!(localized, english);

    // Single-key and write paths are RAW too (same shape as the whole-config read).
    assert_eq!(
        parse_flat_json(
            &sandbox
                .run_lang("zh-TW", &["config", "editor", "--json"])
                .stdout
        ),
        BTreeMap::from([("editor".to_owned(), String::new())])
    );
    assert_eq!(
        parse_flat_json(
            &sandbox
                .run_lang("zh-TW", &["config", "js.runner", "--json"])
                .stdout
        ),
        BTreeMap::from([("js.runner".to_owned(), String::new())])
    );
    let writer = Sandbox::new();
    let written = parse_flat_json(
        &writer
            .run_lang("zh-TW", &["config", "mirror.pypi", "tsinghua", "--json"])
            .stdout,
    );
    assert_eq!(written["mirror.pypi"], "tsinghua"); // a machine token, never a localized word

    // Human (non-json) output KEEPS the display sentinels — this is UI copy. The oracle asserts the
    // zh-TW-localized sentinels appear in the bare `config` LIST (`assert editor_sentinel in human`,
    // `assert js_sentinel in human`). The localization half is not restorable here: these sentinel
    // keys do not exist in the Rust i18n catalog at all, so this asserts the English source
    // sentinels under `en` — the divergence (Rust has no human display column) fails identically in
    // any locale.
    let human = sandbox.run(&["config"]).stdout;
    assert!(
        human.contains("default ($VISUAL / $EDITOR)"),
        "config LIST must show the editor default sentinel: {human:?}"
    );
    assert!(
        human.contains("auto (deno > bun > node)"),
        "config LIST must show the js.runner auto sentinel: {human:?}"
    );
}

#[test]
fn test_read_one_key_json_emits_single_pair() {
    // `config KEY --json` -> {KEY: value} (the flag was previously ignored on this read path):
    // stdout is exactly one JSON document, same shape as the whole-config read.
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "form", "--json"]);
    assert_eq!(result.code, 0, "{}", result.both());
    assert_eq!(result.stdout.trim(), r#"{"form":"tui"}"#);
}

#[test]
fn test_set_one_key_json_emits_final_pair() {
    // `config KEY VALUE --json` -> the FINAL {KEY: value} after the write (the flag was ignored on
    // this write path too), not the human "key = value" line.
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "form", "plain", "--json"]);
    assert_eq!(result.code, 0, "{}", result.both());
    assert_eq!(result.stdout.trim(), r#"{"form":"plain"}"#); // the written state
    assert_eq!(read_key(&sandbox, "form"), "plain"); // really persisted
}

#[test]
fn test_unknown_key_exits_2() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "theme"]);
    assert_eq!(result.code, 2);
}

// --- lang ---

#[test]
fn test_set_lang_writes_language_key() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "lang", "zh-CN"]);
    assert_eq!(result.code, 0);
    // The raw "lang" json value is exactly the stored language override (config.load_config()["language"]).
    assert_eq!(
        sandbox.run(&["config", "lang", "--json"]).stdout.trim(),
        r#"{"lang":"zh-CN"}"#
    );
}

#[test]
fn test_read_lang_shows_override() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "lang", "zh-CN"]);
    let result = sandbox.run(&["config", "lang"]);
    assert_eq!(result.code, 0);
    assert!(result.stdout.contains("zh-CN"));
}

#[test]
fn test_lang_auto_clears() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "lang", "zh-CN"]);
    let result = sandbox.run(&["config", "lang", "auto"]);
    assert_eq!(result.code, 0);
    assert!(!sandbox.read_config().contains("language"));
}

#[test]
fn test_unknown_lang_exits_2() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "lang", "xx-YY"]);
    assert_eq!(result.code, 2);
}

// --- mirror: three independent per-ecosystem axes + a master switch ---

#[test]
fn test_set_mirror_pypi_preset() {
    // Parametrized over every PYPI_PRESETS name.
    for (name, url) in PYPI_PRESETS {
        let sandbox = Sandbox::new();
        let result = sandbox.run(&["config", "mirror.pypi", name]);
        assert_eq!(result.code, 0, "{name}: {}", result.both());
        // m.enabled -> the master token reads "on".
        assert_eq!(read_key(&sandbox, "mirror"), "on");
        // m.pypi == PYPI_PRESETS[name] -> the axis reads back its preset name AND the exact URL is on disk.
        assert_eq!(read_key(&sandbox, "mirror.pypi"), *name);
        assert!(
            sandbox.read_config().contains(url),
            "{name}: {}",
            sandbox.read_config()
        );
    }
}

#[test]
fn test_pypi_axis_does_not_drag_other_axes() {
    // The regression this design fix exists for: a PyPI vendor choice must configure the PyPI axis
    // and NOTHING else — the npm/github axes have their own vendor landscapes.
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    // (m.npm, m.python_install, m.uv_binary) == ("", "", "") -> npm and github axes read "off".
    assert_eq!(read_key(&sandbox, "mirror.npm"), "off");
    assert_eq!(read_key(&sandbox, "mirror.github"), "off");
}

#[test]
fn test_set_mirror_npm_alone() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror.npm", "npmmirror"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "mirror"), "on"); // m.enabled
    assert_eq!(read_key(&sandbox, "mirror.npm"), "npmmirror"); // m.npm == NPM_REGISTRY_MIRROR
    assert!(sandbox.read_config().contains(NPM_REGISTRY_MIRROR));
    // (m.pypi, m.python_install, m.uv_binary) == ("", "", "")
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "off");
    assert_eq!(read_key(&sandbox, "mirror.github"), "off");
}

#[test]
fn test_set_mirror_github_expands_both_urls() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror.github", "nju"]);
    assert_eq!(result.code, 0);
    // m.python_install == PYTHON_INSTALL_MIRROR and m.uv_binary == UV_BINARY_MIRROR
    let doc = sandbox.read_config();
    assert!(doc.contains(PYTHON_INSTALL_MIRROR), "{doc}");
    assert!(doc.contains(UV_BINARY_MIRROR), "{doc}");
    // (m.pypi, m.npm) == ("", "")
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "off");
    assert_eq!(read_key(&sandbox, "mirror.npm"), "off");
}

#[test]
fn test_set_mirror_github_custom_base_expands() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror.github", "https://my.mirror/gh"]);
    assert_eq!(result.code, 0);
    let doc = sandbox.read_config();
    assert!(
        doc.contains("https://my.mirror/gh/astral-sh/python-build-standalone/"),
        "{doc}"
    );
    assert!(doc.contains("https://my.mirror/gh/astral-sh/uv"), "{doc}");
}

#[test]
fn test_set_mirror_github_off_clears_both_urls() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.github", "nju"]);
    sandbox.run(&["config", "mirror.npm", "npmmirror"]);
    let result = sandbox.run(&["config", "mirror.github", "off"]);
    assert_eq!(result.code, 0);
    // (m.python_install, m.uv_binary) == ("", "") -> the github axis reads "off".
    assert_eq!(read_key(&sandbox, "mirror.github"), "off");
    assert_eq!(read_key(&sandbox, "mirror"), "on"); // the npm axis is still on
    assert_eq!(read_key(&sandbox, "mirror.npm"), "npmmirror"); // m.npm == NPM_REGISTRY_MIRROR
    // The RAW doc must hold "" too: load_mirror() blanks a non-https uv_binary on read, so a write
    // that dropped garbage into the file would be invisible above — check the disk.
    let doc = sandbox.read_config();
    assert!(doc.contains("python_install = \"\""), "{doc}");
    assert!(doc.contains("uv_binary = \"\""), "{doc}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): a URL-storing write under pause prints no \"switched off\" stderr notice (cli.py:5335-5345 _finish_axis_write) — the Rust set path emits only the confirmation."]
fn test_paused_github_write_prints_notice_and_clear_does_not() {
    // F2 for the github axis: a URL-storing write under pause emits the stderr notice and stays
    // paused; the clearing write emits none and leaves the other paused axes alone.
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    sandbox.run(&["config", "mirror", "off"]); // pause
    let result = sandbox.run(&["config", "mirror.github", "nju"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "mirror"), "off"); // still paused
    let doc = sandbox.read_config();
    assert!(doc.contains(PYTHON_INSTALL_MIRROR), "{doc}"); // the write landed
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "tsinghua"); // the paused axis survived
    assert!(
        result
            .stderr
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("switched off")
    );
    let clear = sandbox.run(&["config", "mirror.github", "off"]);
    assert_eq!(clear.code, 0);
    assert!(!clear.both().contains("switched off")); // a clear is not a URL write
    assert_eq!(read_key(&sandbox, "mirror.github"), "off");
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "tsinghua");
}

#[test]
fn test_set_mirror_github_rejects_http_base() {
    // The base derives the uv-binary URL (downloaded and executed) -> https only.
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror.github", "http://evil/gh"]);
    assert_eq!(result.code, 2);
    assert_eq!(read_key(&sandbox, "mirror.github"), "off"); // uv_binary == ""
}

#[test]
fn test_set_mirror_axis_custom_url() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror.pypi", "https://my.index/simple"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "https://my.index/simple");
}

#[test]
fn test_set_mirror_axis_off_keeps_the_others() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    sandbox.run(&["config", "mirror.npm", "npmmirror"]);
    let result = sandbox.run(&["config", "mirror.pypi", "off"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "mirror"), "on"); // m.enabled
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "off"); // m.pypi == ""
    assert_eq!(read_key(&sandbox, "mirror.npm"), "npmmirror"); // m.npm == NPM_REGISTRY_MIRROR
}

#[test]
fn test_set_last_axis_off_disables() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.npm", "npmmirror"]);
    sandbox.run(&["config", "mirror.npm", "off"]);
    assert_eq!(read_key(&sandbox, "mirror"), "off"); // not enabled
}

#[test]
fn test_unknown_axis_value_exits_2() {
    // A typo (or another axis's vendor name) must never be saved as a bogus URL.
    for key in ["mirror.pypi", "mirror.npm"] {
        let sandbox = Sandbox::new();
        let result = sandbox.run(&["config", key, "tsnighua"]);
        assert_eq!(result.code, 2, "{key}");
        assert_eq!(read_key(&sandbox, "mirror"), "off"); // not enabled
    }
}

#[test]
fn test_npm_axis_rejects_pypi_vendor_name() {
    // The old single-axis grammar's core lie, now a hard error: PyPI vendors aren't npm vendors.
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror.npm", "tsinghua"]);
    assert_eq!(result.code, 2);
}

#[test]
fn test_mirror_master_off_preserves_urls_and_on_restores() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    let off = sandbox.run(&["config", "mirror", "off"]);
    assert_eq!(off.code, 0);
    assert_eq!(read_key(&sandbox, "mirror"), "off"); // not enabled
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "tsinghua"); // kept for the return trip
    let on = sandbox.run(&["config", "mirror", "on"]);
    assert_eq!(on.code, 0);
    assert_eq!(read_key(&sandbox, "mirror"), "on"); // enabled
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the 'mirror on' with nothing saved error does not point at the mirror.pypi axis keys (cli.py:5368-5372) — the Rust message reads 'no mirror URLs are stored; set one mirror axis first'."]
fn test_mirror_master_on_with_nothing_saved_exits_2() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror", "on"]);
    assert_eq!(result.code, 2);
    assert!(result.both().contains("mirror.pypi")); // points at the axis keys
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the master-level vendor-name rejection does not point at mirror.pypi (cli.py:5354-5364 _set_mirror_master) — the Rust message reads 'invalid configuration value for mirror: tsinghua'."]
fn test_mirror_master_rejects_vendor_names_with_axis_pointer() {
    // `skit config mirror tsinghua` (the old grammar) must fail loudly and point at mirror.pypi —
    // a vendor name at the master level would have to guess the other axes.
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror", "tsinghua"]);
    assert_eq!(result.code, 2);
    assert!(result.both().contains("mirror.pypi"));
    assert_eq!(read_key(&sandbox, "mirror"), "off"); // not enabled
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): a paused axis write prints no 'switched off' stderr notice and its stdout confirmation has no space-padded '= ' (cli.py:5335-5345, 5323 'Set: k=v') — state stays correct but the notice/format diverge."]
fn test_paused_axis_write_preserves_other_axes_and_stays_paused() {
    // F2: after `mirror off` the config is PAUSED (URLs kept, master off). Writing another axis
    // must preserve the paused axes' URLs, keep the master off, and print the re-enable notice —
    // never silently destroy the paused URL nor resurrect every axis behind the user.
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    sandbox.run(&["config", "mirror", "off"]);
    let result = sandbox.run(&["config", "mirror.npm", "npmmirror"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "mirror"), "off"); // stays paused, not silently re-enabled
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "tsinghua"); // the paused axis survived
    assert_eq!(read_key(&sandbox, "mirror.npm"), "npmmirror"); // the asked-for axis landed
    // R2-2: the notice is a skit-side signal and goes to STDERR — an agent piping stdout must see
    // only the confirmation line, never a mixed-in warning.
    let stderr = result
        .stderr
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(stderr.contains("switched off"));
    let stdout = result
        .stdout
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!stdout.contains("switched off"));
    assert!(stdout.contains("mirror.npm = npmmirror")); // stdout carries only the confirmation
}

#[test]
fn test_paused_axis_clear_leaves_other_axes_and_prints_no_notice() {
    // F2 (cont.): under a paused config, clearing one axis (`mirror.npm off`) leaves the other
    // paused axes' URLs intact — and a clear writes no URL, so no re-enable notice.
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    sandbox.run(&["config", "mirror.npm", "npmmirror"]);
    sandbox.run(&["config", "mirror", "off"]); // pause both axes
    let result = sandbox.run(&["config", "mirror.npm", "off"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "mirror"), "off"); // not enabled
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "tsinghua"); // untouched
    assert_eq!(read_key(&sandbox, "mirror.npm"), "off"); // cleared
    assert!(!result.both().contains("switched off"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the human list is 'key=value', not the padded 'key<pad>value' columns the oracle prints (cli.py:5509 f'  {k:<16}{value}') — the space-separated flatten assertions fail against 'mirror=off'."]
fn test_paused_config_is_fully_visible_in_config_list() {
    // F4: a paused config stays legible key-by-key in `skit config` — the master reads off (paused)
    // while each axis key still shows its stored token, so nothing is silently hidden.
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "aliyun"]);
    sandbox.run(&["config", "mirror.npm", "npmmirror"]);
    sandbox.run(&["config", "mirror", "off"]); // pause
    let result = sandbox.run(&["config"]);
    assert_eq!(result.code, 0);
    let flat = result
        .stdout
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(flat.contains("mirror off")); // the master reads off (paused)
    assert!(flat.contains("mirror.pypi aliyun")); // the stored axes are still spelled out, one per key
    assert!(flat.contains("mirror.github off"));
    assert!(flat.contains("mirror.npm npmmirror"));
}

#[test]
fn test_read_mirror_axis_shows_custom_url() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.npm", "https://my.registry"]);
    let result = sandbox.run(&["config", "mirror.npm"]);
    // Exact-equality (not substring): the read echoes the stored URL verbatim and nothing else.
    assert_eq!(result.stdout.trim(), "https://my.registry");
}

#[test]
fn test_mirror_github_read_value_round_trips() {
    // F3: the value `config mirror.github` prints (a preset name or a base URL) writes straight
    // back at exit 0 with no state change — read/write vocabulary is symmetric.
    for value in ["nju", "https://my.mirror/gh"] {
        let sandbox = Sandbox::new();
        sandbox.run(&["config", "mirror.github", value]);
        let shown = read_key(&sandbox, "mirror.github");
        let before = sandbox.read_config();
        let result = sandbox.run(&["config", "mirror.github", &shown]);
        assert_eq!(result.code, 0, "{value}/{shown}: {}", result.both());
        assert_eq!(sandbox.read_config(), before);
    }
}

#[test]
fn test_mirror_github_rejects_display_strings() {
    // F3: a display string (embedded space, or the " · "/" + " summary separators) must fail loudly
    // at exit 2 and never land on disk as a garbage URL.
    for bad in [
        "https://a b/gh",
        "https://a\u{b7}b/gh",
        "https://x/py/ + https://x/uv",
    ] {
        let sandbox = Sandbox::new();
        sandbox.run(&["config", "mirror.github", "nju"]);
        let before = sandbox.read_config();
        let result = sandbox.run(&["config", "mirror.github", bad]);
        assert_eq!(result.code, 2, "{bad}");
        assert_eq!(sandbox.read_config(), before); // disk untouched
    }
}

#[test]
fn test_mirror_axis_rejects_whitespace_url() {
    // The single-URL axes (pypi/npm) reject a whitespace-bearing token the same way: it is neither
    // a preset nor a pastable URL, so exit 2 and nothing persisted.
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "mirror.pypi", "https://a b/simple"]);
    assert_eq!(result.code, 2);
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "off"); // pypi == ""
}

// --- config KEY --json (single-key raw machine tokens, item #11) ---

#[test]
fn test_config_json_single_key_is_raw_master_token() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    assert_eq!(
        sandbox.run(&["config", "mirror", "--json"]).stdout.trim(),
        r#"{"mirror":"on"}"#
    );
}

#[test]
fn test_config_json_single_key_lang_is_raw_override_tag() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "lang", "zh-CN"]);
    // the raw override tag, not "auto (…)" prose
    assert_eq!(
        sandbox.run(&["config", "lang", "--json"]).stdout.trim(),
        r#"{"lang":"zh-CN"}"#
    );
}

#[test]
fn test_config_json_lang_unset_is_empty_string() {
    let sandbox = Sandbox::new();
    // unset/auto reads as "", never a localized fallback string
    assert_eq!(
        sandbox.run(&["config", "lang", "--json"]).stdout.trim(),
        r#"{"lang":""}"#
    );
}

#[test]
fn test_lang_override_non_string_reads_as_empty() {
    // _config_raw_value("lang") must never leak a non-str into the JSON contract, even if
    // config.toml was hand-corrupted to a non-string language. (Oracle asserts this on the private
    // cli._lang_override(); the raw "lang" json value is exactly that helper's output.)
    let sandbox = Sandbox::new();
    sandbox.write_config("language = 123\n");
    assert_eq!(
        sandbox.run(&["config", "lang", "--json"]).stdout.trim(),
        r#"{"lang":""}"#
    );
}

#[test]
fn test_config_json_mirror_github_raw_token() {
    for (setup, expected) in [("nju", "nju"), ("https://my/gh", "https://my/gh")] {
        let sandbox = Sandbox::new();
        sandbox.run(&["config", "mirror.github", setup]);
        assert_eq!(
            parse_flat_json(&sandbox.run(&["config", "mirror.github", "--json"]).stdout),
            BTreeMap::from([("mirror.github".to_owned(), expected.to_owned())])
        );
    }
}

#[test]
fn test_config_json_mirror_github_underivable_pair_is_literal_custom() {
    // Item #11: a hand-edited github pair no base derives reads back as the literal token "custom"
    // — a value that fails loudly if written back, never display prose saved as a URL.
    let sandbox = Sandbox::new();
    sandbox.write_config(
        "[mirror]\nenabled = true\npython_install = \"https://my/py/\"\nuv_binary = \"https://my/uv\"\n",
    );
    assert_eq!(
        parse_flat_json(&sandbox.run(&["config", "mirror.github", "--json"]).stdout),
        BTreeMap::from([("mirror.github".to_owned(), "custom".to_owned())])
    );
}

// --- editor ---

#[test]
fn test_set_editor() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "editor", "code --wait"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "editor"), "code --wait");
    assert!(result.stdout.contains("code --wait")); // confirmation echoes the new value
}

#[test]
fn test_clear_editor_with_empty_value() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "editor", "nano"]);
    let result = sandbox.run(&["config", "editor", ""]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "editor"), "");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the human editor read shows no '$VISUAL / $EDITOR' default sentinel (cli.py:5303-5306) — the Rust read prints the raw \"\"."]
fn test_read_editor_default_line() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "editor"]);
    assert_eq!(result.code, 0);
    assert!(result.both().contains("$VISUAL / $EDITOR"));
}

// --- form ---

#[test]
fn test_form_defaults_to_tui() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "form"]);
    assert_eq!(result.code, 0);
    assert!(result.stdout.contains("tui"));
}

#[test]
fn test_set_form_plain_and_back() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "form", "plain"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "form"), "plain");
    sandbox.run(&["config", "form", "tui"]);
    assert_eq!(read_key(&sandbox, "form"), "tui");
}

#[test]
fn test_unknown_form_style_exits_2() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "form", "fancy"]);
    assert_eq!(result.code, 2);
}

// --- after_run ---

#[test]
fn test_read_after_run_default() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "after_run"]);
    assert_eq!(result.code, 0);
    assert!(result.stdout.contains("exit")); // the launcher default
}

#[test]
fn test_set_after_run_stay_and_back() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "after_run", "stay"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "after_run"), "stay");
    sandbox.run(&["config", "after_run", "exit"]);
    assert_eq!(read_key(&sandbox, "after_run"), "exit");
}

#[test]
fn test_unknown_after_run_exits_2() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "after_run", "loop"]);
    assert_eq!(result.code, 2);
}

#[test]
fn test_after_run_garbage_in_config_file_normalizes_to_exit() {
    let sandbox = Sandbox::new();
    sandbox.write_config("after_run = \"never\"\n");
    assert_eq!(read_key(&sandbox, "after_run"), "exit");
}

// --- writes preserve the other sections ---

#[test]
fn test_mirror_write_preserves_language() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "lang", "zh-CN"]);
    let result = sandbox.run(&["config", "mirror", "off"]);
    assert_eq!(result.code, 0);
    assert_eq!(
        sandbox.run(&["config", "lang", "--json"]).stdout.trim(),
        r#"{"lang":"zh-CN"}"#
    );
}

#[test]
fn test_lang_clear_preserves_mirror() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    let result = sandbox.run(&["config", "lang", "auto"]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "mirror.pypi"), "tsinghua"); // m.pypi == PYPI_PRESETS["tsinghua"]
}

#[test]
fn test_form_write_preserves_mirror_and_language() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "lang", "zh-CN"]);
    sandbox.run(&["config", "mirror.pypi", "tsinghua"]);
    sandbox.run(&["config", "form", "plain"]);
    assert_eq!(
        sandbox.run(&["config", "lang", "--json"]).stdout.trim(),
        r#"{"lang":"zh-CN"}"#
    );
    assert_eq!(read_key(&sandbox, "mirror"), "on"); // load_mirror().enabled
    assert_eq!(read_key(&sandbox, "form"), "plain");
}

// --- shell.bash_path ---

#[test]
#[ignore = "FAILING CONTRACT (divergence): the human shell.bash_path read shows no 'auto' placeholder (cli.py:5318-5321) — the Rust read prints the raw \"\"."]
fn test_read_bash_path_default_line() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "shell.bash_path"]);
    assert_eq!(result.code, 0);
    assert!(result.both().contains("auto")); // the unset placeholder
}

#[test]
fn test_set_bash_path_to_existing_file() {
    let sandbox = Sandbox::new();
    let bash = sandbox.data.path().join("bash");
    fs::write(&bash, "").unwrap();
    let path = bash.to_str().unwrap();
    let result = sandbox.run(&["config", "shell.bash_path", path]);
    assert_eq!(result.code, 0, "{}", result.both());
    assert_eq!(read_key(&sandbox, "shell.bash_path"), path);
    // Confirmation echoes the path (rich soft-wraps it upstream; the flatten mirrors that).
    assert!(result.stdout.replace('\n', "").contains(path));
}

#[test]
fn test_set_bash_path_to_missing_file_is_usage_error() {
    let sandbox = Sandbox::new();
    let ghost = sandbox.data.path().join("nope"); // never created
    let result = sandbox.run(&["config", "shell.bash_path", ghost.to_str().unwrap()]);
    assert_eq!(result.code, 2);
    assert_eq!(read_key(&sandbox, "shell.bash_path"), ""); // nothing written on the rejection
}

#[test]
fn test_clear_bash_path_with_empty_value() {
    let sandbox = Sandbox::new();
    let bash = sandbox.data.path().join("bash");
    fs::write(&bash, "").unwrap();
    sandbox.run(&["config", "shell.bash_path", bash.to_str().unwrap()]);
    let result = sandbox.run(&["config", "shell.bash_path", ""]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "shell.bash_path"), ""); // empty clears, no existence check
}

#[test]
fn test_bare_config_lists_dotted_keys() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config"]);
    assert_eq!(result.code, 0);
    assert!(result.stdout.contains("shell.bash_path"));
    assert!(result.stdout.contains("js.runner"));
}

// --- js.runner ---

#[test]
#[ignore = "FAILING CONTRACT (divergence): the human js.runner read shows no 'auto' placeholder (cli.py:5322-5325) — the Rust read prints the raw \"\"."]
fn test_read_js_runner_default_line() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "js.runner"]);
    assert_eq!(result.code, 0);
    assert!(result.both().contains("auto"));
}

#[test]
fn test_set_js_runner() {
    for name in JS_RUNNERS {
        let sandbox = Sandbox::new();
        let result = sandbox.run(&["config", "js.runner", name]);
        assert_eq!(result.code, 0, "{name}: {}", result.both());
        assert_eq!(read_key(&sandbox, "js.runner"), *name);
        assert!(result.stdout.contains(name));
    }
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the unknown js.runner error does not name the real choices (cli.py:5464-5468 names 'deno, bun, node') — the Rust message reads 'invalid configuration value for js.runner: carrier-pigeon'."]
fn test_set_js_runner_unknown_is_usage_error() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["config", "js.runner", "carrier-pigeon"]);
    assert_eq!(result.code, 2);
    // the error names the real choices so the user can fix it in one step
    assert!(result.both().contains("deno"));
    assert_eq!(read_key(&sandbox, "js.runner"), "");
}

#[test]
fn test_clear_js_runner_with_empty_value() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "js.runner", "bun"]);
    let result = sandbox.run(&["config", "js.runner", ""]);
    assert_eq!(result.code, 0);
    assert_eq!(read_key(&sandbox, "js.runner"), "");
}

// --- the mirror wizard (first-run only now): one question per ecosystem axis ---
//
// CROSS-CRATE: every wizard test drives the private `cli._mirror_wizard()` and stubs `Prompt.ask`
// to CAPTURE each call's `default`/`choices`. The Rust equivalent is the non-`pub`
// `mirror_wizard(store, &dyn FirstRunPrompt)` in `crates/skit-cli/src/cli.rs`, whose per-axis
// `default`/`choices` live inside the `FirstRunPrompt::axis_answer` call. A black-box binary
// integration test can neither call that private fn nor observe the prompt metadata; it is
// exercised white-box inside the crate.

#[test]
#[ignore = "CROSS-CRATE: private interactive helper mirror_wizard(store, &dyn FirstRunPrompt) in crates/skit-cli/src/cli.rs; the captured choices [*PYPI_PRESETS,'custom','off'] are not observable black-box (cli.py:5573-5601, 5643-5653)."]
fn test_mirror_wizard_asks_one_question_per_axis() {
    // Oracle: three axis questions; choices are [*PYPI_PRESETS,'custom','off'],
    // [*GITHUB_RELEASE_PRESETS,'custom','off'], [*NPM_PRESETS,'custom','off']; answers
    // ustc/nju/npmmirror configure each axis independently.
}

#[test]
#[ignore = "CROSS-CRATE: private interactive helper mirror_wizard; the per-axis Prompt default (tsinghua/nju/npmmirror) is not observable black-box (cli.py:5656-5665)."]
fn test_mirror_wizard_defaults_are_the_recommended_presets() {
    // Oracle: each axis defaults to its own recommended preset (tsinghua/nju/npmmirror); explicit
    // "off" x3 still leaves mirrors disabled.
}

#[test]
#[ignore = "CROSS-CRATE: private interactive helper mirror_wizard; not drivable from a black-box binary integration test (cli.py:5669-5676)."]
fn test_mirror_wizard_axes_answer_independently() {
    // Oracle: npm-only answers must not leak any vendor onto the pypi/github axes.
}

#[test]
#[ignore = "CROSS-CRATE: private interactive helper mirror_wizard; the captured default is not observable black-box (cli.py:5679-5686)."]
fn test_mirror_wizard_default_ignores_saved_choice() {
    // Oracle: the PyPI default is unconditionally 'tsinghua', never the previously saved 'aliyun'.
}

#[test]
#[ignore = "CROSS-CRATE: private interactive helper mirror_wizard; the captured default is not observable black-box (cli.py:5689-5697)."]
fn test_mirror_wizard_default_ignores_non_preset_saved_url() {
    // Oracle: a saved non-preset URL must not surface as a 'custom' default; the default stays 'tsinghua'.
}

#[test]
#[ignore = "CROSS-CRATE: private interactive helper mirror_wizard; the captured defaults and the custom-URL prompt are not observable black-box (cli.py:5700-5717)."]
fn test_mirror_wizard_custom() {
    // Oracle: 'custom' answers store the typed URLs; the github base derives both github-release
    // vectors; the choice default stays 'tsinghua' and _prompt_axis_url carries no preset default.
}

#[test]
#[ignore = "CROSS-CRATE: private interactive helper mirror_wizard; the http-base re-prompt loop is not drivable black-box (cli.py:5720-5742)."]
fn test_mirror_wizard_custom_rejects_non_https_github_base() {
    // Oracle: the github base derives an executed download, so an http:// base re-prompts until https://.
}

#[test]
#[ignore = "CROSS-CRATE: private interactive helper mirror_wizard; the custom-URL is_url_token re-prompt loop is not drivable black-box (cli.py:5745-5754)."]
fn test_mirror_wizard_custom_axis_bad_url_reprompts() {
    // Oracle: empty, a vendor-name typo, or display prose all loop until a real one-token URL arrives.
}

// --- first-run probe ---
//
// CROSS-CRATE: every first-run test drives the private `cli._maybe_first_run_setup()` while
// monkeypatching `cli._is_interactive`, `config.looks_blocked`, and `cli.Confirm.ask`. The Rust
// equivalent is the non-`pub` `first_run_mirror_offer(store, &dyn NetworkProbe, &dyn
// FirstRunPrompt, interactive)` in `crates/skit-cli/src/cli.rs`; its decision rules take injected
// ports, not observable from a black-box binary integration test.

#[test]
#[ignore = "CROSS-CRATE: private first_run_mirror_offer(store, &dyn NetworkProbe, &dyn FirstRunPrompt, interactive) in crates/skit-cli/src/cli.rs; the blocked-network + interactive + confirm path is injected, not black-box (cli.py:5760-5767)."]
fn test_first_run_offers_and_configures_when_blocked() {
    // Oracle: interactive + looks_blocked + confirm -> the wizard runs and marks the store configured.
}

#[test]
#[ignore = "CROSS-CRATE: private first_run_mirror_offer; the declined path still writes the marker (cli.py:5770-5776)."]
fn test_first_run_declined_still_marks_done() {
    // Oracle: a declined offer leaves mirrors off but still marks the store configured.
}

#[test]
#[ignore = "CROSS-CRATE: private first_run_mirror_offer; the not-blocked path marks the store done without asking (cli.py:5779-5784)."]
fn test_first_run_not_blocked_marks_done() {
    // Oracle: interactive + not blocked -> marked configured, mirrors off, no offer.
}

#[test]
#[ignore = "CROSS-CRATE: private first_run_mirror_offer; the non-interactive early return (no probe, no write) is not black-box observable (cli.py:5787-5793)."]
fn test_first_run_skipped_when_not_interactive() {
    // Oracle: non-interactive returns before probing and before writing anything.
}

#[test]
#[ignore = "CROSS-CRATE: private first_run_mirror_offer; the already-configured early return (no probe) is not black-box observable (cli.py:5796-5802)."]
fn test_first_run_skipped_when_already_configured() {
    // Oracle: an already-configured store skips the probe entirely.
}

#[test]
#[ignore = "CROSS-CRATE: private first_run_mirror_offer; a language-only config still triggers the mirror offer (mirror_configured, not is_configured) — not black-box observable (cli.py:5805-5818)."]
fn test_first_run_still_offered_after_language_only_config() {
    // Oracle: `config lang zh-CN` writes config.toml but no [mirror] section, so the offer still fires.
}
