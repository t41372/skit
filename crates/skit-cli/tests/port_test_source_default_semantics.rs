//! Black-box CLI port from Python `tests/test_source_default_semantics.py`
//! (`origin/main@206f9ef`). The Python implementation is the behavioral oracle.

use std::fs;

use skit_domain::parameters::{ParameterType, ParameterValue};
use skit_language::managed_params;
use tempfile::TempDir;

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

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }
}

#[test]
fn test_resync_writes_source_default_into_ok_and_type_changed_specs() {
    // One resync exercises both write paths: an unchanged constant refreshes its default, and a
    // type-changed constant takes both its type and default from the current source candidate.
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("resync.py");
    fs::write(
        &source,
        r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "const"
# type = "str"
# default = "old-city"
#
# [[tool.skit.params]]
# name = "RETRIES"
# kind = "const"
# type = "int"
# default = 3
# ///
CITY = "Taipei"
RETRIES = "three"
print(CITY, RETRIES)
"#,
    )
    .unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "Resync defaults",
            "--no-input",
        ])
        .assert()
        .success();
    sandbox
        .command()
        .args(["params", "resync-defaults", "--resync"])
        .assert()
        .success();

    let stored = fs::read_to_string(
        sandbox
            .data
            .path()
            .join("scripts/resync-defaults/script.py"),
    )
    .unwrap();
    let declarations = managed_params("python", &stored);
    let city = declarations
        .iter()
        .find(|declaration| declaration.name == "CITY")
        .unwrap();
    let retries = declarations
        .iter()
        .find(|declaration| declaration.name == "RETRIES")
        .unwrap();
    assert_eq!(city.parameter_type, ParameterType::Str);
    assert_eq!(
        city.default,
        Some(ParameterValue::String("Taipei".to_owned()))
    );
    assert_eq!(retries.parameter_type, ParameterType::Str);
    assert_eq!(
        retries.default,
        Some(ParameterValue::String("three".to_owned()))
    );
}
