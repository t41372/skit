//! Mechanical port of the Python oracle module `tests/test_presets.py`
//! (`origin/main@206f9ef`): "Preset save/resolution order + C3 structural secret
//! stripping. No secret value may ever land on disk." Each `#[test]` keeps its Python
//! `def test_*` name so it traces back to its origin, and every Python "WHY" comment is
//! preserved above it.
//!
//! Concept mapping used throughout (the Rust surface folds the oracle's two-layer
//! `argstate` + `flows` split into one typed service, so calls carry declarations):
//! - Python `argstate.save_preset/save_last/delete_preset/load_state/purge_secret` (free
//!   functions over the disk state file) -> `FormStateService<FileFormStateStore>`
//!   methods. `FileFormStateStore` writes the real TOML at `<state_dir>/values/<slug>.toml`,
//!   which is what the byte-scan assertions read.
//! - Python's per-call `secret_names={...}` -> the `secret` flag on each `ParamDecl`. The
//!   Rust service derives the secret set from the declarations, so a param that "became
//!   secret" is expressed by passing a secret declaration on the later call.
//! - Python's `flows.remembered_values` (default/empty/secret filtering) is folded INTO
//!   `FormStateService::save_last`. Where the oracle stored raw values, the Rust call gets
//!   the same result because the ported specs use no default (so nothing is filtered), and
//!   `save_last` still strips secrets. The one place a default matters — the resolution
//!   test — only ever stores a non-default value, so the fold changes no asserted outcome.
//! - Python `flows.prefill(plan, slug, preset)` (reads state, then default < last-used <
//!   preset) -> `service.load()` + the pure `form_state::prefill(&decls, &state.values,
//!   state.presets.get(name))`. Same precedence, restructured API.
//! - Python `paths.values_dir()` -> `<TempDir>/values`, scanned for `*.toml`.
//!
//! Buckets:
//! - Bucket 1 (state save/resolution + on-disk secret stripping): every test below. Each is
//!   a real asserting `#[test]` driving the reachable public API — no cross-crate or absent
//!   gaps, because `skit-store` reaches both `FileFormStateStore` (disk) and the
//!   `skit-application::form_state` service and pure `prefill`.
//! - No CLI/binary is exercised, so no `SKIT_*_DIR` env vars are needed; the store is rooted
//!   at an explicit `TempDir`, which also avoids env races with sibling agents.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use skit_application::form_state::{FormStateRepository, FormStateService, prefill};
use skit_domain::{
    Slug,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_store::FileFormStateStore;
use tempfile::TempDir;

/// The oracle's module fixture: `SKIT_DATA_DIR`/`SKIT_STATE_DIR` under a temp path. Here the
/// state root is passed to `FileFormStateStore` explicitly, so a fresh `TempDir` per test is
/// the whole sandbox.
fn service(root: &TempDir) -> FormStateService<FileFormStateStore> {
    FormStateService::new(FileFormStateStore::new(root.path()))
}

/// The oracle's slug "s".
fn slug() -> Slug {
    Slug::parse("s").unwrap()
}

/// Oracle `spec(name, *, default=None, secret=False)`:
/// `ParamDecl(binding="const", delivery="inject", type="str", default=..., secret=...)`.
fn spec(name: &str, default: Option<&str>, secret: bool) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = default.map(|value| ParameterValue::String(value.to_owned()));
    declaration.secret = secret;
    declaration
}

/// Build a value/preset map from pairs.
fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// Python `values_dir()` — `state_dir()/values`.
fn values_dir(root: &TempDir) -> PathBuf {
    root.path().join("values")
}

/// Read the text of every `*.toml` under the values directory (Python's
/// `values_dir().glob("*.toml")`).
fn state_texts(root: &TempDir) -> Vec<String> {
    let mut texts = Vec::new();
    if let Ok(entries) = fs::read_dir(values_dir(root)) {
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
                texts.push(fs::read_to_string(&path).unwrap());
            }
        }
    }
    texts
}

/// Python `assert "<needle>" not in p.read_text()` over every state file: the value itself
/// must never appear in any state file's raw bytes.
fn assert_absent_on_disk(root: &TempDir, needle: &str) {
    for text in state_texts(root) {
        assert!(!text.contains(needle), "{needle:?} leaked to disk:\n{text}");
    }
}

#[test]
fn test_preset_roundtrip() {
    let root = TempDir::new().unwrap();
    let service = service(&root);

    service
        .save_preset(
            &slug(),
            "prod",
            &[spec("CITY", None, false)],
            &map(&[("CITY", "Taipei")]),
        )
        .unwrap();
    let state = service.load(&slug());
    assert_eq!(state.presets["prod"], map(&[("CITY", "Taipei")]));
    assert!(service.delete_preset(&slug(), "prod").unwrap());
    assert!(service.load(&slug()).presets.is_empty());
    assert!(!service.delete_preset(&slug(), "nope").unwrap());
}

#[test]
fn test_resolution_order_preset_over_last_over_default() {
    // Resolution moved from argstate.resolve_defaults into flows.prefill (the unified
    // form layer); the contract is the same: preset > last-used > definition default.
    let root = TempDir::new().unwrap();
    let service = service(&root);
    let specs = vec![
        spec("CITY", Some("Osaka"), false),
        spec("N", Some("1"), false),
    ];

    // Definition default only
    let state = service.load(&slug());
    assert_eq!(
        prefill(&specs, &state.values, None),
        map(&[("CITY", "Osaka"), ("N", "1")])
    );

    // Last-used value overrides default
    service
        .save_last(
            &slug(),
            &specs,
            Some(&map(&[("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    let state = service.load(&slug());
    assert_eq!(prefill(&specs, &state.values, None)["CITY"], "Taipei");

    // Preset overrides last-used
    service
        .save_preset(&slug(), "jp", &specs, &map(&[("CITY", "Kyoto")]))
        .unwrap();
    let state = service.load(&slug());
    assert_eq!(
        prefill(&specs, &state.values, state.presets.get("jp"))["CITY"],
        "Kyoto"
    );

    // Stale keys in state must never leak into the form. The Rust service-level `save_last`
    // drops unknown names before disk, so to exercise prefill's OWN filter the stale key is
    // seeded straight into state (the oracle wrote it to the state file, where prefill filters
    // it out).
    service
        .repository()
        .update(&slug(), |state| {
            state.values = map(&[("STALE", "x"), ("CITY", "Taipei")]);
        })
        .unwrap();
    let state = service.load(&slug());
    assert!(!prefill(&specs, &state.values, None).contains_key("STALE"));
}

#[test]
fn test_c3_secret_never_touches_disk() {
    let root = TempDir::new().unwrap();
    let service = service(&root);
    let specs = [spec("API_KEY", None, true), spec("CITY", None, false)];

    service
        .save_last(
            &slug(),
            &specs,
            Some(&map(&[("API_KEY", "hunter2"), ("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    service
        .save_preset(
            &slug(),
            "prod",
            &specs,
            &map(&[("API_KEY", "hunter2"), ("CITY", "Taipei")]),
        )
        .unwrap();
    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY"));
    assert!(!state.presets["prod"].contains_key("API_KEY"));
    // Scan the raw bytes of every state file to guarantee the value itself was never written
    assert_absent_on_disk(&root, "hunter2");
}

#[test]
fn test_preset_preserved_across_save_last() {
    let root = TempDir::new().unwrap();
    let service = service(&root);
    let specs = [spec("CITY", None, false)];

    service
        .save_preset(&slug(), "prod", &specs, &map(&[("CITY", "Taipei")]))
        .unwrap();
    service
        .save_last(
            &slug(),
            &specs,
            Some(&map(&[("CITY", "Tainan")])),
            Some(vec!["-v".to_owned()]),
            false,
        )
        .unwrap();
    let state = service.load(&slug());
    assert_eq!(state.presets["prod"], map(&[("CITY", "Taipei")]));
    assert_eq!(state.values["CITY"], "Tainan");
    assert_eq!(state.extra_args, ["-v"]);
}

// --------------------------------------------------------------------------
// purge_secret + save_last stale-key dropping ("secrets are not fully secret" gap)
// --------------------------------------------------------------------------

#[test]
fn test_purge_secret_removes_from_values_and_every_preset() {
    // A value stored while a param was still public, plus a copy saved to two presets, must all
    // disappear once the param transitions to secret.
    let root = TempDir::new().unwrap();
    let service = service(&root);
    // API_KEY is PUBLIC when first stored (no secret flag): the pre-transition plaintext.
    let public = [spec("API_KEY", None, false), spec("CITY", None, false)];

    service
        .save_last(
            &slug(),
            &public,
            Some(&map(&[("API_KEY", "shown"), ("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    service
        .save_preset(
            &slug(),
            "prod",
            &public,
            &map(&[("API_KEY", "shown"), ("CITY", "Taipei")]),
        )
        .unwrap();
    service
        .save_preset(
            &slug(),
            "dev",
            &[spec("API_KEY", None, false)],
            &map(&[("API_KEY", "shown")]),
        )
        .unwrap();

    // API_KEY becomes secret: purge scrubs it everywhere.
    let removed = service
        .purge_secrets(&slug(), &[spec("API_KEY", None, true)])
        .unwrap();
    assert_eq!(removed, BTreeSet::from(["API_KEY".to_owned()]));

    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY"));
    assert_eq!(state.values["CITY"], "Taipei"); // unrelated, still-public value untouched
    assert!(!state.presets["prod"].contains_key("API_KEY"));
    assert_eq!(state.presets["prod"]["CITY"], "Taipei");
    // 'dev' held only API_KEY, so purging it leaves the preset empty -> it is dropped entirely
    // (mirroring delete_preset), not kept as a confusing value-less table.
    assert!(!state.presets.contains_key("dev"));
    assert_absent_on_disk(&root, "shown");
}

#[test]
fn test_purge_secret_drops_a_preset_left_empty_but_keeps_others() {
    // A preset whose only key was the now-secret param is removed; a preset with surviving
    // public keys is retained (minus the secret). No dangling empty [presets.*] table remains.
    let root = TempDir::new().unwrap();
    let service = service(&root);

    service
        .save_preset(
            &slug(),
            "onlysecret",
            &[spec("API_KEY", None, false)],
            &map(&[("API_KEY", "shown")]),
        )
        .unwrap();
    service
        .save_preset(
            &slug(),
            "mixed",
            &[spec("API_KEY", None, false), spec("CITY", None, false)],
            &map(&[("API_KEY", "shown"), ("CITY", "Taipei")]),
        )
        .unwrap();
    service
        .purge_secrets(&slug(), &[spec("API_KEY", None, true)])
        .unwrap();

    let state = service.load(&slug());
    assert!(!state.presets.contains_key("onlysecret"));
    assert_eq!(state.presets["mixed"], map(&[("CITY", "Taipei")]));
    let text = fs::read_to_string(values_dir(&root).join("s.toml")).unwrap();
    assert!(!text.contains("onlysecret"));
    assert!(!text.contains("shown"));
}

#[test]
fn test_purge_secret_empty_names_is_noop() {
    let root = TempDir::new().unwrap();
    let service = service(&root);
    service
        .save_last(
            &slug(),
            &[spec("CITY", None, false)],
            Some(&map(&[("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    let path = values_dir(&root).join("s.toml");
    let before = fs::read_to_string(&path).unwrap();
    // Python passes an EMPTY names set; here that is an empty declaration set, so the derived
    // secret set is empty and nothing is scrubbed.
    assert!(service.purge_secrets(&slug(), &[]).unwrap().is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn test_purge_secret_reports_only_names_actually_stored() {
    let root = TempDir::new().unwrap();
    let service = service(&root);
    service
        .save_last(
            &slug(),
            &[spec("CITY", None, false)],
            Some(&map(&[("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    // Both API_KEY and CITY are now secret; only CITY was ever stored, so it alone is reported.
    let removed = service
        .purge_secrets(
            &slug(),
            &[spec("API_KEY", None, true), spec("CITY", None, true)],
        )
        .unwrap();
    assert_eq!(removed, BTreeSet::from(["CITY".to_owned()])); // API_KEY was never stored, so it cannot have been removed
}

#[test]
fn test_save_last_drops_stale_value_once_param_becomes_secret() {
    // Reproduces the argstate.py gap directly: save_last's read-modify-write only replaced
    // doc["values"] when this call's own (now-stripped) snapshot was non-empty. A script with a
    // single, newly-secret parameter collects nothing else, so `clean` is empty and the old guard
    // left the stale plaintext in place forever.
    let root = TempDir::new().unwrap();
    let service = service(&root);

    service
        .save_last(
            &slug(),
            &[spec("API_KEY", None, false)],
            Some(&map(&[("API_KEY", "old-secret")])),
            None,
            false,
        )
        .unwrap();
    assert_eq!(service.load(&slug()).values["API_KEY"], "old-secret");
    service
        .save_last(
            &slug(),
            &[spec("API_KEY", None, true)],
            Some(&map(&[("API_KEY", "new-typed")])),
            None,
            false,
        )
        .unwrap();
    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY"));
    assert_absent_on_disk(&root, "old-secret");
    assert_absent_on_disk(&root, "new-typed");
}

#[test]
fn test_save_last_values_are_a_snapshot_not_a_merge() {
    // values is the run's complete snapshot: replace semantics are what make "the user
    // cleared this field" persist (the old merge semantics resurrected cleared values).
    let root = TempDir::new().unwrap();
    let service = service(&root);

    service
        .save_last(
            &slug(),
            &[spec("API_KEY", None, false), spec("CITY", None, false)],
            Some(&map(&[("API_KEY", "old-secret"), ("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    service
        .save_last(
            &slug(),
            &[spec("API_KEY", None, true)],
            Some(&map(&[("API_KEY", "x")])),
            None,
            false,
        )
        .unwrap();
    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY")); // C3 strip on the new write
    assert!(!state.values.contains_key("CITY")); // not in this snapshot -> gone
}

#[test]
fn test_save_last_none_values_still_scrubs_stale_secret() {
    // No new data at all (values=None): stored values stay, EXCEPT names that just
    // became secret — those are scrubbed even without a resupply.
    let root = TempDir::new().unwrap();
    let service = service(&root);

    service
        .save_last(
            &slug(),
            &[spec("API_KEY", None, false), spec("CITY", None, false)],
            Some(&map(&[("API_KEY", "old-secret"), ("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    service
        .save_last(&slug(), &[spec("API_KEY", None, true)], None, None, false)
        .unwrap();
    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY"));
    assert_eq!(state.values["CITY"], "Taipei");
}

#[test]
fn test_save_last_regression_non_secret_values_persist_normally() {
    // Non-secret params must keep behaving exactly as before: stored and read back verbatim.
    let root = TempDir::new().unwrap();
    let service = service(&root);

    service
        .save_last(
            &slug(),
            &[spec("CITY", None, false), spec("N", None, false)],
            Some(&map(&[("CITY", "Taipei"), ("N", "3")])),
            None,
            false,
        )
        .unwrap();
    assert_eq!(
        service.load(&slug()).values,
        map(&[("CITY", "Taipei"), ("N", "3")])
    );
}
