//! Paint a visible row band without materializing the hidden rows.

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style, text::Line, widgets::Widget};
use ratatui_widgets::{block::Block, borders::Borders, paragraph::Paragraph};

/// One visible band from a taller virtual item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RowClip {
    full_height: usize,
    top: u16,
    area: Rect,
}

impl RowClip {
    /// Describe the visible band of an item with its complete virtual height.
    pub(crate) const fn new(full_height: usize, top: u16, area: Rect) -> Self {
        Self {
            full_height,
            top,
            area,
        }
    }

    /// Return the terminal rectangle that receives the band.
    pub(crate) const fn area(self) -> Rect {
        self.area
    }

    /// Return the first source row in the band.
    pub(crate) const fn top(self) -> usize {
        self.top as usize
    }

    /// Report whether the complete item is visible.
    pub(crate) fn is_full(self) -> bool {
        self.top == 0 && usize::from(self.area.height) >= self.full_height
    }

    /// Iterate only the source rows that survive, paired with their terminal rows.
    pub(crate) fn rows(self) -> impl Iterator<Item = (usize, Rect)> {
        let start = self.top();
        let end = (start + usize::from(self.area.height)).min(self.full_height);
        (start..end).zip(self.area.rows())
    }

    /// Return the terminal row for one source row when it survives.
    pub(crate) fn row(self, source: usize) -> Option<Rect> {
        self.rows()
            .find_map(|(candidate, area)| (candidate == source).then_some(area))
    }

    /// Paint a wrapped paragraph at the source offset of this band.
    pub(crate) fn paint_paragraph(self, buffer: &mut Buffer, paragraph: Paragraph<'_>) {
        paragraph.scroll((self.top, 0)).render(self.area, buffer);
    }

    /// Paint a bordered paragraph while keeping its border rows in virtual-row order.
    pub(crate) fn paint_bordered_paragraph(
        self,
        buffer: &mut Buffer,
        paragraph: Paragraph<'_>,
        title: Line<'_>,
        border_style: Style,
        horizontal_scroll: u16,
    ) {
        if let Some(top) = self.row(0) {
            Block::default()
                .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                .border_style(border_style)
                .title(title)
                .render(top, buffer);
        }

        let content_start = self.top().max(1);
        let content_end = (self.top() + usize::from(self.area.height)).min(self.full_height - 1);
        if content_start < content_end {
            let content_rows = content_end - content_start;
            let area = self
                .rows()
                .find_map(|(source, area)| (source == content_start).then_some(area))
                .expect("the content band starts on a visible row");
            let sides = Rect::new(
                area.x,
                area.y,
                area.width,
                u16::try_from(content_rows).expect("the content band fits its terminal area"),
            );
            let block = Block::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .border_style(border_style);
            let inner = block.inner(sides);
            block.render(sides, buffer);
            paragraph
                .scroll((
                    u16::try_from(content_start - 1)
                        .expect("the paragraph offset fits Ratatui's row offset"),
                    horizontal_scroll,
                ))
                .render(inner, buffer);
        }

        if let Some(bottom) = self.row(self.full_height - 1) {
            Block::default()
                .borders(Borders::BOTTOM | Borders::LEFT | Borders::RIGHT)
                .border_style(border_style)
                .render(bottom, buffer);
        }
    }
}
