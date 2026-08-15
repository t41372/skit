use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{SettingsScreenSession, render_localized, render_settings};
use skit_ui::{
    Action, Effect, HostRequest, LibraryEntryDetail, LibraryPromptRunner, LibraryState,
    SettingsInputs, SettingsView,
};

fn prompt_entry() -> EntrySummary {
    EntrySummary {
        slug: Slug::parse("p").unwrap(),
        name: "p".to_owned(),
        kind: EntryKind::parse("prompt").unwrap(),
        mode: StorageMode::Copy,
        description: String::new(),
        target: Some("/work/p.prompt.md".to_owned()),
    }
}

fn state(runner: Option<LibraryPromptRunner>) -> LibraryState {
    let entry = prompt_entry();
    LibraryState::from_surface(
        LibraryScan {
            entries: vec![entry.clone()],
            diagnostics: Vec::new(),
        },
        BTreeMap::from([(
            entry.slug,
            LibraryEntryDetail {
                prompt_runner: runner,
                ..LibraryEntryDetail::default()
            },
        )]),
    )
}

fn text(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_library(state: &LibraryState, locale: Locale) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_localized(frame, state, locale);
        })
        .unwrap();
    text(terminal.backend().buffer())
}

fn render_prompt_settings(locale: Locale) -> String {
    let view = SettingsView::from_inputs(&SettingsInputs {
        selector: "p".to_owned(),
        kind: "prompt".to_owned(),
        name: "p".to_owned(),
        source: "/work/p.prompt.md".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        declared_schema: true,
        interpolate: true,
        ..SettingsInputs::default()
    });
    let mut session = SettingsScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            let _ = render_settings(frame, area, &view, &mut session, locale);
        })
        .unwrap();
    text(terminal.backend().buffer())
}

#[test]
fn test_prompt_only_library_uses_entry_taxonomy_everywhere() {
    let mut state = state(Some(LibraryPromptRunner::PickOnRunForm));
    let rendered = render_library(&state, Locale::En);
    for required in [
        "Library",
        "Entry settings",
        "Edit source",
        "Add entry",
        "add one in Entry settings",
    ] {
        assert!(rendered.contains(required), "entry-neutral Library contract lost {required:?}:\n{rendered}");
    }
    assert!(!rendered.contains("Script settings"), "prompt-only Library fell back to script taxonomy:\n{rendered}");

    assert_eq!(
        state.update(Action::OpenSettings),
        Effect::Open {
            request: HostRequest::Settings,
            selector: Some("p".to_owned()),
        },
        "the visible Entry settings command did not request settings for the selected prompt"
    );
    let settings = render_prompt_settings(Locale::En);
    assert!(settings.contains("Entry settings"), "settings surface lost its entry-neutral title:\n{settings}");
}

#[test]
fn test_prompt_only_chinese_library_stays_entry_neutral() {
    for (locale, library, script_library) in [
        (Locale::ZhCn, "工具库", "脚本库"),
        (Locale::ZhTw, "工具庫", "腳本庫"),
    ] {
        let rendered = render_library(&state(Some(LibraryPromptRunner::PickOnRunForm)), locale);
        assert!(rendered.contains(library), "localized Library title missing for {locale:?}:\n{rendered}");
        assert!(!rendered.contains(script_library), "prompt-only Library regressed to script-specific taxonomy for {locale:?}:\n{rendered}");

        let settings = render_prompt_settings(locale);
        assert!(settings.contains(library), "localized settings copy stopped naming the Library for {locale:?}:\n{settings}");
        assert!(!settings.contains(script_library), "localized settings copy reintroduced script-library wording for {locale:?}:\n{settings}");
    }
}

#[test]
fn test_detail_pane_names_the_runner() {
    let rendered = render_library(
        &state(Some(LibraryPromptRunner::Configured("claude".to_owned()))),
        Locale::En,
    );
    assert!(rendered.contains("Runs with claude"), "configured runner missing from detail pane:\n{rendered}");
    assert!(!rendered.contains("Runner picked on the run form"), "configured runner was presented as unpinned:\n{rendered}");
}

#[test]
fn test_detail_pane_unpinned_prompt_says_the_form_asks() {
    let rendered = render_library(
        &state(Some(LibraryPromptRunner::PickOnRunForm)),
        Locale::En,
    );
    assert!(rendered.contains("Runner picked on the run form"), "unpinned prompt detail hid the per-run choice:\n{rendered}");
}

#[test]
fn test_detail_pane_stale_pin_says_no_longer_configured() {
    let rendered = render_library(
        &state(Some(LibraryPromptRunner::Missing("nonesuch-agent".to_owned()))),
        Locale::En,
    );
    assert!(rendered.contains("nonesuch-agent"), "stale runner identity disappeared:\n{rendered}");
    assert!(rendered.contains("no longer configured"), "stale runner was presented as launchable:\n{rendered}");
    assert!(!rendered.contains("Runs with nonesuch-agent"), "stale pin was dishonestly presented as configured:\n{rendered}");
}
