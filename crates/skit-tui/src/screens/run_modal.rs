//! Mature launch-form modal widgets.

use std::path::{Path, PathBuf};

use ratatui_core::{
    layout::Rect,
    style::{Color, Modifier, Style},
    terminal::Frame,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui_interact::{
    components::{ListPicker, ListPickerState, ListPickerStyle},
    traits::ClickRegionRegistry,
};
use ratatui_widgets::{
    block::Block,
    borders::{BorderType, Borders},
    clear::Clear,
    paragraph::Paragraph,
};
use skit_application::tokens::environment_token;
use skit_i18n::{Locale, format_text, text};
use skit_ui::{
    Action, ModalState, PathOutputPolicy, PathPickerState, PathSelectionMode, PickerPurpose,
    RunPathContext, RunPathInsertMode, RunTokenOption,
};
use tui_input::{Input as LineInput, InputRequest, backend::crossterm::EventHandler as _};

use crate::{
    EventHandling, ViewGeometry,
    screens::picker::{FilePickerEvent, FilePickerGeometry, FilePickerSession, render_file_picker},
    session::render_line_input,
};

/// Result that needs the parent run-input session's cursor state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RunModalEvent {
    /// A complete frontend-neutral action.
    Handling(EventHandling),
    /// Insert direct text at the target widget cursor.
    Insert { field: usize, text: String },
    /// Chain into the environment-variable picker.
    OpenEnvironment { field: usize },
    /// Chain into the filesystem picker.
    OpenFile { field: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ModalSignature {
    Preset,
    Token {
        field: usize,
        options: Vec<RunTokenOption>,
    },
    Environment {
        field: usize,
        names: Vec<String>,
    },
    File {
        field: usize,
        context: RunPathContext,
        mode: RunPathInsertMode,
    },
    Other,
}

#[derive(Clone, Debug)]
enum ModalHit {
    PresetInput,
    TokenOption(usize),
    EnvironmentInput,
    EnvironmentOption(usize),
}

/// Ephemeral cursor and selection state for typed launch modals.
#[derive(Debug, Default)]
pub(crate) struct RunModalSession {
    signature: Option<ModalSignature>,
    preset: LineInput,
    token: ListPickerState,
    token_view_height: usize,
    environment: LineInput,
    environment_list: ListPickerState,
    environment_view_height: usize,
    file: Option<FilePickerSession>,
    file_geometry: FilePickerGeometry,
    file_missing_root: bool,
    clicks: ClickRegionRegistry<ModalHit>,
}

impl RunModalSession {
    /// Keep widget state aligned with the serializable modal contract.
    pub(crate) fn sync(&mut self, modal: &ModalState) {
        let signature = match modal {
            ModalState::RunPresetName { .. } => ModalSignature::Preset,
            ModalState::RunTokenMenu { field, options } => ModalSignature::Token {
                field: *field,
                options: options.clone(),
            },
            ModalState::RunEnvironmentPicker { field, names, .. } => ModalSignature::Environment {
                field: *field,
                names: names.clone(),
            },
            ModalState::RunFilePicker {
                field,
                context,
                mode,
            } => ModalSignature::File {
                field: *field,
                context: context.clone(),
                mode: *mode,
            },
            ModalState::Help
            | ModalState::ConfirmRemove { .. }
            | ModalState::ConfirmDiscardChanges
            | ModalState::RunnerEditor { .. } => ModalSignature::Other,
        };
        if self.signature.as_ref() != Some(&signature) {
            match &signature {
                ModalSignature::Preset => self.preset = LineInput::default(),
                ModalSignature::Token { options, .. } => {
                    self.token = ListPickerState::new(options.len());
                }
                ModalSignature::Environment { .. } => {
                    self.environment = LineInput::default();
                    self.environment_list = ListPickerState::default();
                }
                ModalSignature::File { context, .. } => {
                    let (contract, missing_root) = file_picker_contract(context);
                    self.file = Some(FilePickerSession::new(contract));
                    self.file_geometry = FilePickerGeometry::default();
                    self.file_missing_root = missing_root;
                }
                ModalSignature::Other => {}
            }
            self.signature = Some(signature);
        }
        if let ModalState::RunPresetName { value, .. } = modal
            && self.preset.value() != value
        {
            self.preset = LineInput::new(value.clone());
        }
        if let ModalState::RunEnvironmentPicker { query, visible, .. } = modal {
            if self.environment.value() != query {
                self.environment = LineInput::new(query.clone());
            }
            self.environment_list.set_total(visible.len());
        }
    }

    /// Draw one launch modal and register its external click regions.
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        modal: &ModalState,
        locale: Locale,
    ) -> ViewGeometry {
        self.sync(modal);
        self.clicks.clear();
        match modal {
            ModalState::RunPresetName {
                value, existing, ..
            } => self.render_preset(frame, area, value, existing.contains(value.trim()), locale),
            ModalState::RunTokenMenu { options, .. } => {
                self.render_token_menu(frame, area, options, locale)
            }
            ModalState::RunEnvironmentPicker { visible, .. } => {
                self.render_environment(frame, area, visible, locale)
            }
            ModalState::RunFilePicker { .. } => self.render_file(frame, area, locale),
            ModalState::Help
            | ModalState::ConfirmRemove { .. }
            | ModalState::ConfirmDiscardChanges
            | ModalState::RunnerEditor { .. } => {}
        }
        ViewGeometry::default()
    }

    /// Send input through the active mature widget before the command registry.
    pub(crate) fn handle_event(&mut self, event: Event, modal: &ModalState) -> RunModalEvent {
        self.sync(modal);
        match modal {
            ModalState::RunPresetName { .. } => self.handle_preset(event),
            ModalState::RunTokenMenu { field, options } => {
                self.handle_token(event, *field, options)
            }
            ModalState::RunEnvironmentPicker { field, visible, .. } => {
                self.handle_environment(event, *field, visible)
            }
            ModalState::RunFilePicker { field, .. } => self.handle_file(event, *field),
            ModalState::Help
            | ModalState::ConfirmRemove { .. }
            | ModalState::ConfirmDiscardChanges
            | ModalState::RunnerEditor { .. } => RunModalEvent::Handling(EventHandling::Ignored),
        }
    }

    fn render_file(&mut self, frame: &mut Frame, area: Rect, locale: Locale) {
        let (notice, picker_area) = if self.file_missing_root && area.height > 1 {
            (
                Some(Rect::new(area.x, area.y, area.width, 1)),
                Rect::new(
                    area.x,
                    area.y.saturating_add(1),
                    area.width,
                    area.height.saturating_sub(1),
                ),
            )
        } else {
            (None, area)
        };
        if let Some(notice) = notice {
            frame.render_widget(
                Paragraph::new(text(
                    locale,
                    "The entry's working directory is missing — starting here instead.",
                ))
                .style(Style::default().fg(Color::Yellow)),
                notice,
            );
        }
        if let Some(file) = &mut self.file {
            self.file_geometry = render_file_picker(frame, picker_area, file, locale);
        }
    }

    fn handle_file(&mut self, event: Event, field: usize) -> RunModalEvent {
        let Some(file) = &mut self.file else {
            return ignored();
        };
        match file.handle_event(event, &self.file_geometry) {
            Some(FilePickerEvent::Changed) => consumed(),
            Some(FilePickerEvent::Cancelled) => action(Action::Back),
            Some(FilePickerEvent::Accepted(paths)) => paths.first().map_or_else(consumed, |path| {
                action(Action::SetRunPickedPathAndCloseModal {
                    field,
                    path: picked_path_text(path),
                })
            }),
            None => ignored(),
        }
    }

    fn render_preset(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        value: &str,
        overwrites: bool,
        locale: Locale,
    ) {
        let panel = centered(area, 60, 7);
        frame.render_widget(Clear, panel);
        let title = text(locale, "Save as preset");
        let block = modal_block(&title);
        let inner = block.inner(panel);
        frame.render_widget(block, panel);
        let input = Rect::new(inner.x, inner.y, inner.width, inner.height.min(3));
        render_line_input(
            frame,
            input,
            &self.preset,
            false,
            true,
            &text(locale, "Preset name"),
        );
        self.clicks.register(input, ModalHit::PresetInput);
        if overwrites && inner.height > 3 {
            frame.render_widget(
                Paragraph::new(format_text(
                    locale,
                    "This overwrites the existing preset {}.",
                    &[&value.trim()],
                ))
                .style(Style::default().fg(Color::Yellow)),
                Rect::new(inner.x, inner.y.saturating_add(3), inner.width, 1),
            );
        }
    }

    fn render_token_menu(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        options: &[RunTokenOption],
        locale: Locale,
    ) {
        let desired = u16::try_from(options.len().saturating_add(4)).unwrap_or(u16::MAX);
        let panel = centered(area, 64, desired.max(5));
        frame.render_widget(Clear, panel);
        let title = text(locale, "Insert a run-time value");
        let block = modal_block(&title);
        let inner = block.inner(panel);
        frame.render_widget(block, panel);
        self.token_view_height = usize::from(inner.height);
        self.token.ensure_visible(self.token_view_height);
        let labels = options
            .iter()
            .map(|option| token_label(option, locale))
            .collect::<Vec<_>>();
        frame.render_widget(
            ListPicker::new(&labels, &self.token).style(list_style()),
            inner,
        );
        for visible in 0..self.token_view_height {
            let index = usize::from(self.token.scroll).saturating_add(visible);
            if index >= options.len() {
                break;
            }
            self.clicks.register(
                Rect::new(
                    inner.x,
                    inner
                        .y
                        .saturating_add(u16::try_from(visible).unwrap_or(u16::MAX)),
                    inner.width,
                    1,
                ),
                ModalHit::TokenOption(index),
            );
        }
    }

    fn render_environment(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        visible: &[String],
        locale: Locale,
    ) {
        let desired = u16::try_from(visible.len().min(12).saturating_add(6)).unwrap_or(u16::MAX);
        let panel = centered(area, 64, desired.max(7));
        frame.render_widget(Clear, panel);
        let title = text(locale, "Environment variable");
        let block = modal_block(&title);
        let inner = block.inner(panel);
        frame.render_widget(block, panel);
        let input = Rect::new(inner.x, inner.y, inner.width, inner.height.min(3));
        render_line_input(
            frame,
            input,
            &self.environment,
            false,
            true,
            &text(locale, "type to filter…"),
        );
        self.clicks.register(input, ModalHit::EnvironmentInput);
        let list = Rect::new(
            inner.x,
            inner.y.saturating_add(input.height),
            inner.width,
            inner.height.saturating_sub(input.height),
        );
        self.environment_view_height = usize::from(list.height);
        self.environment_list
            .ensure_visible(self.environment_view_height);
        frame.render_widget(
            ListPicker::new(visible, &self.environment_list).style(list_style()),
            list,
        );
        for row in 0..self.environment_view_height {
            let index = usize::from(self.environment_list.scroll).saturating_add(row);
            if index >= visible.len() {
                break;
            }
            self.clicks.register(
                Rect::new(
                    list.x,
                    list.y
                        .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
                    list.width,
                    1,
                ),
                ModalHit::EnvironmentOption(index),
            );
        }
    }

    fn handle_preset(&mut self, event: Event) -> RunModalEvent {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return action(Action::Quit);
                }
                if key.code == KeyCode::Esc {
                    return action(Action::Back);
                }
                if key.code == KeyCode::Enter {
                    return action(Action::Submit);
                }
                let before = self.preset.value().to_owned();
                if self.preset.handle_event(&Event::Key(key)).is_some() {
                    if before == self.preset.value() {
                        consumed()
                    } else {
                        action(Action::SetModalInput(self.preset.value().to_owned()))
                    }
                } else {
                    ignored()
                }
            }
            Event::Paste(value) => {
                for character in value.chars() {
                    let _ = self.preset.handle(InputRequest::InsertChar(character));
                }
                action(Action::SetModalInput(self.preset.value().to_owned()))
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                match self.clicks.handle_click(mouse.column, mouse.row) {
                    Some(ModalHit::PresetInput) => consumed(),
                    Some(
                        ModalHit::TokenOption(_)
                        | ModalHit::EnvironmentInput
                        | ModalHit::EnvironmentOption(_),
                    )
                    | None => ignored(),
                }
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Key(_)
            | Event::Resize(_, _) => ignored(),
        }
    }

    fn handle_environment(
        &mut self,
        event: Event,
        field: usize,
        visible: &[String],
    ) -> RunModalEvent {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return action(Action::Quit);
                }
                match key.code {
                    KeyCode::Esc => return action(Action::Back),
                    KeyCode::Up => {
                        return move_list(
                            &mut self.environment_list,
                            self.environment_view_height,
                            0,
                        );
                    }
                    KeyCode::Down => {
                        return move_list(
                            &mut self.environment_list,
                            self.environment_view_height,
                            1,
                        );
                    }
                    KeyCode::Enter => return self.accept_environment(field, visible),
                    _ => {}
                }
                let before = self.environment.value().to_owned();
                if self.environment.handle_event(&Event::Key(key)).is_some() {
                    if before == self.environment.value() {
                        consumed()
                    } else {
                        action(Action::SetRunEnvironmentQuery(
                            self.environment.value().to_owned(),
                        ))
                    }
                } else {
                    ignored()
                }
            }
            Event::Paste(value) => {
                for character in value.chars() {
                    let _ = self.environment.handle(InputRequest::InsertChar(character));
                }
                action(Action::SetRunEnvironmentQuery(
                    self.environment.value().to_owned(),
                ))
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                match self.clicks.handle_click(mouse.column, mouse.row) {
                    Some(ModalHit::EnvironmentOption(index)) => visible
                        .get(*index)
                        .map_or_else(consumed, |name| insert_environment(field, name)),
                    Some(ModalHit::EnvironmentInput) => consumed(),
                    Some(ModalHit::PresetInput | ModalHit::TokenOption(_)) | None => ignored(),
                }
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Key(_)
            | Event::Resize(_, _) => ignored(),
        }
    }

    fn accept_environment(&self, field: usize, visible: &[String]) -> RunModalEvent {
        let typed = self.environment.value().trim();
        let name = if environment_token(typed).is_some() {
            Some(typed.to_owned())
        } else {
            visible.get(self.environment_list.selected_index).cloned()
        };
        name.map_or_else(consumed, |name| insert_environment(field, &name))
    }

    fn handle_token(
        &mut self,
        event: Event,
        field: usize,
        options: &[RunTokenOption],
    ) -> RunModalEvent {
        let picked = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return action(Action::Quit);
                }
                KeyCode::Esc => return action(Action::Back),
                KeyCode::Up => {
                    return move_list(&mut self.token, self.token_view_height, 0);
                }
                KeyCode::Down => {
                    return move_list(&mut self.token, self.token_view_height, 1);
                }
                KeyCode::Home => {
                    return move_list(&mut self.token, self.token_view_height, 2);
                }
                KeyCode::End => {
                    return move_list(&mut self.token, self.token_view_height, 3);
                }
                KeyCode::Enter => Some(self.token.selected_index),
                _ => return ignored(),
            },
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                match self.clicks.handle_click(mouse.column, mouse.row) {
                    Some(ModalHit::TokenOption(index)) => Some(*index),
                    Some(
                        ModalHit::PresetInput
                        | ModalHit::EnvironmentInput
                        | ModalHit::EnvironmentOption(_),
                    )
                    | None => return ignored(),
                }
            }
            Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
            | Event::Key(_)
            | Event::Resize(_, _) => return ignored(),
        };
        let Some(option) = picked.and_then(|index| options.get(index)) else {
            return consumed();
        };
        match option {
            RunTokenOption::Environment => RunModalEvent::OpenEnvironment { field },
            RunTokenOption::FileOrFolder => RunModalEvent::OpenFile { field },
            option => RunModalEvent::Insert {
                field,
                text: option.insertion().unwrap_or_default().to_owned(),
            },
        }
    }
}

fn move_list(list: &mut ListPickerState, visible_height: usize, movement: u8) -> RunModalEvent {
    if movement == 0 {
        list.select_prev();
    } else if movement == 1 {
        list.select_next();
    } else if movement == 2 {
        list.select_first();
    } else {
        list.select_last();
    }
    list.ensure_visible(visible_height);
    consumed()
}

fn action(action: Action) -> RunModalEvent {
    RunModalEvent::Handling(EventHandling::Action(action))
}

fn insert_environment(field: usize, name: &str) -> RunModalEvent {
    environment_token(name).map_or_else(consumed, |text| RunModalEvent::Insert { field, text })
}

const fn consumed() -> RunModalEvent {
    RunModalEvent::Handling(EventHandling::Consumed)
}

const fn ignored() -> RunModalEvent {
    RunModalEvent::Handling(EventHandling::Ignored)
}

fn centered(area: Rect, maximum_width: u16, desired_height: u16) -> Rect {
    let width = maximum_width.min(area.width);
    let height = desired_height.min(area.height);
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    )
}

fn modal_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(0xD9, 0x77, 0x57)))
        .title(title)
}

fn token_label(option: &RunTokenOption, locale: Locale) -> String {
    match option {
        RunTokenOption::FileOrFolder => text(locale, "File or folder…").into_owned(),
        RunTokenOption::RuntimeDirectory => format!(
            "{}  {{cwd}}",
            text(locale, "Directory at run time (changes with where you run)")
        ),
        RunTokenOption::FixedDirectory { path } => format!(
            "{}  {path}",
            text(locale, "This directory, as a fixed path")
        ),
        RunTokenOption::Today => format!("{}  {{today}}", text(locale, "Today's date")),
        RunTokenOption::Now => format!("{}  {{now}}", text(locale, "Current time")),
        RunTokenOption::Home => format!("{}  ~", text(locale, "Home directory")),
        RunTokenOption::Environment => {
            format!("{}  {{env:NAME}}", text(locale, "Environment variable…"))
        }
    }
}

fn list_style() -> ListPickerStyle {
    ListPickerStyle {
        selected_style: Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(0xD9, 0x77, 0x57))
            .add_modifier(Modifier::BOLD),
        normal_style: Style::default().fg(Color::White),
        indicator_style: Style::default().fg(Color::Rgb(0xD9, 0x77, 0x57)),
        border_style: Style::default(),
        indicator: "▶ ",
        indicator_empty: "  ",
        bordered: false,
    }
}

fn file_picker_contract(context: &RunPathContext) -> (PathPickerState, bool) {
    let workdir = PathBuf::from(&context.workdir);
    let missing_root = !workdir.is_dir();
    let start = if missing_root {
        workdir
            .ancestors()
            .skip(1)
            .find(|candidate| candidate.is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(&context.invoke_cwd))
    } else {
        workdir.clone()
    };
    (
        PathPickerState::new(
            PickerPurpose::Argument,
            start,
            PathSelectionMode::FileOrDirectory,
            PathOutputPolicy::RelativeTo(workdir),
            false,
        ),
        missing_root,
    )
}

fn picked_path_text(path: &Path) -> String {
    let text = if path.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        path.to_string_lossy().into_owned()
    };
    text.replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs};

    use ratatui_core::{backend::TestBackend, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{KeyEvent, MouseButton, MouseEvent};
    use skit_ui::{RunnerEditorOwner, RunnerEditorView};
    use tempfile::tempdir;

    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn control(character: char) -> Event {
        Event::Key(KeyEvent::new(
            KeyCode::Char(character),
            KeyModifiers::CONTROL,
        ))
    }

    fn mouse(column: u16, row: u16, kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn preset_and_other_modals_keep_input_mouse_priority_and_reverse_events() {
        let mut session = RunModalSession::default();
        let other = ModalState::RunnerEditor {
            owner: RunnerEditorOwner::Add,
            view: Box::new(RunnerEditorView::default()),
            cancel_status: None,
        };
        session.sync(&other);
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &other, Locale::En);
            })
            .unwrap();
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &other),
            RunModalEvent::Handling(EventHandling::Ignored)
        );

        let modal = ModalState::RunPresetName {
            value: "old".to_owned(),
            existing: BTreeSet::from(["old".to_owned()]),
        };
        session.sync(&modal);
        let changed = ModalState::RunPresetName {
            value: "saved".to_owned(),
            existing: BTreeSet::from(["saved".to_owned()]),
        };
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &changed, Locale::ZhTw);
            })
            .unwrap();
        assert!(!buffer_text(&terminal).trim().is_empty());
        assert_eq!(
            session.handle_event(control('c'), &changed),
            action(Action::Quit)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Esc), &changed),
            action(Action::Back)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &changed),
            action(Action::Submit)
        );
        assert!(matches!(
            session.handle_event(key(KeyCode::Char('x')), &changed),
            RunModalEvent::Handling(EventHandling::Action(Action::SetModalInput(value)))
                if value == "savedx"
        ));
        assert_eq!(
            session.handle_event(key(KeyCode::Left), &changed),
            RunModalEvent::Handling(EventHandling::Consumed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::F(2)), &changed),
            RunModalEvent::Handling(EventHandling::Ignored)
        );
        assert!(matches!(
            session.handle_event(Event::Paste("字".to_owned()), &changed),
            RunModalEvent::Handling(EventHandling::Action(Action::SetModalInput(value)))
                if value.contains('字')
        ));
        assert_eq!(
            session.handle_event(
                mouse(11, 7, MouseEventKind::Down(MouseButton::Left)),
                &changed,
            ),
            RunModalEvent::Handling(EventHandling::Consumed)
        );
        assert_eq!(
            session.handle_event(
                mouse(0, 0, MouseEventKind::Down(MouseButton::Left)),
                &changed,
            ),
            RunModalEvent::Handling(EventHandling::Ignored)
        );
        for event in [
            mouse(11, 7, MouseEventKind::Moved),
            mouse(11, 7, MouseEventKind::Up(MouseButton::Left)),
            Event::FocusGained,
            Event::Resize(40, 10),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Enter,
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
        ] {
            assert_eq!(
                session.handle_event(event, &changed),
                RunModalEvent::Handling(EventHandling::Ignored)
            );
        }
    }

    #[test]
    fn environment_and_token_modals_route_every_key_and_left_down_target() {
        let names = vec!["HOME".to_owned(), "PATH".to_owned(), "SHELL".to_owned()];
        let modal = ModalState::RunEnvironmentPicker {
            field: 4,
            names: names.clone(),
            query: String::new(),
            visible: names.clone(),
        };
        let mut session = RunModalSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &modal, Locale::ZhCn);
            })
            .unwrap();
        assert!(!buffer_text(&terminal).trim().is_empty());
        assert_eq!(
            session.handle_event(control('c'), &modal),
            action(Action::Quit)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Esc), &modal),
            action(Action::Back)
        );
        for code in [KeyCode::Down, KeyCode::Up] {
            assert_eq!(
                session.handle_event(key(code), &modal),
                RunModalEvent::Handling(EventHandling::Consumed)
            );
        }
        assert!(matches!(
            session.handle_event(key(KeyCode::Enter), &modal),
            RunModalEvent::Insert { field: 4, text } if text.starts_with("{env:")
        ));
        assert!(matches!(
            session.handle_event(key(KeyCode::Char('H')), &modal),
            RunModalEvent::Handling(EventHandling::Action(Action::SetRunEnvironmentQuery(value)))
                if value == "H"
        ));
        let typed = ModalState::RunEnvironmentPicker {
            field: 4,
            names: names.clone(),
            query: "H".to_owned(),
            visible: names.clone(),
        };
        assert_eq!(
            session.handle_event(key(KeyCode::Left), &typed),
            RunModalEvent::Handling(EventHandling::Consumed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::F(2)), &modal),
            RunModalEvent::Handling(EventHandling::Ignored)
        );
        assert!(matches!(
            session.handle_event(Event::Paste("OME".to_owned()), &typed),
            RunModalEvent::Handling(EventHandling::Action(Action::SetRunEnvironmentQuery(value)))
                if value == "HOME"
        ));
        let home = ModalState::RunEnvironmentPicker {
            field: 4,
            names: names.clone(),
            query: "HOME".to_owned(),
            visible: names.clone(),
        };
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &home),
            RunModalEvent::Insert {
                field: 4,
                text: "{env:HOME}".to_owned(),
            }
        );
        assert_eq!(
            session.handle_event(mouse(9, 7, MouseEventKind::Down(MouseButton::Left)), &home),
            RunModalEvent::Handling(EventHandling::Consumed)
        );
        assert!(matches!(
            session.handle_event(mouse(9, 10, MouseEventKind::Down(MouseButton::Left)), &home,),
            RunModalEvent::Insert { field: 4, .. }
        ));

        let recomposed = ModalState::RunEnvironmentPicker {
            field: 4,
            names,
            query: "HOME".to_owned(),
            visible: Vec::new(),
        };
        assert_eq!(
            session.handle_event(
                mouse(9, 10, MouseEventKind::Down(MouseButton::Left)),
                &recomposed,
            ),
            RunModalEvent::Handling(EventHandling::Consumed)
        );
        assert_eq!(
            session.handle_event(
                mouse(0, 0, MouseEventKind::Down(MouseButton::Left)),
                &recomposed,
            ),
            RunModalEvent::Handling(EventHandling::Ignored)
        );
        for event in [
            mouse(9, 10, MouseEventKind::Moved),
            mouse(9, 10, MouseEventKind::Up(MouseButton::Left)),
            Event::FocusLost,
            Event::Resize(20, 5),
        ] {
            assert_eq!(
                session.handle_event(event, &recomposed),
                RunModalEvent::Handling(EventHandling::Ignored)
            );
        }

        let options = vec![
            RunTokenOption::FileOrFolder,
            RunTokenOption::RuntimeDirectory,
            RunTokenOption::FixedDirectory {
                path: "/fixed".to_owned(),
            },
            RunTokenOption::Today,
            RunTokenOption::Now,
            RunTokenOption::Home,
            RunTokenOption::Environment,
        ];
        let tokens = ModalState::RunTokenMenu {
            field: 2,
            options: options.clone(),
        };
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &tokens, Locale::En);
            })
            .unwrap();
        assert!(buffer_text(&terminal).contains("Environment variable"));
        assert_eq!(
            session.handle_event(
                mouse(9, 5, MouseEventKind::Down(MouseButton::Left)),
                &tokens,
            ),
            RunModalEvent::OpenFile { field: 2 }
        );
        assert_eq!(
            session.handle_event(control('c'), &tokens),
            action(Action::Quit)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Esc), &tokens),
            action(Action::Back)
        );
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Home, KeyCode::End] {
            assert_eq!(
                session.handle_event(key(code), &tokens),
                RunModalEvent::Handling(EventHandling::Consumed)
            );
        }
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &tokens),
            RunModalEvent::OpenEnvironment { field: 2 }
        );
        assert_eq!(
            session.handle_event(key(KeyCode::F(2)), &tokens),
            RunModalEvent::Handling(EventHandling::Ignored)
        );
        for event in [Event::Paste("ignored".to_owned()), Event::FocusGained] {
            assert_eq!(
                session.handle_event(event, &tokens),
                RunModalEvent::Handling(EventHandling::Ignored)
            );
        }
        assert_eq!(
            session.handle_event(mouse(0, 0, MouseEventKind::Moved), &tokens),
            RunModalEvent::Handling(EventHandling::Ignored)
        );
        assert_eq!(
            session.handle_event(
                mouse(0, 0, MouseEventKind::Down(MouseButton::Left)),
                &tokens,
            ),
            RunModalEvent::Handling(EventHandling::Ignored)
        );

        let empty = ModalState::RunTokenMenu {
            field: 2,
            options: Vec::new(),
        };
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &empty),
            RunModalEvent::Handling(EventHandling::Consumed)
        );
        for (option, expected) in [
            (
                RunTokenOption::FileOrFolder,
                RunModalEvent::OpenFile { field: 2 },
            ),
            (
                RunTokenOption::RuntimeDirectory,
                RunModalEvent::Insert {
                    field: 2,
                    text: "{cwd}".to_owned(),
                },
            ),
        ] {
            let one = ModalState::RunTokenMenu {
                field: 2,
                options: vec![option],
            };
            assert_eq!(session.handle_event(key(KeyCode::Enter), &one), expected);
        }
    }

    #[test]
    fn file_modal_uses_real_picker_events_missing_root_and_dot_output() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("input.txt"), b"input").unwrap();
        let context = RunPathContext {
            workdir: dir.path().display().to_string(),
            invoke_cwd: dir.path().display().to_string(),
        };
        let modal = ModalState::RunFilePicker {
            field: 3,
            context,
            mode: RunPathInsertMode::Replace,
        };
        let mut session = RunModalSession::default();
        assert_eq!(
            session.handle_file(key(KeyCode::Enter), 3),
            RunModalEvent::Handling(EventHandling::Ignored)
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &modal, Locale::En);
            })
            .unwrap();
        assert_eq!(
            session.handle_event(key(KeyCode::F(2)), &modal),
            RunModalEvent::Handling(EventHandling::Ignored)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Esc), &modal),
            action(Action::Back)
        );

        let empty = tempdir().unwrap();
        let dot_modal = ModalState::RunFilePicker {
            field: 8,
            context: RunPathContext {
                workdir: empty.path().display().to_string(),
                invoke_cwd: empty.path().display().to_string(),
            },
            mode: RunPathInsertMode::Arguments,
        };
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &dot_modal, Locale::ZhCn);
            })
            .unwrap();
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &dot_modal),
            action(Action::SetRunPickedPathAndCloseModal {
                field: 8,
                path: ".".to_owned(),
            })
        );

        let missing = dir.path().join("gone/deeper");
        let missing_modal = ModalState::RunFilePicker {
            field: 1,
            context: RunPathContext {
                workdir: missing.display().to_string(),
                invoke_cwd: dir.path().display().to_string(),
            },
            mode: RunPathInsertMode::Shlex,
        };
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &missing_modal, Locale::ZhTw);
            })
            .unwrap();
        assert!(!buffer_text(&terminal).trim().is_empty());
    }
}
