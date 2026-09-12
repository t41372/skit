//! Paint a visible row band without materializing the hidden rows.

use ratatui_core::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::Widget,
};
use ratatui_textarea::TextArea as RichTextArea;
use ratatui_widgets::{block::Block, borders::Borders, paragraph::Paragraph};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// Return the virtual screen row occupied by one editor cursor.
pub(crate) const fn editor_cursor_virtual_row(control_start: usize, cursor_row: usize) -> usize {
    control_start + 1 + cursor_row
}

/// One visible band from a taller virtual item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RowClip {
    full_height: usize,
    top: usize,
    area: Rect,
}

impl RowClip {
    /// Describe the visible band of an item with its complete virtual height.
    pub(crate) const fn new(full_height: usize, top: usize, area: Rect) -> Self {
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
        self.top
    }

    /// Return the complete virtual height of the item.
    pub(crate) const fn full_height(self) -> usize {
        self.full_height
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
        paragraph
            .scroll((u16::try_from(self.top).unwrap_or(u16::MAX), 0))
            .render(self.area, buffer);
    }

    /// Paint text that already contains only this clip's visible content lines.
    ///
    /// This keeps the scratch-free path independent of the virtual source offset. The caller can
    /// borrow or build at most `area.height` lines even when the source contains more rows than a
    /// terminal coordinate can represent.
    pub(crate) fn paint_bordered_lines(
        self,
        buffer: &mut Buffer,
        lines: Vec<Line<'static>>,
        title: Line<'_>,
        border_style: Style,
    ) {
        if let Some(top) = self.row(0) {
            Block::default()
                .borders(Borders::TOP.union(Borders::LEFT).union(Borders::RIGHT))
                .border_style(border_style)
                .title(title)
                .render(top, buffer);
        }

        let bottom_source = self.full_height.saturating_sub(1);
        let content_start = self.top().max(1);
        let content_sources = content_start..bottom_source;
        let content_rows = self
            .rows()
            .filter(|(source, _)| content_sources.contains(source))
            .count();
        if content_rows > 0 {
            let area = self
                .row(content_start)
                .expect("the content band starts on a visible row");
            let sides = Rect::new(
                area.x,
                area.y,
                area.width,
                u16::try_from(content_rows).expect("the content band fits its terminal area"),
            );
            let block = Block::default()
                .borders(Borders::LEFT.union(Borders::RIGHT))
                .border_style(border_style);
            let inner = block.inner(sides);
            block.render(sides, buffer);
            Paragraph::new(Text::from(lines)).render(inner, buffer);
        }

        if let Some(bottom) = self.row(bottom_source) {
            Block::default()
                .borders(Borders::BOTTOM.union(Borders::LEFT).union(Borders::RIGHT))
                .border_style(border_style)
                .render(bottom, buffer);
        }
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
                .borders(Borders::TOP.union(Borders::LEFT).union(Borders::RIGHT))
                .border_style(border_style)
                .title(title)
                .render(top, buffer);
        }

        let bottom_source = self.full_height.saturating_sub(1);
        let content_start = self.top().max(1);
        let content_sources = content_start..bottom_source;
        let content_rows = self
            .rows()
            .filter(|(source, _)| content_sources.contains(source))
            .count();
        if content_rows > 0 {
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
                .borders(Borders::LEFT.union(Borders::RIGHT))
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

        if let Some(bottom) = self.row(bottom_source) {
            Block::default()
                .borders(Borders::BOTTOM.union(Borders::LEFT).union(Borders::RIGHT))
                .border_style(border_style)
                .render(bottom, buffer);
        }
    }
}

/// Build only the terminal-width fragments of visible textarea rows.
///
/// The textarea keeps the complete value and edit history. This adapter reads at most the requested
/// logical rows and allocates at most `width` display cells for each result row. It also keeps the
/// native cursor and selection precedence.
pub(crate) fn bounded_textarea_lines(
    state: &RichTextArea<'_>,
    first_row: usize,
    row_count: usize,
    left_cell: usize,
    width: usize,
    selection_style: Style,
) -> Vec<Line<'static>> {
    let cursor = state.cursor();
    let context = TextAreaLineContext {
        cursor: (cursor.0, cursor.1),
        selection: state.selection_range(),
        left_cell,
        right_cell: left_cell.saturating_add(width),
        tab_length: usize::from(state.tab_length()),
        base_style: state.style(),
        cursor_line_style: state.cursor_line_style(),
        cursor_style: state.cursor_style(),
        selection_style,
    };
    state
        .lines()
        .iter()
        .enumerate()
        .skip(first_row)
        .take(row_count)
        .map(|(row, line)| visible_textarea_line(line, row, &context))
        .collect()
}

struct TextAreaLineContext {
    cursor: (usize, usize),
    selection: Option<((usize, usize), (usize, usize))>,
    left_cell: usize,
    right_cell: usize,
    tab_length: usize,
    base_style: Style,
    cursor_line_style: Style,
    cursor_style: Style,
    selection_style: Style,
}

fn visible_textarea_line(line: &str, row: usize, context: &TextAreaLineContext) -> Line<'static> {
    let base_style = if row == context.cursor.0 {
        context.base_style.patch(context.cursor_line_style)
    } else {
        context.base_style
    };
    let mut display_column = 0_usize;
    let mut spans = Vec::new();
    let mut graphemes = line.graphemes(true).peekable();
    let mut logical_column = 0_usize;
    let mut line_length = 0_usize;
    let mut reached_end = true;
    while let Some(grapheme) = graphemes.next() {
        let grapheme_start = logical_column;
        logical_column = logical_column.saturating_add(grapheme.chars().count());
        line_length = logical_column;
        let grapheme_width = if grapheme == "\t" && context.tab_length > 0 {
            context.tab_length - (display_column % context.tab_length)
        } else {
            grapheme.width()
        };
        let grapheme_end = display_column.saturating_add(grapheme_width);
        if grapheme_end > context.left_cell && display_column < context.right_cell {
            let visible_start = display_column.max(context.left_cell);
            let visible_end = grapheme_end.min(context.right_cell);
            let style =
                textarea_grapheme_style(row, grapheme_start, logical_column, context, base_style);
            let fully_visible = visible_start == display_column && visible_end == grapheme_end;
            if fully_visible && grapheme != "\t" {
                push_textarea_span(&mut spans, grapheme.to_owned(), style);
            } else if visible_start < visible_end {
                push_textarea_span(&mut spans, " ".repeat(visible_end - visible_start), style);
            }
        }
        display_column = grapheme_end;
        if display_column >= context.right_cell {
            reached_end = graphemes.peek().is_none();
            break;
        }
    }

    let end_style = if reached_end && context.cursor == (row, line_length) {
        Some(context.cursor_style)
    } else if reached_end
        && context
            .selection
            .is_some_and(|(start, end)| start <= (row, line_length) && (row, line_length) < end)
    {
        Some(context.selection_style)
    } else {
        None
    };
    if let Some(style) = end_style
        && display_column >= context.left_cell
        && display_column < context.right_cell
    {
        push_textarea_span(&mut spans, " ".to_owned(), style);
    }
    Line::from(spans)
}

fn textarea_grapheme_style(
    row: usize,
    start_column: usize,
    end_column: usize,
    context: &TextAreaLineContext,
    base_style: Style,
) -> Style {
    let start = (row, start_column);
    let end = (row, end_column);
    if context.cursor >= start && context.cursor < end {
        context.cursor_style
    } else if context
        .selection
        .is_some_and(|(selection_start, selection_end)| {
            selection_start < end && start < selection_end
        })
    {
        context.selection_style
    } else {
        base_style
    }
}

fn push_textarea_span(spans: &mut Vec<Span<'static>>, text: String, style: Style) {
    if let Some(previous) = spans.last_mut()
        && previous.style == style
    {
        previous.content.to_mut().push_str(&text);
    } else {
        spans.push(Span::styled(text, style));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::style::{Color, Modifier};
    use ratatui_textarea::CursorMove;

    fn row_text(buffer: &Buffer, row: u16) -> String {
        (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect()
    }

    #[test]
    fn exact_capacity_is_full_and_rows_stop_at_the_virtual_height() {
        let exact = RowClip::new(4, 0, Rect::new(0, 0, 8, 4));
        assert!(exact.is_full());
        assert_eq!(exact.rows().count(), 4);
        assert!(exact.row(3).is_some());
        assert!(exact.row(4).is_none());

        let oversized = RowClip::new(4, 0, Rect::new(0, 0, 8, 5));
        assert!(oversized.is_full());
        assert_eq!(oversized.rows().count(), 4);
        assert!(!RowClip::new(4, 1, Rect::new(0, 0, 8, 4)).is_full());
    }

    #[test]
    fn virtual_rows_above_terminal_coordinates_keep_a_terminal_sized_band() {
        let top = usize::from(u16::MAX) + 100;
        let clip = RowClip::new(top + 10, top, Rect::new(3, 4, 12, 3));
        let rows = clip.rows().collect::<Vec<_>>();

        assert_eq!(
            rows.iter().map(|(source, _)| *source).collect::<Vec<_>>(),
            [top, top + 1, top + 2]
        );
        assert_eq!(
            rows.iter().map(|(_, area)| *area).collect::<Vec<_>>(),
            [
                Rect::new(3, 4, 12, 1),
                Rect::new(3, 5, 12, 1),
                Rect::new(3, 6, 12, 1),
            ]
        );
    }

    #[test]
    fn one_visible_grapheme_does_not_allocate_one_span_per_codepoint() {
        let value = format!("a{}", "\u{301}".repeat(10_000));
        let mut state = RichTextArea::new(vec![value]);
        state.move_cursor(CursorMove::Head);

        let lines = bounded_textarea_lines(&state, 0, 1, 0, 4, Style::default().bg(Color::Blue));

        assert_eq!(lines.len(), 1);
        let span_count = lines[0].spans.len();
        assert_eq!(span_count, 1, "one terminal grapheme must use one span");
    }

    #[test]
    fn bordered_paragraph_paints_only_the_surviving_content_and_border_rows() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 3));
        Paragraph::new("SENTINEL").render(Rect::new(0, 2, 8, 1), &mut buffer);
        RowClip::new(4, 2, Rect::new(0, 0, 8, 3)).paint_bordered_paragraph(
            &mut buffer,
            Paragraph::new("A\nB"),
            Line::from("Field"),
            Style::default(),
            0,
        );

        assert!(row_text(&buffer, 0).contains('B'));
        assert!(row_text(&buffer, 1).contains('└'));
        assert_eq!(row_text(&buffer, 2), "SENTINEL");

        let mut top_clipped = Buffer::empty(Rect::new(0, 0, 8, 2));
        RowClip::new(4, 1, Rect::new(0, 0, 8, 2)).paint_bordered_paragraph(
            &mut top_clipped,
            Paragraph::new("A\nB"),
            Line::from("Field"),
            Style::default(),
            0,
        );
        assert!(row_text(&top_clipped, 0).contains('A'));
        assert!(row_text(&top_clipped, 1).contains('B'));
    }

    #[test]
    fn bordered_paragraph_keeps_every_visible_content_row_at_a_nonzero_offset() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 5));
        Paragraph::new("..........\n..........\n..........\n..........\n..........")
            .render(buffer.area, &mut buffer);

        RowClip::new(6, 2, Rect::new(1, 1, 8, 3)).paint_bordered_paragraph(
            &mut buffer,
            Paragraph::new("A\nB\nC\nD"),
            Line::from("Field"),
            Style::default(),
            0,
        );

        for (row, expected) in [(1, "B"), (2, "C"), (3, "D")] {
            assert_eq!(buffer[(1, row)].symbol(), "│");
            assert_eq!(buffer[(2, row)].symbol(), expected);
            assert_eq!(buffer[(8, row)].symbol(), "│");
            assert_eq!(buffer[(0, row)].symbol(), ".");
            assert_eq!(buffer[(9, row)].symbol(), ".");
        }
        assert_eq!(row_text(&buffer, 4), "..........");
    }

    #[test]
    fn bottom_border_does_not_inherit_the_hidden_content_style() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 4));
        RowClip::new(4, 0, buffer.area).paint_bordered_paragraph(
            &mut buffer,
            Paragraph::new("A\nB\nHIDDEN")
                .style(Style::default().bg(Color::Red).add_modifier(Modifier::BOLD)),
            Line::from("Field"),
            Style::default().fg(Color::Green),
            0,
        );

        let bottom = &buffer[(3, 3)];
        assert_eq!(bottom.symbol(), "─");
        assert_eq!(bottom.fg, Color::Green);
        assert_eq!(bottom.bg, Color::Reset);
        assert!(!bottom.modifier.contains(Modifier::BOLD));
    }
}
