//! Read-only health and diagnostic reports.

use ratatui_core::{layout::Rect, terminal::Frame, text::Line};
use ratatui_widgets::{
    block::Block,
    borders::Borders,
    paragraph::{Paragraph, Wrap},
};
use skit_i18n::{Locale, text};
use skit_ui::ReportView;

use crate::ViewGeometry;

pub(crate) fn render(
    frame: &mut Frame,
    area: Rect,
    report: &ReportView,
    locale: Locale,
) -> ViewGeometry {
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
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL)),
        area,
    );
    ViewGeometry::default()
}
