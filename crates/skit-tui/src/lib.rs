//! Ratatui frontend adapter for skit.

#![forbid(unsafe_code)]

mod footer;
mod layout;
mod pointer;
mod rowclip;
mod screens;
mod session;
mod terminal;
mod theme;
mod viewport;

use ratatui_core::{layout::Rect, terminal::Frame};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use skit_i18n::{Locale, format_text};
use skit_ui::{
    Action, CommandContext, FormField, FormView, InputMode, LibraryState, ModalState, Screen,
    UiBinding, UiCommand, UiKey, UiModifiers, command_specs,
};

use layout::{RootLayoutPlan, ViewportProfile};
use pointer::contains;
pub use screens::add::{
    AddControlId, AddHitRegion, AddScreenEvent, AddScreenGeometry, AddScreenSession, AddTextField,
    render_add,
};
pub use screens::picker::{
    ChoicePickerGeometry, ChoicePickerHit, ChoicePickerHitRegion, FilePickerEvent,
    FilePickerGeometry, FilePickerHit, FilePickerHitRegion, FilePickerSession,
    PromptCandidatePickerEvent, PromptCandidatePickerSession, render_file_picker,
    render_prompt_candidate_picker,
};
pub use screens::settings::{
    SettingsControlId, SettingsHitRegion, SettingsScreenEvent, SettingsScreenGeometry,
    SettingsScreenSession, render_settings,
};
use session::HeaderKind;
pub use session::{EventHandling, TuiSession};
pub use terminal::{
    TuiError, collect_form, collect_run_form, collect_run_form_with_path_completion, run,
    run_add_workflow, run_preflighted, run_preflighted_with_path_completion,
    run_with_path_completion,
};

/// One clickable target produced by a view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitTarget {
    /// A command from the shared command registry.
    Command(UiCommand),
    /// A command whose action must retain the launch-field identity.
    RunFieldCommand {
        /// Field index in the active typed launch form.
        field: usize,
        /// Field-local command rendered by the chip.
        command: RunFieldCommand,
    },
    /// Focus one form row.
    FocusField(usize),
}

/// One command that retains the active launch-field identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunFieldCommand {
    /// Open the filesystem picker for the field.
    BrowsePath,
    /// Open the insertion menu for the field.
    InsertValue,
    /// Restore the field's declared default.
    ResetDefault,
}

/// One clickable rectangular target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HitRegion {
    /// Terminal cells occupied by the target.
    pub rect: Rect,
    /// Frontend-neutral intent represented by the target.
    pub action: HitTarget,
}

/// Geometry emitted by rendering and consumed by mouse-event mapping.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ViewGeometry {
    /// Visible list-row cells, excluding borders.
    pub rows: Rect,
    /// Filtered-entry index represented by the first rendered row.
    pub first_visible: usize,
    /// Clickable footer chips.
    pub hits: Vec<HitRegion>,
    /// Whether the Library detail pane was visible in this rendered frame.
    pub detail_pane_visible: bool,
}

/// Draw the library browser and return its mouse hit map.
#[must_use]
pub fn render(frame: &mut Frame, state: &LibraryState) -> ViewGeometry {
    render_localized(frame, state, Locale::En)
}

/// Draw the library browser with one explicit presentation locale.
#[must_use]
pub fn render_localized(frame: &mut Frame, state: &LibraryState, locale: Locale) -> ViewGeometry {
    let mut session = TuiSession::default();
    render_with_session(frame, state, locale, &mut session)
}

/// Draw with persistent widget cursors, focus, dropdown, and scroll state.
#[must_use]
pub fn render_with_session(
    frame: &mut Frame,
    state: &LibraryState,
    locale: Locale,
    session: &mut TuiSession,
) -> ViewGeometry {
    session.begin_render(state);
    let profile = ViewportProfile::new(frame.area());
    let header = header_kind(state);
    let minimum_body_height = minimum_body_height(state);
    let body_floor =
        monotonic_body_floor(frame.area(), state, locale, profile, minimum_body_height);
    let plan = root_layout_plan(frame.area(), state, locale, body_floor);
    let areas = plan.areas;

    if let Some(kind) = header {
        session.render_header(frame, areas.header, kind, locale);
    }
    let mut geometry = match state.modal() {
        Some(ModalState::Help) => session.render_help(frame, areas.body, locale),
        Some(ModalState::ConfirmRemove {
            name,
            original_file_preserved,
            ..
        }) => session.render_confirm_remove(frame, name, *original_file_preserved, locale),
        Some(ModalState::ConfirmDiscardChanges) => {
            screens::modal::discard_changes(frame, areas.body, locale)
        }
        Some(
            modal @ (ModalState::RunPresetName { .. }
            | ModalState::RunTokenMenu { .. }
            | ModalState::RunEnvironmentPicker { .. }
            | ModalState::RunFilePicker { .. }),
        ) => session.render_run_modal(frame, areas.body, modal, locale),
        Some(ModalState::RunnerEditor { view, .. }) => {
            let geometry = render_screen(frame, areas.body, state, locale, session);
            session.render_runner_editor(frame, areas.body, view, locale);
            geometry
        }
        None => render_screen(frame, areas.body, state, locale, session),
    };
    geometry.hits.extend(session.render_footer(
        frame,
        areas.footer,
        state,
        locale,
        plan.footer_decorated,
    ));
    session.register_geometry(&geometry);
    session.render_quit_toast(frame, locale);
    geometry
}

fn header_kind(state: &LibraryState) -> Option<HeaderKind<'_>> {
    if state.modal().is_some() {
        return None;
    }
    match state.screen() {
        Screen::Library => Some(HeaderKind::Library {
            query: state.query(),
            search: state.input_mode() == InputMode::Search,
        }),
        Screen::Report(report) => Some(HeaderKind::Report(&report.title)),
        Screen::Preferences(_)
        | Screen::Add(_)
        | Screen::Health(_)
        | Screen::Runners(_)
        | Screen::Run(_)
        | Screen::Form(_)
        | Screen::Settings(_) => None,
    }
}

fn preferred_header_height(header: Option<&HeaderKind<'_>>, profile: ViewportProfile) -> u16 {
    match header {
        Some(HeaderKind::Library { .. }) if profile.height() <= 16 => 1,
        Some(_) => 3,
        None => 0,
    }
}

fn minimum_body_height(state: &LibraryState) -> u16 {
    if state.modal().is_some() {
        return 3;
    }
    match state.screen() {
        // The Library panel needs its border, column headings, and up to three primary rows.
        Screen::Library => u16::try_from(state.visible_entry_count())
            .unwrap_or(u16::MAX)
            .clamp(1, 3)
            .saturating_add(3),
        // Keep the first three-row Preferences control inside its titled panel.
        Screen::Preferences(_) => 5,
        Screen::Run(_)
        | Screen::Form(_)
        | Screen::Settings(_)
        | Screen::Add(_)
        | Screen::Health(_)
        | Screen::Runners(_)
        | Screen::Report(_) => 3,
    }
}

fn root_layout_plan(
    area: Rect,
    state: &LibraryState,
    locale: Locale,
    body_floor: u16,
) -> RootLayoutPlan {
    let profile = ViewportProfile::new(area);
    let header = header_kind(state);
    let preferred_header = preferred_header_height(header.as_ref(), profile);
    let preferred_footer = footer::required_height(profile, state, locale);
    let minimum_footer = footer::minimum_height(profile, state, locale);
    let chrome_capacity = area.height.saturating_sub(body_floor.min(area.height));
    let header_height = allocated_header_height(
        preferred_header,
        chrome_capacity,
        minimum_footer.min(chrome_capacity),
    );
    RootLayoutPlan::new(
        area,
        header_height,
        preferred_footer,
        body_floor,
        footer::decorated_minimum_height(profile, state, locale),
    )
}

const fn allocated_header_height(preferred: u16, chrome_capacity: u16, minimum_footer: u16) -> u16 {
    if preferred == 0 {
        return 0;
    }
    if preferred <= 1 {
        if chrome_capacity >= preferred.saturating_add(minimum_footer) {
            return preferred;
        }
        return 0;
    }
    let preferred_budget = preferred.saturating_add(minimum_footer);
    if chrome_capacity >= preferred_budget {
        preferred
    } else if chrome_capacity >= 1_u16.saturating_add(minimum_footer) {
        1
    } else {
        0
    }
}

fn monotonic_body_floor(
    area: Rect,
    state: &LibraryState,
    locale: Locale,
    profile: ViewportProfile,
    minimum: u16,
) -> u16 {
    let Some(previous_height) = profile.previous_tier_max_height() else {
        return minimum;
    };
    let previous_area = Rect::new(area.x, area.y, area.width, previous_height);
    let previous_plan = root_layout_plan(previous_area, state, locale, minimum);
    minimum.max(previous_plan.areas.body.height)
}

fn render_screen(
    frame: &mut Frame,
    area: Rect,
    state: &LibraryState,
    locale: Locale,
    session: &mut TuiSession,
) -> ViewGeometry {
    match state.screen() {
        Screen::Library => session.render_library(frame, area, state, locale),
        Screen::Run(form) => session.render_run(frame, area, form, locale),
        Screen::Preferences(view) => session.render_preferences(frame, area, view, locale),
        Screen::Settings(view) => session.render_settings(frame, area, view, locale),
        Screen::Add(view) => session.render_add(frame, area, view, locale),
        Screen::Health(view) => session.render_health(frame, area, view, locale),
        Screen::Runners(view) => session.render_runners(frame, area, view, locale),
        Screen::Form(form) => session.render_form(frame, area, form, locale),
        Screen::Report(report) => session.render_report(frame, area, report, locale),
    }
}

pub(crate) fn field_label(locale: Locale, field: &FormField) -> String {
    if !field.translate_label {
        return field.label.clone();
    }
    let arguments = field
        .label_arguments
        .iter()
        .map(|value| value as &dyn std::fmt::Display)
        .collect::<Vec<_>>();
    format_text(locale, &field.label, &arguments)
}

pub(crate) fn form_title(locale: Locale, form: &FormView) -> String {
    if !form.translate_title {
        return form.title.clone();
    }
    let arguments = form
        .title_arguments
        .iter()
        .map(|value| value as &dyn std::fmt::Display)
        .collect::<Vec<_>>();
    format_text(locale, &form.title, &arguments)
}

/// Translate Crossterm input into frontend-neutral actions.
#[must_use]
pub fn map_event(event: Event, state: &LibraryState, geometry: &ViewGeometry) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => map_key(key, state, geometry),
        Event::Mouse(mouse) => map_mouse(mouse, state, geometry),
        Event::FocusGained
        | Event::FocusLost
        | Event::Key(_)
        | Event::Paste(_)
        | Event::Resize(_, _) => None,
    }
}

fn map_key(key: KeyEvent, state: &LibraryState, geometry: &ViewGeometry) -> Option<Action> {
    let context = state.command_context();
    if context == CommandContext::Form
        && key.code == KeyCode::Enter
        && state.form().is_some_and(|form| {
            form.fields
                .get(form.focused)
                .is_some_and(|field| field.multiline)
        })
    {
        return None;
    }
    let chord = ui_binding(key)?;
    if let Some(spec) = command_specs(context).find(|spec| {
        spec.bindings
            .iter()
            .any(|binding| binding.accepts(chord.key, chord.modifiers))
    }) {
        // The settings body owns this contextual chord beside its inline picker item. It has no
        // reducer action because another frontend hosts the typed item with its own picker.
        if spec.command == UiCommand::ChooseSettingsVariables {
            return None;
        }
        if matches!(
            (context, spec.command),
            (
                CommandContext::LibrarySearch | CommandContext::Form | CommandContext::RunForm,
                UiCommand::Backspace
            ) | (CommandContext::LibrarySearch, UiCommand::ClearSearch)
        ) {
            return None;
        }
        return Some(command_action(spec.command, context, geometry));
    }
    None
}

fn ui_binding(key: KeyEvent) -> Option<UiBinding> {
    let key_code = match key.code {
        KeyCode::Char(character) => UiKey::Character(character),
        KeyCode::Enter => UiKey::Enter,
        KeyCode::Esc => UiKey::Escape,
        KeyCode::Delete => UiKey::Delete,
        KeyCode::Backspace => UiKey::Backspace,
        KeyCode::Tab => UiKey::Tab,
        KeyCode::BackTab => UiKey::BackTab,
        KeyCode::Up => UiKey::Up,
        KeyCode::Down => UiKey::Down,
        KeyCode::PageUp => UiKey::PageUp,
        KeyCode::PageDown => UiKey::PageDown,
        KeyCode::Home => UiKey::Home,
        KeyCode::End => UiKey::End,
        KeyCode::F(number) => UiKey::Function(number),
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Insert
        | KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => return None,
    };
    Some(UiBinding {
        key: key_code,
        modifiers: UiModifiers {
            control: key.modifiers.contains(KeyModifiers::CONTROL),
            alt: key.modifiers.contains(KeyModifiers::ALT),
            shift: key.modifiers.contains(KeyModifiers::SHIFT),
        },
        hint: "",
        compact_hint: "",
    })
}

fn map_mouse(mouse: MouseEvent, state: &LibraryState, geometry: &ViewGeometry) -> Option<Action> {
    let library_context = matches!(
        state.command_context(),
        CommandContext::LibraryBrowse | CommandContext::LibrarySearch
    );
    match mouse.kind {
        MouseEventKind::ScrollUp
            if library_context && contains(geometry.rows, mouse.column, mouse.row) =>
        {
            Some(Action::Previous)
        }
        MouseEventKind::ScrollDown
            if library_context && contains(geometry.rows, mouse.column, mouse.row) =>
        {
            Some(Action::Next)
        }
        MouseEventKind::Down(_)
        | MouseEventKind::Up(_)
        | MouseEventKind::Drag(_)
        | MouseEventKind::Moved
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight
        | MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown => None,
    }
}

pub(crate) fn command_action(
    command: UiCommand,
    context: CommandContext,
    geometry: &ViewGeometry,
) -> Action {
    match (context, command) {
        (CommandContext::Settings, UiCommand::NewRunner) => {
            Action::Settings(skit_ui::SettingsAction::NewRunner)
        }
        (_, UiCommand::ToggleDetail) => Action::ToggleDetail {
            currently_visible: geometry.detail_pane_visible,
        },
        _ => command
            .direct_action()
            .expect("only detail commands need rendered state"),
    }
}

const fn run_field_command_action(field: usize, command: RunFieldCommand) -> Action {
    match command {
        RunFieldCommand::BrowsePath => Action::OpenRunFilePicker(field),
        RunFieldCommand::InsertValue => Action::OpenRunTokenMenuFor(field),
        RunFieldCommand::ResetDefault => Action::ResetRunField(field),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_allocation_keeps_only_complete_header_forms() {
        assert_eq!(allocated_header_height(1, 2, 0), 1);
        assert_eq!(allocated_header_height(1, 0, 0), 0);
        assert_eq!(allocated_header_height(3, 1, 1), 0);
        assert_eq!(allocated_header_height(3, 3, 2), 1);
    }

    #[test]
    fn minimum_body_height_preserves_screen_and_modal_content() {
        use skit_application::preferences::{
            AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
            PreferencesDraft, PreferencesSnapshot,
        };
        use skit_ui::{FormPurpose, PreferencesView};

        let library = LibraryState::default();
        assert_eq!(minimum_body_height(&library), 4);

        let mut modal = LibraryState::default();
        modal.update(Action::OpenHelp);
        assert_eq!(minimum_body_height(&modal), 3);

        let preferences =
            PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
                language: String::new(),
                available_languages: vec!["en".into(), "zh-CN".into(), "zh-TW".into()],
                effective_language: "en".into(),
                editor: String::new(),
                editor_fallback: Some("vi".into()),
                form: InteractiveFormChoice::Tui,
                after_run: AfterRunChoice::Exit,
                javascript: JavascriptChoice::Automatic,
                bash_path: None,
                runner_names: Vec::new(),
                mirror: MirrorConfiguration::default(),
            }));
        let mut preferences_state = LibraryState::default();
        preferences_state.update(Action::Present(Screen::Preferences(Box::new(preferences))));
        assert_eq!(minimum_body_height(&preferences_state), 5);

        let mut form_state = LibraryState::default();
        form_state.update(Action::Present(Screen::Form(FormView {
            purpose: FormPurpose::Rename,
            title: "Rename".into(),
            title_arguments: Vec::new(),
            translate_title: false,
            selector: Some("entry".into()),
            fields: vec![FormField::text_raw("name", "Name", "Entry")],
            focused: 0,
            submit_label: "Save".into(),
        })));
        assert_eq!(minimum_body_height(&form_state), 3);
    }
}
