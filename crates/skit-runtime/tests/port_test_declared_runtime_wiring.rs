//! Exact-name runtime wiring ports from Python v0.4 `tests/test_declared_params.py`.

use std::{collections::BTreeMap, env, fs, path::PathBuf};

use skit_application::delivery::Assembly;
use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug};
use skit_runtime::{LaunchPaths, LaunchPlan, SystemProbe, build_launch_preview, execute_launch};
use tempfile::TempDir;

fn command_entry(template: &str) -> Entry {
    let mut meta = EntryMeta::minimal("cmd", EntryKind::parse("command").unwrap());
    EntrySettings {
        template: template.to_owned(),
        ..EntrySettings::default()
    }
    .write_to_meta(&mut meta);
    Entry {
        slug: Slug::parse("cmd").unwrap(),
        meta,
    }
}

fn paths(root: &TempDir) -> LaunchPaths {
    LaunchPaths {
        script: root.path().join("unused"),
        entry_dir: root.path().to_owned(),
        invoke_cwd: root.path().to_owned(),
    }
}

#[test]
fn child_env_probe() {
    if env::var("SKIT_PARITY_CHILD").as_deref() != Ok("1") {
        return;
    }
    let output = PathBuf::from(env::var_os("SKIT_PARITY_OUT").expect("child output path"));
    fs::write(output, env::var("PATH").unwrap()).unwrap();
}

#[test]
fn test_run_entry_env_overlay_wins_last() {
    let root = TempDir::new().unwrap();
    let output = root.path().join("observed-path.txt");
    let plan = LaunchPlan {
        program: env::current_exe().unwrap(),
        args: vec![
            "--exact".to_owned(),
            "child_env_probe".to_owned(),
            "--nocapture".to_owned(),
        ],
        env: BTreeMap::from([
            ("SKIT_PARITY_CHILD".to_owned(), "1".to_owned()),
            ("SKIT_PARITY_OUT".to_owned(), output.display().to_string()),
            ("PATH".to_owned(), "skit-overlay-marker".to_owned()),
        ]),
        cwd: root.path().to_owned(),
        display: String::new(),
        warnings: Vec::new(),
    };

    assert_eq!(execute_launch(&plan).unwrap(), 0);
    assert_eq!(
        fs::read_to_string(output).unwrap(),
        "skit-overlay-marker",
        "the explicit launch environment must replace the inherited PATH value"
    );
}

#[test]
fn test_transparency_shows_masked_env_prefix() {
    let root = TempDir::new().unwrap();
    let assembly = Assembly {
        env_values: BTreeMap::from([
            ("API_TOKEN".to_owned(), "hunter2".to_owned()),
            ("GREETING".to_owned(), "hello world".to_owned()),
        ]),
        masked_env: BTreeMap::from([
            ("API_TOKEN".to_owned(), "•••".to_owned()),
            ("GREETING".to_owned(), "hello world".to_owned()),
        ]),
        ..Assembly::default()
    };
    let plan = build_launch_preview(
        &command_entry("echo hi"),
        &paths(&root),
        &assembly,
        None,
        None,
        None,
        &SystemProbe,
    )
    .unwrap();

    assert!(plan.display.contains("API_TOKEN="), "{}", plan.display);
    assert!(!plan.display.contains("hunter2"), "{}", plan.display);
    assert!(plan.display.contains("•••"), "{}", plan.display);
    assert!(
        plan.display.contains("GREETING='hello world'")
            || plan.display.contains("GREETING=\"hello world\""),
        "a spaced public env value must remain copy-pasteably quoted: {}",
        plan.display
    );
}

#[test]
fn test_execute_passes_env_values_to_run_entry() {
    let root = TempDir::new().unwrap();
    let assembly = Assembly {
        env_values: BTreeMap::from([("N".to_owned(), "5".to_owned())]),
        masked_env: BTreeMap::from([("N".to_owned(), "5".to_owned())]),
        ..Assembly::default()
    };
    let plan = build_launch_preview(
        &command_entry("echo hi"),
        &paths(&root),
        &assembly,
        None,
        None,
        None,
        &SystemProbe,
    )
    .unwrap();
    assert_eq!(plan.env, BTreeMap::from([("N".to_owned(), "5".to_owned())]));
}
