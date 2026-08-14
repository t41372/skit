use std::fs;
use assert_cmd::Command;
use tempfile::TempDir;

#[test]
fn test_save_editor_backs_up_corrupt_config_instead_of_wiping_it() {
    let data=TempDir::new().unwrap();let state=TempDir::new().unwrap();let config=TempDir::new().unwrap();let home=TempDir::new().unwrap();
    let corrupt="language = \"zh-CN\"\n[mirror]\nenabled = true\npypi = \"https://saved.example/simple\"\nthis is = = not valid toml";
    let path=config.path().join("config.toml");let backup=config.path().join("config.toml.bak");fs::write(&path,corrupt).unwrap();
    let output=assert_cmd::cargo::cargo_bin_cmd!("skit")
        .env("SKIT_DATA_DIR",data.path()).env("SKIT_STATE_DIR",state.path()).env("SKIT_CONFIG_DIR",config.path()).env("SKIT_LANG","en")
        .env("HOME",home.path()).env("USERPROFILE",home.path()).args(["config","editor","vim"]).output().unwrap();
    let text=format!("{}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(),"{text}");
    let doc:toml::Table=toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();assert_eq!(doc["editor"].as_str(),Some("vim"));
    assert_eq!(fs::read_to_string(&backup).unwrap(),corrupt);
    let err=String::from_utf8_lossy(&output.stderr);assert!(err.contains(&path.display().to_string()),"{err}");assert!(err.contains(&backup.display().to_string()),"{err}");
}
