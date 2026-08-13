//! End-to-end ports of the two Fish env-default flow tests in Python v0.4 `tests/test_fish.py`.
//!
//! The offline test crosses the real CLI and store before using the shared form/delivery layers.
//! The real-child test keeps Python's runtime-availability condition and, when Fish is present,
//! executes the actual Fish program; a fake interpreter is not accepted as a substitute.

use std::{collections::BTreeMap, fs, io, process::Command as StdCommand};

use assert_cmd::Command;
use skit_application::{EntryRepository as _, delivery::{PreparedValue, assemble}};
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
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("PSModulePath")
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
            .args(["--kind", "fish", "--name", name, "--no-input"])
            .assert()
            .success();
        self.command()
            .args(["params", name, "--manage", "PORT"])
            .assert()
            .success();
    }

    fn plan_and_assembly(&self, name: &str) -> (std::path::PathBuf, skit_form::FormPlan, skit_application::delivery::Assembly) {
        let store = self.store();
        let entry = store.resolve(name).unwrap();
        let path = store.payload_path(&entry).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let settings = EntrySettings::from_meta(&entry.meta);
        let plan = form_plan("fish", &text, &settings);
        let assembly = assemble(
            &plan.declarations(),
            &BTreeMap::from([(
                "PORT".to_owned(),
                PreparedValue::Scalar("9090".to_owned()),
            )]),
            &[],
        )
        .unwrap();
        (path, plan, assembly)
    }
}

#[test]
fn test_manage_then_plan_and_assemble_env_delivery() {
    let sandbox = Sandbox::new();
    sandbox.add_port_entry("cfg");

    let store = sandbox.store();
    let entry = store.resolve("cfg").unwrap();
    let path = store.payload_path(&entry).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("# [tool.skit]"), "{text}");
    assert!(text.contains("name = \"PORT\""), "{text}");
    assert!(text.contains("kind = \"envdefault\""), "{text}");

    let (_path, plan, assembly) = sandbox.plan_and_assembly("cfg");
    assert_eq!(plan.source, FormSource::Inject);
    let field = plan
        .fields
        .iter()
        .find(|field| field.declaration.name == "PORT")
        .expect("managed Fish PORT must remain in the run form");
    assert_eq!(field.declaration.delivery, ParameterDelivery::Env);
    assert!(assembly.inject_values.is_empty());
    assert_eq!(
        assembly.env_values,
        BTreeMap::from([("PORT".to_owned(), "9090".to_owned())])
    );
}

fn fish_is_spawnable() -> bool {
    match StdCommand::new("fish").arg("--version").output() {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => panic!("could not probe installed Fish runtime: {error}"),
    }
}

#[test]
fn test_env_overlay_overrides_default_in_real_fish() {
    // Python uses pytest.skipif(shutil.which("fish") is None). Rust's stock test harness has no
    // skipped result state, so preserve the same availability gate and execute a real Fish when it
    // is installed. Do not replace this with a fake interpreter.
    if !fish_is_spawnable() {
        eprintln!("Fish runtime not installed; Python oracle is skip-gated under the same condition");
        return;
    }

    let sandbox = Sandbox::new();
    sandbox.add_port_entry("realcfg");
    let (path, plan, assembly) = sandbox.plan_and_assembly("realcfg");
    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(
        assembly.env_values,
        BTreeMap::from([("PORT".to_owned(), "9090".to_owned())])
    );

    let output = StdCommand::new("fish")
        .arg(path)
        .envs(&assembly.env_values)
        .current_dir(sandbox.home.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Fish failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "9090");
}
