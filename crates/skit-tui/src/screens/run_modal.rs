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
            | ModalState::RunnerEditor { .. } => {
                RunModalEvent::Handling(EventHandling::Ignored)
            }
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
            Event::Mouse(mouse)
                if matches!(mouse.kind, MouseEventKind::Down(_))
                    && matches!(
                        self.clicks.handle_click(mouse.column, mouse.row),
                        Some(ModalHit::PresetInput)
                    ) =>
            {
                consumed()
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
                        self.environment_list.select_prev();
                        self.environment_list
                            .ensure_visible(self.environment_view_height);
                        return consumed();
                    }
                    KeyCode::Down => {
                        self.environment_list.select_next();
                        self.environment_list
                            .ensure_visible(self.environment_view_height);
                        return consumed();
                    }
                    KeyCode::Enter => {
                        let typed = self.environment.value().trim();
                        let name = if environment_token(typed).is_some() {
                            Some(typed.to_owned())
                        } else {
                            visible.get(self.environment_list.selected_index).cloned()
                        };
                        return name.map_or_else(consumed, |name| insert_environment(field, &name));
                    }
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
                    self.token.select_prev();
                    self.token.ensure_visible(self.token_view_height);
                    return consumed();
                }
                KeyCode::Down => {
                    self.token.select_next();
                    self.token.ensure_visible(self.token_view_height);
                    return consumed();
                }
                KeyCode::Home => {
                    self.token.select_first();
                    self.token.ensure_visible(self.token_view_height);
                    return consumed();
                }
                KeyCode::End => {
                    self.token.select_last();
                    self.token.ensure_visible(self.token_view_height);
                    return consumed();
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
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    }
}
