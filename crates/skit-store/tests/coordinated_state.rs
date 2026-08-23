use std::{collections::BTreeMap, fs};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions,
    form_state::{FormStateRepository as _, FormStateService, StateWriteError, scrub_secrets},
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery},
};
use skit_store::ExternalRollbackOutcome;
use skit_store::{CoordinatedStateError, FileFormStateStore, FileStore};
use tempfile::TempDir;

fn tree_paths(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn visit(root: &std::path::Path, path: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            output.push(path.strip_prefix(root).unwrap().to_path_buf());
            if path.is_dir() {
                visit(root, &path, output);
            }
        }
    }
    let mut output = Vec::new();
    visit(root, root, &mut output);
    output
}

fn registry_product_rows(bytes: &[u8]) -> toml::Table {
    let mut document = toml::from_str::<toml::Table>(&String::from_utf8_lossy(bytes)).unwrap();
    if let Some(row) = document
        .get_mut("entries")
        .and_then(toml::Value::as_table_mut)
        .and_then(|entries| entries.get_mut("coherent"))
        .and_then(toml::Value::as_table_mut)
    {
        row.remove("mtime_ns");
        row.remove("skit_cache");
    }
    document
}

#[test]
fn failed_state_commit_rolls_back_a_real_copy_edit_byte_exactly() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("data");
    let state_root = root.path().join("state");
    let store = FileStore::new(&data);
    let entry = store
        .create(CreateEntry {
            name: "Coherent".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"TOKEN=public\n".to_vec(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    store.claim_identity(&entry).unwrap();
    let source_path = data.join("scripts/coherent/script.sh");
    let meta_path = data.join("scripts/coherent/meta.toml");
    let source_before = fs::read(&source_path).unwrap();
    let meta_before = fs::read(&meta_path).unwrap();
    let registry_path = data.join("registry.toml");
    let registry_before = fs::read(&registry_path).unwrap();
    let tree_before = tree_paths(&data);
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
    let state_path = state_root.join("values/coherent.toml");
    let state_before = fs::read(&state_path).unwrap();
    let mut secret = public;
    secret.secret = true;

    let error = state_store
        .update_after_external_commit_with(
            &entry.slug,
            || store.commit_copy_edit(&entry, b"TOKEN=secret\n", &entry.meta.source_hash),
            |state| scrub_secrets(&[secret], state),
            |updated| {
                store
                    .commit_copy_edit(updated, &source_before, &updated.meta.source_hash)
                    .map(|_| ())
            },
            |path, _| {
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
    let registry_after = fs::read(registry_path).unwrap();
    assert_eq!(
        registry_product_rows(&registry_after),
        registry_product_rows(&registry_before),
        "rollback may refresh only the rebuildable cache proof"
    );
    assert_eq!(tree_paths(&data), tree_before);
    assert_eq!(fs::read(state_path).unwrap(), state_before);
}

#[test]
fn external_commit_failure_does_not_write_prepared_state() {
    let root = TempDir::new().unwrap();
    let state_store = FileFormStateStore::new(root.path());
    let slug = skit_domain::Slug::parse("commit-failure").unwrap();

    let error = state_store
        .update_after_external_commit_with(
            &slug,
            || Err::<(), _>("commit failed"),
            |state| {
                state
                    .values
                    .insert("TOKEN".to_owned(), "plaintext".to_owned());
            },
            |_| Ok::<(), &str>(()),
            |_, _| panic!("state must not write when the external commit fails"),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CoordinatedStateError::Commit("commit failed")
    ));
    assert!(!root.path().join("values/commit-failure.toml").exists());
}

#[test]
fn rollback_failure_is_kept_with_the_state_failure() {
    let root = TempDir::new().unwrap();
    let state_store = FileFormStateStore::new(root.path());
    let slug = skit_domain::Slug::parse("rollback-failure").unwrap();

    let error = state_store
        .update_after_external_commit_with(
            &slug,
            || Ok::<_, &str>(()),
            |state| {
                state
                    .values
                    .insert("TOKEN".to_owned(), "plaintext".to_owned());
            },
            |_| Err("rollback failed"),
            |path, _| {
                Err(StateWriteError::Io {
                    operation: "write",
                    path: path.display().to_string(),
                    reason: "state failed".to_owned(),
                })
            },
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CoordinatedStateError::StateAfterCommit {
            rollback: Some("rollback failed"),
            ..
        }
    ));
}

#[test]
fn unreadable_state_refuses_before_the_external_commit() {
    let root = TempDir::new().unwrap();
    let state_store = FileFormStateStore::new(root.path());
    let slug = skit_domain::Slug::parse("unreadable").unwrap();
    let state_path = root.path().join("values/unreadable.toml");
    fs::create_dir_all(&state_path).unwrap();

    let error = state_store
        .update_after_external_commit(
            &slug,
            || -> Result<(), &str> {
                panic!("external commit ran before the state read succeeded")
            },
            |state| {
                state
                    .values
                    .insert("TOKEN".to_owned(), "plaintext".to_owned());
            },
            |_| Ok(()),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CoordinatedStateError::State(StateWriteError::Io {
            operation: "read",
            ..
        })
    ));
    assert!(state_path.is_dir());
    assert_eq!(fs::read_dir(state_path).unwrap().count(), 0);
}

#[test]
fn refused_state_update_keeps_the_document_byte_and_semantic_exact() {
    let root = TempDir::new().unwrap();
    let state_store = FileFormStateStore::new(root.path());
    let slug = skit_domain::Slug::parse("refused-update").unwrap();
    let state_path = root.path().join("values/refused-update.toml");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let original = b"unknown = 'keep me'\n\n[values]\nTOKEN='plaintext'\n";
    fs::write(&state_path, original).unwrap();
    let semantic_before = state_store.load(&slug);

    let result = state_store
        .try_update(&slug, |state| {
            state.values.clear();
            Err::<(), _>("schema changed while the run was active")
        })
        .unwrap();

    assert_eq!(result, Err("schema changed while the run was active"));
    assert_eq!(fs::read(&state_path).unwrap(), original);
    assert_eq!(state_store.load(&slug), semantic_before);
}

#[test]
fn finalize_failure_restores_original_state_bytes_or_absence_after_external_rollback() {
    for original in [
        Some(b"unknown='keep'\n\n[values]\nTOKEN='plain'\n".as_slice()),
        None,
    ] {
        let root = TempDir::new().unwrap();
        let state_store = FileFormStateStore::new(root.path());
        let slug = skit_domain::Slug::parse("finalize-restore").unwrap();
        let state_path = root.path().join("values/finalize-restore.toml");
        if let Some(original) = original {
            fs::create_dir_all(state_path.parent().unwrap()).unwrap();
            fs::write(&state_path, original).unwrap();
        }
        let rollback_ran = std::cell::Cell::new(false);

        let error = state_store
            .update_after_external_commit_and_finalize(
                &slug,
                || Ok::<_, &str>(()),
                |state| {
                    state.values.clear();
                    state.values.insert("NEXT".to_owned(), "new".to_owned());
                },
                |_| Err("final cleanup failed"),
                |_| {
                    rollback_ran.set(true);
                    Ok(())
                },
            )
            .unwrap_err();

        assert!(rollback_ran.get());
        assert!(matches!(
            error,
            CoordinatedStateError::FinalizeAfterState {
                finalize: "final cleanup failed",
                rollback: None,
                state_rollback: None,
                authoritative_restored: true,
            }
        ));
        match original {
            Some(original) => assert_eq!(fs::read(state_path).unwrap(), original),
            None => assert!(!state_path.exists()),
        }
    }
}

#[test]
fn finalize_failure_never_restores_plaintext_state_when_external_rollback_fails() {
    let root = TempDir::new().unwrap();
    let state_store = FileFormStateStore::new(root.path());
    let slug = skit_domain::Slug::parse("unsafe-restore").unwrap();
    let state_path = root.path().join("values/unsafe-restore.toml");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let original = b"[values]\nTOKEN='plaintext'\n";
    fs::write(&state_path, original).unwrap();
    let source_is_secret = std::cell::Cell::new(false);

    let error = state_store
        .update_after_external_commit_and_finalize(
            &slug,
            || {
                source_is_secret.set(true);
                Ok::<_, &str>(())
            },
            |state| {
                state.values.clear();
            },
            |_| Err("final cleanup failed"),
            |_| Err("entry rollback failed"),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CoordinatedStateError::FinalizeAfterState {
            finalize: "final cleanup failed",
            rollback: Some("entry rollback failed"),
            state_rollback: None,
            authoritative_restored: false,
        }
    ));
    assert_ne!(fs::read(&state_path).unwrap(), original);
    assert!(
        source_is_secret.get(),
        "failed entry rollback keeps the new secret source"
    );
    assert!(!state_store.load(&slug).values.contains_key("TOKEN"));
}

#[test]
fn derived_cleanup_rollback_failure_still_restores_state_after_entry_rollback() {
    let root = TempDir::new().unwrap();
    let state_store = FileFormStateStore::new(root.path());
    let slug = skit_domain::Slug::parse("derived-rollback-failure").unwrap();
    let state_path = root.path().join("values/derived-rollback-failure.toml");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let original = b"unknown='keep'\n[values]\nTOKEN='plaintext'\n";
    fs::write(&state_path, original).unwrap();

    let error = state_store
        .update_after_external_commit_and_finalize(
            &slug,
            || Ok::<_, &str>(()),
            |state| {
                state.values.clear();
            },
            |_| Err("final cleanup failed"),
            |_| ExternalRollbackOutcome::restored_with_error("cleanup rollback failed"),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CoordinatedStateError::FinalizeAfterState {
            finalize: "final cleanup failed",
            rollback: Some("cleanup rollback failed"),
            state_rollback: None,
            authoritative_restored: true,
        }
    ));
    assert_eq!(fs::read(state_path).unwrap(), original);
}

#[test]
fn finalize_failure_reports_an_incomplete_byte_exact_state_restore() {
    let root = TempDir::new().unwrap();
    let state_store = FileFormStateStore::new(root.path());
    let slug = skit_domain::Slug::parse("state-restore-failure").unwrap();
    let state_path = root.path().join("values/state-restore-failure.toml");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let original = b"unknown='keep'\n[values]\nTOKEN='plaintext'\n";
    fs::write(&state_path, original).unwrap();

    let error = state_store
        .update_after_external_commit_and_finalize_with(
            &slug,
            || Ok::<_, &str>(()),
            |state| {
                state.values.clear();
            },
            |_| Err("final cleanup failed"),
            |_| Ok(()),
            |path, bytes| {
                fs::write(path, bytes).map_err(|error| StateWriteError::Io {
                    operation: "write",
                    path: path.display().to_string(),
                    reason: error.to_string(),
                })
            },
            |path, _| {
                Err(StateWriteError::Io {
                    operation: "rollback write",
                    path: path.display().to_string(),
                    reason: "injected restore failure".to_owned(),
                })
            },
        )
        .unwrap_err();

    assert!(matches!(
        error,
        CoordinatedStateError::FinalizeAfterState {
            finalize: "final cleanup failed",
            rollback: None,
            state_rollback: Some(StateWriteError::Io {
                operation: "rollback write",
                ..
            }),
            authoritative_restored: true,
        }
    ));
    assert_ne!(fs::read(&state_path).unwrap(), original);
    assert!(!state_store.load(&slug).values.contains_key("TOKEN"));
}

fn test_state_writer(path: &std::path::Path, bytes: &[u8]) -> Result<(), StateWriteError> {
    fs::create_dir_all(path.parent().unwrap()).map_err(|error| StateWriteError::Io {
        operation: "write",
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    fs::write(path, bytes).map_err(|error| StateWriteError::Io {
        operation: "write",
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

#[test]
fn unchanged_state_still_commits_and_finalizes_or_rolls_back_the_external_value() {
    let root = TempDir::new().unwrap();
    let state_store = FileFormStateStore::new(root.path());
    let slug = skit_domain::Slug::parse("unchanged").unwrap();

    let success = state_store
        .update_after_external_commit_with(
            &slug,
            || Ok::<_, &str>("committed"),
            |_| "unchanged result",
            |_| Ok(()),
            test_state_writer,
        )
        .unwrap();
    assert_eq!(success, ("committed", "unchanged result"));
    assert!(!root.path().join("values/unchanged.toml").exists());

    let rolled_back = std::cell::Cell::new(false);
    let error = state_store
        .update_after_external_commit_and_finalize(
            &slug,
            || Ok::<_, &str>("committed"),
            |_| "unchanged result",
            |_| Err("finalize failed"),
            |_| {
                rolled_back.set(true);
                Ok(())
            },
        )
        .unwrap_err();
    assert!(rolled_back.get());
    assert!(matches!(
        error,
        CoordinatedStateError::FinalizeAfterState {
            finalize: "finalize failed",
            rollback: None,
            state_rollback: None,
            authoritative_restored: true,
        }
    ));
    assert!(!root.path().join("values/unchanged.toml").exists());
}

#[test]
fn state_restore_handles_a_finalize_that_removes_or_replaces_the_new_state_file() {
    for replace_with_directory in [false, true] {
        let root = TempDir::new().unwrap();
        let state_store = FileFormStateStore::new(root.path());
        let slug = skit_domain::Slug::parse("finalize-path-change").unwrap();
        let state_path = root.path().join("values/finalize-path-change.toml");

        let error = state_store
            .update_after_external_commit_and_finalize(
                &slug,
                || Ok::<_, &str>(()),
                |state| {
                    state.values.insert("NEXT".to_owned(), "value".to_owned());
                },
                |_| {
                    fs::remove_file(&state_path).unwrap();
                    if replace_with_directory {
                        fs::create_dir(&state_path).unwrap();
                    }
                    Err("finalize changed state path")
                },
                |_| Ok(()),
            )
            .unwrap_err();

        if replace_with_directory {
            assert!(matches!(
                error,
                CoordinatedStateError::FinalizeAfterState {
                    state_rollback: Some(StateWriteError::Io {
                        operation: "rollback remove",
                        ..
                    }),
                    authoritative_restored: true,
                    ..
                }
            ));
            assert!(state_path.is_dir());
        } else {
            assert!(matches!(
                error,
                CoordinatedStateError::FinalizeAfterState {
                    state_rollback: None,
                    authoritative_restored: true,
                    ..
                }
            ));
            assert!(!state_path.exists());
        }
    }
}
