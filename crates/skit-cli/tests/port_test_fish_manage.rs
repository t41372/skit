//! End-to-end port of the Fish env-default management flow in Python v0.4.

use std::{collections::BTreeMap, fs};

use assert_cmd::Command;
use skit_application::{
    EntryRepository as _,
    delivery::{PreparedValue, assemble},
};
use skit_domain::{EntrySettings, parameters::ParameterDelivery};
use skit_form::{FormSource, form_plan};
use skit_runtime::{ProgramProbe as _, SystemProbe};
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

    fn plan_and_assembly(
        &self,
        name: &str,
    ) -> (skit_form::FormPlan, skit_application::delivery::Assembly) {
        let store = self.store();
        let entry = store.resolve(name).unwrap();
        let path = store.payload_path(&entry).unwrap();
        let text = fs::read_to_string(path).unwrap();
        let settings = EntrySettings::from_meta(&entry.meta);
        let plan = form_plan("fish", &text, &settings);
        let assembly = assemble(
            &plan.declarations(),
            &BTreeMap::from([("PORT".to_owned(), PreparedValue::Scalar("9090".to_owned()))]),
            &[],
        )
        .unwrap();
        (plan, assembly)
    }
}

#[test]
fn test_manage_then_plan_and_assemble_env_delivery() {
    let sandbox = Sandbox::new();
    sandbox.add_port_entry("cfg");

    let store = sandbox.store();
    let entry = store.resolve("cfg").unwrap();
    let path = store.payload_path(&entry).unwrap();
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains("# [tool.skit]"), "{text}");
    assert!(text.contains("name = \"PORT\""), "{text}");
    assert!(text.contains("kind = \"envdefault\""), "{text}");

    let (plan, assembly) = sandbox.plan_and_assembly("cfg");
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

#[test]
fn test_env_overlay_overrides_default_in_real_fish() {
    let Some(fish) = SystemProbe.find_program("fish") else {
        assert!(
            std::env::var("SKIT_REQUIRE_FISH_RUNTIME").as_deref() != Ok("1"),
            "the Fish runtime owner is required, but SystemProbe did not find Fish"
        );
        eprintln!("Fish is not installed; the frozen Python owner has the same availability gate");
        return;
    };
    assert!(
        SystemProbe.is_executable(&fish),
        "SystemProbe must return an executable Fish path"
    );

    let sandbox = Sandbox::new();
    sandbox.add_port_entry("realcfg");
    let output = sandbox
        .command()
        .args(["run", "realcfg", "--set", "PORT=9090", "--no-input"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "skit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "9090");
}
