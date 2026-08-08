use std::fs;

use crate::{Entry, parse_pep723};

/// Return the dependency and Python-version axes that actually govern a run.
///
/// Metadata wins independently per axis. Only a copy-mode Python entry may fall back
/// to the stored copy's PEP 723 block; reference entries and every other language use
/// metadata only. Unreadable or non-UTF-8 bytes are decoded lossily for this read-only
/// view, matching the current Python implementation.
#[must_use]
pub fn effective_uv_metadata(entry: &Entry) -> (Vec<String>, String) {
    let mut dependencies = entry.meta.dependencies.clone().unwrap_or_default();
    let mut requires_python = entry.meta.requires_python.clone();
    if entry.meta.kind != "python"
        || entry.meta.mode != "copy"
        || (!dependencies.is_empty() && !requires_python.is_empty())
    {
        return (dependencies, requires_python);
    }

    let path = entry.script_path();
    let Ok(bytes) = fs::read(path) else {
        return (dependencies, requires_python);
    };
    let text = String::from_utf8_lossy(&bytes);
    let Some(block) = parse_pep723(&text, "#") else {
        return (dependencies, requires_python);
    };
    if dependencies.is_empty() {
        dependencies = block.dependencies;
    }
    if requires_python.is_empty() {
        requires_python = block.requires_python;
    }
    (dependencies, requires_python)
}
