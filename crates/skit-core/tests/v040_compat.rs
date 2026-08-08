use std::fs;
use std::path::Path;

use skit_core::{LibraryRoots, Store};
use tempfile::tempdir;

const LEGACY_REGISTRY: &str = r#"[entries.hello]
name = "Hello"
kind = "python"
description = "Greets a person"

[entries.future]
name = "Future"
kind = "future-kind"
description = "Unknown kinds stay visible"
"#;

const HELLO_META: &str = r#"schema = 1
name = "Hello"
kind = "python"
mode = "copy"
source = "/home/alice/hello.py"
source_hash = "sha256:0123456789abcdef"
added_at = "2026-07-22T00:00:00+00:00"
workdir = "origin"
description = "Greets a person"
"#;

const FUTURE_META: &str = r#"schema = 1
name = "Future"
kind = "future-kind"
mode = "reference"
source = "/home/alice/future.tool"
source_hash = "sha256:fedcba9876543210"
added_at = "2026-07-22T00:00:00+00:00"
workdir = "invoke"
description = "Unknown kinds stay visible"
future_key = "must not make the reader fail"
"#;

fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

#[test]
fn reads_v040_library_without_mutating_it() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    let state = root.path().join("state");
    let config = root.path().join("config");
    let registry = data.join("registry.toml");
    let hello_meta = data.join("scripts/hello/meta.toml");
    let future_meta = data.join("scripts/future/meta.toml");

    write(&registry, LEGACY_REGISTRY)?;
    write(&hello_meta, HELLO_META)?;
    write(&future_meta, FUTURE_META)?;

    let before_registry = fs::read(&registry)?;
    let before_hello = fs::read(&hello_meta)?;
    let before_future = fs::read(&future_meta)?;

    let store = Store::new(LibraryRoots::new(data, state, config));
    let entries = store.list()?;

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].slug, "future");
    assert_eq!(entries[0].name, "Future");
    assert_eq!(entries[0].kind, "future-kind");
    assert_eq!(entries[1].slug, "hello");
    assert_eq!(entries[1].name, "Hello");
    assert_eq!(entries[1].kind, "python");

    let resolved = store.resolve("Hello")?;
    assert_eq!(resolved.slug, "hello");
    assert_eq!(resolved.meta.name, "Hello");
    assert_eq!(resolved.meta.schema, 1);
    assert_eq!(resolved.meta.mode, "copy");

    assert_eq!(fs::read(&registry)?, before_registry);
    assert_eq!(fs::read(&hello_meta)?, before_hello);
    assert_eq!(fs::read(&future_meta)?, before_future);
    Ok(())
}

#[test]
fn resolve_accepts_slug_and_unknown_kind() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let data = root.path().join("data");
    let state = root.path().join("state");
    let config = root.path().join("config");
    write(&data.join("scripts/future/meta.toml"), FUTURE_META)?;

    let store = Store::new(LibraryRoots::new(data, state, config));
    let entry = store.resolve("future")?;

    assert_eq!(entry.slug, "future");
    assert_eq!(entry.meta.kind, "future-kind");
    assert_eq!(
        entry
            .meta
            .extra
            .get("future_key")
            .and_then(toml::Value::as_str),
        Some("must not make the reader fail")
    );
    Ok(())
}
