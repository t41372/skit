//! Read-only health and diagnostic reports.

use ratatui_core::{layout::Rect, terminal::Frame, text::Line};
use ratatui_crossterm::crossterm::event::{Event, KeyEventKind};
use ratatui_interact::components::{
    ScrollableContentState, handle_scrollable_content_key, handle_scrollable_content_mouse,
};
use ratatui_widgets::{
    block::Block,
    borders::Borders,
    paragraph::{Paragraph, Wrap},
    scrollbar::{Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use skit_i18n::{Locale, text};
use skit_ui::ReportView;

use crate::{ViewGeometry, viewport::Viewport};

/// Persistent reader position for a report screen.
#[derive(Debug, Default)]
pub(crate) struct ReportScreenSession {
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    report: Option<ReportView>,
}

impl ReportScreenSession {
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        report: &ReportView,
        locale: Locale,
    ) -> ViewGeometry {
        if area.is_empty() {
            self.viewport = area;
            self.visible_height = 0;
            return ViewGeometry {
                rows: area,
                first_visible: self.scroll.scroll_offset(),
                ..ViewGeometry::default()
            };
        }

        if self.report.as_ref() != Some(report) {
            self.scroll.set_scroll_offset(0);
            self.report = Some(report.clone());
        }

        let lines = report
            .items
            .iter()
            .map(|item| {
                Line::from(format!(
                    "[{}] {}: {}",
                    text(locale, &item.status),
                    if item.translate_label {
                        text(locale, &item.label).into_owned()
                    } else {
                        item.label.clone()
                    },
                    if item.translate_detail {
                        text(locale, &item.detail).into_owned()
                    } else {
                        item.detail.clone()
                    }
                ))
            })
            .collect::<Vec<_>>();
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let initial_height = paragraph.line_count(inner.width.max(1));
        self.viewport = if initial_height > usize::from(inner.height) {
            Rect {
                width: inner.width.saturating_sub(1),
                ..inner
            }
        } else {
            inner
        };
        self.visible_height = usize::from(self.viewport.height);
        let content_height = paragraph.line_count(self.viewport.width.max(1));
        self.scroll.set_lines(vec![String::new(); content_height]);
        Viewport::new(self.viewport, content_height).clamp_scroll(&mut self.scroll);

        frame.render_widget(block, area);
        frame.render_widget(
            paragraph.scroll((
                u16::try_from(self.scroll.scroll_offset()).unwrap_or(u16::MAX),
                0,
            )),
            self.viewport,
        );
        let mut scrollbar = ScrollbarState::new(content_height.saturating_sub(self.visible_height))
            .position(self.scroll.scroll_offset())
            .viewport_content_length(self.visible_height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            inner,
            &mut scrollbar,
        );

        ViewGeometry {
            rows: self.viewport,
            first_visible: self.scroll.scroll_offset(),
            hits: Vec::new(),
            detail_pane_visible: false,
        }
    }

    pub(crate) fn handle_event(&mut self, event: &Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                handle_scrollable_content_key(&mut self.scroll, key, self.visible_height).is_some()
            }
            Event::Mouse(mouse) => handle_scrollable_content_mouse(
                &mut self.scroll,
                mouse,
                self.viewport,
                self.visible_height,
            )
            .is_some(),
            Event::FocusGained
            | Event::FocusLost
            | Event::Key(_)
            | Event::Paste(_)
            | Event::Resize(_, _) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui_core::{backend::TestBackend, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    use skit_ui::ReportItem;

    use super::*;

    fn report() -> ReportView {
        ReportView {
            title: "Report".to_owned(),
            items: vec![ReportItem {
                status: "ok".to_owned(),
                label: "Check".to_owned(),
                translate_label: false,
                detail: "wrapped detail ".repeat(20),
                translate_detail: false,
            }],
        }
    }

    #[test]
    fn report_handles_an_empty_viewport_without_inventing_geometry() {
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        let mut session = ReportScreenSession::default();
        let empty = Rect::new(3, 2, 0, 2);
        terminal
            .draw(|frame| {
                let geometry = session.render(frame, empty, &report(), Locale::En);
                assert_eq!(geometry.rows, empty);
                assert_eq!(geometry.first_visible, 0);
                assert!(geometry.hits.is_empty());
                assert!(!geometry.detail_pane_visible);
            })
            .unwrap();
    }

    #[test]
    fn report_owns_vertical_wheel_events_inside_its_viewport() {
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        let mut session = ReportScreenSession::default();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &report(), Locale::En);
            })
            .unwrap();
        assert!(session.handle_event(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: session.viewport.x,
            row: session.viewport.y,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(!session.handle_event(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: session.viewport.x,
            row: session.viewport.y,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(!session.handle_event(&Event::FocusGained));
        assert!(!session.handle_event(&Event::FocusLost));
        assert!(!session.handle_event(&Event::Key(
            ratatui_crossterm::crossterm::event::KeyEvent::new_with_kind(
                ratatui_crossterm::crossterm::event::KeyCode::Down,
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ),
        )));
        assert!(!session.handle_event(&Event::Paste("ignored".to_owned())));
        assert!(!session.handle_event(&Event::Resize(40, 10)));
    }

    fn one_line_report(items: usize) -> ReportView {
        ReportView {
            title: "Report".to_owned(),
            items: (0..items)
                .map(|index| ReportItem {
                    status: "ok".to_owned(),
                    label: format!("C{index}"),
                    translate_label: false,
                    detail: "x".to_owned(),
                    translate_detail: false,
                })
                .collect(),
        }
    }

    #[test]
    fn report_reserves_a_scrollbar_column_only_for_strict_overflow() {
        for (items, expected_width) in [(2, 18), (3, 18), (4, 17)] {
            let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
            let mut session = ReportScreenSession::default();
            let mut geometry = ViewGeometry::default();
            terminal
                .draw(|frame| {
                    geometry =
                        session.render(frame, frame.area(), &one_line_report(items), Locale::En);
                })
                .unwrap();

            assert_eq!(geometry.rows, Rect::new(1, 1, expected_width, 3));
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            if items == 4 {
                assert!(
                    rendered.contains('█') || rendered.contains('▲') || rendered.contains('▼'),
                    "strict overflow has no scrollbar: {rendered}",
                );
            }
        }
    }

    #[test]
    fn report_keeps_its_scroll_position_through_a_zero_height_frame() {
        let report = report();
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        let mut session = ReportScreenSession::default();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &report, Locale::En);
            })
            .unwrap();
        assert!(session.handle_event(&Event::Key(
            ratatui_crossterm::crossterm::event::KeyEvent::new(
                ratatui_crossterm::crossterm::event::KeyCode::End,
                KeyModifiers::NONE,
            ),
        )));
        let end = session.scroll.scroll_offset();
        assert!(end > 0);

        terminal
            .draw(|frame| {
                let geometry = session.render(frame, Rect::new(0, 0, 20, 0), &report, Locale::En);
                assert_eq!(geometry.rows, Rect::new(0, 0, 20, 0));
                assert_eq!(geometry.first_visible, end);
            })
            .unwrap();
        terminal
            .draw(|frame| {
                let geometry = session.render(frame, frame.area(), &report, Locale::En);
                assert_eq!(geometry.first_visible, end);
            })
            .unwrap();
    }

    #[test]
    fn a_distinct_report_starts_at_its_first_row_instead_of_reusing_the_old_tail() {
        let mut first = one_line_report(12);
        first.title = "First".to_owned();
        let mut second = one_line_report(12);
        second.title = "Second".to_owned();
        second.items[0].label = "SECOND-TOP".to_owned();
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        let mut session = ReportScreenSession::default();
        terminal
            .draw(|frame| {
                let _ = session.render(frame, frame.area(), &first, Locale::En);
            })
            .unwrap();
        assert!(session.handle_event(&Event::Key(
            ratatui_crossterm::crossterm::event::KeyEvent::new(
                ratatui_crossterm::crossterm::event::KeyCode::End,
                KeyModifiers::NONE,
            ),
        )));
        assert!(session.scroll.scroll_offset() > 0);

        terminal
            .draw(|frame| {
                let geometry = session.render(frame, frame.area(), &second, Locale::En);
                assert_eq!(geometry.first_visible, 0);
            })
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("SECOND-TOP"), "{rendered}");
    }
}
