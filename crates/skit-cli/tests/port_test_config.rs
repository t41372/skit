//! Mechanical port of the Python oracle module `tests/test_config.py`
//! (`origin/main@206f9ef`): "Config + mirror settings (config.py): persistence, per-axis
//! presets, env injection with the defer rule, plus editor / bash_path / js.runner round-trips
//! and corrupt-config recovery." Each `#[test]` keeps its Python `def test_*` name, and each
//! Python "WHY" comment is kept above it, so it traces back to its origin.
//!
//! WHY `skit-cli-rs`: the oracle's `config.py` is refactored across three Rust crates, and only
//! the composition-root crate depends on all three, so one integration file can drive them
//! together as libraries (no binary, so no `SKIT_*` env vars are needed):
//! - `skit-store` `FileConfigStore` — persistence, mirror state, env overlay, recovery.
//! - `skit-application::preferences` `PreferencesDraft` — the per-axis choice / base readers.
//! - `skit-runtime` `network_looks_blocked` — the first-run reachability probe.
//!
//! Concept mapping:
//! - `config.load_mirror()` -> `store.mirror()`; `config.MirrorConfig` -> `MirrorSettings`.
//! - `config.save_mirror(...)` / `config.update_mirror_axes(axis=...)` / `config.disable()` ->
//!   `store.set("mirror.<axis>", ...)` / `store.set("mirror", "off")`. The store's axis setters
//!   auto-enable, which IS `compose`'s "enabled iff any axis on" rule on this surface.
//! - `config.enable()` -> `store.set("mirror", "on")`: True -> `Ok`, False (nothing saved) ->
//!   `Err(Usage)` while the master stays off (same outcome, Rust `Result` convention).
//! - `config.mirror_env(base)` -> `store.mirror_environment(&base)`.
//! - `config.uv_binary_base()` -> the private run-lane composition
//!   `mirror.enabled.then_some(mirror.uv_binary)` (crates/skit-cli/src/run/command.rs:412-414);
//!   its public pieces are `store.mirror().enabled` + `.uv_binary`, reproduced by `uv_binary_base`.
//! - `config.is_configured()` -> `store.mirror_configured()`: both call sites here are
//!   mirror-context, and the two markers agree in both (no file / no [mirror] -> false; after a
//!   mirror save -> true).
//! - `config.save_config(raw_dict)` -> writing `config.toml` bytes directly.
//! - `config.load_config()` / `load_editor` / `load_bash_path` / `load_js_runner` ->
//!   `store.mirror()` / `store.settings()` / `store.get(key)`.
//! - `config.save_editor` / `save_bash_path` / `save_js_runner` -> `store.set(key, ...)`.
//! - `config.is_url_token(v)` -> a `PreferencesDraft` with PyPI set to Custom + `v` resolves
//!   (`preferences.rs:578-582`); http is allowed on the pypi axis, as in the oracle.
//! - `config.github_release_urls(base)` -> `store.set("mirror.github", base)` then `store.mirror()`.
//! - `config.pypi_choice` / `github_choice` / `npm_choice` -> `PreferencesDraft.{pypi,github,npm}`
//!   (`MirrorChoice::Off` / `Custom` / `Preset(name)`); `github_base` -> `PreferencesDraft.github_url`.
//! - `config.pypi_display` / `npm_display` -> `store.settings()["mirror.pypi" / "mirror.npm"]`;
//!   `config.github_display` -> the private `skit-cli::config_display_value`. Its exact contract
//!   runs as `cli::tests::test_axis_display_helpers_exact`, where the private owner is visible.
//! - `config.looks_blocked(timeout=0.01)` -> `network_looks_blocked(&probe)` with a scripted
//!   `NetworkProbe` (the fixed `REACHABILITY_TIMEOUT` replaces the timeout argument).
//!
//! Buckets:
//! - Bucket 1 (API EXISTS, asserting): 59 tests — 58 below and the private CLI unit named above.
//! - Bucket 2 (DIVERGENCE, full body kept, `#[ignore]`): two tests. The store `set` requires an
//!   EXISTING bash file where the oracle's low-level `save_bash_path` does not. The oracle's CLI
//!   layer validates identically (cli.py:5454-5463), so this is a module-layer difference, not a
//!   lost gate.
//! - Bucket 3 (CROSS-CRATE, `#[ignore]` stub): `axes_summary` and `mirrors_line` — the composed
//!   axes string + "Mirrors:" line are built in the private `CliHealthInspector::collect()`
//!   (cli.rs:6445) and rendered by skit-tui (management.rs:1236-1279); no public function on this
//!   dependency surface returns the composed string.
//!
//! Accounting: all 63 oracle tests are 59 REAL + 2 divergence + 2 architecture closure.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use tempfile::TempDir;
use toml::Value;

use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorChoice, MirrorConfiguration,
    PreferencesDraft, PreferencesSnapshot,
};
use skit_runtime::{NetworkProbe, REACHABILITY_HOSTS, network_looks_blocked};
use skit_store::{FileConfigStore, MirrorSettings};

// --- The oracle's constant tables (config.py:31-53, 75-76) as literal expected values. ---
const PYPI_TSINGHUA: &str = "https://pypi.tuna.tsinghua.edu.cn/simple";
const PYPI_ALIYUN: &str = "https://mirrors.aliyun.com/pypi/simple";
const PYPI_USTC: &str = "https://pypi.mirrors.ustc.edu.cn/simple";
const GITHUB_NJU_BASE: &str = "https://mirror.nju.edu.cn/github-release";
const PYTHON_INSTALL_MIRROR: &str =
    "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/";
const UV_BINARY_MIRROR: &str = "https://mirror.nju.edu.cn/github-release/astral-sh/uv";
const NPM_REGISTRY_MIRROR: &str = "https://registry.npmmirror.com";

/// A fresh store over an isolated config dir — the oracle's autouse `cfg_dir` fixture, which
/// points `SKIT_CONFIG_DIR` at a `tmp_path`. The `TempDir` must be kept alive by the caller.
fn fixture() -> (TempDir, FileConfigStore) {
    let dir = TempDir::new().expect("temp dir");
    let store = FileConfigStore::new(dir.path());
    (dir, store)
}

/// `config.save_config(raw_dict)` on this surface: write the `config.toml` bytes directly.
fn write_config(dir: &TempDir, toml: &str) {
    fs::write(dir.path().join("config.toml"), toml).expect("write config.toml");
}

/// Parse the on-disk `config.toml` back to a document — for the "preserves other keys" assertions.
fn read_config(dir: &TempDir) -> Value {
    let text = fs::read_to_string(dir.path().join("config.toml")).expect("read config.toml");
    toml::from_str(&text).expect("valid config.toml")
}

/// The oracle's `full_mirror()` fixture (conftest.py:119-130): all three axes on their presets,
/// master on. The store's axis setters auto-enable, so setting each preset reproduces it.
fn save_full_mirror(store: &FileConfigStore) {
    store
        .set_many(&BTreeMap::from([
            ("mirror.pypi".to_owned(), "tsinghua".to_owned()),
            ("mirror.github".to_owned(), "nju".to_owned()),
            ("mirror.npm".to_owned(), "npmmirror".to_owned()),
        ]))
        .expect("save full mirror");
}

/// `config.uv_binary_base()` (config.py:481-484): the uv-bootstrap base, or "" when the master
/// is off. This is the exact private run-lane rule (command.rs:412-414) over public fields.
fn uv_binary_base(mirror: &MirrorSettings) -> &str {
    if mirror.enabled {
        &mirror.uv_binary
    } else {
        ""
    }
}

/// `config.github_release_urls(base)` (config.py:56-59) — a fixture builder for the base tests.
fn github_release_urls(base: &str) -> (String, String) {
    let base = base.strip_suffix('/').unwrap_or(base);
    (
        format!("{base}/astral-sh/python-build-standalone/"),
        format!("{base}/astral-sh/uv"),
    )
}

/// `config.compose(...)` (config.py:292-307) as a `MirrorConfiguration` (enabled iff any axis on).
fn compose(pypi: &str, python_install: &str, uv_binary: &str, npm: &str) -> MirrorConfiguration {
    MirrorConfiguration {
        enabled: !(pypi.is_empty()
            && python_install.is_empty()
            && uv_binary.is_empty()
            && npm.is_empty()),
        pypi: pypi.to_owned(),
        python_install: python_install.to_owned(),
        uv_binary: uv_binary.to_owned(),
        npm: npm.to_owned(),
    }
}

/// Build the preferences workflow whose `.pypi` / `.github` / `.npm` / `.github_url` fields ARE
/// the oracle's per-axis choice / base readers.
fn draft_for(mirror: MirrorConfiguration) -> PreferencesDraft {
    PreferencesDraft::from_snapshot(PreferencesSnapshot {
        language: String::new(),
        available_languages: Vec::new(),
        effective_language: String::new(),
        editor: String::new(),
        editor_fallback: None,
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names: Vec::new(),
        mirror,
    })
}

/// `config.is_url_token(value)` (config.py:62-71): the shared custom-URL gate. On this surface a
/// draft with the PyPI axis set to Custom + `value` resolves iff the token is a pastable http(s)
/// URL (no whitespace, no "·"); the choice flips Off->Custom, so no unchanged-mirror shortcut fires.
fn is_url_token(value: &str) -> bool {
    let mut draft = draft_for(MirrorConfiguration::default());
    draft.pypi = MirrorChoice::Custom;
    draft.pypi_url = value.to_owned();
    draft.resolve(|_| true).is_ok()
}

// A scripted reachability probe: `config.py`'s tests monkeypatch `socket.create_connection`; the
// Rust seam is the `NetworkProbe` port, so a fake records which hosts were asked.
#[derive(Debug)]
struct ScriptedProbe {
    reachable: Vec<&'static str>,
    asked: RefCell<Vec<String>>,
}

impl ScriptedProbe {
    fn new(reachable: Vec<&'static str>) -> Self {
        Self {
            reachable,
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl NetworkProbe for ScriptedProbe {
    fn can_connect(&self, host: &str, _port: u16, _timeout: Duration) -> bool {
        self.asked.borrow_mut().push(host.to_owned());
        self.reachable.contains(&host)
    }
}

#[test]
fn test_defaults_when_no_config() {
    let (_dir, store) = fixture();
    // is_configured() / load_config() == {}: no file, no [mirror] section.
    assert!(!store.mirror_configured().unwrap());
    assert_eq!(store.mirror().unwrap(), MirrorSettings::default());
    assert!(
        store
            .mirror_environment(&BTreeMap::new())
            .unwrap()
            .is_empty()
    );
    assert_eq!(uv_binary_base(&store.mirror().unwrap()), "");
}

#[test]
fn test_full_mirror_saves_all_four_vectors() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    assert!(store.mirror_configured().unwrap());
    let m = store.mirror().unwrap();
    assert!(m.enabled);
    assert_eq!(m.pypi, PYPI_TSINGHUA);
    assert_eq!(m.python_install, PYTHON_INSTALL_MIRROR);
    assert_eq!(m.uv_binary, UV_BINARY_MIRROR);
    assert_eq!(m.npm, NPM_REGISTRY_MIRROR);
}

#[test]
fn test_compose_enables_iff_any_axis_on() {
    // compose() with nothing set -> master off.
    let (_d0, s0) = fixture();
    assert!(!s0.mirror().unwrap().enabled);
    // any ONE axis on -> master on. pypi / npm / github are the store's axis keys (github carries
    // both python_install and uv_binary), so this covers every underlying vector.
    for (key, value) in [
        ("mirror.pypi", "tsinghua"),
        ("mirror.npm", "npmmirror"),
        ("mirror.github", "nju"),
    ] {
        let (_d, s) = fixture();
        s.set(key, value).unwrap();
        assert!(s.mirror().unwrap().enabled, "{key}");
    }
}

#[test]
fn test_axes_are_independent() {
    // One axis's vendor choice must never drag another axis along: the PyPI providers are
    // not npm or github-release vendors, and each axis works alone.
    let (_d1, s1) = fixture();
    s1.set("mirror.pypi", "aliyun").unwrap();
    let m = s1.mirror().unwrap();
    assert_eq!(
        (
            m.python_install.as_str(),
            m.uv_binary.as_str(),
            m.npm.as_str()
        ),
        ("", "", "")
    );

    let (_d2, s2) = fixture();
    s2.set("mirror.npm", "npmmirror").unwrap();
    let m = s2.mirror().unwrap();
    assert_eq!(
        (
            m.pypi.as_str(),
            m.python_install.as_str(),
            m.uv_binary.as_str()
        ),
        ("", "", "")
    );

    // The github axis (here: python_install alone, hand-seeded) leaves pypi and npm off.
    let (_d3, s3) = fixture();
    let (python_install, _uv) = github_release_urls(GITHUB_NJU_BASE);
    write_config(
        &_d3,
        &format!("[mirror]\npython_install = \"{python_install}\"\n"),
    );
    let m = s3.mirror().unwrap();
    assert_eq!((m.pypi.as_str(), m.npm.as_str()), ("", ""));
}

#[test]
fn test_is_url_token_accepts_pastable_http_urls() {
    // The shared custom-URL gate (CLI axis keys, TUI inputs, wizard prompts all route here).
    assert!(is_url_token("https://pypi.tuna.tsinghua.edu.cn/simple"));
    assert!(is_url_token("http://corp.internal/simple")); // http allowed (pypi/npm)
}

#[test]
fn test_is_url_token_rejects_non_urls() {
    for bad in [
        "",                             // empty
        "tsinghua",                     // a vendor name, not a URL
        "ftp://x",                      // non-http(s) scheme
        "https://a b/x",                // embedded space (display prose)
        "https://a\tb",                 // any whitespace, not just spaces
        "https://a\nb",                 //
        "https://a\u{b7}b",             // the axes_summary display separator "·"
        "pypi=tsinghua \u{b7} npm=off", // a round-tripped display string
    ] {
        assert!(!is_url_token(bad), "{bad:?}");
    }
}

#[test]
fn test_github_release_urls_expand_from_one_base() {
    let (_dir, store) = fixture();
    store.set("mirror.github", "https://my.mirror/gh/").unwrap();
    let m = store.mirror().unwrap();
    assert_eq!(
        m.python_install,
        "https://my.mirror/gh/astral-sh/python-build-standalone/"
    );
    assert_eq!(m.uv_binary, "https://my.mirror/gh/astral-sh/uv");
}

#[test]
fn test_axis_choice_readers() {
    let full = draft_for(compose(
        PYPI_TSINGHUA,
        PYTHON_INSTALL_MIRROR,
        UV_BINARY_MIRROR,
        NPM_REGISTRY_MIRROR,
    ));
    assert_eq!(full.pypi, MirrorChoice::Preset("tsinghua".to_owned()));
    assert_eq!(full.github, MirrorChoice::Preset("nju".to_owned()));
    assert_eq!(full.npm, MirrorChoice::Preset("npmmirror".to_owned()));

    let custom = draft_for(compose(
        "https://my/simple",
        "https://my/py/",
        "",
        "https://my/npm",
    ));
    assert_eq!(custom.pypi, MirrorChoice::Custom);
    assert_eq!(custom.github, MirrorChoice::Custom); // half-set pair is custom, not a preset
    assert_eq!(custom.npm, MirrorChoice::Custom);

    let off = draft_for(MirrorConfiguration::default());
    assert_eq!(off.pypi, MirrorChoice::Off);
    assert_eq!(off.github, MirrorChoice::Off);
    assert_eq!(off.npm, MirrorChoice::Off);
}

#[test]
fn test_github_base_recovers_a_custom_derivable_base() {
    // github_base reverses a base-derived custom pair back to exactly its base (the value the
    // single-URL github input round-trips), and returns "" for a pair no base expands to.
    let base = "https://my.mirror/gh";
    let (python_install, uv_binary) = github_release_urls(base);
    let derivable = draft_for(compose("", &python_install, &uv_binary, ""));
    assert_eq!(derivable.github_url, base);
    // A hand-edited pair that no single base expands to is not derivable.
    let underivable = draft_for(compose("", "https://x/py/", "https://x/uv", ""));
    assert_eq!(underivable.github_url, "");
}

#[test]
fn test_axis_choice_readers_are_blind_to_the_master_switch() {
    // Three-state storage (on / paused / empty): a paused config keeps its URLs on disk and
    // the readers must still REPORT them. Visibility is the readers' job; whether an axis is
    // APPLIED is the master's (mirror_env / mirrors_line fold that in — never these readers).
    let mut paused = compose(
        PYPI_USTC,
        PYTHON_INSTALL_MIRROR,
        UV_BINARY_MIRROR,
        NPM_REGISTRY_MIRROR,
    );
    paused.enabled = false;
    let draft = draft_for(paused);
    assert_eq!(draft.pypi, MirrorChoice::Preset("ustc".to_owned()));
    assert_eq!(draft.github, MirrorChoice::Preset("nju".to_owned()));
    assert_eq!(draft.npm, MirrorChoice::Preset("npmmirror".to_owned()));
}

// MIGRATION LEDGER: `test_axis_display_helpers_exact` runs in `cli::tests`. That child module can
// call the private human-display owner without changing the store's raw `mirror.github` contract.

#[test]
#[ignore = "CROSS-CRATE (skit-cli private + skit-tui): axes_summary joins the per-axis display \
    tokens ('pypi={} · github={} · npm={}') and collapses all-off to 'off' inline in the private \
    CliHealthInspector::collect() (crates/skit-cli/src/cli.rs:6445) and the skit-tui management \
    screen (crates/skit-tui/src/screens/management.rs:1236-1279 already pins these exact strings); \
    no public function on this dependency surface returns the composed string. The per-axis tokens \
    are covered by cli::tests::test_axis_display_helpers_exact through config_display_value."]
fn test_axes_summary_exact_strings() {
    // Python: axes_summary(full_mirror()) == "pypi=tsinghua · github=nju · npm=npmmirror";
    //   axes_summary(MirrorConfig()) == "off";
    //   axes_summary(compose(npm=npmmirror)) == "pypi=off · github=off · npm=npmmirror".
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli private + skit-tui): mirrors_line prefixes 'Mirrors:' and folds \
    in the master switch to tell off / on / paused apart. The classification lives in the private \
    CliHealthInspector::collect() (crates/skit-cli/src/cli.rs:6447-6451) and the render + wording \
    live in crates/skit-tui/src/screens/management.rs:170-173 (and its test at :1236-1279); no \
    public function on this dependency surface returns the composed line."]
fn test_mirrors_line_three_states_exact() {
    // Python: mirrors_line(MirrorConfig()) == "Mirrors: off";
    //   mirrors_line(full_mirror()) == "Mirrors: pypi=tsinghua · github=nju · npm=npmmirror";
    //   paused -> "Mirrors: off (saved: pypi=tsinghua · github=nju · npm=npmmirror)".
}

#[test]
fn test_update_mirror_axes_fresh_url_auto_enables() {
    // Fresh (off, nothing saved): a first URL turns the master on — one-command setup.
    let (_dir, store) = fixture();
    store.set("mirror.pypi", "tsinghua").unwrap();
    let m = store.mirror().unwrap();
    assert!(m.enabled);
    assert_eq!(m.pypi, PYPI_TSINGHUA);
    assert!(store.mirror().unwrap().enabled); // and it persisted
}

#[test]
fn test_update_mirror_axes_off_on_empty_stays_off() {
    // Off applied to an empty config is a no-op that must NOT flip the master on.
    let (_dir, store) = fixture();
    store.set("mirror.pypi", "off").unwrap();
    assert!(!store.mirror().unwrap().enabled);
}

#[test]
fn test_update_mirror_axes_enabled_stays_on_while_a_url_remains() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    store.set("mirror.pypi", "off").unwrap(); // drop one of several
    let m = store.mirror().unwrap();
    assert!(m.enabled);
    assert_eq!(m.pypi, "");
    assert_eq!(m.npm, NPM_REGISTRY_MIRROR); // the others survive
}

#[test]
fn test_update_mirror_axes_clearing_the_last_url_disables() {
    let (_dir, store) = fixture();
    store.set("mirror.npm", "npmmirror").unwrap();
    store.set("mirror.npm", "off").unwrap();
    let m = store.mirror().unwrap();
    assert!(!m.enabled);
    assert_eq!(m.npm, "");
}

#[test]
fn test_update_mirror_axes_paused_stays_paused_and_preserves_others() {
    // Paused (off with URLs saved): a write keeps the master off — flipping it would
    // resurrect every other saved axis behind the user's back.
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    store.set("mirror", "off").unwrap(); // disable()
    store.set("mirror.npm", "https://new/npm").unwrap();
    let m = store.mirror().unwrap();
    assert!(!m.enabled); // still paused, not silently resurrected
    assert_eq!(m.npm, "https://new/npm"); // the asked-for change landed
    assert_eq!(m.pypi, PYPI_TSINGHUA); // untouched axis preserved
}

#[test]
fn test_update_mirror_axes_none_leaves_axes_untouched() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    store.set("mirror.npm", "https://new/npm").unwrap(); // only npm passed
    let m = store.mirror().unwrap();
    assert_eq!(m.pypi, PYPI_TSINGHUA);
    assert_eq!(m.python_install, PYTHON_INSTALL_MIRROR);
    assert_eq!(m.uv_binary, UV_BINARY_MIRROR);
}

// `test_enable_works_for_each_single_axis` (parametrized over the four vectors): any ONE saved URL
// is enough to re-enable — each guard operand stands alone (an or-chain), because each axis is
// independently meaningful. Split by vector, since the store bundles python_install + uv_binary.

#[test]
fn test_enable_works_for_each_single_axis_pypi() {
    let (_dir, store) = fixture();
    store.set("mirror.pypi", "https://x").unwrap();
    store.set("mirror", "off").unwrap();
    assert!(store.set("mirror", "on").is_ok());
    assert!(store.mirror().unwrap().enabled);
}

#[test]
fn test_enable_works_for_each_single_axis_python_install() {
    let (dir, store) = fixture();
    write_config(
        &dir,
        "[mirror]\nenabled = false\npython_install = \"https://x\"\n",
    );
    assert!(store.set("mirror", "on").is_ok());
    assert!(store.mirror().unwrap().enabled);
}

#[test]
fn test_enable_works_for_each_single_axis_uv_binary() {
    let (dir, store) = fixture();
    write_config(
        &dir,
        "[mirror]\nenabled = false\nuv_binary = \"https://x\"\n",
    );
    assert!(store.set("mirror", "on").is_ok());
    assert!(store.mirror().unwrap().enabled);
}

#[test]
fn test_enable_works_for_each_single_axis_npm() {
    let (_dir, store) = fixture();
    store.set("mirror.npm", "https://x").unwrap();
    store.set("mirror", "off").unwrap();
    assert!(store.set("mirror", "on").is_ok());
    assert!(store.mirror().unwrap().enabled);
}

#[test]
fn test_enable_restores_saved_urls() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    store.set("mirror", "off").unwrap();
    assert!(store.set("mirror", "on").is_ok());
    let m = store.mirror().unwrap();
    assert!(m.enabled);
    assert_eq!(m.pypi, PYPI_TSINGHUA);
}

#[test]
fn test_enable_refuses_when_nothing_saved() {
    // enable() -> False on this surface: the setter refuses with a usage error, master stays off.
    let (_dir, store) = fixture();
    assert!(store.set("mirror", "on").is_err());
    assert!(!store.mirror().unwrap().enabled);
}

#[test]
fn test_save_mirror_preserves_other_keys() {
    let (dir, store) = fixture();
    write_config(&dir, "language = \"zh-CN\"\n");
    store.set("mirror.pypi", "ustc").unwrap();
    let doc = read_config(&dir);
    assert_eq!(doc.get("language").and_then(Value::as_str), Some("zh-CN")); // not clobbered
    assert_eq!(
        doc.get("mirror")
            .and_then(|mirror| mirror.get("pypi"))
            .and_then(Value::as_str),
        Some(PYPI_USTC)
    );
}

#[test]
fn test_mirror_env_overlays_all_vectors() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    let env = store.mirror_environment(&BTreeMap::new()).unwrap();
    assert_eq!(env["UV_DEFAULT_INDEX"], PYPI_TSINGHUA);
    assert_eq!(env["UV_PYTHON_INSTALL_MIRROR"], PYTHON_INSTALL_MIRROR);
    assert_eq!(uv_binary_base(&store.mirror().unwrap()), UV_BINARY_MIRROR);
}

#[test]
fn test_mirror_env_defers_to_user_index() {
    // Parametrized over config._INDEX_ENV = ("UV_DEFAULT_INDEX", "UV_INDEX_URL").
    for index_var in ["UV_DEFAULT_INDEX", "UV_INDEX_URL"] {
        let (_dir, store) = fixture();
        save_full_mirror(&store);
        let base = BTreeMap::from([(index_var.to_owned(), "https://mine/simple".to_owned())]);
        let env = store.mirror_environment(&base).unwrap();
        assert!(!env.contains_key("UV_DEFAULT_INDEX"), "{index_var}"); // the user's index wins
        assert!(env.contains_key("UV_PYTHON_INSTALL_MIRROR"), "{index_var}"); // untouched vector still injected
    }
}

#[test]
fn test_mirror_env_defers_to_user_python_mirror() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    let base = BTreeMap::from([(
        "UV_PYTHON_INSTALL_MIRROR".to_owned(),
        "https://mine/py/".to_owned(),
    )]);
    let env = store.mirror_environment(&base).unwrap();
    assert!(!env.contains_key("UV_PYTHON_INSTALL_MIRROR"));
    assert!(env.contains_key("UV_DEFAULT_INDEX"));
}

#[test]
fn test_mirror_env_does_not_defer_on_extra_index_url() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    let base = BTreeMap::from([("UV_EXTRA_INDEX_URL".to_owned(), "https://x".to_owned())]);
    let env = store.mirror_environment(&base).unwrap();
    // UV_EXTRA_INDEX_URL is additive, so the blocked default index is still live -> skit must inject.
    assert_eq!(env["UV_DEFAULT_INDEX"], PYPI_TSINGHUA);
}

#[test]
fn test_mirror_env_does_not_defer_on_uv_index() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    let base = BTreeMap::from([("UV_INDEX".to_owned(), "https://x".to_owned())]);
    let env = store.mirror_environment(&base).unwrap();
    // UV_INDEX is additive too (F1: dropped from _INDEX_ENV), so injection must still happen.
    assert_eq!(env["UV_DEFAULT_INDEX"], PYPI_TSINGHUA);
}

#[test]
fn test_mirror_env_injects_when_index_env_blank() {
    // An empty-string user var means "unset": it must NOT suppress the mirror.
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    let base = BTreeMap::from([("UV_INDEX_URL".to_owned(), String::new())]);
    let env = store.mirror_environment(&base).unwrap();
    assert_eq!(env["UV_DEFAULT_INDEX"], PYPI_TSINGHUA);
}

#[test]
fn test_mirror_env_injects_when_python_mirror_blank() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    let base = BTreeMap::from([("UV_PYTHON_INSTALL_MIRROR".to_owned(), String::new())]);
    let env = store.mirror_environment(&base).unwrap();
    assert_eq!(env["UV_PYTHON_INSTALL_MIRROR"], PYTHON_INSTALL_MIRROR);
}

#[test]
fn test_disable_keeps_urls_but_turns_off() {
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    store.set("mirror", "off").unwrap();
    let m = store.mirror().unwrap();
    assert!(!m.enabled);
    assert_eq!(m.pypi, PYPI_TSINGHUA); // URL retained for easy re-enable
    assert!(
        store
            .mirror_environment(&BTreeMap::new())
            .unwrap()
            .is_empty()
    );
    assert_eq!(uv_binary_base(&m), "");
}

#[test]
fn test_mirror_env_skips_empty_urls() {
    // enabled but with blank URLs (e.g. hand-edited config): nothing to inject
    let (dir, store) = fixture();
    write_config(&dir, "[mirror]\nenabled = true\n");
    assert!(
        store
            .mirror_environment(&BTreeMap::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_load_mirror_ignores_malformed_section() {
    let (dir, store) = fixture();
    write_config(&dir, "mirror = \"not-a-table\"\n");
    assert_eq!(store.mirror().unwrap(), MirrorSettings::default());
}

#[test]
fn test_load_mirror_rejects_string_enabled() {
    // A hand-edited `enabled = "false"` is a truthy string; it must NOT enable the mirror.
    let (dir, store) = fixture();
    write_config(
        &dir,
        "[mirror]\nenabled = \"false\"\npypi = \"https://x/simple\"\n",
    );
    assert!(!store.mirror().unwrap().enabled);
}

#[test]
fn test_load_mirror_ignores_non_str_url() {
    // A non-string URL (e.g. `pypi = 123`) must become blank, never the coerced string "123".
    let (dir, store) = fixture();
    write_config(&dir, "[mirror]\nenabled = true\npypi = 123\n");
    let m = store.mirror().unwrap();
    assert!(m.enabled);
    assert_eq!(m.pypi, "");
}

#[test]
fn test_load_mirror_blanks_non_https_uv_binary() {
    // A hand-edited http:// uv_binary must be blanked so the download falls back to the GitHub
    // default rather than fetching an executable over plain http.
    let (dir, store) = fixture();
    write_config(
        &dir,
        "[mirror]\nenabled = true\nuv_binary = \"http://evil/uv\"\n",
    );
    assert_eq!(store.mirror().unwrap().uv_binary, "");
    assert_eq!(uv_binary_base(&store.mirror().unwrap()), ""); // -> uses the GitHub default
}

#[test]
fn test_load_mirror_preserves_https_uv_binary() {
    let (dir, store) = fixture();
    write_config(
        &dir,
        "[mirror]\nenabled = true\nuv_binary = \"https://ok/uv\"\n",
    );
    assert_eq!(store.mirror().unwrap().uv_binary, "https://ok/uv");
    assert_eq!(uv_binary_base(&store.mirror().unwrap()), "https://ok/uv");
}

#[test]
fn test_nju_preset_uv_binary_stays_https() {
    // Sanity: the NJU github-release preset is https and must survive the https enforcement.
    let (_dir, store) = fixture();
    save_full_mirror(&store);
    assert_eq!(store.mirror().unwrap().uv_binary, UV_BINARY_MIRROR);
    assert!(UV_BINARY_MIRROR.starts_with("https://"));
}

#[test]
fn test_load_config_tolerates_corrupt_toml() {
    let (dir, store) = fixture();
    write_config(&dir, "this is = = not [valid toml");
    // load_config() == {}: an unparseable file reads as the empty document.
    assert!(!store.mirror_configured().unwrap());
    assert_eq!(store.mirror().unwrap(), MirrorSettings::default());
}

#[test]
fn test_save_editor_backs_up_corrupt_config_instead_of_wiping_it() {
    let (dir, store) = fixture();
    let corrupt = "language = \"zh-CN\"\n[mirror]\nenabled = true\npypi = \"https://tsinghua\"\nthis is = = not valid toml";
    write_config(&dir, corrupt);
    let recovery = store.set_with_recovery("editor", "vim").unwrap();
    // The just-requested change still takes effect...
    assert_eq!(store.get("editor").unwrap(), "vim");
    // ...but the corrupt original is preserved verbatim in a backup rather than vanishing.
    let backup = dir.path().join("config.toml.bak");
    assert!(backup.is_file());
    assert_eq!(fs::read_to_string(&backup).unwrap(), corrupt);
    // ...and the frontend is told (the returned ConfigRecovery names both paths, replacing the
    // oracle's stderr notice).
    let recovery = recovery.expect("a corrupt file yields a recovery record");
    assert_eq!(recovery.path.file_name().unwrap(), "config.toml");
    assert_eq!(
        recovery
            .backup_path
            .as_deref()
            .and_then(|path| path.file_name())
            .unwrap(),
        "config.toml.bak"
    );
}

#[test]
fn test_save_mirror_backs_up_corrupt_config_instead_of_wiping_it() {
    let (dir, store) = fixture();
    let corrupt = "language = \"zh-CN\"\nthis is = = not valid toml";
    write_config(&dir, corrupt);
    store.set("mirror.pypi", "aliyun").unwrap();
    assert_eq!(store.mirror().unwrap().pypi, PYPI_ALIYUN);
    let backup = dir.path().join("config.toml.bak");
    assert!(backup.is_file());
    assert_eq!(fs::read_to_string(&backup).unwrap(), corrupt);
}

#[test]
fn test_save_editor_warns_when_corrupt_config_cannot_even_be_backed_up() {
    // Double failure (corrupt file + backup itself fails): the save must still not crash, and must
    // still land the change. A directory at the nested copy target makes the backup refuse.
    let (dir, store) = fixture();
    let backup = dir.path().join("config.toml.bak");
    let blocker = backup.join("config.toml");
    fs::create_dir_all(&blocker).unwrap();
    fs::write(blocker.join("owned"), "keep").unwrap();
    write_config(&dir, "this is = = not valid toml");
    store.set("editor", "vim").unwrap();
    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(fs::read_to_string(blocker.join("owned")).unwrap(), "keep");
}

#[test]
fn test_save_editor_still_preserves_other_keys_when_config_is_valid() {
    // Sanity: the fix must not regress the ordinary (non-corrupt) preserve-other-keys path.
    let (dir, store) = fixture();
    write_config(&dir, "language = \"zh-CN\"\n");
    store.set("editor", "code --wait").unwrap();
    let doc = read_config(&dir);
    assert_eq!(doc.get("language").and_then(Value::as_str), Some("zh-CN"));
    assert_eq!(
        doc.get("editor").and_then(Value::as_str),
        Some("code --wait")
    );
    assert!(!dir.path().join("config.toml.bak").exists());
}

#[test]
fn test_looks_blocked_true_when_unreachable() {
    let probe = ScriptedProbe::new(Vec::new());
    assert!(network_looks_blocked(&probe));
}

#[test]
fn test_looks_blocked_false_when_reachable() {
    let probe = ScriptedProbe::new(vec!["pypi.org", "github.com"]);
    assert!(!network_looks_blocked(&probe));
}

#[test]
fn test_looks_blocked_short_circuits_on_first_host() {
    // First host (pypi.org) is unreachable -> return immediately, never probe github.com.
    let probe = ScriptedProbe::new(Vec::new());
    assert!(network_looks_blocked(&probe));
    assert_eq!(probe.asked.borrow().as_slice(), ["pypi.org"]);
}

#[test]
fn test_looks_blocked_true_when_second_host_unreachable() {
    // first host reachable, github.com unreachable -> blocked, both probed in order.
    let probe = ScriptedProbe::new(vec!["pypi.org"]);
    assert!(network_looks_blocked(&probe));
    assert_eq!(probe.asked.borrow().as_slice(), REACHABILITY_HOSTS);
}

#[test]
fn test_bash_path_defaults_to_empty() {
    let (_dir, store) = fixture();
    assert_eq!(store.get("shell.bash_path").unwrap(), "");
}

#[test]
fn test_bash_path_round_trip() {
    let (dir, store) = fixture();
    let bash = dir.path().join("bash");
    fs::write(&bash, "").unwrap();
    let path = bash.to_str().unwrap();
    store.set("shell.bash_path", path).unwrap();
    assert_eq!(store.get("shell.bash_path").unwrap(), path);
}

#[test]
fn test_bash_path_strips_and_clears() {
    let (dir, store) = fixture();
    store.set("shell.bash_path", "  /opt/bash  ").unwrap();
    assert_eq!(store.get("shell.bash_path").unwrap(), "/opt/bash"); // stripped on save
    store.set("shell.bash_path", "").unwrap();
    assert_eq!(store.get("shell.bash_path").unwrap(), ""); // empty clears the key
    assert!(read_config(&dir).get("shell").is_none()); // and drops the now-empty section
}

#[test]
fn test_bash_path_garbage_normalizes_to_empty() {
    let (dir, store) = fixture();
    write_config(&dir, "[shell]\nbash_path = 123\n"); // not a string
    assert_eq!(store.get("shell.bash_path").unwrap(), "");
}

#[test]
fn test_bash_path_garbage_section_normalizes_to_empty() {
    let (dir, store) = fixture();
    write_config(&dir, "shell = \"not-a-table\"\n"); // section isn't a dict
    assert_eq!(store.get("shell.bash_path").unwrap(), "");
}

#[test]
fn test_bash_path_save_preserves_other_keys() {
    let (dir, store) = fixture();
    write_config(&dir, "language = \"zh-CN\"\n");
    store.set("shell.bash_path", "/opt/bash").unwrap();
    let doc = read_config(&dir);
    assert_eq!(doc.get("language").and_then(Value::as_str), Some("zh-CN")); // untouched
    assert_eq!(
        doc.get("shell")
            .and_then(|shell| shell.get("bash_path"))
            .and_then(Value::as_str),
        Some("/opt/bash")
    );
}

#[test]
fn test_bash_path_clear_preserves_other_shell_keys() {
    let (dir, store) = fixture();
    write_config(&dir, "[shell]\nbash_path = \"/x\"\nother = \"keep\"\n");
    store.set("shell.bash_path", "").unwrap();
    let doc = read_config(&dir);
    let shell = doc
        .get("shell")
        .and_then(Value::as_table)
        .expect("shell table kept");
    assert_eq!(shell.get("other").and_then(Value::as_str), Some("keep")); // section stays
    assert!(shell.get("bash_path").is_none()); // only bash_path removed
}

#[test]
fn test_js_runner_defaults_to_empty() {
    let (_dir, store) = fixture();
    assert_eq!(store.get("js.runner").unwrap(), "");
}

#[test]
fn test_js_runner_round_trip() {
    // Parametrized over config.JS_RUNNERS = ("deno", "bun", "node").
    for name in ["deno", "bun", "node"] {
        let (_dir, store) = fixture();
        store.set("js.runner", name).unwrap();
        assert_eq!(store.get("js.runner").unwrap(), name);
    }
}

#[test]
fn test_js_runner_unknown_value_normalizes_to_empty() {
    let (dir, store) = fixture();
    write_config(&dir, "[js]\nrunner = \"carrier-pigeon\"\n");
    assert_eq!(store.get("js.runner").unwrap(), ""); // a hand-edited bad value must not poison runs
}

#[test]
fn test_js_runner_garbage_section_normalizes_to_empty() {
    let (dir, store) = fixture();
    write_config(&dir, "js = [\"not\", \"a\", \"table\"]\n");
    assert_eq!(store.get("js.runner").unwrap(), "");
}

#[test]
fn test_js_runner_clears_and_drops_section() {
    let (dir, store) = fixture();
    store.set("js.runner", "deno").unwrap();
    store.set("js.runner", "").unwrap();
    assert_eq!(store.get("js.runner").unwrap(), "");
    assert!(read_config(&dir).get("js").is_none());
}

#[test]
fn test_js_runner_save_preserves_other_keys() {
    let (dir, store) = fixture();
    write_config(&dir, "language = \"en\"\n");
    store.set("js.runner", "bun").unwrap();
    let doc = read_config(&dir);
    assert_eq!(doc.get("language").and_then(Value::as_str), Some("en"));
    assert_eq!(
        doc.get("js")
            .and_then(|js| js.get("runner"))
            .and_then(Value::as_str),
        Some("bun")
    );
}
