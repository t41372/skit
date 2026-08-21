use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs, io,
    io::{Read as _, Write as _},
    path::Path,
};

use clap::{CommandFactory as _, Parser as _};
use skit_application::{ExitClass, LibraryService, RepositoryError};
use skit_domain::{
    Entry, EntryKind, EntryMeta, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_store::FileStore;
use skit_ui::{
    DraftDeleteOutcome, FormControl, FormField, FormPurpose, FormView, KnownEntryKind, Screen,
    SettingsSectionId, SettingsView,
};
use tempfile::TempDir;

use super::*;

fn write_meta(root: &TempDir, slug: &str, name: &str, description: &str) {
    let directory = root.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        format!(
            "name = {name:?}\nkind = \"command\"\nmode = \"copy\"\ndescription = {description:?}\n"
        ),
    )
    .unwrap();
    FileStore::new(root.path()).rebuild_registry().unwrap();
}

/// The add options a command line produces when it names nothing but the lane.
fn add_options() -> AddOptions {
    AddOptions {
        source: None,
        kind: None,
        name: None,
        description: None,
        reference: false,
        command_template: None,
        prompt: false,
        executable: false,
        runner: None,
        no_interpolate: false,
        dependencies: Vec::new(),
        dependencies_explicit: false,
        requires_python: None,
        no_input: false,
    }
}

fn add_command(service: &LibraryService<FileStore>, name: &str, template: &str) -> Entry {
    add(
        service,
        AddOptions {
            source: None,
            kind: Some("command".to_owned()),
            name: Some(name.to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: Some(template.to_owned()),
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    service.show(name).unwrap()
}

/// Put one text value on a submission, the way a text control would.
fn set(values: &mut SubmittedValues, key: &str, value: &str) {
    values.insert(key.to_owned(), FieldValue::text(value));
}

/// Open the entry-settings screen the way the composition root does.
fn settings_view(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    selector: &str,
) -> SettingsView {
    let entry = service.show(selector).unwrap();
    // Every test lays its directories out as `<root>/state` beside `<root>/config`.
    let config_dir = state_dir.parent().unwrap_or(state_dir).join("config");
    let Screen::Settings(view) =
        tui_settings_screen(service, store, &config_dir, state_dir, &entry, None).unwrap()
    else {
        panic!("settings must open the typed screen");
    };
    *view
}

/// Drive the settings screen the way a person does: open it, move some controls, then save.
///
/// The submission carries only what moved, which is the contract every axis of the save depends on.
fn settings_edits(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    selector: &str,
    edits: &[(&str, &str)],
) -> SubmittedValues {
    let mut view = settings_view(service, store, state_dir, selector);
    for (key, value) in edits {
        assert!(
            view.set_value(key, FieldValue::text(*value)),
            "the settings screen offers no {key} control"
        );
    }
    view.submitted_values()
}

/// Read one submitted value as the text an axis stores.
fn got(values: &SubmittedValues, key: &str) -> String {
    values.get(key).map(FieldValue::as_text).unwrap_or_default()
}

#[test]
fn completion_candidates_include_each_entry_slug_and_display_name() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "alpha", "Alpha tool", "Human description");
    let candidates = entry_candidates_from(&FileStore::new(root.path()));
    let values = candidates
        .iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(values, ["alpha", "Alpha tool"]);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.get_help().is_some())
    );
}

#[test]
fn completion_candidates_include_prompt_runners_and_saved_presets() {
    let root = TempDir::new().unwrap();
    fs::write(
        root.path().join("config.toml"),
        concat!(
            "[[prompt.runners]]\n",
            "name = \"codex\"\n",
            "argv = [\"codex\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let runners = runner_candidates_from(&skit_store::FileConfigStore::new(root.path()))
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(runners.contains(&"codex".to_owned()));
    assert_eq!(
        runners
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        runners.len()
    );

    let values = root.path().join("state/values");
    fs::create_dir_all(&values).unwrap();
    fs::write(
        values.join("alpha.toml"),
        "[presets.fast]\ncount = \"1\"\n[presets.safe]\ncount = \"2\"\n",
    )
    .unwrap();
    fs::write(
        values.join("beta.toml"),
        "[presets.fast]\ncount = \"3\"\n[presets.thorough]\ncount = \"4\"\n",
    )
    .unwrap();
    let presets = preset_candidates_from(&root.path().join("state"))
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(presets, ["fast", "safe", "thorough"]);
}

#[test]
fn completion_adapters_follow_each_latest_main_degradation_contract() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("scripts"), "not a directory").unwrap();
    assert!(entry_candidates_from(&FileStore::new(root.path())).is_empty());

    let config = root.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("config.toml"), "[").unwrap();
    let candidates = runner_candidates_from(&FileConfigStore::new(&config))
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(candidates.contains(&"claude".to_owned()));
    assert!(candidates.contains(&"codex".to_owned()));
    assert_eq!(fs::read_to_string(config.join("config.toml")).unwrap(), "[");

    let state = root.path().join("state");
    assert!(preset_candidates_from(&state).is_empty());
    let values = state.join("values");
    fs::create_dir_all(&values).unwrap();
    fs::write(values.join("ignore.txt"), "[presets]\n").unwrap();
    fs::write(values.join("binary.toml"), [0xff]).unwrap();
    fs::write(values.join("invalid.toml"), "[").unwrap();
    fs::write(values.join("empty.toml"), "future = true\n").unwrap();
    fs::write(
        values.join("mixed.toml"),
        "[presets]\nnot_a_table = 1\n[presets.valid]\nvalue = \"x\"\n",
    )
    .unwrap();
    fs::create_dir(values.join("directory.toml")).unwrap();
    let candidates = preset_candidates_from(&state)
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(candidates, ["valid"]);
}

#[test]
fn zsh_completion_keeps_runtime_validated_targets_without_parser_conflict_groups() {
    let mut output = Vec::new();
    write_completion(Shell::Zsh, &mut output);
    let output = String::from_utf8(output).unwrap();
    let row_spec = concat!(
        "'--row=[Remove one malformed raw row by its zero-based index or ",
        "\\`container\\`]:ROW:_default' \\\n"
    );

    assert!(output.contains(row_spec), "{output}");
    assert!(!output.contains(&format!("(){}", row_spec)), "{output}");
    assert!(
        output.contains("'::name -- Stable runner name:_default' \\\n"),
        "{output}"
    );

    for spec in [
        "'--cmd=[Register a command template instead of a file]:COMMAND_TEMPLATE:_default' \\\n",
        "'-e[Write a new source in the configured editor, then add it]' \\\n",
        "'--edit[Write a new source in the configured editor, then add it]' \\\n",
        "'--prompt[Treat the source as a prompt entry]' \\\n",
        "'--exe[Force executable kind inference]' \\\n",
    ] {
        assert!(output.contains(spec), "missing spec {spec:?}: {output}");
    }
    for stale in [
        "(--prompt --exe --kind --runner --no-interpolate)--cmd=",
        "(--cmd --prompt --ref --exe --kind --runner)-e",
        "(--cmd --prompt --ref --exe --kind --runner)--edit",
        "(--exe --kind)--prompt",
        "(--prompt --kind --runner --no-interpolate)--exe",
    ] {
        assert!(
            !output.contains(stale),
            "stale parser conflict {stale}: {output}"
        );
    }
}

#[test]
fn destructive_and_create_prompts_have_explicit_automation_paths() {
    assert!(user_confirmed("y", false));
    assert!(user_confirmed("YES", false));
    assert!(user_confirmed("", true));
    assert!(!user_confirmed("", false));
    assert!(!user_confirmed("no", true));

    let remove = Cli::try_parse_from(["skit", "remove", "alpha", "-y", "--no-input"]).unwrap();
    assert!(matches!(
        remove.command,
        Some(Command::Remove {
            yes: true,
            no_input: true,
            ..
        })
    ));
    let edit = Cli::try_parse_from(["skit", "edit", "new-tool", "--no-input"]).unwrap();
    assert!(matches!(
        edit.command,
        Some(Command::Edit { no_input: true, .. })
    ));
}

#[test]
fn every_clap_help_message_has_complete_chinese_catalog_rows() {
    fn collect(command: &clap::Command, output: &mut Vec<String>) {
        if let Some(about) = command.get_about() {
            output.push(about.to_string());
        }
        if let Some(about) = command.get_long_about() {
            output.push(about.to_string());
        }
        for argument in command.get_arguments() {
            if let Some(help) = argument.get_help() {
                output.push(help.to_string());
            }
            if let Some(help) = argument.get_long_help() {
                output.push(help.to_string());
            }
        }
        for child in command.get_subcommands() {
            collect(child, output);
        }
    }

    let mut messages = Vec::new();
    collect(&Cli::command(), &mut messages);
    messages.sort();
    messages.dedup();
    let missing = messages
        .into_iter()
        .filter(|message| {
            skit_i18n::text(skit_i18n::Locale::ZhCn, message).as_ref() == message
                || skit_i18n::text(skit_i18n::Locale::ZhTw, message).as_ref() == message
        })
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "missing help translations:\n{missing:#?}"
    );
}

#[test]
fn source_helpers_preserve_bytes_names_and_storage_conventions() {
    let root = TempDir::new().unwrap();
    let source = root.path().join("archive.custom");
    fs::write(&source, b"alpha\r\nbeta\r\n").unwrap();

    let snapshot = read_source(&source, false, false).unwrap();

    assert_eq!(snapshot.bytes, b"alpha\r\nbeta\r\n");
    assert_eq!(source_default_name(&source, false), "archive");
    assert_eq!(source_default_name(Path::new(""), false), "script");
    assert_eq!(
        source_default_name(Path::new("review.prompt.md"), true),
        "review"
    );
    assert_eq!(
        [
            ("python", "script.py"),
            ("shell", "script.sh"),
            ("js", "script.js"),
            ("ts", "script.ts"),
            ("fish", "script.fish"),
            ("powershell", "script.ps1"),
            ("ruby", "script.rb"),
            ("perl", "script.pl"),
            ("lua", "script.lua"),
            ("r", "script.r"),
            ("future-kind", "payload"),
        ]
        .map(|(kind, _)| { payload_stored_name(&EntryKind::parse(kind).unwrap(), &source) }),
        [
            "script.py",
            "script.sh",
            "script.js",
            "script.ts",
            "script.fish",
            "script.ps1",
            "script.rb",
            "script.pl",
            "script.lua",
            "script.r",
            "payload",
        ]
        .map(str::to_owned)
    );
    #[cfg(unix)]
    assert!(snapshot.permissions.unix_mode.is_some());
    #[cfg(not(unix))]
    assert!(snapshot.permissions.unix_mode.is_none());

    let missing = root.path().join("missing");
    let error = read_source(&missing, false, false).unwrap_err();
    assert!(matches!(
        error,
        CliError::Source {
            operation: "inspect",
            ..
        }
    ));
}

#[cfg(unix)]
#[test]
fn source_intake_errors_keep_real_files_and_typed_resolution_context() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

    let root = TempDir::new().unwrap();
    let directory = root.path().join("directory");
    fs::create_dir(&directory).unwrap();
    let error = read_source(&directory, false, true).unwrap_err();
    assert_eq!(
        error.message().localize(Locale::En),
        format!("Not a file: {}", directory.display())
    );
    let snapshot = read_source(&directory, true, false).unwrap();
    assert!(snapshot.bytes.is_empty());
    assert!(!snapshot.is_regular);
    let error = read_source(&directory, false, false).unwrap_err();
    assert!(
        error
            .message()
            .localize(Locale::En)
            .starts_with(&format!("Can't read {}: ", directory.display()))
    );

    let missing = root.path().join("missing.py");
    let error = resolve_add_source(&missing).unwrap_err();
    assert_eq!(
        error.message().localize(Locale::En),
        format!("File not found: {}", missing.display())
    );
    let invalid = root
        .path()
        .join(OsString::from_vec(b"invalid-\0-path.py".to_vec()));
    let error = resolve_add_source(&invalid).unwrap_err();
    assert!(matches!(
        error,
        CliError::Source {
            operation: "resolve",
            ..
        }
    ));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn add_uses_the_shared_kind_aware_description_and_keeps_an_explicit_empty_value() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path().join("library"));
    let service = LibraryService::new(store);
    let config = root.path().join("config");
    let derived = root.path().join("derived.py");
    fs::write(&derived, b"\"\"\"Derived documentation.\"\"\"\nprint(1)\n").unwrap();

    add_with_config(
        &service,
        &config,
        AddOptions {
            source: Some(derived),
            kind: None,
            name: Some("Derived".into()),
            description: None,
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: true,
        },
    )
    .unwrap();
    assert_eq!(
        service.show("Derived").unwrap().meta.description,
        "Derived documentation."
    );

    let explicit = root.path().join("explicit.py");
    fs::write(&explicit, b"\"\"\"Must not win.\"\"\"\nprint(1)\n").unwrap();
    add_with_config(
        &service,
        &config,
        AddOptions {
            source: Some(explicit),
            kind: None,
            name: Some("Explicit empty".into()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: true,
        },
    )
    .unwrap();
    assert_eq!(service.show("Explicit empty").unwrap().meta.description, "");
}

#[test]
fn bare_add_refuses_every_unapplied_flag_and_lists_only_compatible_lanes() {
    let mut options = empty_add_options();
    options.name = Some("Named".to_owned());
    options.dependencies = vec!["requests".to_owned()];
    options.dependencies_explicit = true;
    let error = refuse_bare_add_flags(&options).unwrap_err();
    let CliError::Usage(message) = error else {
        panic!("expected a usage refusal");
    };
    assert_eq!(
        message.localize(Locale::En),
        "--name, --dep need a source — pass the path in the same command (skit add PATH …), or pick a lane outright with --edit (nothing was added)."
    );

    let mut options = empty_add_options();
    options.kind = Some("shell".to_owned());
    let error = refuse_bare_add_flags(&options).unwrap_err();
    let CliError::Usage(message) = error else {
        panic!("expected a usage refusal");
    };
    assert_eq!(
        message.localize(Locale::En),
        "--kind need a source — pass the path in the same command (skit add PATH …) (nothing was added)."
    );

    assert!(refuse_bare_add_flags(&empty_add_options()).is_ok());
    let mut empty_name = empty_add_options();
    empty_name.name = Some(String::new());
    assert!(refuse_bare_add_flags(&empty_name).is_ok());
}

#[test]
fn test_ask_kind_plain_lists_sorted_interpreted_plus_exe_and_prompt() {
    let selector = PlainKindSelector::new("weird.xyz", false, true);
    assert_eq!(
        selector.choices(),
        [
            KnownEntryKind::Fish,
            KnownEntryKind::JavaScript,
            KnownEntryKind::Lua,
            KnownEntryKind::Perl,
            KnownEntryKind::PowerShell,
            KnownEntryKind::Python,
            KnownEntryKind::R,
            KnownEntryKind::Ruby,
            KnownEntryKind::Shell,
            KnownEntryKind::TypeScript,
            KnownEntryKind::Executable,
            KnownEntryKind::Prompt,
        ]
    );
}

#[test]
fn test_ask_kind_plain_no_exe_when_offer_exe_false() {
    let selector = PlainKindSelector::new("skit-new-draft", false, false);
    assert!(!selector.choices().contains(&KnownEntryKind::Executable));
    assert_eq!(selector.choices().last(), Some(&KnownEntryKind::Prompt));
    assert_eq!(
        selector.parse("11"),
        Some(PlainKindSelection::Pick(KnownEntryKind::Prompt))
    );
    assert_eq!(selector.parse("12"), None);
}

#[test]
fn test_ask_kind_plain_shebang_question_variant() {
    let plain = PlainKindSelector::new("tool.xyz", false, true);
    assert_eq!(
        plain.question().localize(Locale::En),
        "What is tool.xyz? skit can't tell from the name."
    );
    let shebang = PlainKindSelector::new("tool.xyz", true, true);
    let question = shebang.question().localize(Locale::En);
    assert_eq!(
        question,
        "The #! in tool.xyz names no interpreter skit knows. What is it?"
    );
    assert!(!question.contains("can't tell from the name"));
}

#[test]
fn test_ask_kind_plain_returns_the_picked_language() {
    let selector = PlainKindSelector::new("x.xyz", false, true);
    assert_eq!(
        selector.parse("9"),
        Some(PlainKindSelection::Pick(KnownEntryKind::Shell))
    );
    assert_eq!(
        selector.parse(" 9 "),
        Some(PlainKindSelection::Pick(KnownEntryKind::Shell))
    );
    assert_eq!(selector.parse("-"), Some(PlainKindSelection::Cancel));
    for invalid in ["", "0", "13", "shell"] {
        assert_eq!(selector.parse(invalid), None, "answer={invalid:?}");
    }
    for kind in [io::ErrorKind::UnexpectedEof, io::ErrorKind::Interrupted] {
        assert!(matches!(
            add_dialoguer_error(dialoguer::Error::from(io::Error::from(kind))),
            CliError::AddCancelled
        ));
    }
}

#[test]
fn closed_add_intake_values_are_typed_before_dispatch() {
    for (value, expected) in [
        ("1", PlainAddChoice::Path),
        (" 2 ", PlainAddChoice::Script),
        ("3", PlainAddChoice::Prompt),
        ("4", PlainAddChoice::Command),
    ] {
        assert_eq!(PlainAddChoice::parse(value).unwrap(), expected);
    }
    let error = PlainAddChoice::parse("5").unwrap_err();
    for (locale, expected) in [
        (Locale::En, "Choose a number from 1 to 4."),
        (Locale::ZhCn, "请选择 1 到 4 之间的数字。"),
        (Locale::ZhTw, "請選擇 1 到 4 之間的數字。"),
    ] {
        assert_eq!(error.message().localize(locale), expected);
    }

    let mut without_default = String::new();
    dialoguer::theme::Theme::format_input_prompt(
        &AddChoiceTheme,
        &mut without_default,
        "Which one?",
        None,
    )
    .unwrap();
    assert_eq!(without_default, "Which one? [1/2/3/4]: ");
    let mut with_default = String::new();
    dialoguer::theme::Theme::format_input_prompt(
        &AddChoiceTheme,
        &mut with_default,
        "Which one?",
        Some("1"),
    )
    .unwrap();
    assert_eq!(with_default, "Which one? [1/2/3/4] (1): ");

    assert_eq!(
        selected_add_kind(Some("shell"), Some("python"), None, true).unwrap(),
        "shell"
    );
    assert_eq!(
        selected_add_kind(None, Some("python"), Some(KnownEntryKind::Prompt), false).unwrap(),
        "python"
    );
    assert_eq!(
        selected_add_kind(None, None, Some(KnownEntryKind::Prompt), false).unwrap(),
        "prompt"
    );
    assert_eq!(selected_add_kind(None, None, None, true).unwrap(), "python");
    let error = selected_add_kind(None, None, None, false).unwrap_err();
    assert_eq!(
        error.message().localize(Locale::En),
        "could not infer the entry kind; pass --kind KIND"
    );
}

#[test]
fn test_ask_kind_plain_returns_exe_and_prompt() {
    let selector = PlainKindSelector::new("x.xyz", false, true);
    assert_eq!(
        selector.parse("11"),
        Some(PlainKindSelection::Pick(KnownEntryKind::Executable))
    );
    assert_eq!(
        selector.parse("12"),
        Some(PlainKindSelection::Pick(KnownEntryKind::Prompt))
    );
}

#[test]
fn private_human_paths_render_entries_and_diagnostics() {
    let root = TempDir::new().unwrap();
    write_meta(&root, "alpha", "Alpha", "Human description");
    let broken = root.path().join("scripts").join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("meta.toml"), "name = [broken").unwrap();
    let service = LibraryService::new(FileStore::new(root.path()));

    let store = FileStore::new(root.path());
    list(&service, &store, false).unwrap();
    show(&service, &store, "alpha", false).unwrap();
    write_meta(&root, "empty", "Empty", "");
    show(&service, &store, "empty", false).unwrap();

    let source = root.path().join("source.py");
    fs::write(&source, b"print('source')\n").unwrap();
    let error = add(
        &service,
        AddOptions {
            source: Some(source),
            kind: Some(" ".to_owned()),
            name: Some("Bad".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CliError::Repository(RepositoryError::InvalidMutation { .. })
    ));
}

fn listing_summary(kind: &str, mode: StorageMode, target: Option<&str>) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse("listed").unwrap(),
        name: "Listed".to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode,
        description: String::new(),
        target: target.map(str::to_owned),
    }
}

#[test]
fn summary_missing_uses_only_the_listing_projection_and_derived_copy_target() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry_dir = store.entry_dir_path(&Slug::parse("listed").unwrap());
    fs::create_dir_all(&entry_dir).unwrap();
    let payload = entry_dir.join("script.py");
    fs::write(&payload, "print('ok')\n").unwrap();

    // No meta.toml exists. A listing consumer must still find a copied target from the summary's
    // kind and entry directory instead of resolving and parsing the full entry.
    let copy = listing_summary("python", StorageMode::Copy, None);
    assert!(!summary_missing(&store, &copy));
    fs::remove_file(payload).unwrap();
    assert!(summary_missing(&store, &copy));

    let module = listing_summary("js", StorageMode::Copy, None);
    let module_payload = store.entry_dir_path(&module.slug).join("script.mjs");
    fs::write(&module_payload, "export {};\n").unwrap();
    assert!(
        !summary_missing(&store, &module),
        "a supported JavaScript stored-name variant is a healthy copy target"
    );

    // Python v0.4 uses Path.exists(), not a regular-file-only check.
    let reference_dir = root.path().join("reference-target");
    fs::create_dir(&reference_dir).unwrap();
    let reference = listing_summary(
        "python",
        StorageMode::Reference,
        Some(reference_dir.to_str().unwrap()),
    );
    assert!(!summary_missing(&store, &reference));
    let invoke_directory = listing_summary("python", StorageMode::Reference, Some(""));
    assert_eq!(
        summary_target(&store, &invoke_directory),
        Some(std::path::PathBuf::from("."))
    );
    assert!(!summary_missing(&store, &invoke_directory));

    // This version cannot infer target semantics for a kind written by a newer skit.
    let unknown = listing_summary(
        "future-kind",
        StorageMode::Reference,
        Some(root.path().join("gone").to_str().unwrap()),
    );
    assert!(!summary_missing(&store, &unknown));
}

#[test]
fn tui_composition_treats_an_unindexed_broken_scripts_path_as_an_empty_library() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("scripts"), "not a directory").unwrap();
    let scan = LibraryService::new(FileStore::new(root.path()))
        .list()
        .unwrap();
    assert!(scan.entries.is_empty());
    assert!(scan.diagnostics.is_empty());
}

#[test]
fn data_directory_mode_and_error_taxonomy_helpers_are_stable() {
    let root = TempDir::new().unwrap();
    assert_eq!(
        resolve_data_dir(Some(root.path().to_path_buf())).unwrap(),
        root.path()
    );
    assert!(platform_data_dir().is_some());
    assert!(resolve_data_dir(None).is_ok());
    assert_eq!(mode_name(StorageMode::Copy), "copy");
    assert_eq!(mode_name(StorageMode::Reference), "reference");

    let json_error = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let errors = [
        (
            CliError::Repository(RepositoryError::NotFound {
                query: "missing".to_owned(),
            }),
            ExitClass::Failure.code(),
        ),
        (CliError::ConfirmationRequired, ExitClass::Usage.code()),
        (CliError::Json(json_error), ExitClass::Skit.code()),
        (
            CliError::Io(io::Error::other("output")),
            ExitClass::Skit.code(),
        ),
        (
            CliError::Tui(skit_tui::TuiError::Io(io::Error::other("terminal"))),
            ExitClass::Skit.code(),
        ),
        (
            source_error("read", Path::new("source"), io::Error::other("source")),
            ExitClass::Failure.code(),
        ),
        (
            CliError::Dependencies(DependencyError::Io {
                operation: "read",
                path: "dependency".to_owned(),
                reason: "failed".to_owned(),
            }),
            ExitClass::Skit.code(),
        ),
        (CliError::DataDirectoryUnavailable, ExitClass::Skit.code()),
    ];

    for (error, expected) in errors {
        assert_eq!(error.exit_code(), i32::from(expected));
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn adapter_only_error_paths_do_not_require_process_global_configuration() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("data");
    let config_dir = root.path().join("config");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("scripts"), "not a directory").unwrap();
    let store = FileStore::new(&data);
    let service = LibraryService::new(store.clone());
    assert!(matches!(
        edit_with_config(&service, &store, &config_dir, "missing", true).unwrap_err(),
        CliError::Failure(_)
    ));
    assert!(
        config_in(
            &FileConfigStore::new(&config_dir),
            None,
            Some("orphan"),
            false,
        )
        .is_err()
    );
    let runner_error =
        validate_prompt_runner_in(&FileConfigStore::new(&config_dir), Some("missing")).unwrap_err();
    assert!(
        runner_error
            .to_string()
            .contains(
                "Unknown runner: missing. Configured runners: claude, codex, opencode, amp, antigravity, copilot, cursor, pi"
            ),
        "{runner_error}"
    );
    let valid_store = FileStore::new(root.path().join("valid-data"));
    let valid_service = LibraryService::new(valid_store);
    assert!(
        add_with_config(
            &valid_service,
            &config_dir,
            AddOptions {
                source: None,
                kind: None,
                name: None,
                description: Some(String::new()),
                reference: false,
                command_template: None,
                prompt: false,
                executable: false,
                runner: None,
                no_interpolate: false,
                dependencies: Vec::new(),
                dependencies_explicit: false,
                requires_python: None,
                no_input: false,
            },
        )
        .is_err()
    );
}

#[test]
fn prompt_runner_validation_keeps_empty_configuration_typed_and_read_only() {
    let root = TempDir::new().unwrap();
    let config_dir = root.path().join("config");
    let config = FileConfigStore::new(&config_dir);

    // A blank optional pin is absent. It must not seed or create the config file.
    validate_prompt_runner_in(&config, Some("  ")).unwrap();
    assert!(test_tree_snapshot(root.path()).is_empty());

    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[prompt]\nrunners_seeded = true\n",
    )
    .unwrap();
    let before = test_tree_snapshot(root.path());

    let usage = validate_prompt_runner_in(&config, Some("missing")).unwrap_err();
    assert!(matches!(usage, CliError::Usage(_)), "{usage:?}");
    let pinned = validate_prompt_runner_pin_in(&config, Some("missing")).unwrap_err();
    assert!(matches!(pinned, CliError::Failure(_)), "{pinned:?}");
    for (locale, usage_expected, pinned_expected) in [
        (
            Locale::En,
            "Unknown runner: missing. Configured runners: —",
            "The runner missing isn't configured (known: —). Manage runners with: skit runner list",
        ),
        (
            Locale::ZhCn,
            "未知运行器：missing。已配置的运行器：—",
            "执行器 missing 未配置(已知:—)。管理执行器:skit runner list",
        ),
        (
            Locale::ZhTw,
            "未知執行器：missing。已設定的執行器：—",
            "執行器 missing 未設定(已知:—)。管理執行器:skit runner list",
        ),
    ] {
        assert_eq!(usage.message().localize(locale), usage_expected);
        assert_eq!(pinned.message().localize(locale), pinned_expected);
    }
    assert_eq!(test_tree_snapshot(root.path()), before);
}

#[test]
fn onboarding_candidate_labels_project_input_and_unset_shapes_in_every_locale() {
    let plan = onboarding_plan(
        "python",
        concat!(
            "first = input(\"Name?\")\n",
            "password = input(\"Password?\")\n",
            "COUNT = 0\n",
            "COUNT += 1\n",
        ),
    );
    let password = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "input-2")
        .expect("the analyzer must publish the second input call");
    assert_eq!(password.declaration.binding, ParameterBinding::Input);
    assert_eq!(password.declaration.order, 1);
    assert!(password.declaration.secret);
    for (locale, expected) in [
        (Locale::En, "input() #2: \"Password?\" (secret)"),
        (Locale::ZhCn, "input() 第 2 个：\"Password?\" (机密)"),
        (Locale::ZhTw, "input() 第 2 個：\"Password?\" (機密)"),
    ] {
        assert_eq!(onboarding_candidate_label(locale, password), expected);
    }

    let count = plan
        .candidates
        .iter()
        .find(|candidate| candidate.declaration.name == "COUNT")
        .expect("the analyzer must publish the constant");
    assert_eq!(
        count.demotion,
        Some(skit_language::DegradationReason::Accumulator)
    );
    let mut unset = count.clone();
    unset.declaration.default = None;
    unset.demotion = None;
    for (locale, expected) in [
        (Locale::En, "COUNT (int) = not set"),
        (Locale::ZhCn, "COUNT（int）= 未设置"),
        (Locale::ZhTw, "COUNT（int）= 未設定"),
    ] {
        assert_eq!(onboarding_candidate_label(locale, &unset), expected);
    }
    assert!(
        onboarding_candidate_label(Locale::En, count)
            .ends_with(" — ⚠ looks like a loop accumulator — probably not a parameter")
    );
}

#[test]
fn test_axis_display_helpers_exact() {
    let displayed = |store: &FileConfigStore, key: &str| {
        let raw = store.get(key).unwrap();
        config_display_value(store, key, &raw).unwrap()
    };

    let full_dir = TempDir::new().unwrap();
    let full = FileConfigStore::new(full_dir.path());
    full.set_many(&BTreeMap::from([
        ("mirror.pypi".to_owned(), "tsinghua".to_owned()),
        ("mirror.github".to_owned(), "nju".to_owned()),
        ("mirror.npm".to_owned(), "npmmirror".to_owned()),
    ]))
    .unwrap();
    assert_eq!(displayed(&full, "mirror.pypi"), "tsinghua");
    assert_eq!(displayed(&full, "mirror.github"), "nju");
    assert_eq!(displayed(&full, "mirror.npm"), "npmmirror");

    let custom_dir = TempDir::new().unwrap();
    fs::write(
        custom_dir.path().join("config.toml"),
        concat!(
            "[mirror]\n",
            "enabled = true\n",
            "pypi = \"https://my/simple\"\n",
            "python_install = \"https://my/py/\"\n",
            "uv_binary = \"https://my/uv\"\n",
            "npm = \"https://my/npm\"\n",
        ),
    )
    .unwrap();
    let custom = FileConfigStore::new(custom_dir.path());
    assert_eq!(displayed(&custom, "mirror.pypi"), "https://my/simple");
    assert_eq!(custom.get("mirror.github").unwrap(), "custom");
    assert_eq!(
        displayed(&custom, "mirror.github"),
        "https://my/py/ + https://my/uv"
    );
    assert_eq!(displayed(&custom, "mirror.npm"), "https://my/npm");

    for (source, expected) in [
        (
            "[mirror]\npython_install = \"https://my/py/\"\n",
            "https://my/py/ + off",
        ),
        (
            "[mirror]\nuv_binary = \"https://my/uv\"\n",
            "off + https://my/uv",
        ),
    ] {
        let half_dir = TempDir::new().unwrap();
        fs::write(half_dir.path().join("config.toml"), source).unwrap();
        let half = FileConfigStore::new(half_dir.path());
        assert_eq!(displayed(&half, "mirror.github"), expected);
    }

    let off_dir = TempDir::new().unwrap();
    let off = FileConfigStore::new(off_dir.path());
    assert_eq!(displayed(&off, "mirror.pypi"), "off");
    assert_eq!(displayed(&off, "mirror.github"), "off");
    assert_eq!(displayed(&off, "mirror.npm"), "off");
}

#[test]
fn tui_run_forms_preserve_saved_values_but_never_prefill_secrets() {
    let mut entry = Entry {
        slug: Slug::parse("alpha").unwrap(),
        meta: EntryMeta::minimal("Alpha", EntryKind::parse("prompt").unwrap()),
    };
    let mut token = ParamDecl::new("token");
    token.secret = true;
    let mut count = ParamDecl::new("count");
    count.parameter_type = ParameterType::Int;
    count.prompt = "Count".to_owned();
    let saved = BTreeMap::from([
        ("token".to_owned(), "do-not-show".to_owned()),
        ("count".to_owned(), "4".to_owned()),
    ]);
    let settings = EntrySettings {
        params: vec!["token".to_owned(), "count".to_owned()],
        parameters: vec![token, count],
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut entry.meta);
    let plan = form_plan("prompt", "{{token}} {{count}}", &settings);

    let Screen::Run(form) = tui_run_form(
        &entry,
        &plan,
        &saved,
        &["codex".to_owned()],
        "codex",
        &BTreeMap::from([(
            "fast".to_owned(),
            BTreeMap::from([("count".to_owned(), "8".to_owned())]),
        )]),
        "--model fast",
        RunFormContext {
            entry_kind: "prompt".to_owned(),
            path: None,
            tokens: skit_application::tokens::TokenContext {
                cwd: "/invoke".to_owned(),
                home: None,
                env: BTreeMap::new(),
                today: "2026-08-09".to_owned(),
                now: "12-00-00".to_owned(),
            },
        },
        Locale::En,
    ) else {
        panic!("run must use the typed launch form");
    };
    assert_eq!(form.purpose(), FormPurpose::Run);
    assert_eq!(form.selector(), "alpha");
    let fields = form
        .fields()
        .iter()
        .map(|field| {
            (
                field.key.as_str(),
                field.control.value(),
                matches!(&field.control, FormControl::Text(control) if control.secret),
            )
        })
        .collect::<Vec<_>>();
    assert!(fields.contains(&("value:token", String::new(), true)));
    assert!(fields.contains(&("value:count", "4".to_owned(), false)));
    assert!(fields.contains(&("_skit_runner", "codex".to_owned(), false)));
    assert!(fields.contains(&("_skit_preset", String::new(), false)));
    assert!(fields.contains(&("_skit_args", "--model fast".to_owned(), false)));
    assert_eq!(form.context().unwrap().entry_kind, "prompt");
    assert_eq!(
        form.fields()
            .iter()
            .find(|field| field.key == "_skit_args")
            .unwrap()
            .label,
        "Extra agent arguments"
    );
}

#[test]
fn interactive_runner_value_tracks_selection_separately_from_the_default() {
    let mut args = RunArgs {
        selector: "prompt".to_owned(),
        values: Vec::new(),
        preset: None,
        save_preset: None,
        runner: None,
        runner_was_picked: false,
        dry_run: false,
        no_input: false,
        plain: false,
        raw: false,
        forget_args: false,
        extra_args: Vec::new(),
    };
    let baseline = BTreeMap::new();
    let mut values =
        SubmittedValues::from([("_skit_runner".to_owned(), FieldValue::text("codex"))]);

    apply_interactive_run_values(&mut args, &values, &baseline).unwrap();
    assert_eq!(args.runner.as_deref(), Some("codex"));
    assert!(!args.runner_was_picked);

    values.insert("_skit_runner_picked".to_owned(), FieldValue::boolean(true));
    apply_interactive_run_values(&mut args, &values, &baseline).unwrap();
    assert!(args.runner_was_picked);
}

#[test]
fn tui_add_opens_the_typed_workflow_with_owned_drafts_and_runner_history() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let drafts = data_dir.join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    fs::write(drafts.join("skit-new-script.py"), "print(1)\n").unwrap();
    fs::write(drafts.join("not-owned.py"), "print(2)\n").unwrap();
    fs::create_dir(drafts.join("skit-new-directory.py")).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        concat!(
            "[[prompt.runners]]\n",
            "name = \"my-agent\"\n",
            "argv = [\"agent\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    PromptSelectionService::new(FilePromptSelectionStore::new(&state_dir))
        .remember_runner("my-agent")
        .unwrap();

    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let Screen::Add(workflow) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Add,
        None,
    )
    .unwrap() else {
        panic!("add must use its typed workflow");
    };
    assert_eq!(workflow.source().listed_drafts().len(), 1);
    assert_eq!(
        workflow.source().listed_drafts()[0].path,
        drafts.join("skit-new-script.py")
    );
    assert_eq!(workflow.review_defaults().runner_names, ["my-agent"]);
    assert_eq!(
        workflow.review_defaults().last_runner.as_deref(),
        Some("my-agent")
    );

    assert_eq!(
        tui_split_list("alpha, beta\ngamma  delta"),
        ["alpha", "beta", "gamma", "delta"]
    );
    assert_eq!(
        tui_dependency_list("requests>=2,<3\nrich; python_version >= '3.12'"),
        ["requests>=2,<3", "rich; python_version >= '3.12'"]
    );
    for (kind, source, expected) in [
        ("js", "module.mjs", "script.mjs"),
        ("js", "module.cjs", "script.cjs"),
        ("ts", "module.mts", "script.mts"),
        ("ts", "module.cts", "script.cts"),
    ] {
        assert_eq!(
            payload_stored_name(&EntryKind::parse(kind).unwrap(), Path::new(source)),
            expected
        );
    }
}

#[test]
fn tui_add_host_reads_exact_source_and_commits_through_the_reducer_seam() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("hello.sh");
    let raw = b"#!/bin/sh\nprintf '\\377\\n'\n";
    fs::write(&source, raw).unwrap();
    let canonical = fs::canonicalize(&source).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(source.display().to_string()));
    let effects = workflow.reduce(AddAction::Continue);

    let action = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap();
    let UiAction::Add(AddAction::SourceInspected { request, result }) = action else {
        panic!("source inspection must return through the typed reducer");
    };
    let snapshot = result.unwrap();
    assert_eq!(snapshot.path, canonical);
    assert_eq!(snapshot.source_record, canonical.display().to_string());
    assert_eq!(snapshot.bytes, raw);
    assert!(snapshot.is_regular);
    assert!(!snapshot.is_directory);
    assert!(!snapshot.is_draft);
    let effects = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(snapshot),
    });
    assert!(effects.is_empty());
    assert!(workflow.review().is_some());

    let effects = workflow.reduce(AddAction::Save);
    let [
        AddEffect::Commit {
            source: Some(expected),
            ..
        },
    ] = effects.as_slice()
    else {
        panic!("a reviewed file commit must carry the byte-exact source expectation");
    };
    assert_eq!(expected.bytes, raw);
    let action = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap();
    let UiAction::Add(AddAction::CommitFinished { request, result }) = action else {
        panic!("repository completion must return through the typed reducer");
    };
    let slug = result.unwrap();
    let effects = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Ok(slug.clone()),
    });
    let UiAction::AddCompleted {
        slug: completed,
        message,
        ..
    } = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap()
    else {
        panic!("the final host effect must refresh and select the new entry");
    };
    assert_eq!(completed.as_str(), slug);
    assert_eq!(message, "Entry added");
    let entry = service.show(&slug).unwrap();
    assert_eq!(entry.meta.source, canonical.display().to_string());
}

#[test]
fn tui_add_refuses_a_source_that_changed_after_review_without_writing_an_entry() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("reviewed.sh");
    fs::write(&source, "#!/bin/sh\necho before\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(source.display().to_string()));
    let inspect = workflow.reduce(AddAction::Continue);
    let UiAction::Add(AddAction::SourceInspected { request, result }) =
        tui_add_effect(&service, &store, &state_dir, &config_dir, inspect).unwrap()
    else {
        panic!("source inspection must return to the reducer");
    };
    let _ = workflow.reduce(AddAction::SourceInspected { request, result });
    fs::write(&source, "#!/bin/sh\necho after\n").unwrap();

    let commit = workflow.reduce(AddAction::Save);
    let UiAction::Add(AddAction::CommitFinished {
        request,
        result: Err(reason),
    }) = tui_add_effect(&service, &store, &state_dir, &config_dir, commit).unwrap()
    else {
        panic!("a stale review must return a typed commit refusal");
    };
    assert!(reason.contains("source changed while the add review was open"));
    let _ = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Err(reason),
    });
    assert!(service.list().unwrap().entries.is_empty());
    assert!(workflow.problem().is_some());
}

#[cfg(any(unix, windows))]
#[test]
fn tui_add_refuses_a_same_bytes_identity_replacement_without_any_write() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("reviewed.sh");
    const BYTES: &[u8] = b"#!/bin/sh\necho unchanged\n";
    fs::write(&source, BYTES).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(source.display().to_string()));
    let inspect = workflow.reduce(AddAction::Continue);
    let UiAction::Add(AddAction::SourceInspected {
        request,
        result: Ok(expected),
    }) = tui_add_effect(&service, &store, &state_dir, &config_dir, inspect).unwrap()
    else {
        panic!("source inspection must return a snapshot");
    };
    let old_identity = expected.identity.clone();
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(expected),
    });

    let staged = root.path().join("replacement.sh");
    fs::write(&staged, BYTES).unwrap();
    fs::remove_file(&source).unwrap();
    fs::rename(&staged, &source).unwrap();
    let replacement = tui_add_source(&data_dir, &source).unwrap();
    assert_eq!(replacement.bytes, BYTES);
    assert_ne!(replacement.identity, old_identity);
    let before = test_tree_snapshot(root.path());

    let commit = workflow.reduce(AddAction::Save);
    let UiAction::Add(AddAction::CommitFinished {
        result: Err(reason),
        ..
    }) = tui_add_effect(&service, &store, &state_dir, &config_dir, commit).unwrap()
    else {
        panic!("a replaced source must return a typed commit refusal");
    };
    assert!(reason.contains("source changed while the add review was open"));
    assert_eq!(test_tree_snapshot(root.path()), before);
    assert_eq!(fs::read(&source).unwrap(), BYTES);
}

#[cfg(any(unix, windows))]
#[test]
fn tui_add_cleanup_refusals_keep_the_committed_entry_and_the_replacement() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let unusable_state = root.path().join("state-is-a-file");
    fs::write(&unusable_state, "not a directory").unwrap();
    let drafts = data_dir.join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let draft = drafts.join("skit-new-replaced.py");
    fs::write(&draft, "print('reviewed')\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let expected = tui_add_source(&data_dir, &draft).unwrap();
    let created = add_command(&service, "Kept", "printf kept");
    fs::write(&draft, "print('replacement')\n").unwrap();

    let action = tui_add_effect(
        &service,
        &store,
        &unusable_state,
        &config_dir,
        vec![
            AddEffect::ConsumeDraft(expected),
            AddEffect::RememberRunner("agent".to_owned()),
            AddEffect::Complete(created.slug.as_str().to_owned()),
        ],
    )
    .unwrap();
    let UiAction::AddCompleted { message, slug, .. } = action else {
        panic!("auxiliary cleanup failures must not turn a committed add into failure");
    };
    assert_eq!(slug, created.slug);
    assert!(message.starts_with("Entry added\nwarning: "));
    assert_eq!(message.matches("warning: ").count(), 2);
    assert!(message.contains("changed"));
    assert!(message.contains("skit-new-replaced.py"));
    assert_eq!(fs::read(&draft).unwrap(), b"print('replacement')\n");
    assert_eq!(service.show("Kept").unwrap().slug, created.slug);
}

fn complete_after_draft_cleanup(root: &TempDir, expected: AddSourceSnapshot) -> String {
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let created = add_command(&service, "Committed", "printf committed");
    let action = tui_add_effect(
        &service,
        &store,
        &state_dir,
        &config_dir,
        vec![
            AddEffect::ConsumeDraft(expected),
            AddEffect::Complete(created.slug.as_str().to_owned()),
        ],
    )
    .unwrap();
    let UiAction::AddCompleted { message, slug, .. } = action else {
        panic!("cleanup outcomes must not roll back a committed add");
    };
    assert_eq!(slug, created.slug);
    assert_eq!(service.show("Committed").unwrap().slug, created.slug);
    message
}

#[cfg(any(unix, windows))]
#[test]
fn tui_add_maps_every_draft_cleanup_outcome_without_rolling_back_the_entry() {
    let removed = TempDir::new().unwrap();
    let removed_data = removed.path().join("data");
    let removed_source = removed_data.join("drafts/skit-new-removed.py");
    fs::create_dir_all(removed_source.parent().unwrap()).unwrap();
    fs::write(&removed_source, b"print(1)\n").unwrap();
    let removed_claim = tui_add_source(&removed_data, &removed_source).unwrap();
    assert_eq!(
        complete_after_draft_cleanup(&removed, removed_claim),
        "Entry added"
    );
    assert!(!removed_source.exists());

    let missing = TempDir::new().unwrap();
    let missing_data = missing.path().join("data");
    let missing_source = missing_data.join("drafts/skit-new-missing.py");
    fs::create_dir_all(missing_source.parent().unwrap()).unwrap();
    fs::write(&missing_source, b"print(1)\n").unwrap();
    let missing_claim = tui_add_source(&missing_data, &missing_source).unwrap();
    fs::remove_file(&missing_source).unwrap();
    assert_eq!(
        complete_after_draft_cleanup(&missing, missing_claim),
        "Entry added"
    );

    let changed = TempDir::new().unwrap();
    let changed_data = changed.path().join("data");
    let changed_source = changed_data.join("drafts/skit-new-changed.py");
    fs::create_dir_all(changed_source.parent().unwrap()).unwrap();
    fs::write(&changed_source, b"print('old')\n").unwrap();
    let changed_claim = tui_add_source(&changed_data, &changed_source).unwrap();
    fs::write(&changed_source, b"print('new')\n").unwrap();
    let message = complete_after_draft_cleanup(&changed, changed_claim);
    assert_eq!(
        message,
        format!(
            "Entry added\nwarning: The kept draft changed before cleanup. skit kept it at {}.",
            changed_source.display()
        )
    );
    assert_eq!(fs::read(&changed_source).unwrap(), b"print('new')\n");

    let failed = TempDir::new().unwrap();
    let failed_data = failed.path().join("data");
    let failed_source = failed_data.join("drafts/skit-new-error.py");
    fs::create_dir_all(failed_source.parent().unwrap()).unwrap();
    fs::write(&failed_source, b"print(1)\n").unwrap();
    let failed_claim = tui_add_source(&failed_data, &failed_source).unwrap();
    fs::rename(failed_data.join("drafts"), failed_data.join("drafts-old")).unwrap();
    fs::write(failed_data.join("drafts"), b"not a directory").unwrap();
    let message = complete_after_draft_cleanup(&failed, failed_claim);
    assert_eq!(
        message,
        format!(
            "Entry added\nwarning: skit's drafts path is not an owned directory: {}",
            failed_data.join("drafts").display()
        )
    );
    assert_eq!(
        fs::read(failed_data.join("drafts-old/skit-new-error.py")).unwrap(),
        b"print(1)\n"
    );
}

#[cfg(unix)]
#[test]
fn tui_add_never_treats_a_symlinked_drafts_directory_as_owned_storage() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let outside = root.path().join("outside");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, data_dir.join("drafts")).unwrap();
    let victim = outside.join("skit-victim.py");
    fs::write(&victim, "print('keep me')\n").unwrap();

    let snapshot = tui_add_source(&data_dir, &data_dir.join("drafts/skit-victim.py")).unwrap();
    assert!(!snapshot.is_draft);
    assert!(consume_owned_draft(&data_dir, &snapshot).is_err());
    assert_eq!(fs::read(&victim).unwrap(), b"print('keep me')\n");
    assert!(tui_drafts(&data_dir).is_empty());
}

#[cfg(any(unix, windows))]
#[test]
fn tui_draft_rows_change_identity_when_a_path_is_replaced() {
    let root = TempDir::new().unwrap();
    let drafts = root.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let path = drafts.join("skit-new-row.py");
    fs::write(&path, b"print(1)\n").unwrap();
    let first = tui_drafts(root.path());
    assert_eq!(first.len(), 1);
    assert_application_identity(first[0].identity.clone());

    fs::remove_file(&path).unwrap();
    fs::write(&path, b"print(1)\n").unwrap();
    let second = tui_drafts(root.path());
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].path, first[0].path);
    assert_ne!(second[0].identity, first[0].identity);
}

#[cfg(any(unix, windows))]
#[test]
fn tui_delete_changed_outcome_returns_a_refreshed_draft_claim() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let drafts = data.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let path = drafts.join("skit-new-refresh.py");
    fs::write(&path, b"print('reviewed')\n").unwrap();
    let initial = tui_drafts(data.path());
    assert_eq!(initial.len(), 1);
    assert!(initial[0].identity.is_some());
    let mut workflow = AddWorkflowState::new(initial.clone());
    let _ = workflow.reduce(AddAction::SelectDraft(0));
    let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
    let effects = workflow.reduce(AddAction::ConfirmDraftDelete(true));

    let staged = drafts.join("replacement.tmp");
    fs::write(&staged, b"print('replacement')\n").unwrap();
    fs::remove_file(&path).unwrap();
    fs::rename(&staged, &path).unwrap();
    let store = FileStore::new(data.path());
    let service = LibraryService::new(store.clone());
    let UiAction::Add(AddAction::DraftDeleted {
        request,
        result: Ok(DraftDeleteOutcome::Changed(refreshed)),
    }) = tui_add_effect(&service, &store, state.path(), config.path(), effects).unwrap()
    else {
        panic!("an identity mismatch must return the new row, not stale success");
    };
    assert_eq!(refreshed.path, path);
    assert_ne!(refreshed.identity, initial[0].identity);
    let _ = workflow.reduce(AddAction::DraftDeleted {
        request,
        result: Ok(DraftDeleteOutcome::Changed(refreshed.clone())),
    });
    assert_eq!(workflow.source().listed_drafts(), &[refreshed]);
    assert_eq!(fs::read(&path).unwrap(), b"print('replacement')\n");
}

#[test]
fn tui_draft_discovery_returns_no_rows_when_the_owned_directory_changes_before_read() {
    let root = TempDir::new().unwrap();
    let drafts = root.path().join("drafts");
    let parked = root.path().join("drafts-parked");
    fs::create_dir_all(&drafts).unwrap();
    fs::write(drafts.join("skit-new-race.py"), b"print('parked')\n").unwrap();

    let rows = tui_drafts_with_test_hook(root.path(), |verified| {
        assert_eq!(verified, drafts);
        fs::rename(verified, &parked).unwrap();
        fs::write(verified, b"not a directory").unwrap();
    });

    assert!(rows.is_empty());
    assert_eq!(
        fs::read(parked.join("skit-new-race.py")).unwrap(),
        b"print('parked')\n"
    );
    assert_eq!(fs::read(&drafts).unwrap(), b"not a directory");
}

#[cfg(unix)]
#[test]
fn tui_changed_draft_refresh_refuses_a_symlink_row_without_touching_either_file() {
    use std::os::unix::fs::symlink;

    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let drafts = data.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let path = drafts.join("skit-new-symlink-refresh.py");
    fs::write(&path, b"print('claimed')\n").unwrap();
    let initial = tui_drafts(data.path());
    assert_eq!(initial.len(), 1);
    assert_application_identity(initial[0].identity.clone());
    let mut workflow = AddWorkflowState::new(initial.clone());
    let _ = workflow.reduce(AddAction::SelectDraft(0));
    let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
    let effects = workflow.reduce(AddAction::ConfirmDraftDelete(true));

    let outside = data.path().join("outside-replacement.py");
    fs::write(&outside, b"print('outside')\n").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&outside, &path).unwrap();
    let before = test_tree_snapshot(data.path());
    let store = FileStore::new(data.path());
    let service = LibraryService::new(store.clone());

    let UiAction::Add(AddAction::DraftDeleted {
        request,
        result: Err(problem),
    }) = tui_add_effect(&service, &store, state.path(), config.path(), effects).unwrap()
    else {
        panic!("a changed row that is no longer a regular draft must return a typed error");
    };
    assert_eq!(
        problem,
        format!(
            "the kept draft changed and could not be inspected at {}",
            path.display()
        )
    );
    let _ = workflow.reduce(AddAction::DraftDeleted {
        request,
        result: Err(problem.clone()),
    });
    assert_eq!(
        workflow.problem(),
        Some(&skit_ui::AddProblem::DraftDeleteFailed {
            reason: problem.clone(),
        })
    );
    assert_eq!(workflow.source().listed_drafts(), initial);

    let error = refreshed_draft(data.path(), &path).unwrap_err();
    for (locale, expected) in [
        (
            Locale::En,
            format!(
                "the kept draft changed and could not be inspected at {}",
                path.display()
            ),
        ),
        (
            Locale::ZhCn,
            format!("保留的草稿已更改，无法在 {} 检查它", path.display()),
        ),
        (
            Locale::ZhTw,
            format!("保留的草稿已變更，無法在 {} 檢查它", path.display()),
        ),
    ] {
        assert_eq!(error.message().localize(locale), expected);
    }
    assert_eq!(fs::read_link(&path).unwrap(), outside);
    assert_eq!(fs::read(&outside).unwrap(), b"print('outside')\n");
    assert_eq!(test_tree_snapshot(data.path()), before);
    assert!(owned_draft_quarantines(data.path()).is_empty());
    assert!(test_tree_snapshot(state.path()).is_empty());
    assert!(test_tree_snapshot(config.path()).is_empty());
}

#[cfg(any(unix, windows))]
#[test]
fn tui_delete_in_place_content_and_permission_changes_return_refreshed_claims() {
    let content = TempDir::new().unwrap();
    let content_state = TempDir::new().unwrap();
    let content_config = TempDir::new().unwrap();
    let content_drafts = content.path().join("drafts");
    fs::create_dir_all(&content_drafts).unwrap();
    let content_path = content_drafts.join("skit-new-content.py");
    fs::write(&content_path, b"print('old')\n").unwrap();
    let initial = tui_drafts(content.path()).pop().unwrap();
    let mut workflow = AddWorkflowState::new(vec![initial.clone()]);
    let _ = workflow.reduce(AddAction::SelectDraft(0));
    let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
    let effects = workflow.reduce(AddAction::ConfirmDraftDelete(true));
    fs::write(&content_path, b"print('new')\n").unwrap();
    let store = FileStore::new(content.path());
    let service = LibraryService::new(store.clone());
    let result = tui_add_effect(
        &service,
        &store,
        content_state.path(),
        content_config.path(),
        effects,
    )
    .unwrap();
    let UiAction::Add(AddAction::DraftDeleted {
        result: Ok(DraftDeleteOutcome::Changed(refreshed)),
        ..
    }) = result
    else {
        panic!("an in-place content edit must refresh instead of deleting");
    };
    assert_eq!(refreshed.path, content_path);
    assert!(refreshed.modified != initial.modified || refreshed.identity != initial.identity);
    assert_eq!(fs::read(&content_path).unwrap(), b"print('new')\n");

    let permissions = TempDir::new().unwrap();
    let permission_state = TempDir::new().unwrap();
    let permission_config = TempDir::new().unwrap();
    let permission_drafts = permissions.path().join("drafts");
    fs::create_dir_all(&permission_drafts).unwrap();
    let permission_path = permission_drafts.join("skit-new-permissions.py");
    fs::write(&permission_path, b"print(1)\n").unwrap();
    let initial = tui_drafts(permissions.path()).pop().unwrap();
    let mut workflow = AddWorkflowState::new(vec![initial.clone()]);
    let _ = workflow.reduce(AddAction::SelectDraft(0));
    let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
    let effects = workflow.reduce(AddAction::ConfirmDraftDelete(true));
    let mut mode = fs::metadata(&permission_path).unwrap().permissions();
    mode.set_readonly(!mode.readonly());
    fs::set_permissions(&permission_path, mode).unwrap();
    let store = FileStore::new(permissions.path());
    let service = LibraryService::new(store.clone());
    let result = tui_add_effect(
        &service,
        &store,
        permission_state.path(),
        permission_config.path(),
        effects,
    )
    .unwrap();
    let UiAction::Add(AddAction::DraftDeleted {
        result: Ok(DraftDeleteOutcome::Changed(refreshed)),
        ..
    }) = result
    else {
        panic!("an in-place permission edit must refresh instead of deleting");
    };
    assert_ne!(refreshed.permissions, initial.permissions);
    assert!(permission_path.exists());
}

#[cfg(unix)]
#[test]
fn tui_add_authoring_uses_a_real_owned_draft_and_discards_only_unchanged_starters() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let editor = root.path().join("editor.sh");
    fs::write(
        &editor,
        r#"#!/bin/sh
printf "print('written')\n" >> "$1"
"#,
    )
    .unwrap();
    let mut mode = fs::metadata(&editor).unwrap().permissions();
    mode.set_mode(0o755);
    fs::set_permissions(&editor, mode).unwrap();
    FileConfigStore::new(&config_dir)
        .set("editor", editor.to_str().unwrap())
        .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut workflow = AddWorkflowState::new(Vec::new());
    let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Script));
    let UiAction::Add(AddAction::DraftEdited {
        request,
        result: Ok(Some(snapshot)),
    }) = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap()
    else {
        panic!("an edited starter must return an owned byte-exact draft");
    };
    assert!(snapshot.is_draft);
    assert!(snapshot.path.exists());
    assert_eq!(
        snapshot.bytes,
        b"#!/usr/bin/env python3\nprint('written')\n"
    );
    let _ = workflow.reduce(AddAction::DraftEdited {
        request,
        result: Ok(Some(snapshot)),
    });

    FileConfigStore::new(&config_dir)
        .set("editor", "true")
        .unwrap();
    let mut unchanged = AddWorkflowState::new(Vec::new());
    let effects = unchanged.reduce(AddAction::NewDraft(DraftKind::Prompt));
    let UiAction::Add(AddAction::DraftEdited {
        request,
        result: Ok(None),
    }) = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap()
    else {
        panic!("an unchanged prompt starter must be discarded as scaffolding");
    };
    let _ = unchanged.reduce(AddAction::DraftEdited {
        request,
        result: Ok(None),
    });
    assert_eq!(
        unchanged.notice(),
        Some(&skit_ui::AddNotice::NothingWritten)
    );
}

#[cfg(unix)]
#[test]
fn tui_author_draft_launch_error_cleanup_keeps_an_editor_replacement() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let editor = root.path().join("replace-and-fail.sh");
    fs::write(
        &editor,
        "#!/bin/sh\nprintf \"print('replacement')\\n\" > \"$1.replacement\"\nmv \"$1.replacement\" \"$1\"\nexit 7\n",
    )
    .unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    FileConfigStore::new(&config_dir)
        .set("editor", editor.to_str().unwrap())
        .unwrap();

    let error = tui_author_draft(&data_dir, &config_dir, DraftKind::Script).unwrap_err();
    let message = error.message().localize(Locale::En);
    assert!(message.contains("warning:"), "{message}");
    assert!(message.contains("changed before cleanup"), "{message}");
    let drafts = tui_drafts(&data_dir);
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        fs::read(&drafts[0].path).unwrap(),
        b"print('replacement')\n"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn tui_author_draft_unchanged_cleanup_keeps_a_postinspection_replacement() {
    let root = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-unchanged.py", b"");
    let staged = root.path().join("drafts/replacement.tmp");
    fs::write(&staged, b"").unwrap();
    fs::remove_file(&snapshot.path).unwrap();
    fs::rename(&staged, &snapshot.path).unwrap();

    let error = discard_authored_draft(root.path(), &snapshot).unwrap_err();
    assert!(
        error
            .message()
            .localize(Locale::En)
            .contains("changed before cleanup")
    );
    assert!(snapshot.path.is_file());
    assert!(owned_draft_quarantines(root.path()).is_empty());
}

#[test]
fn tui_settings_offers_a_declared_row_exactly_the_axes_version_04_makes_editable() {
    let root = TempDir::new().unwrap();
    let directory = root.path().join("scripts").join("alpha");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "name = \"Alpha\"\n",
            "kind = \"command\"\n",
            "mode = \"copy\"\n",
            "source = \"\"\n",
            "workdir = \"invoke\"\n",
            "[[parameters]]\n",
            "name = \"count\"\n",
            "binding = \"none\"\n",
            "delivery = \"flag\"\n",
            "type = \"int\"\n",
            "default = 4\n",
            "required = true\n",
            "multiple = true\n",
            "repeat = true\n",
            "choices = [\"4\", \"8\"]\n",
            "prompt = \"Count\"\n",
            "help = \"Set the count.\"\n",
            "secret = true\n",
            "env_source = \"SKIT_COUNT\"\n",
            "env_target = \"COUNT\"\n",
            "flag = \"--count\"\n",
            "action = \"append\"\n",
        ),
    )
    .unwrap();
    let store = FileStore::new(root.path());
    store.rebuild_registry().unwrap();
    let service = LibraryService::new(store.clone());

    let view = settings_view(&service, &store, &root.path().join("state"), "alpha");
    let controls = view
        .fields()
        .map(|field| field.key.as_str())
        .collect::<Vec<_>>();

    // The nine axes the flat form used to carry are all reachable through the typed screen. Two of
    // them have no key of their own: unticking a row is the remove, and unticking a source-managed
    // row is the unmanage, which is version 0.4's own affordance (`:115-117`, `:180`).
    for key in ["parameter:add", "template"] {
        assert!(controls.contains(&key), "missing settings control {key}");
    }

    // Version 0.4's DeclParamRow makes exactly these editable
    // (`src/skit/tui_settings.py:170-227`).
    for axis in [
        "keep",
        "type",
        "default",
        "choices",
        "help",
        "required",
        "prompt",
        "secret",
        "env_source",
    ] {
        assert!(
            controls.contains(&format!("parameter:count:{axis}").as_str()),
            "a declared row lost its editable {axis}"
        );
    }
    // Delivery is dim header text inside the keep toggle's own label because a different command
    // changes it (`:152-154`, `:180`), and version 0.4 has no control at all for the rest. A
    // control that does not exist cannot make a promise the save has to take back.
    for axis in [
        "name",
        "binding",
        "delivery",
        "multiple",
        "repeat",
        "env_target",
        "action",
        "baseline",
    ] {
        assert!(
            !controls.contains(&format!("parameter:count:{axis}").as_str()),
            "a declared row offered {axis}, which version 0.4 never edits here"
        );
    }
    // A command template takes no argument vector, so it never offers a flag.
    assert!(!controls.contains(&"parameter:count:flag"));
    assert_eq!(
        view.field("parameter:count:default")
            .unwrap()
            .value()
            .as_text(),
        "4"
    );
    assert_eq!(
        view.field("parameter:count:choices")
            .unwrap()
            .value()
            .as_text(),
        "4, 8"
    );
    // Every field of one row is addressed by the parameter's own name, so a save merges each edit
    // onto the declaration it was made on. No serialized snapshot rides along to say which one.
    assert!(
        controls
            .iter()
            .filter(|key| key.starts_with("parameter:"))
            .all(|key| key.starts_with("parameter:count:") || *key == "parameter:add"),
        "{controls:?}"
    );
}

#[test]
fn settings_read_projection_refreshes_only_sound_public_defaults() {
    let mut name = ParamDecl::new("NAME");
    name.binding = ParameterBinding::Const;
    name.delivery = ParameterDelivery::Inject;
    name.default = Some(ParameterValue::String("hello".to_owned()));

    let mut token = ParamDecl::new("TOKEN");
    token.binding = ParameterBinding::Const;
    token.delivery = ParameterDelivery::Inject;
    token.default = Some(ParameterValue::String("stored-secret".to_owned()));
    token.secret = true;

    let mut count = ParamDecl::new("COUNT");
    count.binding = ParameterBinding::Const;
    count.delivery = ParameterDelivery::Inject;
    count.parameter_type = ParameterType::Int;
    count.default = Some(ParameterValue::Integer(3));

    let mut fresh = ParamDecl::new("FRESH");
    fresh.binding = ParameterBinding::Const;
    fresh.delivery = ParameterDelivery::Inject;

    let stored = write_managed_params(
        "python",
        concat!(
            "NAME = \"hello\"\n",
            "TOKEN = \"stored-secret\"\n",
            "COUNT = 3\n",
            "FRESH = \"new\"\n",
        ),
        &[name, token, count, fresh],
    )
    .unwrap();
    let live = stored
        .replace("NAME = \"hello\"", "NAME = \"bonjour\"")
        .replace("TOKEN = \"stored-secret\"", "TOKEN = \"live-secret\"")
        .replace("COUNT = 3", "COUNT = \"not-an-int\"");
    let projected = settings_managed_params("python", &live);
    let default = |name: &str| {
        projected
            .iter()
            .find(|declaration| declaration.name == name)
            .and_then(|declaration| declaration.default.clone())
    };
    assert_eq!(
        default("NAME"),
        Some(ParameterValue::String("bonjour".to_owned()))
    );
    assert_eq!(
        default("TOKEN"),
        Some(ParameterValue::String("stored-secret".to_owned())),
        "a secret's live source literal must not enter the Settings view"
    );
    assert_eq!(default("COUNT"), Some(ParameterValue::Integer(3)));
    assert_eq!(
        default("FRESH"),
        Some(ParameterValue::String("new".to_owned())),
        "a sound live default must display even when the block has no cached value"
    );

    let broken = stored.replace("NAME = \"hello\"", "if (");
    let fallback = settings_managed_params("python", &broken);
    assert_eq!(
        fallback
            .iter()
            .find(|declaration| declaration.name == "NAME")
            .and_then(|declaration| declaration.default.clone()),
        Some(ParameterValue::String("hello".to_owned())),
        "syntax failure must keep the stored fallback"
    );
}

/// A row's edits merge onto the declaration that owns it, and nothing else moves.
///
/// Version 0.4 merges onto the row's own declaration rather than re-deriving it
/// (`src/skit/tui_settings.py:115-133`). An axis the row never offered has no control to move it,
/// so it must come out of a save exactly as it went in — which is also what makes a submit-time
/// filter unnecessary.
#[test]
fn tui_parameter_rows_merge_edits_onto_the_declaration_that_owns_them() {
    let mut stored = ParamDecl::new("token");
    stored.binding = ParameterBinding::EnvDefault;
    stored.delivery = ParameterDelivery::Env;
    stored.parameter_type = ParameterType::Str;
    stored.multiple = true;
    stored.repeat = true;
    stored.env_target = "TOKEN_TARGET".to_owned();
    stored.action = "append".to_owned();
    stored.flag = "--token".to_owned();

    let values = BTreeMap::from([
        ("parameter:token:keep".to_owned(), FieldValue::text("true")),
        (
            "parameter:token:type".to_owned(),
            FieldValue::text("choice"),
        ),
        (
            "parameter:token:default".to_owned(),
            FieldValue::text("green"),
        ),
        (
            "parameter:token:choices".to_owned(),
            FieldValue::text("red, green"),
        ),
        (
            "parameter:token:required".to_owned(),
            FieldValue::text("true"),
        ),
        (
            "parameter:token:prompt".to_owned(),
            FieldValue::text("Token"),
        ),
        (
            "parameter:token:help".to_owned(),
            FieldValue::text("Select a token."),
        ),
        (
            "parameter:token:secret".to_owned(),
            FieldValue::text("false"),
        ),
        (
            "parameter:token:env_source".to_owned(),
            FieldValue::text(""),
        ),
    ]);

    let mut declarations = vec![stored.clone()];
    tui_apply_parameter_edits(&values, &mut declarations).unwrap();
    assert_eq!(declarations.len(), 1);
    let declaration = &declarations[0];

    // The axes the row offered moved.
    assert_eq!(declaration.parameter_type, ParameterType::Choice);
    assert_eq!(
        declaration.default,
        Some(ParameterValue::String("green".to_owned()))
    );
    assert_eq!(declaration.choices, ["red".to_owned(), "green".to_owned()]);
    assert!(declaration.required);
    assert_eq!(declaration.prompt, "Token");
    assert_eq!(declaration.help, "Select a token.");

    // The axes the row never offered hold at what the set already stored, including the name.
    assert_eq!(declaration.name, "token");
    assert_eq!(declaration.binding, ParameterBinding::EnvDefault);
    assert_eq!(declaration.delivery, ParameterDelivery::Env);
    assert!(declaration.multiple);
    assert!(declaration.repeat);
    assert_eq!(declaration.env_target, "TOKEN_TARGET");
    assert_eq!(declaration.action, "append");
    assert_eq!(declaration.flag, "--token");

    // Clearing a default and changing a flag use the same row-owned edit. Neither change may
    // rebuild or replace the declaration that owns the other stored axes.
    let mut editable = stored.clone();
    editable.default = Some(ParameterValue::String("stored".to_owned()));
    let clear_and_flag = BTreeMap::from([
        ("parameter:token:default".to_owned(), FieldValue::text("")),
        ("parameter:token:flag".to_owned(), FieldValue::text("-t")),
    ]);
    let mut declarations = vec![editable];
    tui_apply_parameter_edits(&clear_and_flag, &mut declarations).unwrap();
    assert_eq!(declarations[0].default, None);
    assert_eq!(declarations[0].flag, "-t");
    assert_eq!(declarations[0].binding, ParameterBinding::EnvDefault);
    assert_eq!(declarations[0].delivery, ParameterDelivery::Env);
    assert!(declarations[0].multiple);
    assert!(declarations[0].repeat);
    assert_eq!(declarations[0].env_target, "TOKEN_TARGET");
    assert_eq!(declarations[0].action, "append");

    // A row's edit reaches that row and no other, even when a neighbour shares every axis name.
    let mut neighbours = vec![ParamDecl::new("first"), ParamDecl::new("second")];
    let one_row = BTreeMap::from([(
        "parameter:second:prompt".to_owned(),
        FieldValue::text("Second"),
    )]);
    tui_apply_parameter_edits(&one_row, &mut neighbours).unwrap();
    assert_eq!(neighbours[0].prompt, "");
    assert_eq!(neighbours[1].prompt, "Second");

    // Unticking keep removes the row, exactly as version 0.4's checkbox returns None (`:115-117`).
    let mut removed = values.clone();
    set(&mut removed, "parameter:token:keep", "false");
    let mut declarations = vec![stored.clone()];
    tui_apply_parameter_edits(&removed, &mut declarations).unwrap();
    assert!(declarations.is_empty());

    // Marking a public row secret drops the cached literal (`:125-131`).
    let mut plain = ParamDecl::new("token");
    plain.default = Some(ParameterValue::String("visible".to_owned()));
    let secret = BTreeMap::from([
        ("parameter:token:keep".to_owned(), FieldValue::text("true")),
        (
            "parameter:token:secret".to_owned(),
            FieldValue::text("true"),
        ),
        (
            "parameter:token:env_source".to_owned(),
            FieldValue::text("TOKEN_ENV"),
        ),
    ]);
    let mut declarations = vec![plain];
    tui_apply_parameter_edits(&secret, &mut declarations).unwrap();
    assert!(declarations[0].secret);
    assert!(
        declarations[0].default.is_none(),
        "a source literal survived into the block"
    );
    assert_eq!(declarations[0].env_source, "TOKEN_ENV");

    // Two declarations that resolve to one name are refused before anything is written.
    let mut duplicate = vec![stored.clone(), stored];
    assert!(
        tui_apply_parameter_edits(&values, &mut duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate parameter")
    );
}

#[test]
fn tui_source_controls_change_only_the_stored_copy() {
    let root = TempDir::new().unwrap();
    let original = root.path().join("source.sh");
    fs::write(&original, "NAME=world\necho \"$NAME\"\n").unwrap();
    let store = FileStore::new(root.path().join("library"));
    let service = LibraryService::new(store.clone());
    add(
        &service,
        AddOptions {
            source: Some(original.clone()),
            kind: None,
            name: Some("Tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let mut entry = service.show("tool").unwrap();
    let stored = store
        .entry_dir_path(&Slug::parse("tool").unwrap())
        .join("script.sh");
    let mut managed = ParamDecl::new("NAME");
    managed.binding = ParameterBinding::Const;
    managed.delivery = ParameterDelivery::Inject;
    managed.prompt = "Keep this prompt".to_owned();
    let rewritten =
        write_managed_params("shell", "NAME=world\necho \"$NAME\"\n", &[managed]).unwrap();
    let claimed = service.claim_identity(&entry).unwrap();
    entry = service
        .commit_copy_edit(&claimed, rewritten.as_bytes(), &entry.meta.source_hash)
        .unwrap();
    let source = fs::read_to_string(&stored).unwrap();
    let (source, managed) =
        prepare_source_management("shell", StorageMode::Copy, source, true, &[], &[], &[]).unwrap();
    let rewritten = write_managed_params("shell", &source, &managed).unwrap();
    let claimed = service.claim_identity(&entry).unwrap();
    entry = service
        .commit_copy_edit(&claimed, rewritten.as_bytes(), &entry.meta.source_hash)
        .unwrap();
    assert_eq!(
        managed_params("shell", &fs::read_to_string(&stored).unwrap())[0].prompt,
        "Keep this prompt"
    );

    let source = fs::read_to_string(&stored).unwrap();
    let (source, managed) = prepare_source_management(
        "shell",
        StorageMode::Copy,
        source,
        false,
        &[],
        &[],
        &["NAME".to_owned()],
    )
    .unwrap();
    let rewritten = write_managed_params("shell", &source, &managed).unwrap();
    let claimed = service.claim_identity(&entry).unwrap();
    service
        .commit_copy_edit(&claimed, rewritten.as_bytes(), &entry.meta.source_hash)
        .unwrap();

    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "NAME=world\necho \"$NAME\"\n"
    );
    assert!(
        fs::read_to_string(stored)
            .unwrap()
            .contains("NAME=\"${NAME:-world}\"")
    );
}

#[test]
fn plain_form_collection_uses_defaults_masks_secrets_and_refuses_end_of_input() {
    let form = FormView {
        purpose: FormPurpose::Run,
        title: "Run".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: Some("alpha".to_owned()),
        fields: vec![
            FormField::text("name", "Name", "default"),
            FormField::secret("token", "Token", ""),
            FormField::text_raw("raw", "User label", ""),
        ],
        focused: 0,
        submit_label: "Run".to_owned(),
    };
    let mut output = Vec::new();
    let values = collect_plain_form(
        &form,
        skit_i18n::Locale::En,
        &mut "\ncustom\n".as_bytes(),
        &mut output,
        |_| Ok("hidden".to_owned()),
    )
    .unwrap();
    assert_eq!(got(&values, "name"), "default");
    assert_eq!(got(&values, "token"), "hidden");
    assert_eq!(got(&values, "raw"), "custom");
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Name [default]: "));
    assert!(output.contains("Token: "));
    assert!(!output.contains("hidden"));

    let error = collect_plain_form(
        &form,
        skit_i18n::Locale::En,
        &mut "".as_bytes(),
        &mut Vec::new(),
        |_| Ok(String::new()),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("ended before the form was complete")
    );
    assert!(matches!(
        plain_form_error(io::Error::new(io::ErrorKind::UnexpectedEof, "end")),
        CliError::Aborted
    ));
    assert!(matches!(
        plain_form_error(io::Error::other("read")),
        CliError::Io(_)
    ));
}

#[test]
fn interactive_run_submission_keeps_explicit_clears_and_cli_fixed_values() {
    let baseline = BTreeMap::from([
        ("name".to_owned(), "Ada".to_owned()),
        ("count".to_owned(), "2".to_owned()),
    ]);
    let submitted = BTreeMap::from([
        ("value:name".to_owned(), FieldValue::text("")),
        ("value:count".to_owned(), FieldValue::text("3")),
    ]);
    assert_eq!(
        changed_form_values(&submitted, &baseline),
        ["count=3", "name="]
    );

    let declarations = [ParamDecl::new("name"), ParamDecl::new("count")];
    assert_eq!(
        run_fixed_values(
            &declarations,
            &["name=Grace".to_owned(), "count=".to_owned()]
        )
        .unwrap(),
        BTreeMap::from([
            ("count".to_owned(), String::new()),
            ("name".to_owned(), "Grace".to_owned()),
        ])
    );
    assert!(matches!(
        run_fixed_values(&declarations, &["missing=x".to_owned()]),
        Err(CliError::Run(RunError::UnknownSet { .. }))
    ));
    assert!(matches!(
        run_fixed_values(&declarations, &["broken".to_owned()]),
        Err(CliError::Run(RunError::InvalidSet { .. }))
    ));
}

#[test]
fn parameter_json_and_parser_helpers_cover_every_public_spelling() {
    assert_eq!(nonempty(""), None);
    assert_eq!(nonempty("value"), Some("value"));
    let source = |kind: &str, text: &str, parameters: Vec<ParamDecl>| {
        let settings = EntrySettings {
            parameters,
            ..EntrySettings::default()
        };
        form_plan(kind, text, &settings).source.as_str()
    };
    assert_eq!(source("command", "", Vec::new()), "command");
    assert_eq!(source("prompt", "", Vec::new()), "command");
    assert_eq!(source("python", "", Vec::new()), "none");
    assert_eq!(
        source("python", "", vec![ParamDecl::new("plain")]),
        "declared"
    );
    let managed_source = concat!(
        "# /// script\n",
        "# dependencies = []\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "# [[tool.skit.params]]\n",
        "# name = \"bound\"\n",
        "# kind = \"const\"\n",
        "# type = \"str\"\n",
        "# ///\n",
        "bound = 'value'\n",
    );
    assert_eq!(source("python", managed_source, Vec::new()), "inject");
    for (source, origin) in [
        ("command", "command"),
        ("inject", "managed"),
        ("argparse", "reader"),
        ("declared", "declared"),
        ("none", "none"),
    ] {
        assert_eq!(parameter_origin(source), origin);
    }

    let mut declaration = ParamDecl::new("path");
    declaration.prompt = "Select path".to_owned();
    declaration.parameter_type = ParameterType::Path;
    declaration.delivery = ParameterDelivery::Env;
    declaration.default = Some(ParameterValue::String(String::new()));
    let record = field_json(
        &form_plan(
            "python",
            "",
            &EntrySettings {
                parameters: vec![declaration.clone()],
                ..EntrySettings::default()
            },
        )
        .fields[0],
    );
    assert_eq!(record["label"], "Select path");
    assert_eq!(record["delivers_empty"], true);
    declaration.secret = true;
    assert_eq!(
        field_json(
            &form_plan(
                "python",
                "",
                &EntrySettings {
                    parameters: vec![declaration.clone()],
                    ..EntrySettings::default()
                },
            )
            .fields[0]
        )["delivers_empty"],
        false
    );
    declaration.secret = false;
    declaration.multiple = true;
    assert_eq!(
        field_json(
            &form_plan(
                "python",
                "",
                &EntrySettings {
                    parameters: vec![declaration],
                    ..EntrySettings::default()
                },
            )
            .fields[0]
        )["delivers_empty"],
        false
    );

    assert_eq!(
        assignment("name=value", "field").unwrap(),
        ("name", "value")
    );
    assert!(assignment("=value", "field").is_err());
    assert!(assignment("value", "field").is_err());

    for (value, expected) in [
        ("str", ParameterType::Str),
        ("int", ParameterType::Int),
        ("float", ParameterType::Float),
        ("bool", ParameterType::Bool),
        ("choice", ParameterType::Choice),
        ("path", ParameterType::Path),
    ] {
        assert_eq!(parse_parameter_type(value).unwrap(), expected);
    }
    assert!(parse_parameter_type("future").is_err());
}

#[test]
fn params_host_updates_managed_secrets_and_skips_an_invalid_environment_source_without_writes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path().join("data"));
    let service = LibraryService::new(store.clone());
    let mut managed = ParamDecl::new("NAME");
    managed.binding = ParameterBinding::Const;
    managed.delivery = ParameterDelivery::Inject;
    managed.secret = true;
    managed.env_source = "TOKEN".to_owned();
    let source = write_managed_params("shell", "NAME=world\necho \"$NAME\"\n", &[managed]).unwrap();
    let entry = service
        .add(CreateEntry {
            name: "Managed shell".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: source.into_bytes(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                parameters: vec![ParamDecl::new("legacy")],
                ..EntrySettings::default()
            },
        })
        .unwrap();
    let stored = store.entry_dir_path(&entry.slug).join("script.sh");
    let meta = store.entry_dir_path(&entry.slug).join("meta.toml");

    let parse = |arguments: &[&str]| {
        let cli = Cli::try_parse_from(arguments).unwrap();
        let Some(Command::Params(arguments)) = cli.command else {
            panic!("params must parse as the params command");
        };
        *arguments
    };
    params(
        &service,
        &store,
        parse(&[
            "skit",
            "params",
            entry.slug.as_str(),
            "--prompt",
            "NAME=Display name",
            "--env-source",
            "NAME= ALT_TOKEN ",
            "--json",
        ]),
    )
    .unwrap();
    let declaration = &managed_params("shell", &fs::read_to_string(&stored).unwrap())[0];
    assert_eq!(declaration.prompt, "Display name");
    assert_eq!(declaration.env_source, "ALT_TOKEN");
    assert!(declaration.secret);
    assert_eq!(
        EntrySettings::from_meta(&service.show(entry.slug.as_str()).unwrap().meta).parameters,
        [ParamDecl::new("legacy")]
    );

    params(
        &service,
        &store,
        parse(&[
            "skit",
            "params",
            entry.slug.as_str(),
            "--no-secret",
            "NAME",
            "--json",
        ]),
    )
    .unwrap();
    let declaration = &managed_params("shell", &fs::read_to_string(&stored).unwrap())[0];
    assert!(!declaration.secret);
    assert!(declaration.env_source.is_empty());

    let source_before = fs::read(&stored).unwrap();
    let meta_before = fs::read(&meta).unwrap();
    params(
        &service,
        &store,
        parse(&[
            "skit",
            "params",
            entry.slug.as_str(),
            "--env-source",
            "NAME=SHOULD_NOT_APPLY",
            "--json",
        ]),
    )
    .unwrap();
    assert_eq!(fs::read(&stored).unwrap(), source_before);
    assert_eq!(fs::read(&meta).unwrap(), meta_before);
}

#[test]
fn tui_scalar_helpers_reject_incomplete_and_incompatible_rows() {
    assert_eq!(
        tui_parameter_value(&ParameterValue::String("text".to_owned())),
        "text"
    );
    assert_eq!(tui_parameter_value(&ParameterValue::Integer(2)), "2");
    assert_eq!(tui_parameter_value(&ParameterValue::Float(2.5)), "2.5");
    assert_eq!(tui_parameter_value(&ParameterValue::Bool(true)), "true");
    let plain = tui_options_field("key", "Label", "Options", &[], "value");
    assert_eq!(plain.label, "Label");
    assert!(plain.label_arguments.is_empty());
    assert_eq!(
        tui_options_field(
            "key",
            "Label",
            "Options: {}",
            &["one".to_owned(), "two".to_owned()],
            "one",
        )
        .label_arguments,
        ["one, two"]
    );

    let mut untouched = vec![ParamDecl::new("item")];
    tui_apply_parameter_edits(&BTreeMap::new(), &mut untouched).unwrap();
    assert_eq!(untouched, [ParamDecl::new("item")]);
    // A row is addressed by its own name. Name, binding and delivery have no control, so the only
    // rejections left are the values a control can actually produce.
    let refuses = |axis: &str, value: &str| {
        let values = BTreeMap::from([(format!("parameter:item:{axis}"), FieldValue::text(value))]);
        let mut declarations = vec![ParamDecl::new("item")];
        tui_apply_parameter_edits(&values, &mut declarations).is_err()
    };
    assert!(refuses("type", "future"));
    assert!(refuses("keep", "maybe"));
    let mut invalid_default =
        BTreeMap::from([("parameter:item:type".to_owned(), FieldValue::text("int"))]);
    set(&mut invalid_default, "parameter:item:default", "not-an-int");
    let mut declarations = vec![ParamDecl::new("item")];
    assert!(tui_apply_parameter_edits(&invalid_default, &mut declarations).is_err());
    // A choice type with no choices is refused before anything is written.
    assert!(refuses("type", "choice"));

    assert!(tui_selector(&None).is_err());
    assert_eq!(tui_selector(&Some(String::new())).unwrap(), "");
    assert_eq!(tui_selector(&Some("item".to_owned())).unwrap(), "item");
    let values = BTreeMap::from([
        ("key".to_owned(), FieldValue::text(" value ")),
        ("yes".to_owned(), FieldValue::text("YES")),
    ]);
    assert_eq!(tui_value(&values, "key"), "value");
    assert_eq!(tui_nonempty_owned(&values, "key"), Some("value".to_owned()));
    assert_eq!(tui_nonempty_owned(&values, "missing"), None);
    assert_eq!(tui_required(&values, "key").unwrap(), "value");
    assert!(tui_required(&values, "missing").is_err());
    for value in ["true", "YES", "1", "on"] {
        assert!(tui_bool(value).unwrap());
    }
    for value in ["", "false", "NO", "0", "off"] {
        assert!(!tui_bool(value).unwrap());
    }
    assert!(tui_bool("sometimes").is_err());
    assert_eq!(
        split_windows_arguments(r#"agent.exe "C:\Program Files\input.txt" """#).unwrap(),
        ["agent.exe", r"C:\Program Files\input.txt", ""]
    );
    let windows = ["agent.exe", r"C:\Program Files\input.txt", "", r#"a\"b"#];
    assert_eq!(
        split_windows_arguments(&join_windows_arguments(&windows)).unwrap(),
        windows
    );
    let executable = EntryKind::parse("exe").unwrap();
    assert_eq!(payload_stored_name(&executable, Path::new("")), "script");
    assert_eq!(
        payload_stored_name(&executable, Path::new("tool.custom")),
        "tool.custom"
    );
}

#[derive(Debug, Default)]
struct HealthProbe {
    programs: BTreeMap<String, std::path::PathBuf>,
    directories: Vec<std::path::PathBuf>,
}

impl ProgramProbe for HealthProbe {
    fn find_program(&self, name: &str) -> Option<std::path::PathBuf> {
        self.programs.get(name).cloned()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.directories.iter().any(|directory| directory == path)
    }

    fn is_executable(&self, path: &Path) -> bool {
        path.is_file()
    }
}

fn health_entry(kind: &str) -> Entry {
    Entry {
        slug: Slug::parse("health").unwrap(),
        meta: EntryMeta::minimal("Health", EntryKind::parse(kind).unwrap()),
    }
}

#[test]
fn doctor_launch_checks_cover_every_runtime_and_workdir_policy() {
    let root = TempDir::new().unwrap();
    let config = FileConfigStore::new(root.path());
    let mut entry = health_entry("shell");
    entry.meta.workdir = "relative".to_owned();
    assert!(
        doctor_launch_block(
            &entry,
            &EntrySettings::default(),
            &config,
            &HealthProbe::default()
        )
        .unwrap()
        .unwrap()
        .localize(Locale::En)
        .contains("must be absolute")
    );
    entry.meta.workdir = root.path().join("missing").display().to_string();
    assert!(
        doctor_launch_block(
            &entry,
            &EntrySettings::default(),
            &config,
            &HealthProbe::default()
        )
        .unwrap()
        .unwrap()
        .localize(Locale::En)
        .contains("does not exist")
    );
    entry.meta.workdir = root.path().display().to_string();
    let mut probe = HealthProbe {
        directories: vec![root.path().to_owned()],
        ..HealthProbe::default()
    };
    assert!(
        doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains("bash")
    );
    let custom_bash = root.path().join("custom-bash");
    fs::write(&custom_bash, "#!/bin/sh\n").unwrap();
    config
        .set("shell.bash_path", custom_bash.to_str().unwrap())
        .unwrap();
    assert!(
        doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains(custom_bash.to_str().unwrap())
    );

    // A python entry is never launch-blocked over uv: the oracle python preflight
    // checks only that the script exists, and uv is bootstrapped at run time.
    let python = health_entry("python");
    assert_eq!(
        doctor_launch_block(
            &python,
            &EntrySettings::default(),
            &config,
            &HealthProbe::default()
        )
        .unwrap(),
        None,
    );

    for (kind, program) in [
        ("fish", "fish"),
        ("powershell", "pwsh"),
        ("ruby", "ruby"),
        ("perl", "perl"),
        ("lua", "lua"),
        ("r", "Rscript"),
        ("command", if cfg!(windows) { "cmd.exe" } else { "sh" }),
    ] {
        let mut entry = health_entry(kind);
        entry.meta.workdir = "invoke".to_owned();
        assert!(
            doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe)
                .unwrap()
                .unwrap()
                .localize(Locale::En)
                .contains(program),
            "kind={kind}",
        );
        probe
            .programs
            .insert(program.to_owned(), std::path::PathBuf::from(program));
        assert_eq!(
            doctor_launch_block(&entry, &EntrySettings::default(), &config, &probe).unwrap(),
            None,
            "kind={kind}",
        );
    }

    let javascript = health_entry("js");
    assert!(
        doctor_launch_block(
            &javascript,
            &EntrySettings::default(),
            &config,
            &HealthProbe::default(),
        )
        .unwrap()
        .unwrap()
        .localize(Locale::En)
        .contains("deno, bun, or node")
    );
    probe
        .programs
        .insert("node".to_owned(), std::path::PathBuf::from("node"));
    assert_eq!(
        doctor_launch_block(&javascript, &EntrySettings::default(), &config, &probe).unwrap(),
        None
    );

    let prompt = health_entry("prompt");
    let pinned = EntrySettings {
        runner: "agent".to_owned(),
        ..EntrySettings::default()
    };
    assert!(
        doctor_launch_block(&prompt, &pinned, &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains("not configured")
    );
    config
        .set_runner(
            skit_store::PromptRunner {
                name: "agent".to_owned(),
                argv: vec!["agent-bin".to_owned(), "{{prompt}}".to_owned()],
            },
            false,
        )
        .unwrap();
    assert!(
        doctor_launch_block(&prompt, &pinned, &config, &probe)
            .unwrap()
            .unwrap()
            .localize(Locale::En)
            .contains("agent-bin")
    );
    probe.programs.insert(
        "agent-bin".to_owned(),
        std::path::PathBuf::from("agent-bin"),
    );
    assert_eq!(
        doctor_launch_block(&prompt, &pinned, &config, &probe).unwrap(),
        None
    );
    assert_eq!(
        doctor_launch_block(&prompt, &EntrySettings::default(), &config, &probe).unwrap(),
        None
    );
    assert_eq!(
        doctor_launch_block(
            &health_entry("exe"),
            &EntrySettings::default(),
            &config,
            &probe
        )
        .unwrap(),
        None
    );
    assert!(
        doctor_launch_block(
            &health_entry("future"),
            &EntrySettings::default(),
            &config,
            &probe,
        )
        .unwrap()
        .unwrap()
        .localize(Locale::En)
        .contains("unknown entry kind")
    );
    assert_eq!(
        interpreter_name(&EntrySettings::default(), "default"),
        "default"
    );
    assert_eq!(
        interpreter_name(
            &EntrySettings {
                interpreter: "custom".to_owned(),
                ..EntrySettings::default()
            },
            "default",
        ),
        "custom"
    );
    let custom = EntrySettings {
        interpreter: "custom-shell".to_owned(),
        ..EntrySettings::default()
    };
    let mut custom_probe = HealthProbe::default();
    custom_probe
        .programs
        .insert("custom-shell".to_owned(), root.path().join("custom-shell"));
    assert_eq!(
        doctor_launch_block(&health_entry("shell"), &custom, &config, &custom_probe).unwrap(),
        None
    );
}

#[test]
fn doctor_source_and_size_helpers_are_total_on_missing_corrupt_and_nested_paths() {
    let root = TempDir::new().unwrap();
    assert_eq!(directory_size(&root.path().join("missing")), 0);
    fs::write(root.path().join("one"), b"1234").unwrap();
    assert_eq!(directory_size(&root.path().join("one")), 0);
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("nested/two"), b"12").unwrap();
    assert_eq!(directory_size(root.path()), 6);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.path().join("one"), root.path().join("link")).unwrap();
        assert_eq!(directory_size(&root.path().join("link")), 0);
        assert_eq!(directory_size(root.path()), 10);
        std::os::unix::fs::symlink(root.path().join("nested"), root.path().join("dir-link"))
            .unwrap();
        assert_eq!(directory_size(root.path()), 10);
    }

    let library = root.path().join("library");
    let store = FileStore::new(&library);
    let directory = library.join("scripts/health");
    fs::create_dir_all(&directory).unwrap();
    let mut prompt = health_entry("prompt");
    EntrySettings {
        params: vec!["missing".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut prompt.meta);
    fs::write(directory.join("prompt.md"), "Hello {{name}}").unwrap();
    assert!(doctor_entry_drifted(&store, &prompt));
    EntrySettings {
        interpolate: false,
        params: vec!["missing".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut prompt.meta);
    assert!(!doctor_entry_drifted(&store, &prompt));
    fs::write(directory.join("prompt.md"), [0xff]).unwrap();
    assert!(!doctor_entry_drifted(&store, &prompt));
    fs::remove_file(directory.join("prompt.md")).unwrap();
    assert!(!doctor_entry_drifted(&store, &prompt));

    let mut current = health_entry("shell");
    let mut declaration = ParamDecl::new("NAME");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    fs::write(
        directory.join("script.sh"),
        write_managed_params("shell", "NAME=world\n", &[declaration]).unwrap(),
    )
    .unwrap();
    assert!(!doctor_entry_drifted(&store, &current));
    current.meta.kind = EntryKind::parse("prompt").unwrap();
    EntrySettings {
        params: vec!["name".to_owned()],
        ..EntrySettings::default()
    }
    .write_to_meta(&mut current.meta);
    fs::write(directory.join("prompt.md"), "Hello {{name}}").unwrap();
    assert!(!doctor_entry_drifted(&store, &current));

    assert!(!entry_missing(&store, &health_entry("command")));
    let mut executable = health_entry("exe");
    executable.meta.mode = StorageMode::Reference;
    executable.meta.source.clear();
    assert_eq!(
        entry_target(&store, &executable),
        Some(std::path::PathBuf::from("."))
    );
    assert!(!entry_missing(&store, &executable));
    executable.meta.source = root.path().join("gone").display().to_string();
    assert!(entry_missing(&store, &executable));
    fs::write(root.path().join("gone"), "x").unwrap();
    assert!(!entry_missing(&store, &executable));
}

#[test]
fn shared_health_inspector_reports_typed_entry_runner_and_rebuild_facts() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[prompt]\nrunners_seeded = true\nrunners = \"broken\"\n",
    )
    .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    service
        .add(CreateEntry {
            name: "Missing program".to_owned(),
            kind: EntryKind::parse("exe").unwrap(),
            mode: StorageMode::Reference,
            source: root.path().join("gone-program").display().to_string(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    service
        .add(CreateEntry {
            name: "Missing need".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings {
                template: "printf ok".to_owned(),
                needs: vec!["skit-health-tool-that-does-not-exist".to_owned()],
                ..EntrySettings::default()
            },
        })
        .unwrap();

    let inspector = CliHealthInspector::new(&service, &store, &config_dir);
    let debug = format!("{inspector:?}");
    assert!(debug.contains(data_dir.to_str().unwrap()));
    assert!(debug.contains(config_dir.to_str().unwrap()));
    let snapshot = inspector.inspect().unwrap();
    assert_eq!(snapshot.entry_count, 2);
    assert!(!matches!(snapshot.uv, UvHealth::Missing));
    assert!(snapshot.issues.iter().any(|issue| {
        issue.name == "Missing program" && issue.kind == HealthIssueKind::MissingTarget
    }));
    assert!(snapshot.issues.iter().any(|issue| {
        issue.name == "Missing need"
            && issue.kind
                == HealthIssueKind::MissingNeeds {
                    tools: vec!["skit-health-tool-that-does-not-exist".to_owned()],
                }
    }));
    assert_eq!(snapshot.invalid_runner_rows.len(), 1);
    assert_eq!(
        snapshot.library_size,
        health_size_text(directory_size(&data_dir.join("scripts")))
    );

    let rebuilt = inspector.rebuild().unwrap();
    assert_eq!(rebuilt.outcome.entry_count, 2);
    assert_eq!(rebuilt.snapshot.entry_count, 2);

    fs::write(
        config_dir.join("config.toml"),
        concat!(
            "[mirror]\n",
            "enabled = true\n",
            "pypi = \"https://mirror.example/simple\"\n",
            "[prompt]\n",
            "runners_seeded = true\n",
            "runners = []\n",
        ),
    )
    .unwrap();
    let config_on = fs::read(config_dir.join("config.toml")).unwrap();
    let snapshot = inspector.inspect().unwrap();
    let MirrorHealth::On { axes } = snapshot.mirror else {
        panic!("enabled mirror must report On: {:?}", snapshot.mirror);
    };
    assert_eq!(
        axes,
        "pypi=https://mirror.example/simple · github=off · npm=off"
    );
    assert_eq!(fs::read(config_dir.join("config.toml")).unwrap(), config_on);

    fs::write(
        config_dir.join("config.toml"),
        concat!(
            "[mirror]\n",
            "enabled = false\n",
            "pypi = \"https://mirror.example/simple\"\n",
            "[prompt]\n",
            "runners_seeded = true\n",
            "runners = []\n",
        ),
    )
    .unwrap();
    let config_paused = fs::read(config_dir.join("config.toml")).unwrap();
    let snapshot = inspector.inspect().unwrap();
    let MirrorHealth::Paused { axes } = snapshot.mirror else {
        panic!(
            "disabled configured mirror must report Paused: {:?}",
            snapshot.mirror
        );
    };
    assert_eq!(
        axes,
        "pypi=https://mirror.example/simple · github=off · npm=off"
    );
    assert_eq!(
        fs::read(config_dir.join("config.toml")).unwrap(),
        config_paused
    );

    let stale = add_command(&service, "Stale health row", "printf stale");
    let registry_path = data_dir.join("registry.toml");
    let registry_before = fs::read(&registry_path).unwrap();
    fs::remove_file(
        data_dir
            .join("scripts")
            .join(stale.slug.as_str())
            .join("meta.toml"),
    )
    .unwrap();
    let snapshot = inspector.inspect().unwrap();
    assert!(
        snapshot
            .diagnostics
            .iter()
            .any(|line| line.contains(stale.slug.as_str()))
    );
    assert_eq!(fs::read(&registry_path).unwrap(), registry_before);

    let preserved = service.show("Missing program").unwrap();
    let preserved_meta = data_dir
        .join("scripts")
        .join(preserved.slug.as_str())
        .join("meta.toml");
    let preserved_bytes = fs::read(&preserved_meta).unwrap();
    let state_dir = root.path().join("state");
    let action = tui_effect(
        &service,
        &store,
        &state_dir,
        &config_dir,
        UiEffect::HealthRebuild,
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Health(HealthAction::Rebuilt { .. })
    ));
    assert_ne!(fs::read(&registry_path).unwrap(), registry_before);
    assert_eq!(fs::read(&preserved_meta).unwrap(), preserved_bytes);
    assert_eq!(
        fs::read(config_dir.join("config.toml")).unwrap(),
        config_paused
    );
    assert!(!state_dir.exists());
}

#[test]
fn health_size_text_matches_the_latest_main_units_and_thresholds() {
    assert_eq!(health_size_text(0), "0 B");
    assert_eq!(health_size_text(1023), "1023 B");
    assert_eq!(health_size_text(1024), "1.0 KB");
    assert_eq!(health_size_text(1024 * 1024), "1.0 MB");
    assert_eq!(health_size_text(1024 * 1024 * 1024), "1.0 GB");
}

#[test]
fn tui_host_opens_every_frontend_neutral_screen_and_handles_simple_effects() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let mut entry = add_command(&service, "Alpha", "printf '%s' '{name}'");
    let declarations = entry_parameters(&store, &entry);
    let state = FormStateService::new(FileFormStateStore::new(&state_dir));
    state
        .save_last(
            &entry.slug,
            &declarations,
            Some(&BTreeMap::from([("name".to_owned(), "Ada".to_owned())])),
            None,
            false,
        )
        .unwrap();
    let mut settings = EntrySettings::from_meta(&entry.meta);
    settings.runner = "codex".to_owned();
    let claimed = service.claim_identity(&entry).unwrap();
    entry = service
        .update_settings(&claimed, &settings, "invoke")
        .unwrap();

    let broken = data_dir.join("scripts/broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("meta.toml"), "name = [broken").unwrap();
    state
        .save_preset(
            &entry.slug,
            "fast",
            &declarations,
            &BTreeMap::from([("name".to_owned(), "Grace".to_owned())]),
        )
        .unwrap();

    FileConfigStore::new(&config_dir)
        .ensure_runners_seeded()
        .unwrap();

    let Screen::Run(run_form) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Run,
        Some(entry.slug.as_str().to_owned()),
    )
    .unwrap() else {
        panic!("run must open the typed launch form");
    };
    assert!(
        run_form
            .fields()
            .iter()
            .all(|field| field.key != "_skit_runner"),
        "non-prompt entries must not show the prompt-runner picker"
    );
    let context = run_form
        .context()
        .expect("the real host must attach context");
    assert_eq!(context.entry_kind, "command");
    assert_eq!(
        context.path.as_ref().unwrap().workdir,
        std::env::current_dir().unwrap().display().to_string()
    );
    assert_eq!(
        context.tokens.cwd,
        context.path.as_ref().unwrap().invoke_cwd
    );
    assert_eq!(
        run_form
            .fields()
            .iter()
            .find(|field| field.key == "_skit_args")
            .unwrap()
            .label,
        "Extra command arguments"
    );
    // Both entry-settings doors open the one typed screen. `s` names the section it lands on, and
    // version 0.4 has no second screen to open (`src/skit/tui.py:991-992`).
    for (request, revealed) in [
        (HostRequest::Settings, None),
        (HostRequest::Presets, Some(SettingsSectionId::Presets)),
    ] {
        let Screen::Settings(view) = tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            request,
            Some(entry.slug.as_str().to_owned()),
        )
        .unwrap() else {
            panic!("{request:?} must open the typed settings screen");
        };
        assert_eq!(view.revealed(), revealed);
    }
    assert!(matches!(
        tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Rename,
            Some(entry.slug.as_str().to_owned()),
        )
        .unwrap(),
        Screen::Form(_)
    ));
    assert!(matches!(
        tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Add,
            None,
        )
        .unwrap(),
        Screen::Add(_)
    ));
    assert!(matches!(
        tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Runners,
            None,
        )
        .unwrap(),
        Screen::Runners(_)
    ));
    let Screen::Preferences(preferences) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Preferences,
        None,
    )
    .unwrap() else {
        panic!("Preferences must use its typed workflow");
    };
    assert_eq!(preferences.draft().language_options[0], "auto");
    assert_eq!(preferences.draft().runner_names.len(), 8);
    assert!(matches!(
        tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Health,
            None,
        )
        .unwrap(),
        Screen::Health(_)
    ));

    for effect in [UiEffect::None, UiEffect::Quit] {
        assert_eq!(
            tui_effect(&service, &store, &state_dir, &config_dir, effect,).unwrap(),
            UiAction::ClearStatus
        );
    }
    // A reload carries the complete projection, not a bare scan: `Action::Replace` clears every
    // detail fact, so a scan-only reload would empty the detail pane and lose activity order.
    assert!(matches!(
        tui_effect(&service, &store, &state_dir, &config_dir, UiEffect::Reload,).unwrap(),
        UiAction::ReplaceSurface { .. }
    ));
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Open {
                request: HostRequest::Run,
                selector: Some(entry.slug.as_str().to_owned()),
            },
        )
        .unwrap(),
        UiAction::Present(Screen::Run(_))
    ));

    let source = root.path().join("editable.sh");
    fs::write(&source, "echo ok\n").unwrap();
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Editable".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    FileConfigStore::new(&config_dir)
        .set("editor", "true")
        .unwrap();
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Edit {
                selector: "editable".to_owned(),
            },
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    let editable = service.show("editable").unwrap();
    let editable_values = settings_edits(&service, &store, &state_dir, "editable", &[]);
    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Settings,
            Some(editable.slug.as_str().to_owned()),
            &editable_values,
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));

    let disposable = add_command(&service, "Disposable", "true");
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Remove {
                selector: disposable.slug.as_str().to_owned(),
            },
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    assert!(service.show("Disposable").is_err());
}

#[test]
fn tui_runner_host_preserves_editor_input_on_stale_rows_and_rechecks_prompt_pins() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        concat!(
            "[prompt]\n",
            "runners_seeded = true\n",
            "[[prompt.runners]]\n",
            "name = \"agent\"\n",
            "argv = [\"agent\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());

    let row = tui_runner_rows(&service, &config_dir)
        .unwrap()
        .into_iter()
        .find(|row| row.name.as_deref() == Some("agent"))
        .unwrap();
    let mut stale = row.key_identities.clone();
    stale[0].snapshot_token.push_str("-stale");
    let action = tui_save_runner(
        &service,
        &config_dir,
        RunnerSaveRequest {
            name: "agent".to_owned(),
            argv: vec![
                "agent".to_owned(),
                "--once".to_owned(),
                "{{prompt}}".to_owned(),
            ],
            target: RunnerSaveTarget::Named {
                name: "agent".to_owned(),
                expected: stale,
            },
        },
        RunnerSaveOwner::Manager,
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Runners(RunnerManagerAction::MutationFailed(_))
    ));
    assert_eq!(
        FileConfigStore::new(&config_dir).runners().unwrap()[0].argv,
        ["agent", "{{prompt}}"]
    );

    let add_pinned_prompt = |name: &str| {
        let mut settings = EntrySettings {
            runner: "agent".to_owned(),
            ..EntrySettings::default()
        };
        settings.params = Vec::new();
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("prompt").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"Review this".to_vec(),
                    stored_name: Some("prompt.md".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings,
            })
            .unwrap();
    };
    add_pinned_prompt("First prompt");
    let row = tui_runner_rows(&service, &config_dir)
        .unwrap()
        .into_iter()
        .find(|row| row.name.as_deref() == Some("agent"))
        .unwrap();
    assert_eq!(row.pinned_count, 1);
    add_pinned_prompt("Second prompt");
    let action = tui_remove_runner(
        &service,
        &config_dir,
        RunnerRemoveRequest::Named {
            name: "agent".to_owned(),
            expected: row.key_identities,
            expected_pinned_count: row.pinned_count,
        },
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Runners(RunnerManagerAction::MutationFailed(_))
    ));
    assert_eq!(
        FileConfigStore::new(&config_dir).runners().unwrap().len(),
        1
    );
}

#[test]
fn tui_runner_host_routes_success_to_the_exact_standalone_editor_owner() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let owner = skit_ui::RunnerEditorOwner::Run {
        selector: "prompt".to_owned(),
    };
    let action = tui_save_runner(
        &service,
        &config_dir,
        RunnerSaveRequest {
            name: "new-agent".to_owned(),
            argv: vec!["new-agent".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::New,
        },
        RunnerSaveOwner::Editor(owner.clone()),
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::RunnerEditorSaved {
            owner: saved_owner,
            ref name,
            ..
        } if saved_owner == owner && name == "new-agent"
    ));
    assert!(
        FileConfigStore::new(&config_dir)
            .runners()
            .unwrap()
            .iter()
            .any(|runner| runner.name == "new-agent")
    );
    let action = tui_effect(
        &service,
        &store,
        &root.path().join("state"),
        &config_dir,
        UiEffect::RefreshPreferencesAfterRunners,
    )
    .unwrap();
    let UiAction::RunnerManagerClosed { preferences } = action else {
        panic!("closing the runner manager must refresh typed Preferences");
    };
    assert!(
        preferences
            .draft()
            .runner_names
            .iter()
            .any(|name| name == "new-agent")
    );
}

#[test]
fn tui_run_host_publishes_source_drift_degradation_and_runtime_path_context() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");

    let mut managed = ParamDecl::new("NAME");
    managed.binding = ParameterBinding::Const;
    managed.delivery = ParameterDelivery::Inject;
    let drift_dir = data_dir.join("scripts/drift");
    fs::create_dir_all(&drift_dir).unwrap();
    fs::write(
        drift_dir.join("meta.toml"),
        "name = \"Drift\"\nkind = \"shell\"\nmode = \"copy\"\ndescription = \"\"\n",
    )
    .unwrap();
    fs::write(
        drift_dir.join("script.sh"),
        write_managed_params("shell", "echo done\n", &[managed]).unwrap(),
    )
    .unwrap();

    let dynamic_dir = data_dir.join("scripts/dynamic");
    fs::create_dir_all(&dynamic_dir).unwrap();
    fs::write(
        dynamic_dir.join("meta.toml"),
        "name = \"Dynamic\"\nkind = \"python\"\nmode = \"copy\"\ndescription = \"\"\n",
    )
    .unwrap();
    fs::write(
        dynamic_dir.join("script.py"),
        "p.add_argument('--x')\np.add_subparsers()\n",
    )
    .unwrap();

    let store = FileStore::new(&data_dir);
    store.rebuild_registry().unwrap();
    let service = LibraryService::new(store.clone());
    let Screen::Run(drift) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Run,
        Some("drift".to_owned()),
    )
    .unwrap() else {
        panic!("run must open the typed launch form");
    };
    assert!(!drift.drift_lines.is_empty());
    let context = drift.context().unwrap();
    assert_eq!(context.entry_kind, "shell");
    assert_eq!(
        context.path.as_ref().unwrap().workdir,
        std::env::current_dir().unwrap().display().to_string()
    );

    let Screen::Run(dynamic) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Run,
        Some("dynamic".to_owned()),
    )
    .unwrap() else {
        panic!("run must open the typed launch form");
    };
    assert_eq!(dynamic.degraded_reason.as_deref(), Some("subparsers"));
}

#[test]
fn typed_preferences_effects_validate_atomically_and_install_only_after_selection() {
    let root = TempDir::new().unwrap();
    let config_dir = root.path().join("config");
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let config = FileConfigStore::new(&config_dir);
    config.set("editor", "vi").unwrap();
    let config_path = config_dir.join("config.toml");
    let config_before_refusal = fs::read(&config_path).unwrap();

    let missing_bash = root.path().join("missing-bash");
    let directory = root.path().join("bash-directory");
    fs::create_dir(&directory).unwrap();
    for invalid in [&missing_bash, &directory] {
        let refused = skit_application::preferences::PreferencesChangeSet {
            settings: BTreeMap::from([
                ("editor".to_owned(), "micro".to_owned()),
                ("shell.bash_path".to_owned(), invalid.display().to_string()),
            ]),
        };
        assert!(matches!(
            tui_preferences_effect(&service, &config_dir, PreferencesEffect::Save(refused))
                .unwrap(),
            UiAction::Preferences(PreferencesAction::ValidationFailed(_))
        ));
        assert_eq!(config.get("editor").unwrap(), "vi");
        assert_eq!(fs::read(&config_path).unwrap(), config_before_refusal);
        assert!(!config_dir.join("config.toml.bak").exists());
    }

    let accepted = skit_application::preferences::PreferencesChangeSet {
        settings: BTreeMap::from([
            ("editor".to_owned(), "micro".to_owned()),
            ("lang".to_owned(), "zh-TW".to_owned()),
        ]),
    };
    assert_eq!(
        tui_preferences_effect(&service, &config_dir, PreferencesEffect::Save(accepted)).unwrap(),
        UiAction::PreferencesSaved {
            locale: "zh-TW".to_owned(),
            message: "Preferences saved".to_owned(),
        }
    );
    assert_eq!(config.get("editor").unwrap(), "micro");
    assert_eq!(config.get("lang").unwrap(), "zh-TW");

    assert!(matches!(
        tui_preferences_effect(
            &service,
            &config_dir,
            PreferencesEffect::DiscoverAgentSkillTargets,
        )
        .unwrap(),
        UiAction::Preferences(PreferencesAction::PresentAgentSkillTargets(_))
    ));

    let skills_dir = root.path().join("agent/skills");
    let action = tui_preferences_effect(
        &service,
        &config_dir,
        PreferencesEffect::InstallAgentSkill {
            skills_dir: skills_dir.clone(),
        },
    )
    .unwrap();
    let UiAction::Preferences(PreferencesAction::AgentSkillInstalled { message }) = action else {
        panic!("successful install must stay in typed Preferences");
    };
    let written = skills_dir.join("skit/SKILL.md");
    assert!(message.contains(&written.display().to_string()));
    assert_eq!(
        fs::read(written).unwrap(),
        include_bytes!("../../../../skills/skit/SKILL.md")
    );

    let config_before_noops = fs::read(&config_path).unwrap();
    for effect in [
        PreferencesEffect::None,
        PreferencesEffect::Close,
        PreferencesEffect::ConfirmDiscard,
    ] {
        assert_eq!(
            tui_effect(
                &service,
                &store,
                &state_dir,
                &config_dir,
                UiEffect::Preferences(effect),
            )
            .unwrap(),
            UiAction::ClearStatus
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before_noops);
        assert!(!data_dir.exists());
        assert!(!state_dir.exists());
    }

    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Preferences(PreferencesEffect::ManageAgents),
        )
        .unwrap(),
        UiAction::Present(Screen::Runners(_))
    ));
    assert!(
        !FileConfigStore::new(&config_dir)
            .runners()
            .unwrap()
            .is_empty()
    );
    assert!(!data_dir.exists());
    assert!(!state_dir.exists());

    let broken_config = root.path().join("config-is-a-file");
    fs::write(&broken_config, b"keep config bytes").unwrap();
    let failure = PreferencesChangeSet {
        settings: BTreeMap::from([("editor".to_owned(), "nano".to_owned())]),
    };
    let action = tui_effect(
        &service,
        &store,
        &state_dir,
        &broken_config,
        UiEffect::Preferences(PreferencesEffect::Save(failure)),
    )
    .unwrap();
    assert!(matches!(action, UiAction::SetStatus(ref message) if message.starts_with("Error: ")));
    assert_eq!(fs::read(&broken_config).unwrap(), b"keep config bytes");
    assert!(!root.path().join("config-is-a-file.bak").exists());

    let skill_parent = root.path().join("skill-parent-is-a-file");
    fs::write(&skill_parent, b"keep skill parent").unwrap();
    let skills_dir = skill_parent.join("skills");
    let action = tui_effect(
        &service,
        &store,
        &state_dir,
        &config_dir,
        UiEffect::Preferences(PreferencesEffect::InstallAgentSkill {
            skills_dir: skills_dir.clone(),
        }),
    )
    .unwrap();
    assert!(matches!(action, UiAction::SetStatus(ref message) if message.starts_with("Error: ")));
    assert_eq!(fs::read(&skill_parent).unwrap(), b"keep skill parent");
    assert!(!skills_dir.exists());
}

#[test]
fn typed_run_feedback_and_preset_effects_use_application_and_store_ports() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Snapshot", "printf '%s' '{name} {token}'");
    fs::write(root.path().join("one.txt"), "").unwrap();
    fs::write(root.path().join("two.txt"), "").unwrap();

    assert_eq!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::CountRunGlob {
                selector: entry.slug.as_str().to_owned(),
                field: 2,
                value: "*.txt".to_owned(),
                request: skit_application::form_feedback::GlobCountRequest {
                    cwd: root.path().display().to_string(),
                    pieces: vec!["*.txt".to_owned()],
                },
            },
        )
        .unwrap(),
        UiAction::SetRunGlobCount {
            field: 2,
            value: "*.txt".to_owned(),
            count: 2,
        }
    );

    let action = tui_effect(
        &service,
        &store,
        &state_dir,
        &config_dir,
        UiEffect::SaveRunPreset {
            selector: entry.slug.as_str().to_owned(),
            name: "quick".to_owned(),
            values: BTreeMap::from([
                ("name".to_owned(), "Ada".to_owned()),
                ("token".to_owned(), "do-not-store".to_owned()),
            ]),
            secret_names: std::collections::BTreeSet::from(["token".to_owned()]),
        },
    )
    .unwrap();
    let UiAction::RunPresetSaved { presets, .. } = action else {
        panic!("preset save must refresh the typed run form");
    };
    assert_eq!(
        presets["quick"],
        BTreeMap::from([("name".to_owned(), "Ada".to_owned())])
    );
    assert_eq!(
        FormStateService::new(FileFormStateStore::new(&state_dir))
            .load(&entry.slug)
            .presets["quick"],
        BTreeMap::from([("name".to_owned(), "Ada".to_owned())])
    );

    let fieldless = add_command(&service, "Fieldless", "true");
    let before = test_tree_snapshot(root.path());
    let error = tui_effect(
        &service,
        &store,
        &state_dir,
        &config_dir,
        UiEffect::SaveRunPreset {
            selector: fieldless.slug.as_str().to_owned(),
            name: "empty".to_owned(),
            values: BTreeMap::new(),
            secret_names: std::collections::BTreeSet::new(),
        },
    )
    .unwrap_err();
    assert!(matches!(error, CliError::Usage(_)), "{error:?}");
    assert_eq!(
        error.message().localize(Locale::En),
        "cannot save a preset because the entry has no form fields"
    );
    assert_eq!(test_tree_snapshot(root.path()), before);
}

#[test]
fn tui_run_propagates_typed_preflight_errors_without_writing() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Required", "printf '%s' '{name}'");
    let before = test_tree_snapshot(root.path());

    let error = tui_submit_run(
        &service,
        &store,
        &state_dir,
        &config_dir,
        entry.slug.as_str(),
        &BTreeMap::new(),
    )
    .unwrap_err();

    assert!(
        matches!(error, CliError::Run(RunError::Inputs(_))),
        "{error:?}"
    );
    assert_eq!(test_tree_snapshot(root.path()), before);
}

#[test]
fn tui_rerun_replays_only_honest_valid_state_and_falls_back_to_the_form() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let state = FormStateService::new(FileFormStateStore::new(&state_dir));
    FileConfigStore::new(&config_dir)
        .set("after_run", "stay")
        .unwrap();

    let runnable = add_command(&service, "Runnable", "true");
    state
        .record_run(
            &runnable.slug,
            0,
            "2026-08-08T00:00:00Z",
            &[],
            Some(&BTreeMap::new()),
        )
        .unwrap();
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Rerun {
                selector: runnable.slug.as_str().to_owned(),
            },
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));

    let invalid = add_command(&service, "Invalid", "printf '%s' '{required}'");
    let declarations = entry_parameters(&store, &invalid);
    state
        .record_run(
            &invalid.slug,
            0,
            "2026-08-08T00:00:00Z",
            &declarations,
            Some(&BTreeMap::new()),
        )
        .unwrap();
    assert!(matches!(
        tui_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            UiEffect::Rerun {
                selector: invalid.slug.as_str().to_owned(),
            },
        )
        .unwrap(),
        UiAction::Present(Screen::Run(_))
    ));

    let fresh = add_command(&service, "Fresh", "true");
    assert!(matches!(
        tui_rerun(
            &service,
            &store,
            &state_dir,
            &config_dir,
            fresh.slug.as_str(),
            Locale::En,
        )
        .unwrap(),
        UiAction::SetStatus(message) if message.contains("hasn't run yet")
    ));

    let prompt_source = root.path().join("review.prompt.md");
    fs::write(&prompt_source, "Review {{target}}.\n").unwrap();
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(prompt_source),
            kind: Some("prompt".to_owned()),
            name: Some("Review".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let prompt = service.show("Review").unwrap();
    let prompt_declarations = entry_parameters(&store, &prompt);
    state
        .record_run(
            &prompt.slug,
            0,
            "2026-08-08T00:00:00Z",
            &prompt_declarations,
            Some(&BTreeMap::from([(
                "target".to_owned(),
                "workspace".to_owned(),
            )])),
        )
        .unwrap();
    assert!(matches!(
        tui_rerun(
            &service,
            &store,
            &state_dir,
            &config_dir,
            prompt.slug.as_str(),
            Locale::En,
        )
        .unwrap(),
        UiAction::Present(Screen::Run(_))
    ));

    let scan = service.list().unwrap();
    let rerunnable = tui_rerunnable(&scan, &state_dir);
    assert!(rerunnable.contains(&runnable.slug));
    assert!(rerunnable.contains(&invalid.slug));
    assert!(!rerunnable.contains(&fresh.slug));
}

#[test]
fn tui_rerun_localizes_runner_failures_without_writing() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let state = FormStateService::new(FileFormStateStore::new(&state_dir));
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "[prompt]\nrunners_seeded = true\nrunners = []\n",
    )
    .unwrap();
    let entry = service
        .add(CreateEntry {
            name: "Missing runner".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"Review this.\n".to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                runner: "missing-agent".to_owned(),
                ..EntrySettings::default()
            },
        })
        .unwrap();
    state
        .record_run(
            &entry.slug,
            0,
            "2026-08-08T00:00:00Z",
            &[],
            Some(&BTreeMap::new()),
        )
        .unwrap();
    let before = test_tree_snapshot(root.path());

    for (locale, expected) in [
        (
            Locale::En,
            "Error: The runner missing-agent isn't configured (known: —). Manage runners with: skit runner list",
        ),
        (
            Locale::ZhCn,
            "错误：执行器 missing-agent 未配置(已知:—)。管理执行器:skit runner list",
        ),
        (
            Locale::ZhTw,
            "錯誤：執行器 missing-agent 未設定(已知:—)。管理執行器:skit runner list",
        ),
    ] {
        assert_eq!(
            tui_rerun(
                &service,
                &store,
                &state_dir,
                &config_dir,
                entry.slug.as_str(),
                locale,
            )
            .unwrap(),
            UiAction::SetStatus(expected.to_owned())
        );
        assert_eq!(test_tree_snapshot(root.path()), before);
    }
}

#[test]
fn tui_host_submits_every_form_without_global_process_state() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let alpha = add_command(&service, "Alpha", "printf '%s' '{name}'");
    let runnable = add_command(&service, "Runnable", "true");

    let add_values = BTreeMap::from([
        ("name".to_owned(), FieldValue::text("Added")),
        ("kind".to_owned(), FieldValue::text("command")),
        ("description".to_owned(), FieldValue::text("From TUI")),
        ("mode".to_owned(), FieldValue::text("copy")),
        ("template".to_owned(), FieldValue::text("echo {value}")),
        ("dependencies".to_owned(), FieldValue::text("")),
        ("python".to_owned(), FieldValue::text("")),
    ]);
    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Add,
            None,
            &add_values,
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    assert_eq!(service.show("Added").unwrap().meta.description, "From TUI");
    let invalid_add = BTreeMap::from([
        ("name".to_owned(), FieldValue::text("Invalid add")),
        ("kind".to_owned(), FieldValue::text("command")),
        ("template".to_owned(), FieldValue::text("true")),
        ("dependencies".to_owned(), FieldValue::text("not-supported")),
    ]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Add,
            None,
            &invalid_add,
        )
        .is_err()
    );
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Rename,
            Some(alpha.slug.as_str().to_owned()),
            &BTreeMap::from([("name".to_owned(), FieldValue::text("Runnable"))]),
        )
        .is_err()
    );

    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Rename,
            Some(alpha.slug.as_str().to_owned()),
            &BTreeMap::from([("name".to_owned(), FieldValue::text("Renamed Tool"))]),
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    let renamed = service.show("Renamed Tool").unwrap();

    let config = FileConfigStore::new(&config_dir);
    config.set("editor", "nano").unwrap();
    let config_path = config_dir.join("config.toml");
    let config_before_refusal = fs::read(&config_path).unwrap();
    let missing_bash = root.path().join("missing-legacy-bash");
    let invalid_preferences = BTreeMap::from([
        ("editor".to_owned(), FieldValue::text("micro")),
        (
            "shell.bash_path".to_owned(),
            FieldValue::text(missing_bash.display().to_string()),
        ),
    ]);
    let error = tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Preferences,
        None,
        &invalid_preferences,
    )
    .unwrap_err();
    assert_eq!(
        error.message().localize(Locale::En),
        format!("No such file: {}", missing_bash.display())
    );
    assert_eq!(fs::read(&config_path).unwrap(), config_before_refusal);
    assert_eq!(config.get("editor").unwrap(), "nano");
    assert!(!config_dir.join("config.toml.bak").exists());

    let preferences = BTreeMap::from([
        ("lang".to_owned(), FieldValue::text("en")),
        ("editor".to_owned(), FieldValue::text("vi")),
        ("form".to_owned(), FieldValue::text("plain")),
        ("after_run".to_owned(), FieldValue::text("stay")),
        ("shell.bash_path".to_owned(), FieldValue::text("")),
        ("js.runner".to_owned(), FieldValue::text("")),
        ("mirror".to_owned(), FieldValue::text("off")),
        ("mirror.pypi".to_owned(), FieldValue::text("off")),
        ("mirror.github".to_owned(), FieldValue::text("off")),
        ("mirror.npm".to_owned(), FieldValue::text("off")),
    ]);
    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Preferences,
            None,
            &preferences,
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));

    let runner = BTreeMap::from([
        ("name".to_owned(), FieldValue::text("local")),
        ("argv".to_owned(), FieldValue::text("printf {{prompt}}")),
        ("remove".to_owned(), FieldValue::text("false")),
    ]);
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Runners,
        None,
        &runner,
    )
    .unwrap();
    let invalid_runner = BTreeMap::from([
        ("name".to_owned(), FieldValue::text("bad")),
        ("argv".to_owned(), FieldValue::text("'unterminated")),
    ]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Runners,
            None,
            &invalid_runner,
        )
        .is_err()
    );
    let invalid_definition = BTreeMap::from([
        ("name".to_owned(), FieldValue::text("bad-definition")),
        ("argv".to_owned(), FieldValue::text("true")),
    ]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Runners,
            None,
            &invalid_definition,
        )
        .is_err()
    );
    let remove_unknown = BTreeMap::from([
        ("name".to_owned(), FieldValue::text("missing")),
        ("remove".to_owned(), FieldValue::text("true")),
    ]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Runners,
            None,
            &remove_unknown,
        )
        .is_err()
    );
    let remove_runner = BTreeMap::from([
        ("name".to_owned(), FieldValue::text("local")),
        ("remove".to_owned(), FieldValue::text("true")),
    ]);
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Runners,
        None,
        &remove_runner,
    )
    .unwrap();

    // A preset is created from the run form (`Ctrl+S`) and deleted by unticking it in entry
    // settings. Version 0.4 has no separate presets screen at all, so there is one place each.
    let state = FormStateService::new(FileFormStateStore::new(&state_dir));
    state
        .save_preset(
            &renamed.slug,
            "empty",
            &entry_parameters(&store, &renamed),
            &BTreeMap::new(),
        )
        .unwrap();
    assert!(
        state.load(&renamed.slug).presets.contains_key("empty"),
        "the run form's save did not reach the state file"
    );
    let Screen::Settings(view) = tui_open(
        &service,
        &store,
        &state_dir,
        &config_dir,
        HostRequest::Presets,
        Some(renamed.slug.as_str().to_owned()),
    )
    .unwrap() else {
        panic!("`s` must open the settings screen deep-linked to the presets");
    };
    let mut view = *view;
    assert_eq!(view.revealed(), Some(SettingsSectionId::Presets));
    assert!(view.set_value("preset:empty", FieldValue::boolean(false)));
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Settings,
        Some(renamed.slug.as_str().to_owned()),
        &view.submitted_values(),
    )
    .unwrap();
    assert!(
        !state.load(&renamed.slug).presets.contains_key("empty"),
        "unticking a preset did not delete it"
    );

    let settings_screen = |edits: &[(&str, &str)]| -> SubmittedValues {
        let Screen::Settings(view) = tui_open(
            &service,
            &store,
            &state_dir,
            &config_dir,
            HostRequest::Settings,
            Some(renamed.slug.as_str().to_owned()),
        )
        .unwrap() else {
            panic!("settings must open the typed screen");
        };
        let mut view = *view;
        for (key, value) in edits {
            assert!(view.set_value(key, FieldValue::text(*value)), "no {key}");
        }
        view.submitted_values()
    };
    let duplicate = settings_screen(&[("parameter:add", "same same")]);
    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Settings,
            Some(renamed.slug.as_str().to_owned()),
            &duplicate,
        )
        .is_err()
    );
    let settings = settings_screen(&[
        ("name", "Configured"),
        ("description", "Configured in TUI"),
        ("template", "printf %s {name}"),
        ("parameter:add", "fresh"),
    ]);
    tui_submit(
        &service,
        &store,
        &state_dir,
        &config_dir,
        FormPurpose::Settings,
        Some(renamed.slug.as_str().to_owned()),
        &settings,
    )
    .unwrap();
    let configured = service.show("Configured").unwrap();
    assert_eq!(configured.meta.description, "Configured in TUI");

    let run_values = BTreeMap::from([
        ("value:name".to_owned(), FieldValue::text("Ada")),
        ("_skit_save_preset".to_owned(), FieldValue::text("from-tui")),
        ("_skit_args".to_owned(), FieldValue::text("tail")),
        ("_skit_dry_run".to_owned(), FieldValue::text("false")),
    ]);
    assert!(matches!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Run,
            Some(configured.slug.as_str().to_owned()),
            &run_values,
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    assert!(
        FormStateService::new(FileFormStateStore::new(&state_dir))
            .load(&configured.slug)
            .presets
            .contains_key("from-tui")
    );

    let invalid_args = BTreeMap::from([("_skit_args".to_owned(), FieldValue::text("'bad"))]);
    assert!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            runnable.slug.as_str(),
            &invalid_args,
        )
        .is_err()
    );
    assert!(matches!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            runnable.slug.as_str(),
            &BTreeMap::new(),
        )
        .unwrap(),
        UiAction::Complete { .. }
    ));
    FileConfigStore::new(&config_dir)
        .set("after_run", "exit")
        .unwrap();
    assert_eq!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            runnable.slug.as_str(),
            &BTreeMap::new(),
        )
        .unwrap(),
        UiAction::Quit
    );

    assert!(
        tui_submit(
            &service,
            &store,
            &state_dir,
            &config_dir,
            FormPurpose::Run,
            None,
            &BTreeMap::new(),
        )
        .is_err()
    );
    assert!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            "missing",
            &BTreeMap::new(),
        )
        .is_err()
    );
}

#[test]
fn selected_prompt_runner_preflight_mapping_is_typed_and_scoped() {
    let prompt = health_entry("prompt");
    let missing = RunError::Launch(LaunchError::ProgramNotFound {
        name: "missing-agent".to_owned(),
    });
    for (locale, expected) in [
        (
            Locale::En,
            "Error: required program was not found: missing-agent",
        ),
        (Locale::ZhCn, "错误：找不到所需程序：missing-agent"),
        (Locale::ZhTw, "錯誤：找不到必要程式：missing-agent"),
    ] {
        assert_eq!(
            selected_prompt_runner_preflight_message(
                &prompt,
                Some("missing-agent"),
                &missing,
                locale,
            )
            .as_deref(),
            Some(expected),
        );
    }

    let command = health_entry("command");
    assert_eq!(
        selected_prompt_runner_preflight_message(
            &command,
            Some("missing-agent"),
            &missing,
            Locale::En,
        ),
        None,
    );
    assert_eq!(
        selected_prompt_runner_preflight_message(&prompt, None, &missing, Locale::En),
        None,
    );
    let unknown = RunError::RunnerNotFound {
        name: "missing-agent".to_owned(),
        known: "working".to_owned(),
    };
    assert_eq!(
        selected_prompt_runner_preflight_message(
            &prompt,
            Some("missing-agent"),
            &unknown,
            Locale::En,
        ),
        None,
    );
    let process = RunError::Launch(LaunchError::Process {
        operation: "start",
        source: io::Error::other("late spawn failure"),
    });
    assert_eq!(
        selected_prompt_runner_preflight_message(
            &prompt,
            Some("missing-agent"),
            &process,
            Locale::En,
        ),
        None,
    );
}

#[test]
fn tui_python_dependencies_use_the_same_pep_723_source_as_the_cli() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    fs::write(&source, "print('ok')\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: vec!["requests>=2,<3".to_owned()],
            dependencies_explicit: true,
            requires_python: Some(">=3.12".to_owned()),
            no_input: false,
        },
    )
    .unwrap();
    // The list is one comma-separated control, and the screen splits it per flavour before it
    // travels — a PEP 508 requirement carries commas inside its own specifier.
    let view = settings_view(&service, &store, &state_dir, "python-tool");
    assert_eq!(
        view.field("dependencies").unwrap().value().as_text(),
        "requests>=2,<3"
    );
    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "python-tool",
        &[
            ("dependencies", "requests>=2,<3, rich"),
            ("python", ">=3.13"),
        ],
    );
    assert_eq!(
        tui_list(&values, "dependencies"),
        ["requests>=2,<3".to_owned(), "rich".to_owned()],
        "a specifier's own comma must not split the list"
    );

    tui_submit_settings(&service, &store, &state_dir, "python-tool", &values).unwrap();

    let stored = fs::read_to_string(data_dir.join("scripts/python-tool/script.py")).unwrap();
    assert!(stored.contains("requests>=2,<3"));
    assert!(stored.contains("\"rich\""));
    assert!(stored.contains("requires-python = \">=3.13\""));
    let meta = fs::read_to_string(data_dir.join("scripts/python-tool/meta.toml")).unwrap();
    let meta: toml::Value = toml::from_str(&meta).unwrap();
    assert_eq!(
        meta["dependencies"],
        toml::Value::Array(vec![
            toml::Value::String("requests>=2,<3".to_owned()),
            toml::Value::String("rich".to_owned()),
        ])
    );
    assert_eq!(meta["requires_python"].as_str(), Some(">=3.13"));
}

#[test]
fn tui_settings_source_refusal_does_not_commit_other_fields() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    fs::write(&source, "NAME=world\necho \"$NAME\"\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Shell tool".to_owned()),
            description: Some("before".to_owned()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let source_path = data_dir.join("scripts/shell-tool/script.sh");
    let meta_path = data_dir.join("scripts/shell-tool/meta.toml");
    let source_before = fs::read(&source_path).unwrap();
    let meta_before = fs::read(&meta_path).unwrap();
    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "shell-tool",
        &[
            ("name", "Must not land"),
            ("description", "must not land"),
            ("source:normalize", "MISSING"),
        ],
    );

    assert!(tui_submit_settings(&service, &store, &state_dir, "shell-tool", &values).is_err());
    assert_eq!(fs::read(source_path).unwrap(), source_before);
    assert_eq!(fs::read(meta_path).unwrap(), meta_before);
    assert_eq!(service.show("shell-tool").unwrap().meta.name, "Shell tool");
}

#[test]
fn tui_template_updates_reconcile_the_placeholder_schema() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_command(&service, "Template form", "echo {old}");
    let initial = service.show("template-form").unwrap();
    let initial_settings = EntrySettings::from_meta(&initial.meta);
    assert_eq!(initial_settings.params, ["old"]);
    assert!(
        initial_settings.parameters.is_empty(),
        "template slots are implicit before a schema edit"
    );
    assert_eq!(
        entry_parameters(&store, &initial)
            .into_iter()
            .map(|field| field.name)
            .collect::<Vec<_>>(),
        ["old"]
    );

    let description = settings_edits(
        &service,
        &store,
        &state_dir,
        "template-form",
        &[("description", "Kept")],
    );
    tui_submit_settings(&service, &store, &state_dir, "template-form", &description).unwrap();
    let renamed_axis = service.show("template-form").unwrap();
    let renamed_settings = EntrySettings::from_meta(&renamed_axis.meta);
    assert_eq!(renamed_settings.params, ["old"]);
    assert!(
        renamed_settings.parameters.is_empty(),
        "an unrelated Settings save must not promote an implicit slot"
    );

    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "template-form",
        &[("template", "echo {new} {other}")],
    );

    tui_submit_settings(&service, &store, &state_dir, "template-form", &values).unwrap();

    let updated = service.show("template-form").unwrap();
    let settings = EntrySettings::from_meta(&updated.meta);
    assert_eq!(settings.params, ["new", "other"]);
    assert!(
        settings.parameters.is_empty(),
        "a template edit changes implicit truth without declaring every slot"
    );
    assert_eq!(
        entry_parameters(&store, &updated)
            .into_iter()
            .map(|field| field.name)
            .collect::<Vec<_>>(),
        ["new", "other"]
    );

    let promote = settings_edits(
        &service,
        &store,
        &state_dir,
        "template-form",
        &[("parameter:new:default", "value")],
    );
    tui_submit_settings(&service, &store, &state_dir, "template-form", &promote).unwrap();
    let promoted = service.show("template-form").unwrap();
    let promoted_settings = EntrySettings::from_meta(&promoted.meta);
    assert_eq!(promoted_settings.params, ["new", "other"]);
    assert_eq!(promoted_settings.parameters.len(), 1);
    assert_eq!(promoted_settings.parameters[0].name, "new");
    assert_eq!(
        promoted_settings.parameters[0].delivery,
        ParameterDelivery::Placeholder
    );
    assert_eq!(
        promoted_settings.parameters[0].default,
        Some(ParameterValue::String("value".to_owned()))
    );
    assert_eq!(
        entry_parameters(&store, &promoted)
            .into_iter()
            .map(|field| field.name)
            .collect::<Vec<_>>(),
        ["new", "other"]
    );
}

#[test]
fn tui_selected_preset_replaces_unchanged_last_used_values() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Preset form", "echo {name}");
    let declarations = entry_parameters(&store, &entry);
    let state = FormStateService::new(FileFormStateStore::new(&state_dir));
    let last = BTreeMap::from([("name".to_owned(), "last".to_owned())]);
    let preset = BTreeMap::from([("name".to_owned(), "preset".to_owned())]);
    state
        .save_last(&entry.slug, &declarations, Some(&last), None, false)
        .unwrap();
    state
        .save_preset(&entry.slug, "work", &declarations, &preset)
        .unwrap();
    let values = BTreeMap::from([
        ("value:name".to_owned(), FieldValue::text("last")),
        ("_skit_preset".to_owned(), FieldValue::text("work")),
        ("_skit_save_preset".to_owned(), FieldValue::text("snapshot")),
        ("_skit_dry_run".to_owned(), FieldValue::text("true")),
    ]);

    tui_submit_run(
        &service,
        &store,
        &state_dir,
        &config_dir,
        entry.slug.as_str(),
        &values,
    )
    .unwrap();

    assert_eq!(
        state.load(&entry.slug).presets["snapshot"]["name"],
        "preset"
    );
}

#[test]
fn tui_run_refuses_an_unknown_boolean_value() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let entry = add_command(&service, "Boolean form", "true");
    let values = BTreeMap::from([("_skit_dry_run".to_owned(), FieldValue::text("sometimes"))]);

    assert!(matches!(
        tui_submit_run(
            &service,
            &store,
            &state_dir,
            &config_dir,
            entry.slug.as_str(),
            &values,
        ),
        Err(CliError::Usage(_))
    ));
}

/// Check that English text does not drift and that each locale fills every hole.
fn assert_localized(error: &CliError, values: &[&str]) {
    let message = error.message();
    assert_eq!(error.to_string(), message.localize(Locale::En));
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let text = message.localize(locale);
        let template = message.template();
        assert!(!text.trim().is_empty(), "{template} is empty");
        assert!(!text.contains("{}"), "{template} kept an empty hole");
        for value in values {
            assert!(text.contains(value), "{text} lost the value {value}");
        }
    }
}

#[test]
fn every_cli_error_localizes_and_keeps_its_values() {
    let io_failure = || io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");

    assert_localized(
        &CliError::Repository(RepositoryError::NotFound {
            query: "missing".to_owned(),
        }),
        &["missing"],
    );
    assert_localized(&CliError::Run(crate::run::RunError::RawConflict), &[]);
    assert_localized(
        &CliError::Dependencies(skit_runtime::DependencyError::InstallerNotFound {
            name: "npm".to_owned(),
        }),
        &["npm"],
    );
    assert_localized(
        &CliError::Json(serde_json::from_str::<serde_json::Value>("{").unwrap_err()),
        &[],
    );
    assert_localized(&CliError::Io(io_failure()), &["permission denied"]);
    assert_localized(&CliError::Tui(skit_tui::TuiError::Io(io_failure())), &[]);
    assert_localized(
        &CliError::Config(ConfigError::Encode {
            reason: "unsupported value".to_owned(),
        }),
        &["unsupported value"],
    );
    assert_localized(
        &CliError::State(StateWriteError::Encode {
            reason: "unsupported value".to_owned(),
        }),
        &["unsupported value"],
    );
    assert_localized(
        &CliError::Usage(Message::new("unknown preset: {}").with("nightly")),
        &["nightly"],
    );
    assert_localized(&CliError::ConfirmationRequired, &[]);
    assert_localized(&CliError::Aborted, &[]);
    assert_localized(&CliError::AddCancelled, &[]);
    assert_localized(
        &CliError::Source {
            operation: "read",
            path: "/data/demo.py".to_owned(),
            source: io_failure(),
        },
        &["/data/demo.py", "permission denied"],
    );
    assert_localized(&CliError::DataDirectoryUnavailable, &[]);
    assert_localized(&CliError::DirectoryUnavailable("state"), &["state"]);
}

#[test]
fn the_localized_command_tree_translates_every_description() {
    fn assert_descriptions(locale: Locale, expected: &[(&str, &str)]) {
        let command = translate_command(Cli::command(), locale);
        for (name, expected) in expected {
            let about = if *name == "skit" {
                command.get_about()
            } else {
                command
                    .get_subcommands()
                    .find(|subcommand| subcommand.get_name() == *name)
                    .unwrap_or_else(|| panic!("the {name} subcommand exists"))
                    .get_about()
            };
            assert_eq!(
                about.map(ToString::to_string).as_deref(),
                Some(*expected),
                "locale={locale:?} command={name}"
            );
        }
    }

    assert_descriptions(
        Locale::En,
        &[
            (
                "skit",
                "skit — a launcher and parameter manager for scripts, prompts, programs, and commands. Run it without a subcommand to open the main menu",
            ),
            ("list", "List every registered entry"),
            (
                "show",
                "Show everything about one entry: metadata, dependencies, parameters, presets",
            ),
            (
                "remove",
                "Remove a registered entry (an original source file is left untouched)",
            ),
            (
                "rename",
                "Rename an entry (presets, remembered values and history survive)",
            ),
            (
                "describe",
                "Set an entry's description (shown in the Library and skit list)",
            ),
            (
                "params",
                "Show or edit an entry's managed or declared parameters",
            ),
            (
                "deps",
                "View or update an entry's package dependencies, Python constraint, and needed commands",
            ),
            (
                "doctor",
                "Check that uv is available and the entry library is intact",
            ),
        ],
    );
    assert_descriptions(
        Locale::ZhCn,
        &[
            (
                "skit",
                "skit——用于脚本、提示词、程序和命令的启动器与参数管理器。不带子命令运行即可打开主菜单",
            ),
            ("list", "列出所有已登记的条目"),
            (
                "show",
                "显示一个条目的全部信息：元数据、依赖、参数和参数组合",
            ),
            ("remove", "移除一个已登记的条目（原始来源文件不受影响）"),
            ("rename", "重命名条目（参数组合、记住的值与历史都会保留）"),
            ("describe", "设置条目的说明（显示在工具库和 skit list 中）"),
            ("params", "查看或编辑条目的管理参数或声明参数"),
            ("deps", "查看或更新条目的包依赖、Python 版本约束和所需命令"),
            ("doctor", "检查 uv 是否可用以及工具库是否完好"),
        ],
    );
    assert_descriptions(
        Locale::ZhTw,
        &[
            (
                "skit",
                "skit——用於腳本、提示詞、程式和命令的啟動器與參數管理器。不帶子命令執行即可開啟主選單",
            ),
            ("list", "列出所有已登記的條目"),
            (
                "show",
                "顯示一個條目的全部資訊：中繼資料、依賴、參數和參數組合",
            ),
            ("remove", "移除一個已登記的條目（原始來源檔案不受影響）"),
            ("rename", "重新命名條目（參數組合、記住的值與歷史都會保留）"),
            ("describe", "設定條目的說明（顯示在工具庫和 skit list 中）"),
            ("params", "檢視或編輯條目的管理參數或宣告參數"),
            (
                "deps",
                "檢視或更新條目的套件依賴、Python 版本限制和所需命令",
            ),
            ("doctor", "檢查 uv 是否可用以及工具庫是否完好"),
        ],
    );

    let command = translate_command(Cli::command(), Locale::ZhTw);
    // A Clap token such as `--help` must never change.
    let rendered = command.clone().render_help().to_string();
    assert!(rendered.contains("--help"));
    assert!(!rendered.contains("--說明"));
}

#[test]
fn windows_argument_encoding_round_trips_every_quoting_shape() {
    // Each row is one argument list that must survive join then split byte for byte.
    let lists: &[&[&str]] = &[
        &[],
        &[""],
        &["plain"],
        &["two words"],
        &["tab\there"],
        &["", "after empty", ""],
        &[r"trailing\"],
        &[r"trailing\\"],
        &[r"C:\Program Files\tool.exe", r"--out=C:\dir\"],
        &[r#"quote"inside"#],
        &[r#"back\"slash"#],
        &[r#"a b\"#],
        &[r#"a b\\"#],
        &[r#""quoted whole""#],
        &["-", "--", "--flag=value with spaces"],
    ];

    for list in lists {
        let joined = join_windows_arguments(list);
        let split = split_windows_arguments(&joined).unwrap();
        assert_eq!(
            &split, list,
            "round trip failed for {list:?} via {joined:?}"
        );
    }

    // A trailing backslash run inside a quoted argument doubles before the closing quote.
    assert_eq!(join_windows_arguments(&[r"a b\"]), r#""a b\\""#);
    assert_eq!(join_windows_arguments(&[r"plain\"]), r"plain\");
    assert_eq!(join_windows_arguments(&[""]), "\"\"");
    // A backslash run that no quote follows stays literal on the way back.
    assert_eq!(split_windows_arguments(r"a\").unwrap(), [r"a\"]);
    assert_eq!(split_windows_arguments(r"a\\").unwrap(), [r"a\\"]);
}

#[test]
fn windows_argument_splitting_covers_padding_escapes_and_unclosed_quotes() {
    // Trailing separators end the scan without adding an empty argument.
    assert_eq!(
        split_windows_arguments("  agent.exe   run   ").unwrap(),
        ["agent.exe", "run"]
    );
    assert!(split_windows_arguments("   ").unwrap().is_empty());

    // An even backslash run before a quote keeps half the backslashes and toggles the quote.
    assert_eq!(
        split_windows_arguments(r#"agent.exe "a\\" b"#).unwrap(),
        ["agent.exe", r"a\", "b"]
    );
    // An odd run escapes the quote itself.
    assert_eq!(
        split_windows_arguments(r#"agent.exe a\"b"#).unwrap(),
        ["agent.exe", r#"a"b"#]
    );
    // Backslashes that no quote follows stay as written.
    assert_eq!(
        split_windows_arguments(r"agent.exe a\\b").unwrap(),
        ["agent.exe", r"a\\b"]
    );
    // Two quotes inside a quoted run encode one literal quote.
    assert_eq!(
        split_windows_arguments(r#"agent.exe "a""b""#).unwrap(),
        ["agent.exe", r#"a"b"#]
    );

    let error = split_windows_arguments(r#"agent.exe "unclosed"#).unwrap_err();
    assert!(matches!(error, CliError::Usage(_)));
    assert!(error.to_string().contains("invalid quoting"));
}

#[test]
fn the_command_translator_is_total_over_every_clap_field() {
    // A command can carry a long description, and an argument can carry no help at all.
    let command = translate_command(
        clap::Command::new("probe")
            .about("Library")
            .long_about(
                "skit — a launcher and parameter manager for scripts, prompts, programs, and commands. Run it without a subcommand to open the main menu",
            )
            .arg(clap::Arg::new("bare"))
            .arg(clap::Arg::new("described").help("Entry slug or display name")),
        Locale::ZhTw,
    );

    assert_eq!(
        command.get_about().map(ToString::to_string).unwrap(),
        "工具庫"
    );
    assert_eq!(
        command.get_long_about().map(ToString::to_string).unwrap(),
        "skit——用於腳本、提示詞、程式和命令的啟動器與參數管理器。不帶子命令執行即可開啟主選單"
    );
    let arguments = command.get_arguments().collect::<Vec<_>>();
    assert!(
        arguments
            .iter()
            .find(|argument| argument.get_id() == "bare")
            .unwrap()
            .get_help()
            .is_none()
    );
    assert_eq!(
        arguments
            .iter()
            .find(|argument| argument.get_id() == "described")
            .unwrap()
            .get_help()
            .map(ToString::to_string)
            .unwrap(),
        "項目短名或顯示名稱"
    );
}

#[test]
fn tui_settings_refuse_axes_that_do_not_apply_to_the_entry_kind() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_command(&service, "Command form", "echo ok");

    for (field, value, detail) in [
        (
            "interpreter",
            "bash",
            "the entry does not use a pinnable interpreter",
        ),
        (
            "dependencies",
            "rich",
            "package dependencies apply only to Python and JavaScript entries",
        ),
        (
            "python",
            ">=3.12",
            "a Python constraint applies only to Python entries",
        ),
    ] {
        // A command template offers none of these controls, so the refusal is what a machine path
        // meets rather than something the screen can produce.
        let values = BTreeMap::from([(field.to_owned(), FieldValue::text(value))]);
        let error =
            tui_submit_settings(&service, &store, &state_dir, "command-form", &values).unwrap_err();
        assert!(error.to_string().contains(detail), "{error}");
    }
}

#[test]
fn tui_settings_accept_a_pinnable_interpreter_and_a_managed_binding() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    fs::write(&source, "NAME=world\necho \"$NAME\"\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Pin tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "pin-tool",
        &[("interpreter", "/opt/bash"), ("source:manage", "NAME")],
    );

    tui_submit_settings(&service, &store, &state_dir, "pin-tool", &values).unwrap();

    let updated = service.show("pin-tool").unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).interpreter,
        "/opt/bash"
    );

    // A second submission that changes nothing keeps the managed binding the screen reports back.
    let values = settings_edits(&service, &store, &state_dir, "pin-tool", &[]);
    tui_submit_settings(&service, &store, &state_dir, "pin-tool", &values).unwrap();
    let stored = fs::read_to_string(data_dir.join("scripts/pin-tool/script.sh")).unwrap();
    assert!(stored.contains("name = \"NAME\""), "{stored}");
}

#[test]
fn tui_settings_offers_no_binding_control_so_unmanaging_goes_through_its_own_key() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    fs::write(&source, "NAME=world\necho \"$NAME\"\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Shell tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "shell-tool",
        &[("source:manage", "NAME")],
    );
    tui_submit_settings(&service, &store, &state_dir, "shell-tool", &values).unwrap();

    // The stored source now manages NAME. Version 0.4 offers no binding control at all — a
    // source-managed row exposes the form label, the secret flag and the environment source and
    // nothing else (`src/skit/tui_settings.py:98-118`) — so the form cannot express "unbind this"
    // and the refusal it used to need is unreachable through the screen.
    let view = settings_view(&service, &store, &state_dir, "shell-tool");
    let controls = view
        .fields()
        .map(|field| field.key.clone())
        .collect::<Vec<_>>();
    assert!(
        !controls.iter().any(|key| key.ends_with(":binding")),
        "a source-managed row offered a binding control: {controls:?}"
    );
    for axis in ["prompt", "secret", "env_source", "keep"] {
        assert!(
            controls
                .iter()
                .any(|key| key.ends_with(&format!(":{axis}"))),
            "a source-managed row lost its editable {axis}"
        );
    }

    // Unticking the row's own keep toggle is the unmanage. Version 0.4 rewrites the block from the
    // rows that survived, and an unticked `ParamRow` collects as None
    // (`src/skit/tui_settings.py:115-117`, `:1061`, `:1074`). The toggle is the only unmanage
    // control the screen has, so a save that ignored it would take the edit back in silence.
    let unticked = settings_edits(
        &service,
        &store,
        &state_dir,
        "shell-tool",
        &[("parameter:NAME:keep", "false")],
    );
    tui_submit_settings(&service, &store, &state_dir, "shell-tool", &unticked).unwrap();
    let stored = fs::read_to_string(data_dir.join("scripts/shell-tool/script.sh")).unwrap();
    assert!(
        !stored.contains("[[tool.skit.params]]"),
        "unticking the keep toggle left the parameter managed: {stored}"
    );

    // The named lists still reach the host, so `skit params` and an agent keep their own path even
    // though the screen expresses both through the keep toggle.
    let manage = BTreeMap::from([("source:manage".to_owned(), FieldValue::text("NAME"))]);
    tui_submit_settings(&service, &store, &state_dir, "shell-tool", &manage).unwrap();
    let unmanage = BTreeMap::from([("source:unmanage".to_owned(), FieldValue::text("NAME"))]);
    tui_submit_settings(&service, &store, &state_dir, "shell-tool", &unmanage).unwrap();
    let stored = fs::read_to_string(data_dir.join("scripts/shell-tool/script.sh")).unwrap();
    assert!(!stored.contains("[[tool.skit.params]]"), "{stored}");
}

#[test]
fn tui_settings_manage_source_parameters_without_losing_non_utf8_bytes() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.sh");
    let original = b"NAME=world\n# caf\xe9\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Bytes tool".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let requested = settings_edits(
        &service,
        &store,
        &state_dir,
        "bytes-tool",
        &[("source:manage", "NAME")],
    );

    tui_submit_settings(&service, &store, &state_dir, "bytes-tool", &requested).unwrap();

    let written = fs::read(data_dir.join("scripts/bytes-tool/script.sh")).unwrap();
    assert!(written.contains(&0xe9));
    assert!(written.ends_with(original));
    assert!(
        written
            .windows(b"name = \"NAME\"".len())
            .any(|row| row == b"name = \"NAME\"")
    );
}

#[test]
fn tui_settings_keep_an_unchanged_non_utf8_python_copy_byte_exact() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# coding: latin-1\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let values = settings_edits(&service, &store, &state_dir, "python-bytes", &[]);
    tui_submit_settings(&service, &store, &state_dir, "python-bytes", &values).unwrap();

    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
}

#[test]
fn tui_settings_store_a_dependency_edit_for_non_utf8_python_without_a_block() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# coding: latin-1\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "python-bytes",
        &[("dependencies", "httpx>=0.28")],
    );

    tui_submit_settings(&service, &store, &state_dir, "python-bytes", &values).unwrap();

    let updated = service.show("python-bytes").unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).dependencies,
        ["httpx>=0.28"]
    );
    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
}

#[test]
fn tui_settings_refuse_an_edit_to_a_non_utf8_authoritative_python_block() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# /// script\n# dependencies = [\"requests\"]\n# ///\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let entry = service.show("python-bytes").unwrap();
    let before_settings = EntrySettings::from_meta(&entry.meta);
    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "python-bytes",
        &[("dependencies", "")],
    );

    let error =
        tui_submit_settings(&service, &store, &state_dir, "python-bytes", &values).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Python bytes's stored copy isn't valid UTF-8, so skit can't rewrite the script's own dependency block — and that block is what uv reads. Edit it in the script itself: skit edit Python bytes"
    );
    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
    assert_eq!(
        EntrySettings::from_meta(&service.show("python-bytes").unwrap().meta),
        before_settings
    );
}

#[test]
fn deps_keep_non_utf8_python_bytes_when_metadata_can_deliver_the_edit() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# coding: latin-1\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();

    deps(
        &service,
        &store,
        DepsArgs {
            selector: "python-bytes".to_owned(),
            dependencies: vec![" httpx>=0.28 ".to_owned()],
            clear: false,
            requires_python: None,
            needs: Vec::new(),
            clear_needs: false,
            json: false,
        },
    )
    .unwrap();

    let updated = service.show("python-bytes").unwrap();
    assert_eq!(
        EntrySettings::from_meta(&updated.meta).dependencies,
        ["httpx>=0.28"]
    );
    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
}

#[test]
fn deps_refuse_a_non_utf8_authoritative_block_before_any_metadata_write() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    let original = b"# /// script\n# dependencies = [\"requests\"]\n# ///\nTEXT = 'caf\xe9'\n";
    fs::write(&source, original).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Python bytes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();
    let before = service.show("python-bytes").unwrap();

    let error = deps(
        &service,
        &store,
        DepsArgs {
            selector: "python-bytes".to_owned(),
            dependencies: Vec::new(),
            clear: true,
            requires_python: None,
            needs: vec!["uv".to_owned()],
            clear_needs: false,
            json: false,
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("what uv reads"), "{error}");
    assert_eq!(service.show("python-bytes").unwrap().meta, before.meta);
    assert_eq!(
        fs::read(data_dir.join("scripts/python-bytes/script.py")).unwrap(),
        original
    );
}

#[test]
fn deps_distinguish_an_untouched_axis_from_an_explicit_clear() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    fs::write(
        &source,
        concat!(
            "# /// script\n",
            "# dependencies = [\"requests\"]\n",
            "# requires-python = \">=3.11\"\n",
            "# ///\n",
            "print(1)\n",
        ),
    )
    .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Axes".to_owned()),
            description: Some(String::new()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: false,
        },
    )
    .unwrap();

    deps(
        &service,
        &store,
        DepsArgs {
            selector: "axes".to_owned(),
            dependencies: Vec::new(),
            clear: false,
            requires_python: Some(">=3.12".to_owned()),
            needs: Vec::new(),
            clear_needs: false,
            json: false,
        },
    )
    .unwrap();
    let pinned = fs::read_to_string(data_dir.join("scripts/axes/script.py")).unwrap();
    assert!(pinned.contains("requests"));
    assert!(pinned.contains(">=3.12"));

    deps(
        &service,
        &store,
        DepsArgs {
            selector: "axes".to_owned(),
            dependencies: Vec::new(),
            clear: true,
            requires_python: None,
            needs: Vec::new(),
            clear_needs: false,
            json: false,
        },
    )
    .unwrap();
    let cleared = fs::read_to_string(data_dir.join("scripts/axes/script.py")).unwrap();
    assert!(!cleared.contains("requests"));
    assert!(cleared.contains(">=3.12"));
}

#[test]
fn every_agent_skill_command_example_matches_the_real_cli_tree() {
    let skill = include_str!("../../../../skills/skit/SKILL.md");
    let mut checked = 0;
    let mut in_bash = false;

    for line in skill.lines().map(str::trim) {
        if line == "```bash" {
            in_bash = true;
            continue;
        }
        if line == "```" {
            in_bash = false;
            continue;
        }
        if !in_bash || !line.starts_with("skit ") {
            continue;
        }
        let command = line.split_once(" #").map_or(line, |(command, _)| command);
        let arguments =
            shlex::split(command).unwrap_or_else(|| panic!("invalid shell line: {line}"));
        Cli::try_parse_from(&arguments)
            .unwrap_or_else(|error| panic!("invalid Agent Skill command `{command}`: {error}"));
        checked += 1;
    }

    assert!(checked >= 35, "only checked {checked} Agent Skill commands");
}

#[test]
fn clap_error_localization_keeps_every_context_value_shape_verbatim() {
    use clap::{builder::StyledStr, error::ErrorKind};

    let mut error = clap::Error::raw(
        ErrorKind::InvalidValue,
        "Print help | Entry added | Entry removed | Entry renamed",
    );
    error.insert(
        ContextKind::ValidValue,
        ContextValue::Strings(vec!["Print help".to_owned(), "Entry added".to_owned()]),
    );
    error.insert(
        ContextKind::PriorArg,
        ContextValue::StyledStr(StyledStr::from("Entry removed")),
    );
    error.insert(
        ContextKind::ValidSubcommand,
        ContextValue::StyledStrs(vec![StyledStr::from("Entry renamed")]),
    );
    error.insert(ContextKind::Custom, ContextValue::None);

    assert_eq!(
        localized_clap_error(&error, Locale::ZhCn),
        "错误：Print help | Entry added | Entry removed | Entry renamed"
    );
}

/// The composition root must hand the frontend every Library fact version 0.4 shows.
///
/// The detail pane reports parameters, presets, dependencies, the last run, and drift
/// (`src/skit/tui.py:558-604`), the list marks a missing target (`src/skit/tui.py:414`), and the
/// order is recency (`src/skit/tui.py:99-103` and `:394`). Every one of those reads the projection
/// this test builds, so a scan-only wiring makes it fail.
#[test]
fn the_library_surface_carries_every_detail_fact_the_frontend_renders() {
    let data = TempDir::new().unwrap();
    let state_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let service = LibraryService::new(FileStore::new(data.path()));
    let store = FileStore::new(data.path());

    let original = data.path().join("tool.sh");
    fs::write(&original, "NAME=\"world\"\necho \"$NAME\"\n").unwrap();
    add(
        &service,
        AddOptions {
            source: Some(original.clone()),
            kind: None,
            name: Some("Tool".to_owned()),
            description: Some("A shell tool".to_owned()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: Vec::new(),
            dependencies_explicit: false,
            requires_python: None,
            no_input: true,
        },
    )
    .unwrap();
    add_command(&service, "Deploy", "deploy --env {environment}");

    // An old add with a recent run must outrank a newer add that never ran, so pin both add
    // stamps instead of depending on the wall clock.
    for (slug, added_at) in [
        ("tool", "2020-01-01T00:00:00Z"),
        ("deploy", "2021-01-01T00:00:00Z"),
    ] {
        let path = data.path().join("scripts").join(slug).join("meta.toml");
        let meta = fs::read_to_string(&path).unwrap();
        let rewritten = meta
            .lines()
            .map(|line| {
                if line.starts_with("added_at = ") {
                    format!("added_at = {added_at:?}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{rewritten}\n")).unwrap();
    }
    FileStore::new(data.path()).rebuild_registry().unwrap();

    let tool = service.show("tool").unwrap();
    let declarations = entry_parameters(&store, &tool);
    let form_state = FormStateService::new(FileFormStateStore::new(state_dir.path()));
    form_state
        .record_run(
            &tool.slug,
            7,
            "2026-08-09T12:00:00Z",
            &declarations,
            Some(&BTreeMap::new()),
        )
        .unwrap();
    form_state
        .save_preset(
            &tool.slug,
            "nightly",
            &declarations,
            &BTreeMap::from([("NAME".to_owned(), "ada".to_owned())]),
        )
        .unwrap();
    // The stored copy is gone, so the launch target is missing.
    fs::remove_file(store.payload_path(&tool).unwrap()).unwrap();

    let surface = skit_store::library_surface(&store, state_dir.path(), config_dir.path()).unwrap();

    let detail = surface.details.get(&tool.slug).expect("no detail row");
    assert!(!detail.added_at.is_empty());
    assert_eq!(detail.presets, ["nightly"]);
    let last_run = detail.last_run.as_ref().expect("no last run");
    assert_eq!(last_run.at, "2026-08-09T12:00:00Z");
    assert_eq!(last_run.exit, Some(7));
    let missing = detail.missing_target.as_ref().expect("no missing target");
    assert!(missing.ends_with("script.sh"), "{missing}");
    // The original file is still on disk, so removal may promise it survives.
    assert!(detail.original_file_preserved);
    assert!(detail.template.is_none());
    assert!(detail.prompt_runner.is_none());

    // A command template carries its launch material and never claims an original file.
    let deploy = service.show("deploy").unwrap();
    let template_detail = surface.details.get(&deploy.slug).expect("no template row");
    assert_eq!(
        template_detail.template.as_deref(),
        Some("deploy --env {environment}")
    );
    assert!(!template_detail.original_file_preserved);
    assert!(
        template_detail
            .parameters
            .iter()
            .any(|parameter| parameter.key == "environment")
    );

    // The entry with the newer activity comes first, whatever the slug order is.
    let state = LibraryState::from_library_surface(surface);
    assert_eq!(
        state
            .visible_entries()
            .next()
            .map(|entry| entry.slug.clone()),
        Some(tool.slug.clone())
    );
}

#[cfg(unix)]
#[test]
#[ignore = "runs only as the child of the real PTY mirror-wizard owner"]
fn terminal_first_run_pty_child() {
    let scenario = std::env::var("SKIT_MIRROR_PTY_SCENARIO").unwrap();
    let result = match scenario.as_str() {
        "confirm" => format!("confirm={}", TerminalFirstRun.confirm_mirrors()),
        "axis-pypi" => format!(
            "answer={}",
            TerminalFirstRun.axis_answer(
                "PyPI index (Python packages)",
                &["aliyun".to_owned(), "tsinghua".to_owned()],
                "PyPI index URL",
                false,
            )
        ),
        "axis-github" => format!(
            "answer={}",
            TerminalFirstRun.axis_answer(
                "GitHub releases (Python builds, the uv binary)",
                &["nju".to_owned()],
                "github-release mirror base URL",
                true,
            )
        ),
        "axis-empty" => format!(
            "answer={}",
            TerminalFirstRun.axis_answer(
                "npm registry (JS/TS packages)",
                &[],
                "npm registry URL",
                false,
            )
        ),
        _ => panic!("unknown PTY child scenario: {scenario}"),
    };
    fs::write(std::env::var_os("SKIT_MIRROR_PTY_RESULT").unwrap(), result).unwrap();
}

#[cfg(unix)]
struct MirrorPromptPty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Option<Box<dyn io::Write + Send>>,
    chunks: std::sync::mpsc::Receiver<Vec<u8>>,
    output: Vec<u8>,
    result_path: std::path::PathBuf,
    _root: TempDir,
}

#[cfg(unix)]
impl MirrorPromptPty {
    fn spawn(scenario: &str, locale: &str) -> Self {
        Self::spawn_test(
            "cli::tests::terminal_first_run_pty_child",
            locale,
            &[("SKIT_MIRROR_PTY_SCENARIO", scenario)],
        )
    }

    fn spawn_test(test_name: &str, locale: &str, environment: &[(&str, &str)]) -> Self {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};

        let root = TempDir::new().unwrap();
        let result_path = root.path().join("result");
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 20,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
        command.args(["--ignored", "--exact", test_name, "--nocapture"]);
        command.env("TERM", "xterm-256color");
        command.env("NO_COLOR", "1");
        command.env("SKIT_LANG", locale);
        command.env("SKIT_MIRROR_PTY_RESULT", &result_path);
        command.env("SKIT_PLAIN_DRAFT_PTY_RESULT", &result_path);
        for (key, value) in environment {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (sender, chunks) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if sender.send(buffer[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            child,
            writer: Some(writer),
            chunks,
            output: Vec::new(),
            result_path,
            _root: root,
        }
    }

    fn checkpoint(&mut self) -> usize {
        self.drain();
        self.output.len()
    }

    fn send(&mut self, answer: &str) {
        let writer = self.writer.as_mut().unwrap();
        writer.write_all(answer.as_bytes()).unwrap();
        writer.flush().unwrap();
    }

    fn wait_for(&mut self, needle: &str) -> String {
        self.wait_for_after(0, needle)
    }

    fn wait_for_after(&mut self, checkpoint: usize, needle: &str) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        loop {
            self.drain();
            let visible = self.visible_after(checkpoint);
            if visible.contains(needle) {
                return visible;
            }
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or_else(|| {
                    panic!("timed out waiting for {needle:?}; new terminal output:\n{visible}")
                });
            match self
                .chunks
                .recv_timeout(remaining.min(std::time::Duration::from_millis(100)))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => assert!(
                    self.child.try_wait().unwrap().is_none(),
                    "mirror PTY child exited while waiting for {needle:?}: {visible}"
                ),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("mirror PTY output closed while waiting for {needle:?}: {visible}")
                }
            }
        }
    }

    fn finish(mut self) -> (String, String) {
        self.writer.take();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for mirror PTY child: {}",
                self.visible_after(0)
            );
            match self
                .chunks
                .recv_timeout(std::time::Duration::from_millis(100))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(
                    std::sync::mpsc::RecvTimeoutError::Timeout
                    | std::sync::mpsc::RecvTimeoutError::Disconnected,
                ) => {}
            }
        };
        assert!(
            status.success(),
            "mirror PTY child failed: {status:?}: {}",
            self.visible_after(0)
        );
        self.drain();
        let output = self.visible_after(0);
        let result = fs::read_to_string(&self.result_path).unwrap();
        (output, result)
    }

    fn drain(&mut self) {
        while let Ok(chunk) = self.chunks.try_recv() {
            self.output.extend_from_slice(&chunk);
        }
    }

    fn visible_after(&self, checkpoint: usize) -> String {
        String::from_utf8_lossy(&self.output[checkpoint.min(self.output.len())..]).into_owned()
    }
}

#[cfg(unix)]
#[test]
fn terminal_first_run_questions_use_real_pty_defaults_choices_and_localized_reprompts() {
    let mut confirm = MirrorPromptPty::spawn("confirm", "en");
    confirm.wait_for("Configure mirrors for faster installs (mainland China)?");
    confirm.send("\n");
    assert_eq!(confirm.finish().1, "confirm=true");

    let mut declined = MirrorPromptPty::spawn("confirm", "zh-CN");
    declined.wait_for("是否配置镜像以加速下载(中国大陆)?");
    declined.send("n\n");
    assert_eq!(declined.finish().1, "confirm=false");

    let mut traditional = MirrorPromptPty::spawn("confirm", "zh-TW");
    traditional.wait_for("是否設定鏡像以加速下載(中國大陸)?");
    traditional.send("\n");
    assert_eq!(traditional.finish().1, "confirm=true");

    let mut preset = MirrorPromptPty::spawn("axis-pypi", "en");
    preset.wait_for("PyPI index (Python packages) [aliyun/tsinghua/custom/off]");
    preset.send("invalid\n");
    preset.wait_for("Choose one of: aliyun, tsinghua, custom, off");
    preset.send("tsinghua\n");
    assert_eq!(preset.finish().1, "answer=tsinghua");

    let mut off = MirrorPromptPty::spawn("axis-pypi", "en");
    off.wait_for("PyPI index (Python packages)");
    off.send("off\n");
    assert_eq!(off.finish().1, "answer=off");

    let mut empty = MirrorPromptPty::spawn("axis-empty", "en");
    empty.wait_for("npm registry (JS/TS packages) [custom/off]");
    empty.send("\n");
    assert_eq!(empty.finish().1, "answer=off");

    for (locale, question, warning) in [
        (
            "en",
            "PyPI index (Python packages)",
            "A custom choice needs a URL.",
        ),
        (
            "zh-CN",
            "PyPI 索引（Python 软件包）",
            "自定义选项需要 URL。",
        ),
        ("zh-TW", "PyPI 索引（Python 套件）", "自訂選項需要 URL。"),
    ] {
        let mut custom = MirrorPromptPty::spawn("axis-pypi", locale);
        custom.wait_for(question);
        custom.send("custom\n");
        custom.wait_for(match locale {
            "zh-CN" | "zh-TW" => "PyPI 索引 URL",
            _ => "PyPI index URL",
        });
        let empty_url = custom.checkpoint();
        custom.send("\n");
        custom.wait_for_after(empty_url, warning);
        let invalid_url = custom.checkpoint();
        custom.send("not-a-url\n");
        custom.wait_for_after(invalid_url, warning);
        custom.send(if locale == "en" {
            "http://mirror.test/simple\n"
        } else {
            "https://mirror.test/simple\n"
        });
        assert_eq!(
            custom.finish().1,
            if locale == "en" {
                "answer=http://mirror.test/simple"
            } else {
                "answer=https://mirror.test/simple"
            }
        );
    }

    let mut github = MirrorPromptPty::spawn("axis-github", "en");
    github.wait_for("GitHub releases (Python builds, the uv binary)");
    github.send("custom\n");
    github.wait_for("github-release mirror base URL");
    let insecure = github.checkpoint();
    github.send("http://mirror.test/github-release\n");
    github.wait_for_after(
        insecure,
        "The uv binary is downloaded and executed, so the github-release base URL must use https:// (got: http://mirror.test/github-release).",
    );
    github.send("https://mirror.test/github-release\n");
    assert_eq!(
        github.finish().1,
        "answer=https://mirror.test/github-release"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "runs only as the child of the real PTY plain-draft owner"]
fn add_plain_draft_pty_child() {
    let data_dir = std::path::PathBuf::from(std::env::var_os("SKIT_DATA_DIR").unwrap());
    let config_dir = std::path::PathBuf::from(std::env::var_os("SKIT_CONFIG_DIR").unwrap());
    let expected_name = std::env::var("SKIT_PLAIN_DRAFT_EXPECTED_NAME").unwrap();
    let kind = match std::env::var("SKIT_PLAIN_DRAFT_KIND").unwrap().as_str() {
        "script" => DraftKind::Script,
        "prompt" => DraftKind::Prompt,
        other => panic!("unknown plain-draft kind: {other}"),
    };
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    let result = if std::env::var_os("SKIT_PLAIN_DRAFT_BARE").is_some_and(|value| !value.is_empty())
    {
        bare_add_plain(&service, &config_dir)
    } else {
        add_plain_draft(&service, &config_dir, kind)
    };
    let marker = match result {
        Ok(()) => match service.show(expected_name.trim()) {
            Ok(entry) => format!(
                "ok|name={}|kind={}|source={}",
                entry.meta.name, entry.meta.kind, entry.meta.source
            ),
            Err(RepositoryError::NotFound { .. }) => "ok|none".to_owned(),
            Err(error) => panic!("successful plain add left an unreadable entry: {error}"),
        },
        Err(error) => {
            let message = error.message().localize(active_locale());
            eprintln!("{message}");
            format!("error|code={}|message={message}", error.exit_code())
        }
    };
    fs::write(
        std::env::var_os("SKIT_PLAIN_DRAFT_PTY_RESULT").unwrap(),
        marker,
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "runs only as the child of the real PTY add-command composition owner"]
fn add_command_composition_pty_child() {
    let data_dir = std::path::PathBuf::from(std::env::var_os("SKIT_DATA_DIR").unwrap());
    let scenario = std::env::var("SKIT_ADD_COMMAND_PTY_SCENARIO").unwrap();
    let expected_name = std::env::var("SKIT_ADD_COMMAND_PTY_NAME").unwrap();
    let service = LibraryService::new(FileStore::new(&data_dir));
    let mut options = add_options();
    let edit = match scenario.as_str() {
        "script-repository-error" => {
            options.name = Some(expected_name.clone());
            true
        }
        "prompt-name-repository-error" => {
            options.prompt = true;
            false
        }
        other => panic!("unknown add-command PTY scenario: {other}"),
    };
    let result = super::add_command(&service, options, edit);
    let marker = match result {
        Ok(()) => "ok".to_owned(),
        Err(error) => {
            let message = error.message().localize(active_locale());
            eprintln!("{message}");
            format!("error|code={}|message={message}", error.exit_code())
        }
    };
    fs::write(
        std::env::var_os("SKIT_PLAIN_DRAFT_PTY_RESULT").unwrap(),
        marker,
    )
    .unwrap();
}

#[cfg(unix)]
struct PlainDraftSandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    scratch: TempDir,
}

#[cfg(unix)]
impl PlainDraftSandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            scratch: TempDir::new().unwrap(),
        }
    }

    fn editor(&self, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;

        let path = self.scratch.path().join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        FileConfigStore::new(self.config.path())
            .set("editor", path.to_str().unwrap())
            .unwrap();
        path
    }

    fn set_editor(&self, value: &Path) {
        FileConfigStore::new(self.config.path())
            .set("editor", value.to_str().unwrap())
            .unwrap();
    }

    fn spawn(&self, kind: &str, expected_name: &str) -> MirrorPromptPty {
        self.spawn_with(kind, expected_name, false)
    }

    fn spawn_bare(&self, kind: &str, expected_name: &str) -> MirrorPromptPty {
        self.spawn_with(kind, expected_name, true)
    }

    fn spawn_with(&self, kind: &str, expected_name: &str, bare: bool) -> MirrorPromptPty {
        let bare = if bare { "1" } else { "" };
        MirrorPromptPty::spawn_test(
            "cli::tests::add_plain_draft_pty_child",
            "en",
            &[
                ("SKIT_DATA_DIR", self.data.path().to_str().unwrap()),
                ("SKIT_STATE_DIR", self.state.path().to_str().unwrap()),
                ("SKIT_CONFIG_DIR", self.config.path().to_str().unwrap()),
                ("SKIT_PLAIN_DRAFT_KIND", kind),
                ("SKIT_PLAIN_DRAFT_EXPECTED_NAME", expected_name),
                ("SKIT_PLAIN_DRAFT_BARE", bare),
            ],
        )
    }

    fn service(&self) -> LibraryService<FileStore> {
        LibraryService::new(FileStore::new(self.data.path()))
    }

    fn assert_no_drafts(&self) {
        assert!(tui_drafts(self.data.path()).is_empty());
        assert!(owned_draft_quarantines(self.data.path()).is_empty());
    }

    fn assert_no_state(&self) {
        assert_eq!(fs::read_dir(self.state.path()).unwrap().count(), 0);
    }
}

#[cfg(unix)]
#[test]
fn add_command_pty_preflight_keeps_repository_faults_and_prompted_names_no_write() {
    let corrupt_name = |sandbox: &PlainDraftSandbox, name: &str| {
        add_command(&sandbox.service(), name, "printf broken");
        let entry = sandbox.service().show(name).unwrap();
        let meta = sandbox
            .data
            .path()
            .join("scripts")
            .join(entry.slug.as_str())
            .join("meta.toml");
        fs::remove_file(&meta).unwrap();
        fs::create_dir(&meta).unwrap();
        meta
    };
    let spawn = |sandbox: &PlainDraftSandbox, scenario: &str, name: &str| {
        MirrorPromptPty::spawn_test(
            "cli::tests::add_command_composition_pty_child",
            "en",
            &[
                ("SKIT_DATA_DIR", sandbox.data.path().to_str().unwrap()),
                ("SKIT_STATE_DIR", sandbox.state.path().to_str().unwrap()),
                ("SKIT_CONFIG_DIR", sandbox.config.path().to_str().unwrap()),
                ("SKIT_ADD_COMMAND_PTY_SCENARIO", scenario),
                ("SKIT_ADD_COMMAND_PTY_NAME", name),
            ],
        )
    };

    let script = PlainDraftSandbox::new();
    let script_sentinel = script.scratch.path().join("script-editor-launched");
    script.editor(
        "script-repository-editor.sh",
        &format!("touch {:?}", script_sentinel),
    );
    let script_meta = corrupt_name(&script, "Broken Script");
    let script_before = test_tree_snapshot(script.data.path());
    let script_config_before = test_tree_snapshot(script.config.path());
    let script_state_before = test_tree_snapshot(script.state.path());
    let script_scratch_before = test_tree_snapshot(script.scratch.path());
    let child = spawn(&script, "script-repository-error", "Broken Script");
    let (output, marker) = child.finish();
    assert!(
        marker.contains("error|code=1|message=could not read"),
        "{marker}"
    );
    assert!(output.contains("could not read"), "{output}");
    assert!(script_meta.is_dir());
    assert!(!script_sentinel.exists());
    assert_eq!(test_tree_snapshot(script.data.path()), script_before);
    assert_eq!(
        test_tree_snapshot(script.config.path()),
        script_config_before
    );
    assert_eq!(test_tree_snapshot(script.state.path()), script_state_before);
    assert_eq!(
        test_tree_snapshot(script.scratch.path()),
        script_scratch_before
    );

    let prompt = PlainDraftSandbox::new();
    let prompt_sentinel = prompt.scratch.path().join("prompt-editor-launched");
    prompt.editor(
        "prompt-repository-editor.sh",
        &format!("touch {:?}", prompt_sentinel),
    );
    let prompt_meta = corrupt_name(&prompt, "Broken Prompt");
    let prompt_before = test_tree_snapshot(prompt.data.path());
    let prompt_config_before = test_tree_snapshot(prompt.config.path());
    let prompt_state_before = test_tree_snapshot(prompt.state.path());
    let prompt_scratch_before = test_tree_snapshot(prompt.scratch.path());
    let mut child = spawn(&prompt, "prompt-name-repository-error", "Broken Prompt");
    child.wait_for("Name in skit");
    child.send("Broken Prompt\n");
    let (output, marker) = child.finish();
    assert!(
        marker.contains("error|code=1|message=could not read"),
        "{marker}"
    );
    assert!(output.contains("Name in skit"), "{output}");
    assert!(output.contains("could not read"), "{output}");
    assert!(prompt_meta.is_dir());
    assert!(!prompt_sentinel.exists());
    assert_eq!(test_tree_snapshot(prompt.data.path()), prompt_before);
    assert_eq!(
        test_tree_snapshot(prompt.config.path()),
        prompt_config_before
    );
    assert_eq!(test_tree_snapshot(prompt.state.path()), prompt_state_before);
    assert_eq!(
        test_tree_snapshot(prompt.scratch.path()),
        prompt_scratch_before
    );
}

#[cfg(unix)]
#[test]
fn add_plain_draft_real_pty_covers_preflight_author_commit_and_failure_cleanup() {
    let blank = PlainDraftSandbox::new();
    let blank_sentinel = blank.scratch.path().join("blank-launched");
    blank.editor("blank-editor.sh", &format!("touch {:?}", blank_sentinel));
    let blank_config = fs::read(blank.config.path().join("config.toml")).unwrap();
    let mut child = blank.spawn("script", "");
    child.wait_for("Name in skit");
    child.send("   \n");
    let (output, marker) = child.finish();
    assert!(
        marker.contains("error|code=2|message=A name is required."),
        "{marker}"
    );
    assert!(output.contains("A name is required."), "{output}");
    assert!(!blank_sentinel.exists());
    blank.assert_no_drafts();
    blank.assert_no_state();
    assert_eq!(
        fs::read(blank.config.path().join("config.toml")).unwrap(),
        blank_config
    );

    let taken = PlainDraftSandbox::new();
    add_command(&taken.service(), "Taken", "printf taken");
    let taken_sentinel = taken.scratch.path().join("taken-launched");
    taken.editor("taken-editor.sh", &format!("touch {:?}", taken_sentinel));
    let taken_meta = fs::read(taken.data.path().join("scripts/taken/meta.toml")).unwrap();
    let mut child = taken.spawn("script", "Taken");
    child.wait_for("Name in skit");
    child.send("Taken\n");
    let (output, marker) = child.finish();
    assert!(marker.contains("error|code=1"), "{marker}");
    assert!(output.contains("already taken"), "{output}");
    assert!(!taken_sentinel.exists());
    assert_eq!(
        fs::read(taken.data.path().join("scripts/taken/meta.toml")).unwrap(),
        taken_meta
    );
    taken.assert_no_drafts();
    taken.assert_no_state();

    let repository_error = PlainDraftSandbox::new();
    let repository_sentinel = repository_error.scratch.path().join("repository-launched");
    repository_error.editor(
        "repository-editor.sh",
        &format!("touch {:?}", repository_sentinel),
    );
    add_command(&repository_error.service(), "Broken Name", "printf broken");
    let broken_meta = repository_error
        .data
        .path()
        .join("scripts/broken-name/meta.toml");
    fs::remove_file(&broken_meta).unwrap();
    fs::create_dir(&broken_meta).unwrap();
    let registry = fs::read(repository_error.data.path().join("registry.toml")).unwrap();
    let mut child = repository_error.spawn("script", "Broken Name");
    child.wait_for("Name in skit");
    child.send("Broken Name\n");
    let (output, marker) = child.finish();
    assert!(marker.contains("error|code=1"), "{marker}");
    assert!(output.contains("could not read"), "{output}");
    assert!(!repository_sentinel.exists());
    assert!(broken_meta.is_dir());
    assert_eq!(
        fs::read(repository_error.data.path().join("registry.toml")).unwrap(),
        registry
    );
    repository_error.assert_no_state();

    let empty = PlainDraftSandbox::new();
    empty.set_editor(Path::new("/usr/bin/true"));
    let empty_config = fs::read(empty.config.path().join("config.toml")).unwrap();
    let mut child = empty.spawn("script", "Empty Author");
    child.wait_for("Name in skit");
    child.send("Empty Author\n");
    let (output, marker) = child.finish();
    assert_eq!(marker, "ok|none");
    assert!(
        output.contains("Nothing was written, so nothing was added."),
        "{output}"
    );
    assert!(matches!(
        empty.service().show("Empty Author"),
        Err(RepositoryError::NotFound { .. })
    ));
    empty.assert_no_drafts();
    empty.assert_no_state();
    assert_eq!(
        fs::read(empty.config.path().join("config.toml")).unwrap(),
        empty_config
    );

    for (kind, name, editor_body, expected_kind, expected_bytes, suffix) in [
        (
            "script",
            "Plain Script",
            "printf \"print('plain')\\n\" >> \"$1\"",
            "python",
            b"#!/usr/bin/env python3\nprint('plain')\n".as_slice(),
            ".py",
        ),
        (
            "prompt",
            "Plain Prompt",
            "printf \"Review this.\\n\" >> \"$1\"",
            "prompt",
            b"# New prompt\n\nReview this.\n".as_slice(),
            ".prompt.md",
        ),
    ] {
        let success = PlainDraftSandbox::new();
        success.editor("success-editor.sh", editor_body);
        let config_before = fs::read(success.config.path().join("config.toml")).unwrap();
        let mut child = success.spawn(kind, name);
        child.wait_for("Name in skit");
        child.send(&format!("{name}\n"));
        let (_output, marker) = child.finish();
        assert!(
            marker.contains(&format!("ok|name={name}|kind={expected_kind}|source=")),
            "{marker}"
        );
        let entry = success.service().show(name).unwrap();
        assert_eq!(entry.meta.name, name);
        assert_eq!(entry.meta.kind.as_str(), expected_kind);
        assert!(entry.meta.source.contains("/drafts/skit-new-"));
        assert!(entry.meta.source.ends_with(suffix), "{}", entry.meta.source);
        assert_eq!(
            fs::read(source_path(success.service().repository(), &entry).unwrap()).unwrap(),
            expected_bytes
        );
        success.assert_no_drafts();
        success.assert_no_state();
        assert_eq!(
            fs::read(success.config.path().join("config.toml")).unwrap(),
            config_before
        );
    }

    let collision = PlainDraftSandbox::new();
    collision.editor(
        "collision-editor.sh",
        "printf 'EDITOR_READY\\n'\nIFS= read -r _\nprintf \"print('kept')\\n\" >> \"$1\"",
    );
    let mut child = collision.spawn("script", "Race Name");
    child.wait_for("Name in skit");
    child.send("Race Name\n");
    child.wait_for("EDITOR_READY");
    add_command(&collision.service(), "Race Name", "printf competitor");
    child.send("continue\n");
    let (output, marker) = child.finish();
    assert!(marker.contains("error|code=1"), "{marker}");
    assert!(output.contains("Your draft was kept at"), "{output}");
    assert_eq!(
        collision
            .service()
            .show("Race Name")
            .unwrap()
            .meta
            .kind
            .as_str(),
        "command"
    );
    let drafts = tui_drafts(collision.data.path());
    assert_eq!(drafts.len(), 1);
    assert!(
        fs::read(&drafts[0].path)
            .unwrap()
            .ends_with(b"print('kept')\n")
    );
    collision.assert_no_state();

    let editor_error = PlainDraftSandbox::new();
    editor_error.set_editor(Path::new("/nonexistent/skit-plain-editor"));
    let mut child = editor_error.spawn("script", "Editor Error");
    child.wait_for("Name in skit");
    child.send("Editor Error\n");
    let (output, marker) = child.finish();
    assert!(marker.contains("error|code=1"), "{marker}");
    assert!(output.contains("Could not launch the editor"), "{output}");
    editor_error.assert_no_drafts();
    editor_error.assert_no_state();

    let inspect_error = PlainDraftSandbox::new();
    inspect_error.editor("remove-editor.sh", "rm -f \"$1\"");
    let mut child = inspect_error.spawn("script", "Inspect Error");
    child.wait_for("Name in skit");
    child.send("Inspect Error\n");
    let (output, marker) = child.finish();
    assert!(marker.contains("error|code=1"), "{marker}");
    assert!(output.contains("File not found:"), "{output}");
    inspect_error.assert_no_drafts();
    inspect_error.assert_no_state();
}

#[cfg(unix)]
#[test]
fn bare_plain_editor_choices_reach_the_same_typed_draft_commit() {
    for (choice, kind, name, expected_kind, editor_body, expected_bytes) in [
        (
            "2\n",
            "script",
            "Bare Script",
            "python",
            "printf \"print('bare')\\n\" >> \"$1\"",
            b"#!/usr/bin/env python3\nprint('bare')\n".as_slice(),
        ),
        (
            "3\n",
            "prompt",
            "Bare Prompt",
            "prompt",
            "printf \"Review bare.\\n\" >> \"$1\"",
            b"# New prompt\n\nReview bare.\n".as_slice(),
        ),
    ] {
        let sandbox = PlainDraftSandbox::new();
        sandbox.editor("bare-editor.sh", editor_body);
        let config_before = fs::read(sandbox.config.path().join("config.toml")).unwrap();
        let mut child = sandbox.spawn_bare(kind, name);
        child.wait_for("Which one? [1/2/3/4] (1):");
        child.send(choice);
        child.wait_for("Name in skit");
        child.send(&format!("{name}\n"));
        let (output, marker) = child.finish();
        assert!(marker.contains(&format!("ok|name={name}|kind={expected_kind}|source=")));
        assert!(output.contains("What would you like to add?"), "{output}");
        let entry = sandbox.service().show(name).unwrap();
        assert_eq!(entry.meta.kind.as_str(), expected_kind);
        assert_eq!(
            fs::read(source_path(sandbox.service().repository(), &entry).unwrap()).unwrap(),
            expected_bytes
        );
        sandbox.assert_no_drafts();
        sandbox.assert_no_state();
        assert_eq!(
            fs::read(sandbox.config.path().join("config.toml")).unwrap(),
            config_before
        );
    }
}

#[test]
#[ignore = "runs only as the child of the process-isolated TERM adapter owner"]
fn wants_tui_form_term_child() {
    let result = wants_tui_form(Path::new(
        &std::env::var_os("SKIT_TERM_CHILD_CONFIG").unwrap(),
    ))
    .unwrap();
    fs::write(
        std::env::var_os("SKIT_TERM_CHILD_RESULT").unwrap(),
        result.to_string(),
    )
    .unwrap();
}

#[test]
fn add_intake_refusals_and_term_selection_are_no_write_and_localized() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path().join("data"));
    let service = LibraryService::new(store);
    let config = root.path().join("config");
    let mut options = empty_add_options();
    options.source = Some(PathBuf::from("-"));
    options.name = Some("Pipe".to_owned());
    options.reference = true;
    let error = add_with_config(&service, &config, options).unwrap_err();
    for (locale, expected) in [
        (
            Locale::En,
            "--ref can't apply here — stdin authors a brand-new copy, and --ref/--exe need an existing file (nothing was added).",
        ),
        (
            Locale::ZhCn,
            "--ref 在这里无法应用——stdin 会撰写一份全新副本，而 --ref/--exe 需要现成的文件(未添加任何内容)。",
        ),
        (
            Locale::ZhTw,
            "--ref 在這裡無法套用——stdin 會撰寫一份全新副本，而 --ref/--exe 需要現成的檔案(未加入任何內容)。",
        ),
    ] {
        assert_eq!(error.message().localize(locale), expected);
    }
    assert!(!root.path().join("data").exists());

    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        "form = \"tui\"\nfuture = \"keep\"\n",
    )
    .unwrap();
    let config_before = fs::read(config.join("config.toml")).unwrap();
    let result_dir = TempDir::new().unwrap();
    let result = result_dir.path().join("result");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "cli::tests::wants_tui_form_term_child",
            "--nocapture",
        ])
        .env("TERM", "dumb")
        .env("SKIT_TERM_CHILD_CONFIG", &config)
        .env("SKIT_TERM_CHILD_RESULT", &result)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(result).unwrap(), "false");
    assert_eq!(fs::read(config.join("config.toml")).unwrap(), config_before);
}

#[derive(Debug)]
struct FixedNetwork(bool);

impl NetworkProbe for FixedNetwork {
    fn can_connect(&self, _host: &str, _port: u16, _timeout: std::time::Duration) -> bool {
        self.0
    }
}

/// A scripted answer set for the first-run questions.
#[derive(Debug, Default)]
struct ScriptedFirstRun {
    accept: bool,
    answers: Vec<&'static str>,
    asked: RefCell<Vec<&'static str>>,
}

impl FirstRunPrompt for ScriptedFirstRun {
    fn confirm_mirrors(&self) -> bool {
        self.asked.borrow_mut().push("confirm");
        self.accept
    }

    fn axis_answer(
        &self,
        question: &'static str,
        _presets: &[String],
        _url_question: &'static str,
        _https_only: bool,
    ) -> String {
        let mut asked = self.asked.borrow_mut();
        let index = asked.iter().filter(|entry| **entry != "confirm").count();
        asked.push(question);
        self.answers[index].to_owned()
    }
}

/// The first-run offer probes once and records that it happened.
///
/// Version 0.4 gates on a written `[mirror]` section, not on the file existing: setting a language
/// also writes `config.toml` and must not suppress the offer (`src/skit/config.py:178-183`).
/// Blocked, declined, and not-blocked all write the marker (`src/skit/cli.py:5617-5618`).
#[test]
fn the_first_run_mirror_offer_writes_its_marker_once_and_never_probes_again() {
    let config_dir = TempDir::new().unwrap();
    let store = FileConfigStore::new(config_dir.path().to_path_buf());

    // A language-only configuration is not a mirror decision.
    store.set("lang", "zh-CN").unwrap();
    assert!(!store.mirror_configured().unwrap());

    // An open network asks nothing and still records the offer as done.
    let prompt = ScriptedFirstRun::default();
    first_run_mirror_offer(&store, &FixedNetwork(true), &prompt, true).unwrap();
    assert!(prompt.asked.borrow().is_empty());
    assert!(store.mirror_configured().unwrap());
    let mirror = store.mirror().unwrap();
    assert!(!mirror.enabled);
    assert!(mirror.pypi.is_empty());
    // The language survives the marker write.
    assert_eq!(store.get("lang").unwrap(), "zh-CN");

    // A second run never probes, whatever the network looks like.
    #[derive(Debug)]
    struct Forbidden;
    impl NetworkProbe for Forbidden {
        fn can_connect(&self, _host: &str, _port: u16, _timeout: std::time::Duration) -> bool {
            panic!("the offer probed twice");
        }
    }
    first_run_mirror_offer(&store, &Forbidden, &ScriptedFirstRun::default(), true).unwrap();
}

/// A run with nobody to ask probes nothing and writes nothing.
///
/// Version 0.4 returns before the probe and before the marker (`src/skit/cli.py:5607-5608`), so a
/// piped or scripted first run still gets the offer later on a real terminal.
#[test]
fn a_noninteractive_first_run_neither_probes_nor_marks() {
    let config_dir = TempDir::new().unwrap();
    let store = FileConfigStore::new(config_dir.path().to_path_buf());
    #[derive(Debug)]
    struct Forbidden;
    impl NetworkProbe for Forbidden {
        fn can_connect(&self, _host: &str, _port: u16, _timeout: std::time::Duration) -> bool {
            panic!("a non-interactive run probed the network");
        }
    }
    first_run_mirror_offer(&store, &Forbidden, &ScriptedFirstRun::default(), false).unwrap();
    assert!(!store.mirror_configured().unwrap());
}

/// A blocked network offers the wizard, and a decline still ends the offer for good.
#[test]
fn a_blocked_network_offers_the_wizard_and_a_decline_only_writes_the_marker() {
    let config_dir = TempDir::new().unwrap();
    let store = FileConfigStore::new(config_dir.path().to_path_buf());
    let prompt = ScriptedFirstRun {
        accept: false,
        ..ScriptedFirstRun::default()
    };
    first_run_mirror_offer(&store, &FixedNetwork(false), &prompt, true).unwrap();
    assert_eq!(prompt.asked.borrow().as_slice(), ["confirm"]);
    assert!(store.mirror_configured().unwrap());
    assert!(!store.mirror().unwrap().enabled);
}

/// Accepting asks all three axes independently and stores every answer.
#[test]
fn a_blocked_network_configures_each_axis_from_its_own_answer() {
    let config_dir = TempDir::new().unwrap();
    let store = FileConfigStore::new(config_dir.path().to_path_buf());
    let prompt = ScriptedFirstRun {
        accept: true,
        answers: vec!["aliyun", "off", "https://registry.example.test"],
        asked: RefCell::new(Vec::new()),
    };
    first_run_mirror_offer(&store, &FixedNetwork(false), &prompt, true).unwrap();

    assert_eq!(
        prompt.asked.borrow().as_slice(),
        [
            "confirm",
            "PyPI index (Python packages)",
            "GitHub releases (Python builds, the uv binary)",
            "npm registry (JS/TS packages)",
        ]
    );
    let mirror = store.mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.pypi, "https://mirrors.aliyun.com/pypi/simple");
    // One axis answered off must not disable another.
    assert!(mirror.python_install.is_empty());
    assert!(mirror.uv_binary.is_empty());
    assert_eq!(mirror.npm, "https://registry.example.test");
    assert!(store.mirror_configured().unwrap());
}

/// A marker write must preserve an existing mirror choice exactly.
#[test]
fn the_first_run_marker_keeps_a_configured_mirror_unchanged() {
    let config_dir = TempDir::new().unwrap();
    let store = FileConfigStore::new(config_dir.path().to_path_buf());
    store.set("mirror.pypi", "tsinghua").unwrap();
    let before = store.mirror().unwrap();
    store.mark_mirror_configured().unwrap();
    assert_eq!(store.mirror().unwrap(), before);
    assert!(before.enabled);
    assert_eq!(before.pypi, "https://pypi.tuna.tsinghua.edu.cn/simple");
}

/// Every axis answer the wizard can produce must survive the configuration gate.
///
/// Version 0.4 stores the three axes in one save and derives both github-release URLs from one
/// base (`src/skit/cli.py:5582-5601` and `src/skit/config.py:56-59`).
#[test]
fn the_mirror_wizard_answers_reach_the_store_on_every_axis() {
    let config_dir = TempDir::new().unwrap();
    let store = FileConfigStore::new(config_dir.path().to_path_buf());
    store
        .set_many(&BTreeMap::from([
            ("mirror.pypi".to_owned(), "tsinghua".to_owned()),
            ("mirror.github".to_owned(), "nju".to_owned()),
            ("mirror.npm".to_owned(), "npmmirror".to_owned()),
        ]))
        .unwrap();
    let mirror = store.mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.pypi, "https://pypi.tuna.tsinghua.edu.cn/simple");
    assert_eq!(
        mirror.python_install,
        "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/"
    );
    assert_eq!(
        mirror.uv_binary,
        "https://mirror.nju.edu.cn/github-release/astral-sh/uv"
    );
    assert_eq!(mirror.npm, "https://registry.npmmirror.com");

    // Turning every axis off is a real answer, and it still leaves the marker behind.
    store
        .set_many(&BTreeMap::from([
            ("mirror.pypi".to_owned(), "off".to_owned()),
            ("mirror.github".to_owned(), "off".to_owned()),
            ("mirror.npm".to_owned(), "off".to_owned()),
        ]))
        .unwrap();
    let mirror = store.mirror().unwrap();
    assert!(!mirror.enabled);
    assert!(mirror.pypi.is_empty() && mirror.uv_binary.is_empty() && mirror.npm.is_empty());
    assert!(store.mirror_configured().unwrap());
}

/// The custom-URL gate is the same one every entrance uses (`src/skit/config.py:62-71`).
#[test]
fn a_custom_mirror_url_is_one_token_and_github_demands_https() {
    for value in ["https://example.test/simple", "http://example.test/simple"] {
        assert!(mirror_url_is_acceptable(value, false), "{value}");
    }
    for value in [
        "tsinghua",
        "",
        "example.test",
        "https://a b",
        "https://a\u{b7}b",
    ] {
        assert!(!mirror_url_is_acceptable(value, false), "{value}");
    }
    // The uv binary is downloaded and executed, so its base must be https.
    assert!(mirror_url_is_acceptable(
        "https://mirror.test/github-release",
        true
    ));
    assert!(!mirror_url_is_acceptable(
        "http://mirror.test/github-release",
        true
    ));
}

/// What a save keeps cannot depend on the source moving between open and save.
///
/// The old submit path rebuilt the discard set from a fresh read at save time, so a concurrent
/// edit changed which rows survived, and a source that momentarily could not be read widened the
/// set that did. Both are gone: the row carries the declaration it opened with, and the save reads
/// no source to decide provenance. Version 0.4 captures the same baseline when the screen opens
/// (`src/skit/tui_settings.py:115-133`).
#[test]
fn a_source_that_moves_between_open_and_save_cannot_change_which_rows_persist() {
    let persisted = |interference: Option<&[u8]>| -> Vec<String> {
        let root = TempDir::new().unwrap();
        let data_dir = root.path().join("data");
        let state_dir = root.path().join("state");
        let config_dir = root.path().join("config");
        let source = root.path().join("tool.ps1");
        fs::write(&source, "param([string]$Name = 'World')\n").unwrap();
        let store = FileStore::new(&data_dir);
        let service = LibraryService::new(store.clone());
        add_with_config(
            &service,
            &config_dir,
            AddOptions {
                source: Some(source),
                kind: None,
                name: Some("Ps".to_owned()),
                description: Some(String::new()),
                reference: false,
                command_template: None,
                prompt: false,
                executable: false,
                runner: None,
                no_interpolate: false,
                dependencies: Vec::new(),
                dependencies_explicit: false,
                requires_python: None,
                no_input: true,
            },
        )
        .unwrap();
        // A declared rider is legitimate on PowerShell, which writes no schema into its own file.
        let entry = service.show("ps").unwrap();
        let mut settings = EntrySettings::from_meta(&entry.meta);
        let mut rider = ParamDecl::new("API_TOKEN");
        rider.delivery = ParameterDelivery::Env;
        settings.parameters = vec![rider];
        let claimed = service.claim_identity(&entry).unwrap();
        service
            .update_entry(
                &claimed,
                UpdateEntry {
                    name: entry.meta.name.clone(),
                    description: entry.meta.description.clone(),
                    settings,
                    workdir: entry.meta.workdir.clone(),
                    source: None,
                    expected_source_hash: entry.meta.source_hash.clone(),
                },
            )
            .unwrap();

        let values = settings_edits(
            &service,
            &store,
            &state_dir,
            "ps",
            &[("parameter:API_TOKEN:prompt", "Access token")],
        );

        // The interference happens after the form is built and before it is submitted.
        let stored = data_dir.join("scripts/ps/script.ps1");
        if let Some(bytes) = interference {
            fs::write(&stored, bytes).unwrap();
        }

        tui_submit_settings(&service, &store, &state_dir, "ps", &values).unwrap();
        EntrySettings::from_meta(&service.show("ps").unwrap().meta)
            .parameters
            .into_iter()
            .map(|parameter| format!("{}={}", parameter.name, parameter.prompt))
            .collect()
    };

    let undisturbed = persisted(None);
    assert_eq!(undisturbed, ["API_TOKEN=Access token"]);
    // A concurrent edit that adds a whole CLI surface must not change the outcome.
    assert_eq!(
        persisted(Some(
            b"param([string]$Name = 'World', [string]$API_TOKEN = 'x')\n"
        )),
        undisturbed,
        "a concurrent source edit changed which rows persist"
    );
    // Bytes the host cannot decode must not change it either.
    assert_eq!(
        persisted(Some(b"param(\xff\xfe)\n")),
        undisturbed,
        "an unreadable source changed which rows persist"
    );
}

/// A completed host operation hands back the whole surface, so the detail pane keeps its facts.
///
/// The pane's parameters and run status are projected beside the entry list, and a mutation used to
/// return the list alone. The entry a person is looking at right after an add is the one that had
/// no row in the detail map at all, so its pane showed a name, a kind and a description and stopped
/// — which is what the recorded frame caught. Neither the projection nor the drawing was missing;
/// the refresh path dropped the facts between them.
#[test]
fn a_completed_operation_keeps_the_detail_facts_the_pane_draws() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("banner.py");
    fs::write(&source, "MESSAGE = \"Hello from skit\"\nprint(MESSAGE)\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            name: Some("banner".to_owned()),
            description: Some(String::new()),
            ..add_options()
        },
    )
    .unwrap();
    // Manage the constant so the pane has a parameter to report.
    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "banner",
        &[("source:manage", "MESSAGE")],
    );
    tui_submit_settings(&service, &store, &state_dir, "banner", &values).unwrap();

    let UiAction::Complete { surface, .. } =
        tui_complete(&service, &state_dir, "Settings saved").unwrap()
    else {
        panic!("a completed operation must replace the library surface");
    };
    let surface = surface.expect("the surface must travel");
    let slug = Slug::parse("banner").unwrap();
    let detail = surface
        .details
        .get(&slug)
        .expect("the refreshed surface must carry the entry's detail facts");
    assert!(
        detail
            .parameters
            .iter()
            .any(|parameter| parameter.key == "MESSAGE"),
        "the detail pane lost its parameters: {:?}",
        detail.parameters
    );
    assert!(
        detail.last_run.is_none(),
        "a never-run entry reports no run, which is what draws `Not run yet`"
    );
}

/// `skit add <path>` from a terminal reviews before it writes.
///
/// This is version 0.4's common path — its own tape says so
/// (`docs/assets/demo/demo.tape:8`, `:47`) — and it hosts the same review panel its `a` door hosts
/// for every source kind (`src/skit/cli.py:2001-2009`, `:2076-2086`, `:2116-2126`). Writing the
/// entry and printing a summary skips the one place a person names it, chooses copy or link, and
/// edits the detected dependencies.
///
/// Interactive is both streams being terminals (`:83-84`). It is a parameter here so the rule is
/// testable; the composition root supplies the real probe.
#[test]
fn a_path_add_from_a_terminal_opens_the_review_and_every_other_lane_does_not() {
    let path = || AddOptions {
        source: Some(PathBuf::from("greet.py")),
        ..add_options()
    };
    assert_eq!(
        review_before_add(&path(), true),
        Some("greet.py".to_owned()),
        "the common path must review before it writes"
    );

    // A pipe, CI, or a redirected stream has nobody to ask.
    assert_eq!(review_before_add(&path(), false), None);

    // `--no-input` is the contract that says do not ask.
    assert_eq!(
        review_before_add(
            &AddOptions {
                no_input: true,
                ..path()
            },
            true
        ),
        None
    );

    // Standard input is the non-interactive spelling, and it is also the source.
    assert_eq!(
        review_before_add(
            &AddOptions {
                source: Some(PathBuf::from("-")),
                ..path()
            },
            true
        ),
        None
    );

    // A command template has no file to inspect.
    assert_eq!(
        review_before_add(
            &AddOptions {
                source: None,
                command_template: Some("echo {name}".to_owned()),
                ..path()
            },
            true
        ),
        None
    );

    // A bare add has no path yet; it opens the source picker through its own branch.
    assert_eq!(
        review_before_add(
            &AddOptions {
                source: None,
                ..path()
            },
            true
        ),
        None
    );
}

/// The opening actions land the panel on the review for the path the shell already named.
///
/// The shell door and the `a` door must not answer the same command differently, so the path
/// arrives as the same actions a person's keystrokes produce rather than as a second construction
/// path into the same state.
#[test]
fn the_opening_actions_reach_the_review_for_the_path_the_shell_named() {
    let root = TempDir::new().unwrap();
    let store_root = TempDir::new().unwrap();
    let source = root.path().join("greet.py");
    fs::write(&source, "GREETING = \"hello\"\nprint(GREETING)\n").unwrap();

    let mut workflow = AddWorkflowState::new(Vec::new());
    let effects = workflow.reduce(AddAction::SetSourcePath(source.display().to_string()));
    assert!(
        effects.is_empty(),
        "naming a path inspects nothing by itself"
    );
    let effects = workflow.reduce(AddAction::Continue);
    let AddEffect::InspectSource { request, path } = effects
        .into_iter()
        .next()
        .expect("continuing must ask the host to read the source")
    else {
        panic!("the workflow must inspect the named source");
    };
    assert_eq!(path, source);

    // The host reads the bytes and hands them back, which is what the real loop does.
    let snapshot = tui_add_source(store_root.path(), &path).unwrap();
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(snapshot),
    });

    let review = workflow
        .review()
        .expect("the panel must open on the review, not on a source picker");
    assert_eq!(review.name(), "greet");
}

/// Everything one settings save could damage, read as text so a failure names the axis.
fn settings_axes(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    slug: &str,
) -> BTreeMap<String, String> {
    let entry = service.show(slug).unwrap();
    let settings = EntrySettings::from_meta(&entry.meta);
    let presets = FormStateService::new(FileFormStateStore::new(state_dir))
        .load(&entry.slug)
        .presets;
    let source = source_path(store, &entry)
        .and_then(|path| fs::read(path).ok())
        .unwrap_or_default();
    BTreeMap::from([
        ("name".to_owned(), entry.meta.name.clone()),
        ("description".to_owned(), entry.meta.description.clone()),
        ("workdir".to_owned(), entry.meta.workdir.clone()),
        ("interpreter".to_owned(), settings.interpreter.clone()),
        ("runner".to_owned(), settings.runner.clone()),
        ("template".to_owned(), settings.template.clone()),
        ("interpolate".to_owned(), settings.interpolate.to_string()),
        ("needs".to_owned(), settings.needs.join(",")),
        ("dependencies".to_owned(), settings.dependencies.join(",")),
        ("python".to_owned(), settings.requires_python.clone()),
        (
            "parameters".to_owned(),
            format!("{:?}", settings.parameters),
        ),
        ("presets".to_owned(), format!("{presets:?}")),
        (
            "source".to_owned(),
            String::from_utf8_lossy(&source).into_owned(),
        ),
    ])
}

/// Moving one control must leave every other axis exactly as it was.
///
/// This is the whole risk of a screen that submits only what moved. Version 0.4's save reads one
/// widget per axis and never reads a widget that is not on the screen, so an absent control is
/// never an instruction to clear (`src/skit/tui_settings.py:928-1001`). The typed screen makes that
/// stronger — an untouched control is absent too — which turns every unconditional read in the save
/// into a data-loss defect. The name axis was exactly that: a required read refused any save that
/// did not carry it.
///
/// The check is a table rather than one assertion per axis, so an axis added later is covered by
/// the same loop instead of being forgotten.
fn assert_one_axis_moves(
    service: &LibraryService<FileStore>,
    store: &FileStore,
    state_dir: &Path,
    slug: &str,
    control: &str,
    value: &str,
    expected: &[&str],
) {
    let before = settings_axes(service, store, state_dir, slug);
    let values = settings_edits(service, store, state_dir, slug, &[(control, value)]);
    tui_submit_settings(service, store, state_dir, slug, &values).unwrap();
    let after = settings_axes(service, store, state_dir, slug);
    for (axis, was) in &before {
        if expected.contains(&axis.as_str()) {
            continue;
        }
        assert_eq!(
            &after[axis], was,
            "moving {control} also changed {axis}: {was:?} became {:?}",
            after[axis]
        );
    }
    assert!(
        expected.iter().any(|axis| after[*axis] != before[*axis]),
        "moving {control} changed none of {expected:?}"
    );
}

#[test]
fn a_python_settings_save_that_moves_one_control_leaves_every_other_axis_alone() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let source = root.path().join("tool.py");
    fs::write(&source, "GREETING = \"hello\"\nprint(GREETING)\n").unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());
    add_with_config(
        &service,
        &config_dir,
        AddOptions {
            source: Some(source),
            kind: None,
            name: Some("Py tool".to_owned()),
            description: Some("First".to_owned()),
            reference: false,
            command_template: None,
            prompt: false,
            executable: false,
            runner: None,
            no_interpolate: false,
            dependencies: vec!["requests>=2,<3".to_owned()],
            dependencies_explicit: true,
            requires_python: Some(">=3.11".to_owned()),
            no_input: false,
        },
    )
    .unwrap();
    // Give it something on every axis a save could take away.
    let manage = settings_edits(
        &service,
        &store,
        &state_dir,
        "py-tool",
        &[("source:manage", "GREETING"), ("needs", "jq")],
    );
    tui_submit_settings(&service, &store, &state_dir, "py-tool", &manage).unwrap();
    let entry = service.show("py-tool").unwrap();
    FormStateService::new(FileFormStateStore::new(&state_dir))
        .save_preset(
            &entry.slug,
            "nightly",
            &entry_parameters(&store, &entry),
            &BTreeMap::from([("GREETING".to_owned(), "hi".to_owned())]),
        )
        .unwrap();

    for (control, value, axes) in [
        ("name", "Renamed tool", &["name"][..]),
        ("description", "Second", &["description"]),
        ("workdir", "store", &["workdir"]),
        ("needs", "jq, ffmpeg", &["needs"]),
        // A uv edit lands in the stored copy's own PEP 723 block, and the meta mirror follows it.
        (
            "dependencies",
            "requests>=2,<3, rich",
            &["source", "dependencies"],
        ),
        ("python", ">=3.12", &["source", "python"]),
        // A block-managed row lives in the script, so its edit is a source edit and nothing else.
        ("parameter:GREETING:prompt", "Who to greet", &["source"]),
        ("preset:nightly", "false", &["presets"]),
    ] {
        assert_one_axis_moves(
            &service, &store, &state_dir, "py-tool", control, value, axes,
        );
    }
}

#[test]
fn a_command_settings_save_that_moves_one_control_leaves_every_other_axis_alone() {
    let root = TempDir::new().unwrap();
    let state_dir = root.path().join("state");
    let store = FileStore::new(root.path());
    let service = LibraryService::new(store.clone());
    add_command(&service, "Deploy", "deploy {{target}} --force");

    // Give it a declared parameter and a needs list, so there is something to lose.
    let complete = settings_edits(
        &service,
        &store,
        &state_dir,
        "deploy",
        &[("needs", "ssh"), ("parameter:add", "region")],
    );
    tui_submit_settings(&service, &store, &state_dir, "deploy", &complete).unwrap();
    let stored = EntrySettings::from_meta(&service.show("deploy").unwrap().meta);
    assert_eq!(stored.template, "deploy {{target}} --force");
    assert!(
        stored.parameters.iter().any(|item| item.name == "region"),
        "the declared parameter is stored: {:?}",
        stored.parameters
    );

    for (control, value, axes) in [
        ("name", "Deploy again", &["name"][..]),
        ("description", "Ship it", &["description"]),
        ("needs", "ssh, rsync", &["needs"]),
        ("parameter:region:help", "Which region", &["parameters"]),
        ("parameter:region:keep", "false", &["parameters"]),
    ] {
        assert_one_axis_moves(&service, &store, &state_dir, "deploy", control, value, axes);
    }

    // The template is its own axis, and changing it reconciles the placeholder schema — so it is
    // the one control that is allowed to move the parameters with it.
    let before = settings_axes(&service, &store, &state_dir, "deploy");
    let values = settings_edits(
        &service,
        &store,
        &state_dir,
        "deploy",
        &[("template", "deploy {{target}} --dry-run")],
    );
    tui_submit_settings(&service, &store, &state_dir, "deploy", &values).unwrap();
    let after = settings_axes(&service, &store, &state_dir, "deploy");
    assert_ne!(after["template"], before["template"]);
    for axis in ["name", "description", "needs", "workdir", "presets"] {
        assert_eq!(after[axis], before[axis], "the template edit moved {axis}");
    }
}

/// A typed payload is read as its type, never re-parsed from text.
///
/// The field model already refuses to let a string stand for an intent: a toggle is a Boolean and a
/// closed selection is a list. Flattening those to text at the effect boundary would put the
/// inference back on every host, and a value that carries a comma is where the two answers diverge
/// — a PEP 508 requirement carries one inside its own specifier
/// (`src/skit/tui_settings.py:988-993`).
#[test]
fn a_typed_submission_is_read_as_its_type_and_never_re_split() {
    let values: SubmittedValues = BTreeMap::from([
        ("flag".to_owned(), FieldValue::boolean(true)),
        ("spelled".to_owned(), FieldValue::text("yes")),
        (
            "picked".to_owned(),
            FieldValue::Explicit(TypedValue::Choices(vec![
                "requests>=2,<3".to_owned(),
                "rich".to_owned(),
            ])),
        ),
        ("typed".to_owned(), FieldValue::text("ffmpeg, jq")),
    ]);

    // A Boolean renders as the same word a person types, so both reach the same answer. This is
    // why the toggle needs no typed branch of its own.
    assert!(tui_flag(&values, "flag").unwrap());
    assert!(tui_flag(&values, "spelled").unwrap());
    // An absent toggle is off, and it is not an error.
    assert!(!tui_flag(&values, "missing").unwrap());

    // The selection keeps its own members. Splitting its text would merge the requirement's
    // internal comma into a neighbour and deliver a dependency nobody asked for.
    assert_eq!(
        tui_list(&values, "picked"),
        ["requests>=2,<3".to_owned(), "rich".to_owned()]
    );
    assert_eq!(
        tui_split_list(&tui_value(&values, "picked")),
        ["requests>=2".to_owned(), "<3".to_owned(), "rich".to_owned()],
        "this is what re-splitting the text would have produced"
    );
    // A text control still splits, because text is all it produced.
    assert_eq!(
        tui_list(&values, "typed"),
        ["ffmpeg".to_owned(), "jq".to_owned()]
    );

    // Presence is the "was this offered" answer, and it survives an explicit clear.
    let cleared: SubmittedValues = BTreeMap::from([("needs".to_owned(), FieldValue::text(""))]);
    assert!(
        cleared.contains_key("needs"),
        "a cleared axis is still offered"
    );
    assert!(tui_value(&cleared, "needs").is_empty());
    assert!(!cleared.contains_key("template"));
}

fn test_tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, rows: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if metadata.is_dir() {
                rows.insert(relative, None);
                visit(root, &path, rows);
            } else if metadata.file_type().is_symlink() {
                rows.insert(
                    relative,
                    Some(
                        fs::read_link(&path)
                            .unwrap()
                            .to_string_lossy()
                            .as_bytes()
                            .to_vec(),
                    ),
                );
            } else {
                rows.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }

    let mut rows = BTreeMap::new();
    visit(root, root, &mut rows);
    rows
}

fn owned_draft_quarantines(root: &Path) -> Vec<PathBuf> {
    let drafts = root.join("drafts");
    let Ok(entries) = fs::read_dir(drafts) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".skit-quarantine-"))
        })
        .map(|path| {
            if path.is_dir() {
                path.join("draft")
            } else {
                path
            }
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn assert_application_identity(_: Option<skit_application::SourceIdentity>) {}

fn owned_draft_snapshot(root: &TempDir, name: &str, bytes: &[u8]) -> AddSourceSnapshot {
    let drafts = root.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let path = drafts.join(name);
    fs::write(&path, bytes).unwrap();
    let snapshot = tui_add_source(root.path(), &path).unwrap();
    assert!(snapshot.is_draft);
    #[cfg(any(unix, windows))]
    assert!(
        snapshot.identity.is_some(),
        "a supported host gives every owned draft a CAS identity"
    );
    #[cfg(not(any(unix, windows)))]
    assert!(
        snapshot.identity.is_none(),
        "unsupported hosts keep drafts but do not invent weak identities"
    );
    assert_application_identity(snapshot.identity.clone());
    assert_eq!(
        serde_json::from_slice::<AddSourceSnapshot>(&serde_json::to_vec(&snapshot).unwrap())
            .unwrap(),
        snapshot,
        "a real host identity survives the serialized UI seam"
    );
    snapshot
}

#[cfg(unix)]
#[test]
fn owned_draft_creation_refuses_missing_unowned_and_invalid_roots_without_any_write() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _, os::unix::fs::symlink};

    let claim_root = TempDir::new().unwrap();
    let original =
        owned_draft_snapshot(&claim_root, "skit-new-shape-claim.py", b"print('claim')\n");
    let claim_before = test_tree_snapshot(claim_root.path());
    let missing_parent = TempDir::new().unwrap();
    let missing_data = missing_parent.path().join("missing-data");
    let mut forged = original.clone();
    forged.path = missing_data.join("drafts/skit-new-shaped.py");
    forged.source_record = forged.path.display().to_string();
    assert!(!has_owned_draft_shape(&missing_data, &forged.path));
    let error = consume_owned_draft(&missing_data, &forged).unwrap_err();
    for (locale, expected) in [
        (
            Locale::En,
            "refusing to remove a file outside skit's drafts directory",
        ),
        (Locale::ZhCn, "拒绝删除 skit 草稿目录以外的文件"),
        (Locale::ZhTw, "拒絕移除 skit 草稿目錄以外的檔案"),
    ] {
        assert_eq!(error.message().localize(locale), expected);
    }
    assert!(!missing_data.exists());
    assert_eq!(test_tree_snapshot(claim_root.path()), claim_before);

    let file_root = TempDir::new().unwrap();
    let file_data = file_root.path().join("data");
    fs::create_dir_all(&file_data).unwrap();
    let file_path = file_data.join("drafts");
    fs::write(&file_path, b"user file\n").unwrap();
    let file_before = test_tree_snapshot(file_root.path());
    let error = create_owned_drafts_dir(&file_data).unwrap_err();
    for (locale, expected) in [
        (
            Locale::En,
            format!(
                "skit's drafts path is not an owned directory: {}",
                file_path.display()
            ),
        ),
        (
            Locale::ZhCn,
            format!("skit 的草稿路径不是其自有目录：{}", file_path.display()),
        ),
        (
            Locale::ZhTw,
            format!("skit 的草稿路徑不是其自有目錄：{}", file_path.display()),
        ),
    ] {
        assert_eq!(error.message().localize(locale), expected);
    }
    assert_eq!(test_tree_snapshot(file_root.path()), file_before);

    let link_root = TempDir::new().unwrap();
    let link_data = link_root.path().join("data");
    let outside = link_root.path().join("outside");
    fs::create_dir_all(&link_data).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let outside_file = outside.join("skit-new-outside.py");
    fs::write(&outside_file, b"print('outside')\n").unwrap();
    let link_path = link_data.join("drafts");
    symlink(&outside, &link_path).unwrap();
    let link_before = test_tree_snapshot(link_root.path());
    let error = create_owned_drafts_dir(&link_data).unwrap_err();
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let message = error.message().localize(locale);
        assert!(
            message.contains(&link_path.display().to_string()),
            "{message}"
        );
    }
    assert_eq!(fs::read_link(&link_path).unwrap(), outside);
    assert_eq!(fs::read(&outside_file).unwrap(), b"print('outside')\n");
    assert_eq!(test_tree_snapshot(link_root.path()), link_before);

    let invalid_root = TempDir::new().unwrap();
    let invalid = invalid_root
        .path()
        .join(OsString::from_vec(b"data-with-\0-nul".to_vec()));
    let invalid_before = test_tree_snapshot(invalid_root.path());
    let error = create_owned_drafts_dir(&invalid).unwrap_err();
    assert!(matches!(
        &error,
        CliError::Source {
            operation: "create",
            ..
        }
    ));
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        assert!(!error.message().localize(locale).is_empty());
    }
    assert_eq!(test_tree_snapshot(invalid_root.path()), invalid_before);
}

#[cfg(any(unix, windows))]
#[test]
fn owned_draft_consume_is_idempotent_and_requires_the_exact_snapshot() {
    let root = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-exact.py", b"print(1)\n");

    assert_eq!(
        consume_owned_draft(root.path(), &snapshot).unwrap(),
        DraftConsumeOutcome::Removed
    );
    assert!(!snapshot.path.exists());
    assert_eq!(
        consume_owned_draft(root.path(), &snapshot).unwrap(),
        DraftConsumeOutcome::AlreadyMissing,
        "a repeated successful completion is harmless"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn owned_draft_consume_keeps_in_place_content_and_permission_changes() {
    let changed_bytes = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&changed_bytes, "skit-new-content.py", b"print('old')\n");
    fs::write(&snapshot.path, b"print('new')\n").unwrap();
    assert_eq!(
        consume_owned_draft(changed_bytes.path(), &snapshot).unwrap(),
        DraftConsumeOutcome::Changed
    );
    assert_eq!(fs::read(&snapshot.path).unwrap(), b"print('new')\n");

    let changed_permissions = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &changed_permissions,
        "skit-new-permissions.py",
        b"print(1)\n",
    );
    let mut permissions = fs::metadata(&snapshot.path).unwrap().permissions();
    permissions.set_readonly(!permissions.readonly());
    let replacement_readonly = permissions.readonly();
    fs::set_permissions(&snapshot.path, permissions).unwrap();
    assert_eq!(
        consume_owned_draft(changed_permissions.path(), &snapshot).unwrap(),
        DraftConsumeOutcome::Changed
    );
    assert!(snapshot.path.exists());
    assert_eq!(
        fs::metadata(&snapshot.path)
            .unwrap()
            .permissions()
            .readonly(),
        replacement_readonly
    );
}

#[cfg(any(unix, windows))]
#[test]
fn owned_draft_identity_rejects_a_same_bytes_replacement() {
    let root = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-replaced.py", b"print(1)\n");
    let old_identity = snapshot.identity.clone();
    let replacement_path = snapshot.path.with_extension("replacement");
    fs::write(&replacement_path, &snapshot.bytes).unwrap();
    fs::remove_file(&snapshot.path).unwrap();
    fs::rename(&replacement_path, &snapshot.path).unwrap();
    let replacement = tui_add_source(root.path(), &snapshot.path).unwrap();

    assert_ne!(
        replacement.identity, old_identity,
        "a new file incarnation needs a new identity"
    );
    assert_eq!(
        replacement.bytes, snapshot.bytes,
        "bytes alone cannot prove identity"
    );
    assert_eq!(
        consume_owned_draft(root.path(), &snapshot).unwrap(),
        DraftConsumeOutcome::Changed
    );
    assert!(snapshot.path.exists(), "the replacement is not consumed");
    assert!(owned_draft_quarantines(root.path()).is_empty());
}

#[cfg(any(unix, windows))]
#[test]
fn cli_commit_carries_the_initial_claim_through_postcommit_cleanup() {
    let root = TempDir::new().unwrap();
    const ORIGINAL: &[u8] = b"print('original')\n";
    const REPLACEMENT: &[u8] = b"print('replacement')\n";
    let claim = owned_draft_snapshot(&root, "skit-new-claimed.py", ORIGINAL);
    let store = FileStore::new(root.path());
    let service = LibraryService::new(store.clone());
    let create = skit_application::CreateEntry {
        name: "Claimed".to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: claim.source_record.clone(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(skit_application::EntryPayload {
            bytes: claim.bytes.clone(),
            stored_name: Some("script.py".to_owned()),
            permissions: claim.permissions,
        }),
        settings: EntrySettings::default(),
    };
    let source = claim.path.clone();
    let staged = source.with_extension("postcommit");

    let (created, outcome) = commit_add_source_with_hook(&service, create, Some(claim), || {
        fs::write(&staged, REPLACEMENT).unwrap();
        fs::remove_file(&source).unwrap();
        fs::rename(&staged, &source).unwrap();
    })
    .unwrap();

    assert_eq!(outcome, Some(DraftConsumeOutcome::Changed));
    assert_eq!(fs::read(&source).unwrap(), REPLACEMENT);
    assert_eq!(
        fs::read(store.payload_path(&created).unwrap()).unwrap(),
        ORIGINAL
    );
    assert_eq!(service.show("Claimed").unwrap().slug, created.slug);
}

#[cfg(any(unix, windows))]
#[test]
fn cli_cleanup_receipts_keep_the_committed_entry_and_changed_draft_in_every_locale() {
    let changed = TempDir::new().unwrap();
    const ORIGINAL: &[u8] = b"print('original')\n";
    const REPLACEMENT: &[u8] = b"print('replacement')\n";
    let claim = owned_draft_snapshot(&changed, "skit-new-receipt.py", ORIGINAL);
    let source = claim.path.clone();
    let staged = source.with_extension("replacement");
    let store = FileStore::new(changed.path());
    let service = LibraryService::new(store.clone());
    let create = skit_application::CreateEntry {
        name: "Changed receipt".to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: claim.source_record.clone(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(skit_application::EntryPayload {
            bytes: claim.bytes.clone(),
            stored_name: Some("script.py".to_owned()),
            permissions: claim.permissions,
        }),
        settings: EntrySettings::default(),
    };
    let (created, cleanup) = commit_add_source_after(&service, create, Some(claim), || {
        fs::write(&staged, REPLACEMENT).unwrap();
        fs::remove_file(&source).unwrap();
        fs::rename(&staged, &source).unwrap();
    })
    .unwrap();
    let cleanup = cleanup.expect("a copied owned draft must request cleanup");
    assert!(matches!(cleanup, Ok(DraftConsumeOutcome::Changed)));
    for (locale, expected) in [
        (
            Locale::En,
            format!(
                "warning: The kept draft changed before cleanup. skit kept it at {}.",
                source.display()
            ),
        ),
        (
            Locale::ZhCn,
            format!(
                "警告：保留的草稿在清理前发生了更改。skit 将它保留在 {}。",
                source.display()
            ),
        ),
        (
            Locale::ZhTw,
            format!(
                "警告：保留的草稿在清理前發生了變更。skit 將它保留在 {}。",
                source.display()
            ),
        ),
    ] {
        assert_eq!(
            draft_cleanup_warning_line(&cleanup, &source, locale).as_deref(),
            Some(expected.as_str())
        );
        report_draft_cleanup_warning(Some(&cleanup), &source, locale);
    }
    assert_eq!(fs::read(&source).unwrap(), REPLACEMENT);
    assert_eq!(
        fs::read(store.payload_path(&created).unwrap()).unwrap(),
        ORIGINAL
    );
    assert_eq!(service.show("Changed receipt").unwrap().slug, created.slug);

    let failed = TempDir::new().unwrap();
    let claim = owned_draft_snapshot(&failed, "skit-new-error-receipt.py", ORIGINAL);
    let source = claim.path.clone();
    let store = FileStore::new(failed.path());
    let service = LibraryService::new(store.clone());
    let create = skit_application::CreateEntry {
        name: "Error receipt".to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: claim.source_record.clone(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(skit_application::EntryPayload {
            bytes: claim.bytes.clone(),
            stored_name: Some("script.py".to_owned()),
            permissions: claim.permissions,
        }),
        settings: EntrySettings::default(),
    };
    let parked = failed.path().join("drafts-old");
    let drafts_path = failed.path().join("drafts");
    let (created, cleanup) = commit_add_source_after(&service, create, Some(claim), || {
        fs::rename(&drafts_path, &parked).unwrap();
        fs::write(&drafts_path, b"not a directory").unwrap();
    })
    .unwrap();
    let cleanup = cleanup.expect("a copied owned draft must request cleanup");
    assert!(
        cleanup.is_err(),
        "an unowned drafts path must refuse cleanup"
    );
    for (locale, expected) in [
        (
            Locale::En,
            format!(
                "warning: skit's drafts path is not an owned directory: {}",
                drafts_path.display()
            ),
        ),
        (
            Locale::ZhCn,
            format!(
                "警告：skit 的草稿路径不是其自有目录：{}",
                drafts_path.display()
            ),
        ),
        (
            Locale::ZhTw,
            format!(
                "警告：skit 的草稿路徑不是其自有目錄：{}",
                drafts_path.display()
            ),
        ),
    ] {
        assert_eq!(
            draft_cleanup_warning_line(&cleanup, &source, locale).as_deref(),
            Some(expected.as_str())
        );
        report_draft_cleanup_warning(Some(&cleanup), &source, locale);
    }
    assert_eq!(
        fs::read(parked.join("skit-new-error-receipt.py")).unwrap(),
        ORIGINAL
    );
    assert_eq!(
        fs::read(store.payload_path(&created).unwrap()).unwrap(),
        ORIGINAL
    );
    assert_eq!(service.show("Error receipt").unwrap().slug, created.slug);

    for cleanup in [
        Ok(DraftConsumeOutcome::Removed),
        Ok(DraftConsumeOutcome::AlreadyMissing),
    ] {
        assert!(draft_cleanup_warning_line(&cleanup, &source, Locale::En).is_none());
        report_draft_cleanup_warning(Some(&cleanup), &source, Locale::En);
    }
    report_draft_cleanup_warning(None, &source, Locale::En);
}

#[test]
fn owned_draft_consume_refuses_outside_and_parked_files() {
    let root = TempDir::new().unwrap();
    let valid = owned_draft_snapshot(&root, "skit-new-claim.py", b"print(0)\n");
    let outside = root.path().join("skit-new-outside.py");
    fs::write(&outside, b"print(1)\n").unwrap();
    let mut outside_snapshot = tui_add_source(root.path(), &outside).unwrap();
    assert!(!outside_snapshot.is_draft);
    outside_snapshot.is_draft = true;
    outside_snapshot.identity = valid.identity.clone();
    assert!(consume_owned_draft(root.path(), &outside_snapshot).is_err());
    assert!(outside.exists());

    let drafts = root.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let parked = drafts.join("mine.py");
    fs::write(&parked, b"print(2)\n").unwrap();
    let mut parked_snapshot = tui_add_source(root.path(), &parked).unwrap();
    assert!(!parked_snapshot.is_draft);
    parked_snapshot.is_draft = true;
    parked_snapshot.identity = valid.identity;
    assert!(consume_owned_draft(root.path(), &parked_snapshot).is_err());
    assert!(parked.exists());
}

#[cfg(not(any(unix, windows)))]
#[test]
fn unsupported_platform_keeps_owned_drafts_without_an_identity() {
    let root = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-portable.py", b"print(1)\n");
    assert!(snapshot.identity.is_none());
    assert!(consume_owned_draft(root.path(), &snapshot).is_err());
    assert!(snapshot.path.exists());
}

#[cfg(unix)]
#[test]
fn owned_draft_consume_never_follows_a_replacement_symlink() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-link.py", b"print('draft')\n");
    let target = outside.path().join("target.py");
    fs::write(&target, b"print('outside')\n").unwrap();
    fs::remove_file(&snapshot.path).unwrap();
    symlink(&target, &snapshot.path).unwrap();

    assert_eq!(
        consume_owned_draft(root.path(), &snapshot).unwrap(),
        DraftConsumeOutcome::Changed
    );
    assert!(
        fs::symlink_metadata(&snapshot.path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&target).unwrap(), b"print('outside')\n");
    assert!(owned_draft_quarantines(root.path()).is_empty());
}

#[cfg(any(unix, windows))]
#[test]
fn owned_draft_quarantine_closes_the_final_verify_remove_race() {
    let root = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-final.py", b"print('reviewed')\n");
    let staged = root.path().join("drafts/replacement.tmp");
    fs::write(&staged, b"print('replacement')\n").unwrap();
    let original = snapshot.path.clone();

    let outcome =
        consume_owned_draft_with_test_hook(root.path(), &snapshot, |point, quarantine| {
            if point == DraftConsumeTestPoint::Verified {
                assert!(quarantine.is_file());
                assert!(!original.exists(), "the claimed inode is quarantined");
                fs::rename(&staged, &original).unwrap();
            }
        })
        .unwrap();

    assert_eq!(outcome, DraftConsumeOutcome::Removed);
    assert_eq!(fs::read(&original).unwrap(), b"print('replacement')\n");
    assert!(owned_draft_quarantines(root.path()).is_empty());
}

#[cfg(any(unix, windows))]
#[test]
fn owned_draft_quarantine_restores_a_mismatch() {
    let root = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-restore.py", b"print('reviewed')\n");
    let staged = root.path().join("drafts/replacement.tmp");
    fs::write(&staged, b"print('replacement')\n").unwrap();
    fs::remove_file(&snapshot.path).unwrap();
    fs::rename(&staged, &snapshot.path).unwrap();

    assert_eq!(
        consume_owned_draft(root.path(), &snapshot).unwrap(),
        DraftConsumeOutcome::Changed
    );
    assert_eq!(fs::read(&snapshot.path).unwrap(), b"print('replacement')\n");
    assert!(owned_draft_quarantines(root.path()).is_empty());
}

#[cfg(any(unix, windows))]
#[test]
fn owned_draft_quarantine_restore_failure_keeps_every_file() {
    let root = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-restore-error.py", b"print('reviewed')\n");
    let staged = root.path().join("drafts/replacement.tmp");
    fs::write(&staged, b"print('replacement')\n").unwrap();
    fs::remove_file(&snapshot.path).unwrap();
    fs::rename(&staged, &snapshot.path).unwrap();
    let original = snapshot.path.clone();

    let error = consume_owned_draft_with_test_hook(root.path(), &snapshot, |point, quarantine| {
        if point == DraftConsumeTestPoint::BeforeRestore {
            // The helper already observed that the original path is absent. Inject a new arrival
            // immediately before the no-clobber restore syscall. An exists-then-rename restore
            // would overwrite this file and fail the assertions below.
            assert_eq!(fs::read(quarantine).unwrap(), b"print('replacement')\n");
            fs::write(&original, b"print('new arrival')\n").unwrap();
        }
    })
    .unwrap_err();

    assert!(error.message().localize(Locale::En).contains("restore"));
    assert_eq!(fs::read(&original).unwrap(), b"print('new arrival')\n");
    let quarantines = owned_draft_quarantines(root.path());
    assert_eq!(quarantines.len(), 1);
    assert_eq!(
        fs::read(&quarantines[0]).unwrap(),
        b"print('replacement')\n"
    );
    let visible = tui_drafts(root.path());
    assert_eq!(
        visible.len(),
        1,
        "quarantine files are never resumable rows"
    );
    assert_eq!(visible[0].path, original);
}

#[cfg(unix)]
#[test]
fn owned_draft_preconditions_and_quarantine_move_failures_preserve_every_source() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

    let missing_identity = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &missing_identity,
        "skit-new-no-identity.py",
        b"print('identity')\n",
    );
    let mut no_identity = snapshot.clone();
    no_identity.identity = None;
    let error = consume_owned_draft(missing_identity.path(), &no_identity).unwrap_err();
    let message = error.message();
    assert_eq!(
        message.localize(Locale::En),
        format!(
            "the kept draft has no filesystem identity: {}",
            snapshot.path.display()
        )
    );
    assert_eq!(
        message.localize(Locale::ZhCn),
        format!("保留的草稿没有文件系统标识：{}", snapshot.path.display())
    );
    assert_eq!(
        message.localize(Locale::ZhTw),
        format!("保留的草稿沒有檔案系統識別：{}", snapshot.path.display())
    );
    assert_eq!(fs::read(&snapshot.path).unwrap(), b"print('identity')\n");
    assert!(owned_draft_quarantines(missing_identity.path()).is_empty());

    let lexical_parent = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &lexical_parent,
        "skit-new-lexical-parent.py",
        b"print('parent')\n",
    );
    let mut forged_parent = snapshot.clone();
    forged_parent.path = lexical_parent
        .path()
        .join("drafts/../drafts/skit-new-lexical-parent.py");
    forged_parent.source_record = forged_parent.path.display().to_string();
    let error = consume_owned_draft(lexical_parent.path(), &forged_parent).unwrap_err();
    for (locale, expected) in [
        (
            Locale::En,
            "refusing to remove a file outside skit's drafts directory",
        ),
        (Locale::ZhCn, "拒绝删除 skit 草稿目录以外的文件"),
        (Locale::ZhTw, "拒絕移除 skit 草稿目錄以外的檔案"),
    ] {
        assert_eq!(error.message().localize(locale), expected);
    }
    assert_eq!(fs::read(&snapshot.path).unwrap(), b"print('parent')\n");
    assert!(owned_draft_quarantines(lexical_parent.path()).is_empty());

    let permission_claim = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &permission_claim,
        "skit-new-permission-claim.py",
        b"print('permissions')\n",
    );
    let actual_permissions = snapshot.permissions;
    let mut forged_permissions = snapshot.clone();
    forged_permissions.permissions.readonly = !forged_permissions.permissions.readonly;
    assert_eq!(
        consume_owned_draft(permission_claim.path(), &forged_permissions).unwrap(),
        DraftConsumeOutcome::Changed
    );
    assert_eq!(fs::read(&snapshot.path).unwrap(), b"print('permissions')\n");
    assert_eq!(
        source_permissions(&fs::metadata(&snapshot.path).unwrap()),
        actual_permissions
    );
    assert!(owned_draft_quarantines(permission_claim.path()).is_empty());

    let invalid_path = TempDir::new().unwrap();
    let valid = owned_draft_snapshot(&invalid_path, "skit-new-valid.py", b"print('valid')\n");
    let mut invalid = valid.clone();
    invalid.path = invalid_path
        .path()
        .join("drafts")
        .join(OsString::from_vec(b"skit-new-\0.py".to_vec()));
    invalid.source_record = invalid.path.display().to_string();
    let error = consume_owned_draft(invalid_path.path(), &invalid).unwrap_err();
    assert!(
        error
            .message()
            .localize(Locale::En)
            .contains("could not inspect"),
        "{}",
        error.message().localize(Locale::En)
    );
    assert_eq!(fs::read(&valid.path).unwrap(), b"print('valid')\n");
    assert!(owned_draft_quarantines(invalid_path.path()).is_empty());

    let missing_at_move = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &missing_at_move,
        "skit-new-missing-at-move.py",
        b"print('gone')\n",
    );
    let original = snapshot.path.clone();
    let outcome =
        consume_owned_draft_with_test_hook(missing_at_move.path(), &snapshot, |point, _| {
            if point == DraftConsumeTestPoint::BeforeQuarantine {
                fs::remove_file(&original).unwrap();
            }
        })
        .unwrap();
    assert_eq!(outcome, DraftConsumeOutcome::AlreadyMissing);
    assert!(!original.exists());
    assert!(owned_draft_quarantines(missing_at_move.path()).is_empty());

    let move_failure = TempDir::new().unwrap();
    let snapshot =
        owned_draft_snapshot(&move_failure, "skit-new-move-error.py", b"print('keep')\n");
    let original = snapshot.path.clone();
    let error =
        consume_owned_draft_with_test_hook(move_failure.path(), &snapshot, |point, quarantine| {
            if point == DraftConsumeTestPoint::BeforeQuarantine {
                fs::create_dir(quarantine).unwrap();
            }
        })
        .unwrap_err();
    assert!(
        error
            .message()
            .localize(Locale::En)
            .contains("could not move"),
        "{}",
        error.message().localize(Locale::En)
    );
    assert_eq!(fs::read(&original).unwrap(), b"print('keep')\n");
    let quarantines = owned_draft_quarantines(move_failure.path());
    assert_eq!(quarantines.len(), 1);
    assert!(quarantines[0].is_dir());
    fs::remove_dir_all(quarantines[0].parent().unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn owned_draft_restore_and_cleanup_failures_never_clobber_or_rollback() {
    use std::os::unix::fs::symlink;

    let destination_exists = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &destination_exists,
        "skit-new-occupied.py",
        b"print('claim')\n",
    );
    let staged = destination_exists.path().join("drafts/replacement.tmp");
    fs::write(&staged, b"print('replacement')\n").unwrap();
    fs::remove_file(&snapshot.path).unwrap();
    fs::rename(&staged, &snapshot.path).unwrap();
    let original = snapshot.path.clone();
    let error =
        consume_owned_draft_with_test_hook(destination_exists.path(), &snapshot, |point, _| {
            if point == DraftConsumeTestPoint::Quarantined {
                fs::write(&original, b"print('new arrival')\n").unwrap();
            }
        })
        .unwrap_err();
    assert_eq!(fs::read(&original).unwrap(), b"print('new arrival')\n");
    let quarantines = owned_draft_quarantines(destination_exists.path());
    assert_eq!(quarantines.len(), 1);
    assert_eq!(
        fs::read(&quarantines[0]).unwrap(),
        b"print('replacement')\n"
    );
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let localized = error.message().localize(locale);
        assert!(
            localized.contains(&original.display().to_string()),
            "{localized}"
        );
        assert!(
            localized.contains(&quarantines[0].display().to_string()),
            "{localized}"
        );
    }

    let inspect_failure = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &inspect_failure,
        "skit-new-inspect-restore.py",
        b"print('inspect')\n",
    );
    let drafts = inspect_failure.path().join("drafts");
    let moved = inspect_failure.path().join("drafts-moved");
    let error =
        consume_owned_draft_with_test_hook(inspect_failure.path(), &snapshot, |point, _| {
            if point == DraftConsumeTestPoint::Quarantined {
                fs::rename(&drafts, &moved).unwrap();
                symlink("drafts", &drafts).unwrap();
            }
        })
        .unwrap_err();
    assert!(error.message().localize(Locale::En).contains("restore"));
    fs::remove_file(&drafts).unwrap();
    fs::rename(&moved, &drafts).unwrap();
    let quarantines = owned_draft_quarantines(inspect_failure.path());
    assert_eq!(quarantines.len(), 1);
    assert_eq!(fs::read(&quarantines[0]).unwrap(), b"print('inspect')\n");

    let remove_file_failure = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &remove_file_failure,
        "skit-new-remove-file.py",
        b"print('remove file')\n",
    );
    let saved = remove_file_failure.path().join("saved-claim.py");
    let error = consume_owned_draft_with_test_hook(
        remove_file_failure.path(),
        &snapshot,
        |point, quarantine| {
            if point == DraftConsumeTestPoint::Verified {
                fs::rename(quarantine, &saved).unwrap();
                fs::create_dir(quarantine).unwrap();
            }
        },
    )
    .unwrap_err();
    assert!(
        error
            .message()
            .localize(Locale::En)
            .contains("could not remove")
    );
    assert_eq!(fs::read(&saved).unwrap(), b"print('remove file')\n");
    assert!(!snapshot.path.exists());
    let quarantines = owned_draft_quarantines(remove_file_failure.path());
    assert_eq!(quarantines.len(), 1);
    fs::remove_dir_all(quarantines[0].parent().unwrap()).unwrap();

    let remove_dir_failure = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(
        &remove_dir_failure,
        "skit-new-remove-dir.py",
        b"print('remove dir')\n",
    );
    let mut extra = None;
    let error = consume_owned_draft_with_test_hook(
        remove_dir_failure.path(),
        &snapshot,
        |point, quarantine| {
            if point == DraftConsumeTestPoint::Verified {
                let path = quarantine.parent().unwrap().join("still-here");
                fs::write(&path, b"cleanup evidence").unwrap();
                extra = Some(path);
            }
        },
    )
    .unwrap_err();
    assert!(
        error
            .message()
            .localize(Locale::En)
            .contains("could not remove")
    );
    assert!(!snapshot.path.exists());
    let extra = extra.unwrap();
    assert_eq!(fs::read(&extra).unwrap(), b"cleanup evidence");
    fs::remove_dir_all(extra.parent().unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn owned_draft_consume_rejects_a_replaced_drafts_directory() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let snapshot = owned_draft_snapshot(&root, "skit-new-dir-race.py", b"print('draft')\n");
    fs::rename(root.path().join("drafts"), root.path().join("drafts-old")).unwrap();
    let outside_target = outside.path().join("skit-new-dir-race.py");
    fs::write(&outside_target, b"print('outside')\n").unwrap();
    symlink(outside.path(), root.path().join("drafts")).unwrap();

    assert!(consume_owned_draft(root.path(), &snapshot).is_err());
    assert_eq!(fs::read(&outside_target).unwrap(), b"print('outside')\n");
    assert_eq!(
        fs::read(root.path().join("drafts-old/skit-new-dir-race.py")).unwrap(),
        b"print('draft')\n"
    );
}

#[test]
fn show_formatters_cover_empty_defaults_prompt_help_types_actions_and_every_drift_row() {
    let mut empty_default = ParamDecl::new("EMPTY");
    empty_default.default = Some(ParameterValue::String(String::new()));
    empty_default.prompt = "A helpful prompt".to_owned();
    let mut degraded = ParamDecl::new("DEGRADED");
    degraded.degraded = true;
    degraded.parameter_type = ParameterType::Int;
    let mut boolean = ParamDecl::new("VERBOSE");
    boolean.parameter_type = ParameterType::Bool;
    boolean.flag = "--verbose".to_owned();
    boolean.default = Some(ParameterValue::Bool(false));
    let fields = [empty_default.clone(), degraded.clone(), boolean.clone()]
        .into_iter()
        .map(|declaration| PreparedField {
            declaration,
            input_binding: false,
            empty_uses_default: false,
        })
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    write_show_fields(&mut output, &fields, Locale::En).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("EMPTY"));
    assert!(output.contains("A helpful prompt"));
    assert!(output.contains("VERBOSE"));
    assert_eq!(field_type(&degraded), "str");
    assert_eq!(field_action(&boolean), "store_true");
    assert_eq!(field_json(&fields[1])["type"], "str");
    assert_eq!(field_json(&fields[2])["action"], "store_true");

    let mut env_missing = ParamDecl::new("ENV_VALUE");
    env_missing.binding = ParameterBinding::EnvDefault;
    let plain_missing = ParamDecl::new("PLAIN");
    let mut old_type = ParamDecl::new("COUNT");
    old_type.parameter_type = ParameterType::Int;
    let mut new_type = old_type.clone();
    new_type.parameter_type = ParameterType::Float;
    let rebound = ParamDecl::new("QUESTION");
    let plan = PreparedFormPlan {
        source: skit_form::FormSource::Inject,
        fields: Vec::new(),
        drift: vec![
            FormDrift::Missing {
                declaration: env_missing,
            },
            FormDrift::Missing {
                declaration: plain_missing,
            },
            FormDrift::TypeChanged {
                stored: old_type,
                current: new_type,
            },
            FormDrift::Rebound {
                stored: rebound.clone(),
                current: rebound,
            },
            FormDrift::PromptMissing {
                names: vec!["topic".to_owned()],
            },
        ],
        degradation: None,
    };
    let lines = show_drift_lines(&plan, "Tool", Locale::En);
    assert_eq!(lines.len(), 7);
    for expected in [
        "ENV_VALUE is no longer read from the environment",
        "PLAIN: injection target no longer exists",
        "COUNT: type changed from int to float",
        "QUESTION: its prompt no longer matches",
        "No longer in the prompt",
        "skit params Tool --resync",
    ] {
        assert!(
            lines.iter().any(|line| line.contains(expected)),
            "{lines:?}"
        );
    }
    let prompt_only = PreparedFormPlan {
        drift: vec![FormDrift::PromptMissing {
            names: vec!["one".to_owned(), "two".to_owned()],
        }],
        ..PreparedFormPlan::default()
    };
    let lines = show_drift_lines(&prompt_only, "Prompt", Locale::ZhTw);
    assert_eq!(lines.len(), 1);
    assert!(show_drift_lines(&PreparedFormPlan::default(), "Clean", Locale::En).is_empty());

    let error = uv_edit_error(
        "Tool",
        UvMetadataEditError::Language(skit_language::LanguageError::UnsupportedKind {
            kind: "future".to_owned(),
        }),
    );
    assert!(error.message().localize(Locale::En).contains("future"));
}

#[test]
fn runner_host_updates_repairs_removes_and_refuses_stale_snapshots() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let config_dir = root.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    let config_path = config_dir.join("config.toml");
    fs::write(
        &config_path,
        concat!(
            "[prompt]\n",
            "runners_seeded = true\n",
            "runners = [\n",
            "  { name = \"agent\", argv = [\"agent\", \"{{prompt}}\"] },\n",
            "  { name = \"broken\", argv = [\"broken\"] },\n",
            "  \"not-a-table\",\n",
            "]\n",
        ),
    )
    .unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store);

    let rows = tui_runner_rows(&service, &config_dir).unwrap();
    let agent = rows
        .iter()
        .find(|row| row.name.as_deref() == Some("agent"))
        .expect("the named runner row must be visible");
    let action = tui_save_runner(
        &service,
        &config_dir,
        RunnerSaveRequest {
            name: "agent".to_owned(),
            argv: vec![
                "agent".to_owned(),
                "--updated".to_owned(),
                "{{prompt}}".to_owned(),
            ],
            target: RunnerSaveTarget::Named {
                name: "agent".to_owned(),
                expected: agent.key_identities.clone(),
            },
        },
        RunnerSaveOwner::Manager,
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Runners(RunnerManagerAction::MutationSucceeded {
            selected_name: Some(ref name),
            ref message,
            ..
        }) if name == "agent" && message.contains("updated")
    ));
    assert_eq!(
        FileConfigStore::new(&config_dir).runners().unwrap()[0].argv,
        ["agent", "--updated", "{{prompt}}"]
    );

    let broken = tui_runner_rows(&service, &config_dir)
        .unwrap()
        .into_iter()
        .find(|row| row.name.as_deref() == Some("broken"))
        .expect("the malformed named row must be visible");
    let action = tui_save_runner(
        &service,
        &config_dir,
        RunnerSaveRequest {
            name: "repaired".to_owned(),
            argv: vec!["repaired".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::RawRow {
                expected: broken.identity,
            },
        },
        RunnerSaveOwner::Editor(skit_ui::RunnerEditorOwner::Add),
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::RunnerEditorSaved {
            owner: skit_ui::RunnerEditorOwner::Add,
            ref name,
            ..
        } if name == "repaired"
    ));
    assert!(
        FileConfigStore::new(&config_dir)
            .runners()
            .unwrap()
            .iter()
            .any(|runner| runner.name == "repaired")
    );

    let raw_row = tui_runner_rows(&service, &config_dir)
        .unwrap()
        .into_iter()
        .find(|row| row.reason.as_deref() == Some("row-not-table"))
        .expect("the malformed scalar row must remain visible");
    let action = tui_remove_runner(
        &service,
        &config_dir,
        RunnerRemoveRequest::RawRow {
            expected: raw_row.identity,
        },
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Runners(RunnerManagerAction::MutationSucceeded {
            ref message,
            ..
        }) if message.contains("row 2")
    ));

    let stale = RunnerRowIdentity {
        index: Some(1),
        snapshot_token: "stale".to_owned(),
    };
    let before = fs::read(&config_path).unwrap();
    let action = tui_save_runner(
        &service,
        &config_dir,
        RunnerSaveRequest {
            name: "never-written".to_owned(),
            argv: vec!["never-written".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::RawRow { expected: stale },
        },
        RunnerSaveOwner::Editor(skit_ui::RunnerEditorOwner::Add),
    )
    .unwrap();
    assert!(matches!(action, UiAction::RunnerEditorSaveFailed { .. }));
    assert_eq!(fs::read(&config_path).unwrap(), before);

    let agent = tui_runner_rows(&service, &config_dir)
        .unwrap()
        .into_iter()
        .find(|row| row.name.as_deref() == Some("agent"))
        .expect("updated named runner remains visible");
    let action = tui_remove_runner(
        &service,
        &config_dir,
        RunnerRemoveRequest::Named {
            name: "agent".to_owned(),
            expected: agent.key_identities,
            expected_pinned_count: 0,
        },
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Runners(RunnerManagerAction::MutationSucceeded {
            selected_name: None,
            ref message,
            ..
        }) if message.contains("agent")
    ));
    assert!(
        FileConfigStore::new(&config_dir)
            .runners()
            .unwrap()
            .iter()
            .all(|runner| runner.name != "agent")
    );

    let stale_remove = RunnerRemoveRequest::Named {
        name: "repaired".to_owned(),
        expected: vec![RunnerRowIdentity {
            index: Some(0),
            snapshot_token: "stale".to_owned(),
        }],
        expected_pinned_count: 0,
    };
    let before = fs::read(&config_path).unwrap();
    assert!(matches!(
        tui_remove_runner(&service, &config_dir, stale_remove).unwrap(),
        UiAction::Runners(RunnerManagerAction::MutationFailed(_))
    ));
    assert_eq!(fs::read(&config_path).unwrap(), before);

    fs::write(&config_path, "language = \"zh-TW\"\nprompt = \"garbage\"\n").unwrap();
    let raw = tui_runner_rows(&service, &config_dir)
        .unwrap()
        .pop()
        .expect("the malformed prompt container is one raw row");
    assert_eq!(raw.identity.index, None);
    let action = tui_remove_runner(
        &service,
        &config_dir,
        RunnerRemoveRequest::RawRow {
            expected: raw.identity,
        },
    )
    .unwrap();
    assert!(matches!(
        action,
        UiAction::Runners(RunnerManagerAction::MutationSucceeded {
            ref message,
            ..
        }) if message.contains("container")
    ));
    let document = fs::read_to_string(&config_path).unwrap();
    assert!(document.contains("language = \"zh-TW\""));
    assert!(document.contains("runners = []"));

    let raw_stale = RunnerRemoveRequest::RawRow {
        expected: RunnerRowIdentity {
            index: None,
            snapshot_token: "stale".to_owned(),
        },
    };
    let before = fs::read(&config_path).unwrap();
    assert!(matches!(
        tui_remove_runner(&service, &config_dir, raw_stale).unwrap(),
        UiAction::Runners(RunnerManagerAction::MutationFailed(_))
    ));
    assert_eq!(fs::read(&config_path).unwrap(), before);
}

#[test]
fn runner_closed_store_outcomes_project_to_typed_actions_in_every_locale() {
    let root = TempDir::new().unwrap();
    let invalid = root.path().join("config-file");
    fs::write(&invalid, b"keep").unwrap();
    let config_error = || {
        FileConfigStore::new(&invalid)
            .set("editor", "nano")
            .unwrap_err()
    };
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        assert!(project_runner_save_result(Ok(true), RunnerSaveOwner::Manager, locale).is_none());
        for result in [Ok(false), Err(config_error())] {
            assert!(matches!(
                project_runner_save_result(result, RunnerSaveOwner::Manager, locale),
                Some(UiAction::Runners(RunnerManagerAction::MutationFailed(_)))
            ));
        }
        for result in [
            Ok(RunnerRemovalCas::RowsChanged),
            Ok(RunnerRemovalCas::PinsChanged { actual: 2 }),
            Err(RunnerManagementStoreError::Library(
                RepositoryError::NotFound {
                    query: "runner".to_owned(),
                },
            )),
            Err(RunnerManagementStoreError::Config(config_error())),
        ] {
            assert!(matches!(
                project_named_runner_removal(result, locale),
                Some(UiAction::Runners(RunnerManagerAction::MutationFailed(_)))
            ));
        }
        assert!(project_named_runner_removal(Ok(RunnerRemovalCas::Removed), locale).is_none());
        assert!(project_raw_runner_removal(Ok(true), locale).is_none());
        for result in [Ok(false), Err(config_error())] {
            assert!(matches!(
                project_raw_runner_removal(result, locale),
                Some(UiAction::Runners(RunnerManagerAction::MutationFailed(_)))
            ));
        }
    }
    assert_eq!(fs::read(&invalid).unwrap(), b"keep");
}

#[test]
fn settings_host_updates_prompt_javascript_reference_python_and_source_management_axes() {
    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());

    let prompt = service
        .add(CreateEntry {
            name: "Prompt settings".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"Review {{topic}}.".to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let mut values = SubmittedValues::new();
    set(&mut values, "runner", "  codex  ");
    values.insert("interpolate".to_owned(), FieldValue::boolean(false));
    values.insert(
        "parameter:add".to_owned(),
        FieldValue::Explicit(TypedValue::Choices(vec!["topic".to_owned()])),
    );
    tui_submit_settings(&service, &store, &state_dir, prompt.slug.as_str(), &values).unwrap();
    let prompt = service.show(prompt.slug.as_str()).unwrap();
    let prompt_settings = EntrySettings::from_meta(&prompt.meta);
    assert_eq!(prompt_settings.runner, "codex");
    assert!(!prompt_settings.interpolate);
    assert_eq!(prompt_settings.params, ["topic"]);
    assert!(
        prompt_settings
            .parameters
            .iter()
            .any(|parameter| parameter.name == "topic"
                && parameter.delivery == ParameterDelivery::Placeholder)
    );

    let javascript = service
        .add(CreateEntry {
            name: "JavaScript settings".to_owned(),
            kind: EntryKind::parse("js").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"console.log('ok');\n".to_vec(),
                stored_name: Some("script.js".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let mut values = SubmittedValues::new();
    values.insert(
        "dependencies".to_owned(),
        FieldValue::Explicit(TypedValue::Choices(vec![
            "@scope/pkg@1".to_owned(),
            "zod@4".to_owned(),
        ])),
    );
    set(&mut values, "python", "");
    tui_submit_settings(
        &service,
        &store,
        &state_dir,
        javascript.slug.as_str(),
        &values,
    )
    .unwrap();
    let javascript = service.show(javascript.slug.as_str()).unwrap();
    let javascript_settings = EntrySettings::from_meta(&javascript.meta);
    assert_eq!(javascript_settings.dependencies, ["@scope/pkg@1", "zod@4"]);
    assert_eq!(javascript_settings.requires_python, "");

    let referenced_source = root.path().join("referenced.py");
    fs::write(&referenced_source, b"print('reference')\n").unwrap();
    let referenced = service
        .add(CreateEntry {
            name: "Referenced Python settings".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Reference,
            source: referenced_source.display().to_string(),
            workdir: "origin".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    let source_before = fs::read(&referenced_source).unwrap();
    let mut values = SubmittedValues::new();
    values.insert(
        "dependencies".to_owned(),
        FieldValue::Explicit(TypedValue::Choices(vec!["requests>=2".to_owned()])),
    );
    set(&mut values, "python", ">=3.11");
    tui_submit_settings(
        &service,
        &store,
        &state_dir,
        referenced.slug.as_str(),
        &values,
    )
    .unwrap();
    let referenced = service.show(referenced.slug.as_str()).unwrap();
    let referenced_settings = EntrySettings::from_meta(&referenced.meta);
    assert_eq!(referenced_settings.dependencies, ["requests>=2"]);
    assert_eq!(referenced_settings.requires_python, ">=3.11");
    assert_eq!(fs::read(&referenced_source).unwrap(), source_before);

    let shell = service
        .add(CreateEntry {
            name: "Shell settings".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"NAME=world\necho \"$NAME\"\n".to_vec(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let mut values = SubmittedValues::new();
    values.insert(
        "source:normalize".to_owned(),
        FieldValue::Explicit(TypedValue::Choices(vec!["NAME".to_owned()])),
    );
    tui_submit_settings(&service, &store, &state_dir, shell.slug.as_str(), &values).unwrap();
    let shell = service.show(shell.slug.as_str()).unwrap();
    let shell_source = fs::read(source_path(&store, &shell).unwrap()).unwrap();
    assert!(
        String::from_utf8(shell_source)
            .unwrap()
            .contains("${NAME:-world}")
    );

    let managed_source = |default: &str| {
        let mut managed = ParamDecl::new("NAME");
        managed.binding = ParameterBinding::Const;
        managed.delivery = ParameterDelivery::Inject;
        managed.default = Some(ParameterValue::String(default.to_owned()));
        (
            managed.clone(),
            write_managed_params("shell", "NAME=world\necho \"$NAME\"\n", &[managed]).unwrap(),
        )
    };
    let (mut collision, collision_source) = managed_source("world");
    collision.binding = ParameterBinding::None;
    let refused = service
        .add(CreateEntry {
            name: "Refused source binding".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: collision_source.into_bytes(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                parameters: vec![collision],
                ..EntrySettings::default()
            },
        })
        .unwrap();
    let refused_payload = source_path(&store, &refused).unwrap();
    let refused_meta = refused_payload.parent().unwrap().join("meta.toml");
    let refused_payload_before = fs::read(&refused_payload).unwrap();
    let refused_meta_before = fs::read(&refused_meta).unwrap();
    let mut values = SubmittedValues::new();
    set(&mut values, "source:resync", "true");
    set(&mut values, "parameter:NAME:help", "refused");
    assert!(
        tui_submit_settings(&service, &store, &state_dir, refused.slug.as_str(), &values,).is_err()
    );
    assert_eq!(fs::read(&refused_payload).unwrap(), refused_payload_before);
    assert_eq!(fs::read(&refused_meta).unwrap(), refused_meta_before);

    let (managed, updated_source) = managed_source("world");
    let updated = service
        .add(CreateEntry {
            name: "Updated source binding".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: updated_source.into_bytes(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                parameters: vec![managed],
                ..EntrySettings::default()
            },
        })
        .unwrap();
    let updated_payload = source_path(&store, &updated).unwrap();
    let before = fs::read(&updated_payload).unwrap();
    let mut values = SubmittedValues::new();
    set(&mut values, "source:resync", "true");
    set(&mut values, "parameter:NAME:default", "updated");
    tui_submit_settings(&service, &store, &state_dir, updated.slug.as_str(), &values).unwrap();
    let after = fs::read(&updated_payload).unwrap();
    assert_ne!(after, before);
    assert!(String::from_utf8(after).unwrap().contains("updated"));
}

#[test]
fn settings_inputs_read_prompt_runners_and_keep_future_kinds_empty_without_hidden_writes() {
    let prompt_root = TempDir::new().unwrap();
    let prompt_data = prompt_root.path().join("data");
    let prompt_state = prompt_root.path().join("state");
    let prompt_config = prompt_root.path().join("config");
    fs::create_dir_all(&prompt_state).unwrap();
    let prompt_store = FileStore::new(&prompt_data);
    let prompt_service = LibraryService::new(prompt_store.clone());
    let prompt = prompt_service
        .add(CreateEntry {
            name: "Runner projection".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: "Choose a configured runner".to_owned(),
            payload: Some(EntryPayload {
                bytes: b"Review {{topic}}.".to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                runner: "codex".to_owned(),
                params: vec!["topic".to_owned()],
                ..EntrySettings::default()
            },
        })
        .unwrap();
    let prompt_data_before = test_tree_snapshot(&prompt_data);
    let prompt_state_before = test_tree_snapshot(&prompt_state);

    let inputs = settings_inputs(
        &prompt_service,
        &prompt_store,
        &prompt_config,
        &prompt_state,
        &prompt,
        Some(SettingsSectionId::Runner),
    )
    .unwrap();

    assert_eq!(inputs.kind, "prompt");
    assert_eq!(inputs.runner, "codex");
    assert_eq!(inputs.managed.len(), 1);
    assert_eq!(inputs.managed[0].name, "topic");
    assert_eq!(inputs.revealed, Some(SettingsSectionId::Runner));
    assert_eq!(
        inputs.configured_runners,
        [
            "claude",
            "codex",
            "opencode",
            "amp",
            "antigravity",
            "copilot",
            "cursor",
            "pi",
        ]
        .map(str::to_owned)
    );
    let config_bytes = fs::read(prompt_config.join("config.toml")).unwrap();
    let config_text = String::from_utf8(config_bytes).unwrap();
    assert!(config_text.contains("runners_seeded = true"));
    assert!(config_text.contains("name = \"codex\""));
    assert_eq!(test_tree_snapshot(&prompt_data), prompt_data_before);
    assert_eq!(test_tree_snapshot(&prompt_state), prompt_state_before);

    let future_root = TempDir::new().unwrap();
    let future_data = future_root.path().join("data");
    let future_state = future_root.path().join("state");
    let future_config = future_root.path().join("config");
    fs::create_dir_all(&future_state).unwrap();
    let future_store = FileStore::new(&future_data);
    let future_service = LibraryService::new(future_store.clone());
    let future = future_service
        .add(CreateEntry {
            name: "Future settings".to_owned(),
            kind: EntryKind::parse("future-kind").unwrap(),
            mode: StorageMode::Reference,
            source: "opaque future source".to_owned(),
            workdir: "invoke".to_owned(),
            description: "Keep unknown metadata".to_owned(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    let future_before = test_tree_snapshot(future_root.path());

    let inputs = settings_inputs(
        &future_service,
        &future_store,
        &future_config,
        &future_state,
        &future,
        None,
    )
    .unwrap();

    assert_eq!(inputs.kind, "future-kind");
    assert!(inputs.reference_mode);
    assert!(!inputs.has_analyzer);
    assert!(!inputs.declared_schema);
    assert_eq!(inputs.reader_fields, 0);
    assert!(inputs.managed.is_empty());
    assert!(inputs.candidates.is_empty());
    assert!(inputs.configured_runners.is_empty());
    assert_eq!(inputs.dependency_flavor, None);
    assert_eq!(test_tree_snapshot(future_root.path()), future_before);
}

#[cfg(unix)]
#[test]
fn add_host_deletes_edits_keeps_and_degrades_completion_without_pty_input() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let data_dir = root.path().join("data");
    let state_dir = root.path().join("state");
    let config_dir = root.path().join("config");
    let drafts = data_dir.join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let store = FileStore::new(&data_dir);
    let service = LibraryService::new(store.clone());

    let removed_path = drafts.join("skit-new-removed.py");
    fs::write(&removed_path, b"print('remove')\n").unwrap();
    let removed = tui_drafts(&data_dir).pop().unwrap();
    let mut remove_workflow = AddWorkflowState::new(vec![removed]);
    let _ = remove_workflow.reduce(AddAction::SelectDraft(0));
    let _ = remove_workflow.reduce(AddAction::DeleteSelectedDraft);
    let effects = remove_workflow.reduce(AddAction::ConfirmDraftDelete(true));
    let action = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap();
    assert!(matches!(
        action,
        UiAction::Add(AddAction::DraftDeleted {
            result: Ok(DraftDeleteOutcome::Removed),
            ..
        })
    ));
    assert!(!removed_path.exists());

    let missing_path = drafts.join("skit-new-missing.py");
    fs::write(&missing_path, b"print('missing')\n").unwrap();
    let missing = tui_drafts(&data_dir).pop().unwrap();
    let mut missing_workflow = AddWorkflowState::new(vec![missing]);
    let _ = missing_workflow.reduce(AddAction::SelectDraft(0));
    let _ = missing_workflow.reduce(AddAction::DeleteSelectedDraft);
    let effects = missing_workflow.reduce(AddAction::ConfirmDraftDelete(true));
    fs::remove_file(&missing_path).unwrap();
    let action = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap();
    assert!(matches!(
        action,
        UiAction::Add(AddAction::DraftDeleted {
            result: Ok(DraftDeleteOutcome::AlreadyMissing),
            ..
        })
    ));

    let source = root.path().join("editable.py");
    fs::write(&source, b"print('before')\n").unwrap();
    let editor = root.path().join("editor.sh");
    fs::write(
        &editor,
        "#!/bin/sh\nprintf \"print('after')\\n\" > \"$1\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&editor, permissions).unwrap();
    FileConfigStore::new(&config_dir)
        .set("editor", editor.to_str().unwrap())
        .unwrap();
    let snapshot = tui_add_source(&data_dir, &source).unwrap();
    let review = skit_ui::ReviewState::from_source(
        snapshot,
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    let mut edit_workflow = AddWorkflowState::from_review(review);
    let effects = edit_workflow.reduce(AddAction::EditSource);
    let action = tui_add_effect(&service, &store, &state_dir, &config_dir, effects).unwrap();
    assert!(matches!(
        action,
        UiAction::Add(AddAction::SourceEdited {
            result: Ok(ref snapshot),
            ..
        }) if snapshot.bytes == b"print('after')\n"
    ));
    assert_eq!(fs::read(&source).unwrap(), b"print('after')\n");

    assert_eq!(
        tui_add_effect(
            &service,
            &store,
            &state_dir,
            &config_dir,
            vec![AddEffect::DraftKept(source)],
        )
        .unwrap(),
        UiAction::ClearStatus
    );
    assert_eq!(
        tui_add_effect(&service, &store, &state_dir, &config_dir, Vec::new()).unwrap(),
        UiAction::ClearStatus
    );

    let invalid = tui_add_effect(
        &service,
        &store,
        &state_dir,
        &config_dir,
        vec![AddEffect::Complete("not a slug!".to_owned())],
    )
    .unwrap();
    assert!(matches!(
        invalid,
        UiAction::Complete {
            surface: None,
            rerunnable: None,
            ref message,
        } if message == "Entry added"
    ));

    let created = service
        .add(CreateEntry {
            name: "Completed without surface".to_owned(),
            kind: EntryKind::parse("prompt").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"Review this".to_vec(),
                stored_name: Some("prompt.md".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let scripts_dir = data_dir.join("scripts");
    let original_permissions = fs::metadata(&scripts_dir).unwrap().permissions();
    let mut unreadable_permissions = original_permissions.clone();
    unreadable_permissions.set_mode(0o000);
    fs::set_permissions(&scripts_dir, unreadable_permissions).unwrap();
    let degraded = tui_add_effect(
        &service,
        &store,
        &state_dir,
        &config_dir,
        vec![AddEffect::Complete(created.slug.as_str().to_owned())],
    )
    .unwrap();
    fs::set_permissions(&scripts_dir, original_permissions).unwrap();
    assert!(
        matches!(
            degraded,
            UiAction::Complete {
                surface: None,
                rerunnable: None,
                ref message,
            } if message.starts_with("Entry added\nwarning: ")
        ),
        "{degraded:?}"
    );
    assert_eq!(
        service.show(created.slug.as_str()).unwrap().slug,
        created.slug
    );
}
