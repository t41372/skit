//! High-layer Fish ports from Python v0.4 `tests/test_fish.py` at `main@206f9ef`.
//!
//! These tests cross the real CLI/store/form/delivery boundaries. The real-fish contract keeps the
//! Python suite's availability gate: when `fish` is installed it executes the stored script, rather
//! than substituting a dry run or a fake interpreter.

use std::{collections::BTreeMap, env, fs, path::{Path, PathBuf}, process::Command as ProcessCommand};

use assert_cmd::Command;
use skit_application::{EntryRepository as _, delivery::{Assembly, PreparedValue, assemble}};
use skit_domain::{EntrySettings, parameters::ParameterDelivery};
use skit_form::{FormSource, form_plan};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        };
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        sandbox
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .current_dir(self.home.path());
        command
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn add_port_entry(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.fish"));
        fs::write(
            &source,
            "#!/usr/bin/env fish\nset -q PORT; or set PORT 8080\necho $PORT\n",
        )
        .unwrap();
        self.command()
            .arg("add")
            .arg(&source)
            .args(["--name", name, "--no-input"])
            .assert()
            .success();
    }

    fn manage_port(&self, name: &str) {
        self.command()
            .args(["params", name, "--manage", "PORT"])
            .assert()
            .success();
    }

    fn stored_script(&self, name: &str) -> PathBuf {
        self.data.path().join("scripts").join(name).join("script.fish")
    }

    fn plan_and_assemble(&self, name: &str) -> (PathBuf, Assembly) {
        let entry = self.store().resolve(name).unwrap();
        let script = self.stored_script(name);
        let text = fs::read_to_string(&script).unwrap();
        let plan = form_plan("fish", &text, &EntrySettings::from_meta(&entry.meta));
        assert_eq!(plan.source, FormSource::Inject);
        let port = plan
            .fields
            .iter()
            .find(|field| field.declaration.name == "PORT")
            .expect("managed Fish PORT field must remain in the form plan");
        assert_eq!(port.declaration.delivery, ParameterDelivery::Env);
        let values = BTreeMap::from([(
            "PORT".to_owned(),
            PreparedValue::Scalar("9090".to_owned()),
        )]);
        let assembly = assemble(&plan.declarations(), &values, &[]).unwrap();
        assert_eq!(
            assembly.env_values,
            BTreeMap::from([("PORT".to_owned(), "9090".to_owned())])
        );
        (script, assembly)
    }
}

#[test]
fn test_manage_then_plan_and_assemble_env_delivery() {
    let sandbox = Sandbox::new();
    sandbox.add_port_entry("cfg");
    sandbox.manage_port("cfg");

    let script = sandbox.stored_script("cfg");
    let block = fs::read_to_string(&script).unwrap();
    assert!(block.contains("# [tool.skit]"), "{block}");
    assert!(block.contains("name = \"PORT\""), "{block}");
    assert!(block.contains("kind = \"envdefault\""), "{block}");

    let (planned_script, _assembly) = sandbox.plan_and_assemble("cfg");
    assert_eq!(planned_script, script);
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        for candidate in [name.to_owned(), format!("{name}.exe"), format!("{name}.cmd")] {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[test]
fn test_env_overlay_overrides_default_in_real_fish() {
    let Some(fish) = find_on_path("fish") else {
        // Python marks this exact contract skipif(shutil.which("fish") is None).
        return;
    };
    let sandbox = Sandbox::new();
    sandbox.add_port_entry("realcfg");
    sandbox.manage_port("realcfg");
    let (script, assembly) = sandbox.plan_and_assemble("realcfg");

    let output = ProcessCommand::new(fish)
        .arg(&script)
        .envs(&assembly.env_values)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "real Fish execution failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "9090");
}

#[test]
fn rust_additive_managed_fish_script_path_is_a_file() {
    let sandbox = Sandbox::new();
    sandbox.add_port_entry("cfg");
    sandbox.manage_port("cfg");
    assert!(Path::new(&sandbox.stored_script("cfg")).is_file());
}
