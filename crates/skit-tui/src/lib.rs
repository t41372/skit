//! Ratatui frontend adapter for skit.

#![forbid(unsafe_code)]

mod footer;
mod layout;
mod screens;
mod session;
mod terminal;
mod theme;

use ratatui_core::{layout::Rect, terminal::Frame};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_i18n::{Locale, format_text};
use skit_ui::{
    Action, CommandContext, FormField, FormView, InputMode, LibraryState, ModalState, Screen,
    UiBinding, UiCommand, UiKey, UiModifiers, command_specs,
};

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
pub use session::{EventHandling, TuiSession};
pub use terminal::{TuiError, collect_form, collect_run_form, run, run_add_workflow};

/// One clickable target produced by a view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitTarget {
    /// A command from the shared command registry.
    Command(UiCommand),
    /// A command whose action must retain the launch-field identity.
    RunFieldCommand {
        /// Field index in the active typed launch form.
        field: usize,
        /// Shared command identity rendered by the chip.
        command: UiCommand,
    },
    /// Focus one form row.
    FocusField(usize),
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
    let footer_height =
        footer::required_height(frame.area().width, frame.area().height, state, locale);
    let areas = layout::split_with_header(
        frame.area(),
        footer_height,
        header_height(state, frame.area().height),
    );

    if areas.header.height > 0 {
        session.render_header(frame, areas.header, state, locale);
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
    if areas.footer.height > 0 {
        geometry
            .hits
            .extend(session.render_footer(frame, areas.footer, state, locale));
    }
    session.register_geometry(&geometry);
    session.render_quit_toast(frame, locale);
    geometry
}

/// Return the rows the shared header takes on one screen.
///
/// A screen that titles its own panel gets the whole body (`src/skit/tui_form.py:606-611`,
/// `src/skit/tui_settings.py:869-871`). Drawing the header above it prints the same title twice and
/// spends three rows saying so — on entry settings that was three of the rows the parameter section
/// needed to be on screen at all. Most modals keep the header. The compact environment picker owns
/// its title and uses those rows for its input.
fn header_height(state: &LibraryState, terminal_height: u16) -> u16 {
    // The environment picker owns a titled panel. On short and tiny terminals,
    // omit the duplicate global title so its bordered input and the global
    // Cancel chip both fit. This matches the short-tier modal chrome budget.
    if terminal_height < 16
        && matches!(state.modal(), Some(ModalState::RunEnvironmentPicker { .. }))
    {
        return 0;
    }
    if state.modal().is_none()
        && matches!(
            state.screen(),
            Screen::Run(_) | Screen::Form(_) | Screen::Settings(_)
        )
    {
        0
    } else if state.modal().is_none()
        && matches!(state.screen(), Screen::Library)
        && state.input_mode() == InputMode::Search
        && layout::is_short(terminal_height)
    {
        1
    } else {
        3
    }
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
        Screen::Report(report) => screens::report::render(frame, area, report, locale),
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
            .any(|binding| binding.key == chord.key && binding.modifiers == chord.modifiers)
    }) {
        if matches!(
            (context, spec.command),
            (
                CommandContext::LibrarySearch | CommandContext::Form | CommandContext::RunForm,
                UiCommand::Backspace
            ) | (CommandContext::LibrarySearch, UiCommand::ClearSearch)
        ) {
            return None;
        }
        return Some(command_action(spec.command, geometry));
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
        MouseEventKind::ScrollUp if library_context => Some(Action::Previous),
        MouseEventKind::ScrollDown if library_context => Some(Action::Next),
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(hit) = geometry
                .hits
                .iter()
                .find(|hit| contains(hit.rect, mouse.column, mouse.row))
            {
                return Some(match hit.action {
                    HitTarget::Command(command) => command_action(command, geometry),
                    HitTarget::RunFieldCommand { field, command } => {
                        run_field_command_action(field, command)
                    }
                    HitTarget::FocusField(index) => Action::FocusField(index),
                });
            }
            library_context
                .then_some(())
                .filter(|()| contains(geometry.rows, mouse.column, mouse.row))
                .map(|()| {
                    Action::SelectVisible(
                        geometry.first_visible + usize::from(mouse.row - geometry.rows.y),
                    )
                })
        }
        MouseEventKind::Down(MouseButton::Right)
        | MouseEventKind::Down(MouseButton::Middle)
        | MouseEventKind::Up(_)
        | MouseEventKind::Drag(_)
        | MouseEventKind::Moved
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight
        | MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown => None,
    }
}

pub(crate) fn command_action(command: UiCommand, geometry: &ViewGeometry) -> Action {
    if matches!(command, UiCommand::ToggleDetail) {
        Action::ToggleDetail {
            currently_visible: geometry.detail_pane_visible,
        }
    } else {
        command
            .direct_action()
            .expect("only detail commands need rendered state")
    }
}

fn run_field_command_action(field: usize, command: UiCommand) -> Action {
    match command {
        UiCommand::BrowsePath => Action::OpenRunFilePicker(field),
        UiCommand::InsertValue => Action::OpenRunTokenMenuFor(field),
        UiCommand::ResetDefault => Action::ResetRunField(field),
        _ => command
            .direct_action()
            .expect("detail commands are not run-field commands"),
    }
}

fn contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect_right(rect)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

const fn rect_right(rect: Rect) -> u16 {
    rect.x.saturating_add(rect.width)
}
