use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::diff::DiffSession;
use crate::layout::model::{Layout, LayoutRowLocation, RenderRow, RowViewState};
use crate::layout::plan::plan_row_to_render_rows;

impl Layout {
    /// Renders the rows visible on screen.
    ///
    /// This is intentionally bounded by `max_rows`; it must not render the whole
    /// layout just to draw one frame.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let max_rows = 40;
    /// let rows = layout.materialize_rows(
    ///     &app.session,
    ///     true,
    ///     &row_view_state(&app, cursor_focused),
    ///     max_rows,
    /// );
    /// // -> Vec<Line<'static>> with rows.len() <= 40
    /// ```
    pub fn materialize_rows(
        &self,
        session: &DiffSession,
        side_by_side: bool,
        view: &RowViewState,
        max_rows: usize,
    ) -> Vec<Line<'static>> {
        let start = view.scroll as usize;
        let end = self.row_count.min(start.saturating_add(max_rows));

        (start..end)
            .map(|absolute_row| {
                let line = self.render_line_for_row(session, side_by_side, view, absolute_row);
                let in_selection = view
                    .selected_rows
                    .is_some_and(|(start, end)| absolute_row >= start && absolute_row <= end);

                if absolute_row == view.cursor_row {
                    highlight_cursor_line(line, view.cursor_focused, in_selection)
                } else if in_selection {
                    highlight_selected_line(line)
                } else {
                    line
                }
            })
            .collect()
    }

    /// Renders one absolute row index, applying selection-aware row variants.
    fn render_line_for_row(
        &self,
        session: &DiffSession,
        side_by_side: bool,
        view: &RowViewState,
        row_index: usize,
    ) -> Line<'static> {
        let row = match self.locate_row(row_index) {
            Some(LayoutRowLocation::Note {
                insertion_index,
                row_offset,
            }) => {
                let Some(insertion) = self.note_insertions.get(insertion_index) else {
                    return Line::default();
                };
                let rows = if side_by_side {
                    &insertion.side_by_side_rows
                } else {
                    &insertion.inline_rows
                };
                rows.get(row_offset).cloned()
            }
            Some(LayoutRowLocation::Base { base_index }) => {
                let (inline, side_by_side_row) = plan_row_to_render_rows(
                    session,
                    &self.base.tree,
                    &self.base.plan,
                    base_index,
                    self.side_by_side_width,
                );
                Some(if side_by_side {
                    side_by_side_row
                } else {
                    inline
                })
            }
            None => None,
        };

        match row.unwrap_or_else(|| RenderRow::Static(Line::default())) {
            RenderRow::Static(line) => line,
            RenderRow::FileHeader {
                file_index,
                normal,
                selected,
            } => {
                if file_index == view.selected_file {
                    selected
                } else {
                    normal
                }
            }
            RenderRow::HunkHeader {
                file_index,
                hunk_index,
                normal,
                selected,
            } => {
                if file_index == view.selected_file && hunk_index == view.selected_hunk {
                    selected
                } else {
                    normal
                }
            }
            RenderRow::Note(line) => line,
        }
    }
}

/// Applies cursor background styling to an already-rendered line.
fn highlight_cursor_line(line: Line<'static>, focused: bool, in_selection: bool) -> Line<'static> {
    let cursor_style = if focused {
        if in_selection {
            Style::default().bg(Color::Rgb(58, 58, 58))
        } else {
            Style::default().bg(Color::Rgb(46, 46, 46))
        }
    } else {
        Style::default().bg(Color::Rgb(34, 34, 34))
    };
    patch_line_background(line, cursor_style)
}

/// Applies selection background styling to an already-rendered line.
fn highlight_selected_line(line: Line<'static>) -> Line<'static> {
    patch_line_background(line, Style::default().bg(Color::Rgb(40, 40, 40)))
}

/// Patches every span in a line with the provided background style.
fn patch_line_background(line: Line<'static>, patch: Style) -> Line<'static> {
    let spans = line
        .spans
        .into_iter()
        .map(|span| {
            let style = span.style.patch(patch);
            Span::styled(span.content, style)
        })
        .collect::<Vec<_>>();

    Line::from(spans)
}
