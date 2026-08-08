use std::fs;
use std::path::Path;

use skit_core::{
    AddMode, LibraryRoots, PlanSource, PythonAutoAddError, PythonAutoAddRequest, Store,
    add_python_auto, plan_for_entry,
};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

fn request(source: &Path, interactive: bool, no_input: bool) -> PythonAutoAddRequest {
    PythonAutoAddRequest {
        source: source.to_owned(),
        name: Some("auto".to_owned()),
        mode: AddMode::Copy,
        description: None,
        workdir: None,
        added_at: "2026-08-08T12:00:00+00:00".to_owned(),
        interactive,
        no_input,
    }
}

#[test]
fn noninteractive_accepts_dependency_suggestions_and_shebang_pin_but_manages_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    fs::write(
        &source,
        "#!/usr/bin/env python3.12\nimport os\nimport requests\nCITY = 'Taipei'\n",
    )?;
    let store = Store::new(roots(root.path()));

    let outcome = add_python_auto(&store, request(&source, false, false))?;
    assert_eq!(outcome.dependencies, vec!["requests"]);
    assert_eq!(outcome.requires_python, ">=3.12,<3.13");
    assert_eq!(outcome.parameter_candidates, vec!["CITY"]);
    assert_eq!(plan_for_entry(&outcome.entry).source, PlanSource::None);

    let stored = fs::read_to_string(outcome.entry.dir.join("script.py"))?;
    assert!(stored.contains("# requires-python = \">=3.12,<3.13\""));
    assert!(stored.contains("#     \"requests\","));
    assert!(!stored.contains("[tool.skit]"));
    Ok(())
}

#[test]
fn interactive_candidates_require_review_before_any_write()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    fs::write(&source, "import requests\nCITY = 'Taipei'\n")?;
    let store = Store::new(roots(root.path()));

    let result = add_python_auto(&store, request(&source, true, false));
    assert!(matches!(
        result,
        Err(PythonAutoAddError::ReviewRequired {
            dependencies,
            parameters,
        }) if dependencies == ["requests"] && parameters == ["CITY"]
    ));
    assert!(store.list()?.is_empty());
    Ok(())
}

#[test]
fn no_input_accepts_dependency_suggestions_but_still_skips_new_managed_params()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    fs::write(&source, "import requests\nCITY = 'Taipei'\n")?;
    let store = Store::new(roots(root.path()));

    let outcome = add_python_auto(&store, request(&source, true, true))?;
    assert_eq!(outcome.dependencies, vec!["requests"]);
    assert_eq!(outcome.parameter_candidates, vec!["CITY"]);
    assert_eq!(plan_for_entry(&outcome.entry).source, PlanSource::None);
    assert!(!fs::read_to_string(outcome.entry.dir.join("script.py"))?.contains("[tool.skit]"));
    Ok(())
}

#[test]
fn existing_pep723_is_authoritative_and_existing_frozen_params_need_no_new_review()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let text = concat!(
        "# /// script\n",
        "# dependencies = [\"source-dep\"]\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "# [[tool.skit.params]]\n",
        "# name = \"CITY\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# default = \"Taipei\"\n",
        "# ///\n",
        "import requests\n",
        "CITY = 'Taipei'\n",
    );
    fs::write(&source, text)?;
    let store = Store::new(roots(root.path()));

    let outcome = add_python_auto(&store, request(&source, true, false))?;
    assert_eq!(outcome.dependencies, vec!["source-dep"]);
    assert!(outcome.requires_python.is_empty());
    assert!(outcome.parameter_candidates.is_empty());
    assert_eq!(plan_for_entry(&outcome.entry).source, PlanSource::Managed);
    assert_eq!(fs::read_to_string(outcome.entry.dir.join("script.py"))?, text);
    Ok(())
}

#[test]
fn non_utf8_intake_copies_bytes_exactly_and_invents_no_analysis()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let bytes = b"import requests\n# bad: \xff\n";
    fs::write(&source, bytes)?;
    let store = Store::new(roots(root.path()));

    let outcome = add_python_auto(&store, request(&source, false, false))?;
    assert!(outcome.dependencies.is_empty());
    assert!(outcome.requires_python.is_empty());
    assert!(outcome.parameter_candidates.is_empty());
    assert_eq!(fs::read(outcome.entry.dir.join("script.py"))?, bytes);
    Ok(())
}
