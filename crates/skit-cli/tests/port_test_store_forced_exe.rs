use std::fs;
use assert_cmd::Command;
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

fn command(data: &TempDir, state: &TempDir, config: &TempDir, home: &TempDir) -> Command {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("skit");
    cmd.env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .current_dir(home.path());
    cmd
}

#[test]
fn test_infer_kind_python_and_forced_exe() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    fs::write(config.path().join("config.toml"), "[mirror]\nenabled = false\n").unwrap();
    let lower = home.path().join("a.py");
    let upper = home.path().join("B.PY");
    fs::write(&lower, "print(1)\n").unwrap();
    fs::write(&upper, "print(2)\n").unwrap();

    command(&data, &state, &config, &home)
        .arg("add").arg(&lower).args(["--name", "normal", "--no-input"]).assert().success();
    command(&data, &state, &config, &home)
        .arg("add").arg(&upper).args(["--name", "upper", "--no-input"]).assert().success();
    command(&data, &state, &config, &home)
        .arg("add").arg(&lower).args(["--exe", "--name", "forced", "--no-input"]).assert().success();

    let store = FileStore::new(data.path());
    assert_eq!(store.resolve("normal").unwrap().meta.kind.as_str(), "python");
    assert_eq!(store.resolve("upper").unwrap().meta.kind.as_str(), "python");
    assert_eq!(store.resolve("forced").unwrap().meta.kind.as_str(), "exe");
}
