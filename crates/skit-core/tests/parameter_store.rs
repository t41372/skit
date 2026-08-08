use std::fs;
use std::path::Path;

use skit_core::{Delivery, LibraryRoots, ParamDecl, ParamDefault, ParamType, Store};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

fn write(path: &Path, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(())
}

#[test]
fn declared_parameters_roundtrip_without_touching_placeholder_cache_or_unknown_meta()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let meta_path = root.path().join("data/scripts/demo/meta.toml");
    write(
        &meta_path,
        r#"name = "Demo"
kind = "command"
mode = "copy"
workdir = "invoke"
template = "convert {size} {target}"
params = ["size", "target"]
future_key = "keep-me"

[future_table]
enabled = true
"#,
    )?;

    let decls = vec![
        ParamDecl {
            name: "size".to_owned(),
            delivery: Delivery::Placeholder,
            param_type: ParamType::Choice,
            default: Some(ParamDefault::String("m".to_owned())),
            choices: vec!["s".to_owned(), "m".to_owned()],
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "RETRIES".to_owned(),
            delivery: Delivery::Env,
            param_type: ParamType::Integer,
            default: Some(ParamDefault::Integer(3)),
            ..ParamDecl::default()
        },
    ];

    let updated = store.write_parameters("Demo", &decls)?;
    assert_eq!(store.read_parameters("Demo")?, decls);
    assert_eq!(updated.meta.params, Some(vec!["size".to_owned(), "target".to_owned()]));
    assert_eq!(
        updated.meta.extra.get("future_key").and_then(toml::Value::as_str),
        Some("keep-me")
    );
    assert!(updated.meta.extra.contains_key("future_table"));

    let raw = fs::read_to_string(meta_path)?;
    assert!(raw.contains("params = [\"size\", \"target\"]"));
    assert!(raw.contains("future_key = \"keep-me\""));
    assert!(raw.contains("[future_table]"));
    assert!(raw.contains("[[parameters]]"));
    Ok(())
}

#[test]
fn clearing_declared_parameters_removes_only_the_parameters_array()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    let meta_path = root.path().join("data/scripts/demo/meta.toml");
    write(
        &meta_path,
        r#"name = "Demo"
kind = "exe"
mode = "reference"
source = "/tmp/tool"
future_key = "keep"

[[parameters]]
name = "width"
delivery = "flag"
type = "int"
flag = "--width"
"#,
    )?;

    let updated = store.write_parameters("demo", &[])?;
    assert!(updated.meta.parameters.is_none());
    assert!(store.read_parameters("demo")?.is_empty());
    let raw = fs::read_to_string(meta_path)?;
    assert!(!raw.contains("[[parameters]]"));
    assert!(raw.contains("future_key = \"keep\""));
    Ok(())
}

#[test]
fn read_parameters_is_total_over_hand_edited_rows() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = Store::new(roots(root.path()));
    write(
        &root.path().join("data/scripts/demo/meta.toml"),
        r#"name = "Demo"
kind = "exe"

[[parameters]]
delivery = "flag"

[[parameters]]
name = "ok"
delivery = "future"
type = "future"
"#,
    )?;

    let decls = store.read_parameters("Demo")?;
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].name, "ok");
    assert_eq!(decls[0].delivery, Delivery::Flag);
    assert_eq!(decls[0].param_type, ParamType::String);
    Ok(())
}
