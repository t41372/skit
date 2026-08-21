use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions, UpdateEntry,
    form_state::{FormStateService, StateWriteError, scrub_secrets},
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery},
};
use skit_runtime::{
    DependencyCommand, DependencyCommandOutput, DependencyCommandRunner, ProgramProbe,
    ensure_javascript_dependencies, javascript_dependencies_need_install,
    prepare_javascript_dependency_cleanup,
};
use skit_store::{CoordinatedStateError, FileFormStateStore, FileStore};
use tempfile::TempDir;

#[derive(Debug)]
struct OfflineProbe;

impl ProgramProbe for OfflineProbe {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        (name == "npm").then(|| PathBuf::from("/offline/npm"))
    }

    fn is_file(&self, _path: &Path) -> bool {
        true
    }

    fn is_dir(&self, _path: &Path) -> bool {
        true
    }

    fn is_executable(&self, _path: &Path) -> bool {
        true
    }
}

#[derive(Debug)]
struct SeedDependencyTree;

impl DependencyCommandRunner for SeedDependencyTree {
    fn run(&self, command: &DependencyCommand) -> std::io::Result<DependencyCommandOutput> {
        fs::create_dir(command.cwd.join("node_modules"))?;
        fs::write(
            command.cwd.join("node_modules/chalk.js"),
            b"offline chalk\n",
        )?;
        fs::write(command.cwd.join("package-lock.json"), b"old lock\n")?;
        Ok(DependencyCommandOutput {
            success: true,
            exit_code: Some(0),
            stderr: Vec::new(),
        })
    }
}

fn dependency_tree(entry_dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let is_dependency_item = matches!(
                relative
                    .components()
                    .next()
                    .and_then(|part| part.as_os_str().to_str()),
                Some(
                    "package.json"
                        | "package-lock.json"
                        | "bun.lock"
                        | "bun.lockb"
                        | "deno.lock"
                        | "node_modules"
                        | ".skit-deps"
                )
            );
            if !is_dependency_item {
                continue;
            }
            if path.is_dir() {
                output.insert(relative, None);
                visit(root, &path, output);
            } else {
                output.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
    }

    let mut output = BTreeMap::new();
    visit(entry_dir, entry_dir, &mut output);
    output
}

fn registry_product_row(bytes: &[u8], slug: &str) -> toml::Table {
    let document = toml::from_str::<toml::Table>(&String::from_utf8_lossy(bytes)).unwrap();
    let mut row = document
        .get("entries")
        .and_then(toml::Value::as_table)
        .and_then(|entries| entries.get(slug))
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap();
    row.remove("mtime_ns");
    row.remove("skit_cache");
    row
}

#[test]
fn failed_state_commit_restores_javascript_cleanup_and_offline_freshness() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("data");
    let state_root = root.path().join("state");
    let store = FileStore::new(&data);
    let dependencies = vec!["chalk@^5".to_owned()];
    let mut old_settings = EntrySettings::default();
    old_settings.dependencies.clone_from(&dependencies);
    let entry = store
        .create(CreateEntry {
            name: "JavaScript rollback".to_owned(),
            kind: EntryKind::parse("js").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/tool.js".to_owned(),
            workdir: "invoke".to_owned(),
            description: "old description".to_owned(),
            payload: Some(EntryPayload {
                bytes: b"const TOKEN = 'public';\n".to_vec(),
                stored_name: Some("script.js".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: old_settings.clone(),
        })
        .unwrap();
    let unrelated = store
        .create(CreateEntry {
            name: "Concurrent registry row".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Reference,
            source: "true".to_owned(),
            workdir: "invoke".to_owned(),
            description: "old unrelated description".to_owned(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    let entry_dir = data.join("scripts/javascript-rollback");
    ensure_javascript_dependencies(
        &entry_dir,
        "node",
        &dependencies,
        &OfflineProbe,
        &SeedDependencyTree,
    )
    .unwrap();
    assert!(!javascript_dependencies_need_install(&entry_dir, "node", &dependencies).unwrap());

    let mut public = ParamDecl::new("TOKEN");
    public.binding = ParameterBinding::Const;
    public.delivery = ParameterDelivery::Inject;
    let state_store = FileFormStateStore::new(&state_root);
    FormStateService::new(state_store.clone())
        .save_last(
            &entry.slug,
            &[public.clone()],
            Some(&BTreeMap::from([(
                "TOKEN".to_owned(),
                "plaintext".to_owned(),
            )])),
            None,
            false,
        )
        .unwrap();
    let source_path = entry_dir.join("script.js");
    let meta_path = entry_dir.join("meta.toml");
    let registry_path = data.join("registry.toml");
    let state_path = state_root.join("values/javascript-rollback.toml");
    let source_before = fs::read(&source_path).unwrap();
    let meta_before = fs::read(&meta_path).unwrap();
    let registry_before = fs::read(&registry_path).unwrap();
    let state_before = fs::read(&state_path).unwrap();
    let dependencies_before = dependency_tree(&entry_dir);
    let mut secret = public;
    secret.secret = true;

    let error = state_store
        .update_after_external_commit_with(
            &entry.slug,
            || {
                let cleanup = prepare_javascript_dependency_cleanup(&entry_dir)?;
                let claimed = store.claim_identity(&entry)?;
                let updated = store.update_entry(
                    &claimed,
                    UpdateEntry {
                        name: entry.meta.name.clone(),
                        description: "new description".to_owned(),
                        settings: EntrySettings::default(),
                        workdir: entry.meta.workdir.clone(),
                        source: Some(b"const TOKEN = 'secret';\n".to_vec()),
                        expected_source_hash: claimed.meta.source_hash.clone(),
                    },
                )?;
                Ok::<_, Box<dyn std::error::Error>>((updated, cleanup))
            },
            |state| scrub_secrets(&[secret], state),
            |(updated, cleanup)| {
                let claimed = store.claim_identity(updated)?;
                store.update_entry(
                    &claimed,
                    UpdateEntry {
                        name: entry.meta.name.clone(),
                        description: entry.meta.description.clone(),
                        settings: old_settings.clone(),
                        workdir: entry.meta.workdir.clone(),
                        source: Some(source_before.clone()),
                        expected_source_hash: claimed.meta.source_hash.clone(),
                    },
                )?;
                cleanup.rollback()?;
                Ok(())
            },
            |path, _| {
                store
                    .describe(&unrelated, "newest unrelated description")
                    .unwrap();
                Err(StateWriteError::Io {
                    operation: "write",
                    path: path.display().to_string(),
                    reason: "injected state replacement failure".to_owned(),
                })
            },
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CoordinatedStateError::StateAfterCommit { rollback: None, .. }
    ));
    assert_eq!(fs::read(source_path).unwrap(), source_before);
    assert_eq!(fs::read(meta_path).unwrap(), meta_before);
    assert_eq!(fs::read(state_path).unwrap(), state_before);
    let registry_after = fs::read(registry_path).unwrap();
    assert_eq!(
        registry_product_row(&registry_after, entry.slug.as_str()),
        registry_product_row(&registry_before, entry.slug.as_str())
    );
    assert_eq!(
        registry_product_row(&registry_after, unrelated.slug.as_str())
            .get("description")
            .and_then(toml::Value::as_str),
        Some("newest unrelated description")
    );
    assert_eq!(
        store
            .resolve(unrelated.slug.as_str())
            .unwrap()
            .meta
            .description,
        "newest unrelated description"
    );
    assert_eq!(dependency_tree(&entry_dir), dependencies_before);
    assert!(!javascript_dependencies_need_install(&entry_dir, "node", &dependencies).unwrap());
}
