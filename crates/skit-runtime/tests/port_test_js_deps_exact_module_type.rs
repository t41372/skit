//! Exact module-type and package-manifest ports from Python v0.4 `tests/test_js_deps.py`.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value as JsonValue;
use skit_runtime::{
    DependencyCommand, DependencyCommandRunner, JavaScriptModuleType, ProgramProbe,
    ensure_javascript_dependencies_for_module, javascript_dependency_manifest,
    javascript_module_type,
};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct Probe {
    programs: BTreeMap<String, PathBuf>,
}

impl ProgramProbe for Probe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn is_executable(&self, _path: &Path) -> bool {
        true
    }
}

#[derive(Debug, Default)]
struct Runner {
    commands: RefCell<Vec<DependencyCommand>>,
}

impl DependencyCommandRunner for Runner {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<bool> {
        self.commands.borrow_mut().push(command.clone());
        fs::create_dir_all(command.cwd.join("node_modules"))?;
        Ok(true)
    }
}

fn probe() -> Probe {
    Probe {
        programs: BTreeMap::from([("npm".to_owned(), PathBuf::from("/bin/npm"))]),
    }
}

fn materialize(
    root: &TempDir,
    dependencies: &[&str],
    module_type: Option<JavaScriptModuleType>,
) -> String {
    ensure_javascript_dependencies_for_module(
        root.path(),
        "node",
        &dependencies.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>(),
        module_type,
        &BTreeMap::new(),
        &probe(),
        &Runner::default(),
    )
    .unwrap();
    fs::read_to_string(root.path().join("package.json")).unwrap()
}

#[test]
fn test_module_type_for() {
    for (source, expected) in [
        ("/home/u/tool.mjs", Some(JavaScriptModuleType::Module)),
        ("/home/u/tool.MJS", Some(JavaScriptModuleType::Module)),
        (r"C:\u\tool.cjs", Some(JavaScriptModuleType::CommonJs)),
        ("/home/u/tool.mts", Some(JavaScriptModuleType::Module)),
        ("/home/u/tool.cts", Some(JavaScriptModuleType::CommonJs)),
        ("/home/u/tool.js", None),
        ("noext", None),
        ("", None),
    ] {
        assert_eq!(javascript_module_type(source), expected, "source={source:?}");
    }
}

#[test]
fn test_manifest_text_carries_the_module_type() {
    let root = TempDir::new().unwrap();
    let typed = materialize(&root, &["chalk"], Some(JavaScriptModuleType::Module));
    assert!(typed.contains("\"type\": \"module\""), "{typed}");

    let plain = javascript_dependency_manifest(&["chalk".to_owned()]).unwrap();
    assert!(!plain.contains("\"type\""), "{plain}");
}

#[test]
fn test_split_requirement_boundary_shapes() {
    for (requirement, expected_name, expected_version) in [
        ("a@5", "a", "5"),
        ("foo/@2", "foo/@2", "*"),
    ] {
        let manifest = javascript_dependency_manifest(&[requirement.to_owned()])
            .unwrap_or_else(|error| panic!("frozen boundary requirement {requirement:?} was rejected: {error}"));
        let value: JsonValue = serde_json::from_str(&manifest).unwrap();
        let rows = value["dependencies"].as_object().unwrap();
        assert_eq!(rows.len(), 1, "{manifest}");
        assert_eq!(
            rows.get(expected_name).and_then(JsonValue::as_str),
            Some(expected_version),
            "requirement={requirement:?}; manifest={manifest}"
        );
    }
}

#[test]
fn test_module_type_for_multi_dot_sources() {
    for (source, expected) in [
        ("/home/u.name/tool.v2.mjs", JavaScriptModuleType::Module),
        ("archive.tar.cjs", JavaScriptModuleType::CommonJs),
    ] {
        assert_eq!(javascript_module_type(source), Some(expected), "source={source:?}");
    }
}

#[test]
fn test_manifest_text_exact_layout() {
    let root = TempDir::new().unwrap();
    let manifest = materialize(&root, &["chalk@^5"], Some(JavaScriptModuleType::Module));
    assert_eq!(
        manifest,
        concat!(
            "{\n",
            "  \"private\": true,\n",
            "  \"type\": \"module\",\n",
            "  \"dependencies\": {\n",
            "    \"chalk\": \"^5\"\n",
            "  }\n",
            "}\n",
        )
    );
}

#[test]
fn test_ensure_installed_writes_the_module_type_into_the_manifest() {
    let root = TempDir::new().unwrap();
    let manifest = materialize(&root, &["chalk"], Some(JavaScriptModuleType::Module));
    assert!(manifest.contains("\"type\": \"module\""), "{manifest}");
}

#[test]
fn test_module_type_for_a_bare_dotfile_name() {
    assert_eq!(javascript_module_type(".mjs"), Some(JavaScriptModuleType::Module));
}

#[test]
fn test_ensure_module_manifest_writes_the_type() {
    let root = TempDir::new().unwrap();
    let manifest = materialize(&root, &[], Some(JavaScriptModuleType::CommonJs));
    let parsed: JsonValue = serde_json::from_str(&manifest).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({"private": true, "type": "commonjs"})
    );
}

#[test]
fn test_ensure_module_manifest_flavorless_writes_nothing() {
    let root = TempDir::new().unwrap();
    ensure_javascript_dependencies_for_module(
        root.path(),
        "node",
        &[],
        None,
        &BTreeMap::new(),
        &Probe::default(),
        &Runner::default(),
    )
    .unwrap();
    assert!(!root.path().join("package.json").exists());
}

#[test]
fn test_ensure_module_manifest_rewrites_a_non_utf8_package_json() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("package.json"), b"\xff\xfe{\"private\":true}").unwrap();
    let manifest = materialize(&root, &[], Some(JavaScriptModuleType::Module));
    let parsed: JsonValue = serde_json::from_str(&manifest).unwrap();
    assert_eq!(parsed, serde_json::json!({"private": true, "type": "module"}));
}