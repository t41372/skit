use std::{collections::BTreeMap, fs};

use skit_application::form_state::FormStateService;
use skit_application::{EntryRepository, LibraryService};
use skit_benchmarks::dataset::{
    DEFAULT_SEED, DEFAULT_STATE_FRACTION, DatasetError, DatasetManifest, GENERATOR_VERSION,
    RUNOVER_JAVASCRIPT, RUNOVER_PYTHON, RUNOVER_SHELL, SEARCH_PROBE_CHAR, check_reusable,
    dataset_dirs, generate, generate_command_only, generate_runover, kind_slots,
};
use skit_domain::{EntrySettings, StorageMode, parameters::ParameterType};
use skit_store::{FileFormStateStore, FileStore};
use tempfile::TempDir;

#[test]
fn kind_grid_preserves_latest_main_mix_and_is_seed_deterministic() {
    let first = kind_slots(DEFAULT_SEED, 100);
    let second = kind_slots(DEFAULT_SEED, 100);
    assert_eq!(first, second);
    assert_eq!(
        &first[..12],
        [
            "lua", "r", "ts", "python", "command", "ts", "prompt", "shell", "prompt", "python",
            "python", "python",
        ],
        "the kind grid must consume the same Python Random stream as latest main"
    );
    let counts = first.into_iter().fold(BTreeMap::new(), |mut counts, kind| {
        *counts.entry(kind).or_insert(0_usize) += 1;
        counts
    });
    assert_eq!(counts["python"], 30);
    assert_eq!(counts["shell"], 20);
    assert_eq!(counts["js"], 10);
    assert_eq!(counts["ts"], 5);
    assert_eq!(counts["command"], 10);
    assert_eq!(counts["prompt"], 10);
    assert_eq!(counts["fish"], 5);
    assert_eq!(counts["exe"], 6);
    for kind in ["ruby", "perl", "lua", "r"] {
        assert_eq!(counts[kind], 1);
    }
}

#[test]
fn generated_library_keeps_latest_main_rng_sequence_and_state_selection() {
    let root = TempDir::new().unwrap();
    let dataset = root.path().join("oracle");
    let manifest = generate(&dataset, 12, DEFAULT_SEED, DEFAULT_STATE_FRACTION).unwrap();
    assert_eq!(manifest.middle_slug().unwrap(), &manifest.slugs[6]);
    let expected = [
        ("python", "alpha-seed-0", ""),
        ("shell", "alpha-1", "runs the bravo task"),
        (
            "shell",
            "omega-alpha-sigma-2",
            "a long description that tells what this entry does, why it was added, and when to reach for it during daily work, in enough words to wrap a line",
        ),
        ("command", "測試腳本-3", ""),
        ("fish", "bravo-delta-gamma-4", "runs the delta task"),
        (
            "command",
            "🚀-tool-5",
            "a long description that tells what this entry does, why it was added, and when to reach for it during daily work, in enough words to wrap a line",
        ),
        ("python", "kilo-omega-lima-6", ""),
        ("command", "gamma-7", "runs the delta task"),
        (
            "fish",
            "delta-kilo-bravo-8",
            "a long description that tells what this entry does, why it was added, and when to reach for it during daily work, in enough words to wrap a line",
        ),
        ("python", "kilo-9", ""),
        ("command", "測試腳本-10", "runs the omega task"),
        (
            "command",
            "delta-11",
            "a long description that tells what this entry does, why it was added, and when to reach for it during daily work, in enough words to wrap a line",
        ),
    ];
    let dirs = dataset_dirs(&dataset).unwrap();
    let store = FileStore::new(dirs.data);
    let scan = store.scan().unwrap();
    let state = FormStateService::new(FileFormStateStore::new(dirs.state));
    for (index, (kind, name, description)) in expected.into_iter().enumerate() {
        let slug = &manifest.slugs[index];
        let summary = scan
            .entries
            .iter()
            .find(|entry| &entry.slug == slug)
            .unwrap();
        assert_eq!(manifest.kinds[slug.as_str()], kind);
        assert_eq!(summary.name, name);
        assert_eq!(summary.description, description);
        assert_eq!(state.last_run(slug).at.is_some(), matches!(index, 0 | 2));
        if kind == "command" {
            let entry = store.resolve(slug.as_str()).unwrap();
            assert_eq!(
                EntrySettings::from_meta(&entry.meta).template,
                format!("echo {{msg}} entry-{index}")
            );
        }
    }
}

#[test]
fn generated_library_uses_public_store_and_state_apis_and_holds_probe_invariants() {
    let root = TempDir::new().unwrap();
    let dataset = root.path().join("dataset");
    let manifest = generate(&dataset, 12, DEFAULT_SEED, 1.0).unwrap();

    assert_eq!(manifest.n, 12);
    assert_eq!(manifest.generator_version, GENERATOR_VERSION);
    assert_eq!(manifest.probe_char, SEARCH_PROBE_CHAR);
    assert_eq!(DatasetManifest::load(&dataset).unwrap(), manifest);
    assert!(check_reusable(&manifest, 12).is_err());

    let dirs = dataset_dirs(&dataset).unwrap();
    let scan = LibraryService::new(FileStore::new(&dirs.data))
        .list()
        .unwrap();
    assert_eq!(scan.entries.len(), 12);
    let probe_free = scan
        .entries
        .iter()
        .find(|entry| entry.slug == manifest.slugs[0])
        .unwrap();
    assert_eq!(probe_free.name, "alpha-seed-0");
    assert!(!format!("{} {}", probe_free.name, probe_free.description).contains(SEARCH_PROBE_CHAR));
    assert!(scan.entries.iter().any(|entry| {
        format!("{} {}", entry.name, entry.description).contains(SEARCH_PROBE_CHAR)
    }));

    let state = FormStateService::new(FileFormStateStore::new(&dirs.state));
    assert!(
        manifest
            .slugs
            .iter()
            .all(|slug| state.last_run(slug).at.is_some())
    );
    assert_eq!(
        state.last_run(&manifest.slugs[9]).at.as_deref(),
        Some("2026-01-01T09:00:00+00:00")
    );
    assert_eq!(
        state
            .load(&manifest.slugs[9])
            .last_run
            .values
            .as_ref()
            .unwrap()["count"],
        "2"
    );
}

#[test]
fn full_kind_grid_keeps_reference_targets_and_exact_parameter_shapes() {
    let root = TempDir::new().unwrap();
    let dataset = root.path().join("full-grid");
    let manifest = generate(&dataset, 100, DEFAULT_SEED, 0.0).unwrap();
    let store = FileStore::new(dataset_dirs(&dataset).unwrap().data);
    let mut executable_targets = Vec::new();

    for (index, slug) in manifest.slugs.iter().enumerate() {
        let kind = manifest.kinds[slug.as_str()].as_str();
        let entry = store.resolve(slug.as_str()).unwrap();
        let settings = EntrySettings::from_meta(&entry.meta);
        let names = settings
            .parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>();
        if kind == "prompt" {
            assert_eq!(names, ["path", "topic"]);
        } else if matches!(kind, "python" | "shell" | "js" | "ts" | "fish")
            && index.is_multiple_of(3)
        {
            assert_eq!(names, ["count", "verbose"]);
            assert_eq!(settings.parameters[0].flag, "--count");
            assert_eq!(settings.parameters[1].flag, "--verbose");
            assert_eq!(settings.parameters[1].parameter_type, ParameterType::Bool);
            assert_eq!(settings.parameters[1].action, "store_true");
        } else {
            assert!(
                names.is_empty(),
                "unexpected parameters for {kind} at {index}"
            );
        }

        if kind == "exe" {
            assert_eq!(entry.meta.mode, StorageMode::Reference);
            executable_targets.push(entry.meta.source);
        }
    }

    assert_eq!(executable_targets.len(), 6);
    assert!(!std::path::Path::new(&executable_targets[0]).exists());
    assert!(
        executable_targets[1..]
            .iter()
            .all(|source| std::path::Path::new(source).exists())
    );
    #[cfg(unix)]
    for source in &executable_targets[1..] {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(source).unwrap().permissions().mode();
        assert_ne!(mode & 0o111, 0, "{source} must remain executable");
    }
}

#[test]
fn generation_is_deterministic_and_refuses_unsafe_reuse() {
    let root = TempDir::new().unwrap();
    let first = generate(&root.path().join("first"), 20, DEFAULT_SEED, 0.0).unwrap();
    let second = generate(&root.path().join("second"), 20, DEFAULT_SEED, 0.0).unwrap();
    assert_eq!(first.slugs, second.slugs);
    assert_eq!(first.kinds, second.kinds);

    let nonempty = root.path().join("nonempty");
    fs::create_dir(&nonempty).unwrap();
    fs::write(nonempty.join("owned.txt"), "keep").unwrap();
    assert!(matches!(
        generate(&nonempty, 1, DEFAULT_SEED, DEFAULT_STATE_FRACTION),
        Err(DatasetError::NonEmpty(_))
    ));
    assert_eq!(
        fs::read_to_string(nonempty.join("owned.txt")).unwrap(),
        "keep"
    );
    assert!(generate(&root.path().join("negative"), -1, DEFAULT_SEED, 0.5).is_err());
    assert!(generate(&root.path().join("fraction"), 1, DEFAULT_SEED, 1.1).is_err());
    assert!(
        generate(
            &root.path().join("negative-fraction"),
            1,
            DEFAULT_SEED,
            -0.1
        )
        .is_err()
    );
    assert!(generate(&root.path().join("nan-fraction"), 1, DEFAULT_SEED, f64::NAN).is_err());

    let existing_empty = root.path().join("existing-empty");
    fs::create_dir(&existing_empty).unwrap();
    assert!(generate(&existing_empty, 0, DEFAULT_SEED, DEFAULT_STATE_FRACTION).is_ok());
}

#[test]
fn manifest_loader_rejects_missing_corrupt_and_semantically_incomplete_stamps() {
    let root = TempDir::new().unwrap();
    assert!(matches!(
        DatasetManifest::load(&root.path().join("missing")),
        Err(DatasetError::Io {
            operation: "read",
            ..
        })
    ));

    let corrupt = root.path().join("corrupt");
    fs::create_dir(&corrupt).unwrap();
    fs::write(corrupt.join("manifest.json"), "{").unwrap();
    assert!(matches!(
        DatasetManifest::load(&corrupt),
        Err(DatasetError::UnreadableManifest { .. })
    ));

    let valid_root = root.path().join("valid");
    let valid = generate(&valid_root, 1, DEFAULT_SEED, DEFAULT_STATE_FRACTION).unwrap();
    assert!(valid.to_json().unwrap().ends_with('\n'));
    assert!(valid.root.is_absolute());

    let mut document: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(valid_root.join("manifest.json")).unwrap())
            .unwrap();
    document["slugs"] = serde_json::json!([]);
    fs::write(
        valid_root.join("manifest.json"),
        serde_json::to_string(&document).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        DatasetManifest::load(&valid_root),
        Err(DatasetError::UnreadableManifest { .. })
    ));

    for (field, value) in [
        ("probe_char", serde_json::json!("")),
        ("state_fraction", serde_json::json!(1.1)),
    ] {
        let mut document: serde_json::Value =
            serde_json::from_str(&valid.to_json().unwrap()).unwrap();
        document[field] = value;
        fs::write(valid_root.join("manifest.json"), document.to_string()).unwrap();
        assert!(matches!(
            DatasetManifest::load(&valid_root),
            Err(DatasetError::UnreadableManifest { .. })
        ));
    }

    document["slugs"] = serde_json::json!([valid.slugs[0].as_str()]);
    document["generator_version"] = 0.into();
    fs::write(
        valid_root.join("manifest.json"),
        serde_json::to_string(&document).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        DatasetManifest::load(&valid_root),
        Err(DatasetError::UnreadableManifest { .. })
    ));
}

#[test]
fn reusable_stamp_and_empty_middle_are_explicit_contracts() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("empty"),
        0,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    assert!(matches!(
        manifest.middle_slug(),
        Err(DatasetError::EmptyMiddle)
    ));
    check_reusable(&manifest, 0).unwrap();

    let occupied = root.path().join("occupied");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("owned"), "keep").unwrap();
    assert!(matches!(
        generate_runover(occupied.clone()),
        Err(DatasetError::NonEmpty(_))
    ));
    assert!(matches!(
        generate_command_only(occupied, 1),
        Err(DatasetError::NonEmpty(_))
    ));
}

#[test]
fn runover_dataset_has_exactly_the_three_product_lanes() {
    let root = TempDir::new().unwrap();
    let dataset = generate_runover(root.path().join("runover")).unwrap();
    assert_eq!(dataset.n, 3);
    assert_eq!(
        dataset
            .slugs
            .iter()
            .map(|slug| dataset.kinds[slug.as_str()].as_str())
            .collect::<Vec<_>>(),
        ["python", "shell", "js"]
    );
    let dirs = dataset_dirs(&dataset.root).unwrap();
    let store = FileStore::new(dirs.data);
    assert_eq!(store.scan().unwrap().entries.len(), 3);
    for (index, expected) in [RUNOVER_PYTHON, RUNOVER_SHELL, RUNOVER_JAVASCRIPT]
        .into_iter()
        .enumerate()
    {
        let entry = store.resolve(dataset.slugs[index].as_str()).unwrap();
        assert_eq!(
            fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap(),
            expected
        );
    }
}

#[test]
fn command_only_dataset_is_a_public_api_reference_mode_worst_case() {
    let root = TempDir::new().unwrap();
    let dataset = generate_command_only(root.path().join("commands"), 20).unwrap();
    let store = FileStore::new(dataset_dirs(&dataset.root).unwrap().data);
    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.len(), 20);
    for summary in scan.entries {
        let entry = store.resolve(summary.slug.as_str()).unwrap();
        assert_eq!(entry.meta.kind.as_str(), "command");
        assert_eq!(entry.meta.mode, StorageMode::Reference);
        let index: usize = summary.name.strip_prefix("cmd-").unwrap().parse().unwrap();
        assert_eq!(
            EntrySettings::from_meta(&entry.meta).template,
            format!("echo {index} {{arg}}")
        );
    }
}
