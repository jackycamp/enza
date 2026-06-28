//! Viewport row rendering.
//!
//! Rendering starts at `RowViewState.scroll` and asks for only the rows that fit
//! on screen. Base rows come from `LayoutPlan` and loaded hunk rows; note rows
//! come from inserted note rows. Avoid APIs here that rebuild all rows just to
//! draw one frame.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::diff::{DiffFile, DiffLine, DiffSession};
use crate::highlight::{DiffKind, FileHighlighter};
use crate::layout::model::{Layout, LayoutRowLocation, RenderRow, RowViewState};
use crate::layout::plan::plan_row_to_render_rows;
use crate::layout::text::{fit_text, format_lineno, pad_to_width, truncate_text};

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

/// Builds the horizontal separator row before a file header.
pub fn file_separator_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(8)),
        Style::default().fg(Color::DarkGray),
    ))
}

/// Wraps normal and selected file header lines in a selectable render row.
pub fn file_header_row(
    file_index: usize,
    normal: Line<'static>,
    selected: Line<'static>,
) -> RenderRow {
    RenderRow::FileHeader {
        file_index,
        normal,
        selected,
    }
}

/// Wraps normal and selected hunk header lines in a selectable render row.
pub fn hunk_header_row(
    file_index: usize,
    hunk_index: usize,
    normal: Line<'static>,
    selected: Line<'static>,
) -> RenderRow {
    RenderRow::HunkHeader {
        file_index,
        hunk_index,
        normal,
        selected,
    }
}

/// Renders one diff line in unified/inline mode.
pub fn build_inline_line(
    diff_line: &DiffLine,
    width: usize,
    highlighter: &mut FileHighlighter<'static>,
) -> Line<'static> {
    match diff_line {
        DiffLine::Context {
            old_lineno,
            new_lineno,
            text,
        } => highlighted_prefixed_line(
            " ",
            Some(*old_lineno),
            Some(*new_lineno),
            text,
            None,
            width,
            highlighter,
            DiffKind::Context,
        ),
        DiffLine::Added { new_lineno, text } => highlighted_prefixed_line(
            "+",
            None,
            Some(*new_lineno),
            text,
            Some(Color::Green),
            width,
            highlighter,
            DiffKind::Added,
        ),
        DiffLine::Removed { old_lineno, text } => highlighted_prefixed_line(
            "-",
            Some(*old_lineno),
            None,
            text,
            Some(Color::Red),
            width,
            highlighter,
            DiffKind::Removed,
        ),
    }
}

/// Renders one diff line in side-by-side mode.
pub fn build_combined_side_line(
    diff_line: &DiffLine,
    width: usize,
    highlighter: &mut FileHighlighter<'static>,
) -> Line<'static> {
    let (left_width, right_width) = split_side_by_side_width(width);

    match diff_line {
        DiffLine::Context {
            old_lineno,
            new_lineno,
            text,
        } => combined_side_line(
            highlighted_side_line(
                " ",
                Some(*old_lineno),
                text,
                left_width,
                None,
                highlighter,
                DiffKind::Context,
            ),
            highlighted_side_line(
                " ",
                Some(*new_lineno),
                text,
                right_width,
                None,
                highlighter,
                DiffKind::Context,
            ),
        ),
        DiffLine::Added { new_lineno, text } => combined_side_line(
            side_line(" ", None, "", left_width, Some(Color::DarkGray)),
            highlighted_side_line(
                "+",
                Some(*new_lineno),
                text,
                right_width,
                Some(Color::Green),
                highlighter,
                DiffKind::Added,
            ),
        ),
        DiffLine::Removed { old_lineno, text } => combined_side_line(
            highlighted_side_line(
                "-",
                Some(*old_lineno),
                text,
                left_width,
                Some(Color::Red),
                highlighter,
                DiffKind::Removed,
            ),
            side_line(" ", None, "", right_width, Some(Color::DarkGray)),
        ),
    }
}

/// Builds the inline-mode file header line.
pub fn file_header_line(file: &DiffFile, selected: bool, width: usize) -> Line<'static> {
    let label = if file.new_path != "/dev/null" {
        file.new_path.as_str()
    } else {
        file.old_path.as_str()
    };

    chrome_line(width, label, file, selected)
}

/// Builds the side-by-side-mode file header line.
pub fn file_side_by_side_header_line(
    file: &DiffFile,
    selected: bool,
    width: usize,
) -> Line<'static> {
    chrome_line(width, &file.path, file, selected)
}

/// Builds the side-by-side hunk header with the center gutter preserved.
pub fn side_by_side_hunk_header_line(header: &str, selected: bool, width: usize) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let divider_style = Style::default().fg(Color::DarkGray);
    let (left_width, right_width) = split_side_by_side_width(width);
    let left = fit_text(header, left_width);
    let right = fit_text("", right_width);

    Line::from(vec![
        Span::styled(left, style),
        Span::styled(" │ ".to_string(), divider_style),
        Span::styled(right, style),
    ])
}

/// Combines left and right side-by-side cells with the gutter divider.
pub fn combined_side_line(left: Line<'static>, right: Line<'static>) -> Line<'static> {
    let divider_style = Style::default().fg(Color::DarkGray);
    let mut spans = left.spans;
    spans.push(Span::styled(" │ ".to_string(), divider_style));
    spans.extend(right.spans);
    Line::from(spans)
}

/// Splits total side-by-side width into left and right content widths.
pub fn split_side_by_side_width(width: usize) -> (usize, usize) {
    let gutter = 3;
    let usable = width.saturating_sub(gutter);
    let left = usable / 2;
    let right = usable.saturating_sub(left);
    (left, right)
}

/// Builds the inline-mode hunk header line.
pub fn hunk_header_line(header: &str, selected: bool) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Line::from(Span::styled(header.to_string(), style))
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

/// Builds the shared file header line with file label and change counts.
fn chrome_line(width: usize, label: &str, file: &DiffFile, selected: bool) -> Line<'static> {
    let (additions, deletions) = file.change_counts();
    let title_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD)
    };

    let additions_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let deletions_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let chrome_style = Style::default().fg(Color::DarkGray);

    let suffix = format!("+{additions}, -{deletions}");
    let available_label_width = width.saturating_sub(suffix.chars().count());
    let label = fit_text(&format!(" {label}"), available_label_width.max(1))
        .trim_end()
        .to_string();
    let rendered_width = label.chars().count() + suffix.chars().count();
    let trailing = " ".repeat(width.saturating_sub(rendered_width));

    Line::from(vec![
        Span::styled(label, title_style),
        Span::styled("  ".to_string(), chrome_style),
        Span::styled(format!("+{additions}"), additions_style),
        Span::styled(", ".to_string(), chrome_style),
        Span::styled(format!("-{deletions}"), deletions_style),
        Span::styled(trailing, chrome_style),
    ])
}

/// Builds a unified diff line with prefix, line numbers, syntax highlighting, and padding.
fn highlighted_prefixed_line(
    prefix: &str,
    old_lineno: Option<usize>,
    new_lineno: Option<usize>,
    text: &str,
    color: Option<Color>,
    width: usize,
    highlighter: &mut FileHighlighter<'static>,
    diff_kind: DiffKind,
) -> Line<'static> {
    let background = diff_background(diff_kind);
    let style = color
        .map(|value| Style::default().fg(value))
        .map(|style| match background {
            Some(background) => style.bg(background),
            None => style,
        })
        .unwrap_or_default();
    let line_number_style = match background {
        Some(background) => Style::default().fg(Color::DarkGray).bg(background),
        None => Style::default().fg(Color::DarkGray),
    };
    let mut spans = vec![
        Span::styled(format!("{prefix:>1} "), style),
        Span::styled(
            format!("{:>4} ", format_lineno(old_lineno)),
            line_number_style,
        ),
        Span::styled(
            format!("{:>4} ", format_lineno(new_lineno)),
            line_number_style,
        ),
    ];
    let prefix_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    let available_width = width.saturating_sub(prefix_width);
    let mut code_spans = highlighter.highlight_line(text, diff_kind);
    let rendered_code_width: usize = code_spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum();
    if rendered_code_width > available_width {
        code_spans = vec![Span::styled(
            truncate_text(text, available_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        )];
    } else if rendered_code_width < available_width {
        code_spans.push(Span::styled(
            " ".repeat(available_width - rendered_code_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        ));
    }
    spans.extend(code_spans);
    let rendered_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    if let Some(background) = background
        && rendered_width < width
    {
        spans.push(Span::styled(
            " ".repeat(width - rendered_width),
            Style::default().bg(background),
        ));
    }
    Line::from(spans)
}

/// Builds an unhighlighted side-by-side cell, usually for an empty added/removed side.
fn side_line(
    prefix: &str,
    lineno: Option<usize>,
    text: &str,
    width: usize,
    color: Option<Color>,
) -> Line<'static> {
    let style = color
        .map(|value| Style::default().fg(value))
        .unwrap_or_default();
    let body = format!(
        "{:>4} {} {}",
        format_lineno(lineno),
        prefix,
        truncate_text(text, width.saturating_sub(8))
    );

    Line::from(Span::styled(fit_text(&body, width), style))
}

/// Builds one highlighted side-by-side cell with line number and prefix.
fn highlighted_side_line(
    prefix: &str,
    lineno: Option<usize>,
    text: &str,
    width: usize,
    color: Option<Color>,
    highlighter: &mut FileHighlighter<'static>,
    diff_kind: DiffKind,
) -> Line<'static> {
    let background = diff_background(diff_kind);
    let prefix_style = color
        .map(|value| Style::default().fg(value))
        .map(|style| match background {
            Some(background) => style.bg(background),
            None => style,
        })
        .unwrap_or_default();
    let line_number = format!("{:>4} {} ", format_lineno(lineno), prefix);
    let available_width = width.saturating_sub(line_number.chars().count());
    let mut spans = vec![Span::styled(
        pad_to_width(&line_number, line_number.chars().count()),
        prefix_style,
    )];
    let mut code_spans = highlighter.highlight_line(text, diff_kind);
    let rendered_code_width: usize = code_spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum();
    if rendered_code_width > available_width {
        code_spans = vec![Span::styled(
            truncate_text(text, available_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        )];
    } else if rendered_code_width < available_width {
        code_spans.push(Span::styled(
            " ".repeat(available_width - rendered_code_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        ));
    }

    spans.extend(code_spans);
    let rendered_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    if rendered_width < width {
        spans.push(Span::styled(
            " ".repeat(width - rendered_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        ));
    }

    Line::from(spans)
}

/// Returns the background tint for added and removed diff rows.
fn diff_background(diff_kind: DiffKind) -> Option<Color> {
    match diff_kind {
        DiffKind::Context => None,
        DiffKind::Added => Some(Color::Rgb(18, 48, 24)),
        DiffKind::Removed => Some(Color::Rgb(60, 24, 24)),
    }
}

impl DiffLine {
    /// Returns the old-file line number occupied by this diff line.
    pub fn old_lineno(&self) -> Option<usize> {
        match self {
            Self::Context { old_lineno, .. } | Self::Removed { old_lineno, .. } => {
                Some(*old_lineno)
            }
            Self::Added { .. } => None,
        }
    }

    /// Returns the new-file line number occupied by this diff line.
    pub fn new_lineno(&self) -> Option<usize> {
        match self {
            Self::Context { new_lineno, .. } | Self::Added { new_lineno, .. } => Some(*new_lineno),
            Self::Removed { .. } => None,
        }
    }
}
