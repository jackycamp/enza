//! Terminal line rendering helpers.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::diff::{DiffFile, DiffLine};
use crate::highlight::{DiffKind, FileHighlighter};
use crate::layout::text::{fit_text, format_lineno, pad_to_width, truncate_text};

/// Builds the horizontal separator row before a file header.
pub fn file_separator_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(8)),
        Style::default().fg(Color::DarkGray),
    ))
}

/// Builds the inline-mode file header line.
pub fn file_header_line(file: &DiffFile, selected: bool, width: usize) -> Line<'static> {
    let label = if file.new_path != "/dev/null" {
        file.new_path.as_str()
    } else {
        file.old_path.as_str()
    };

    header_line(width, label, file, selected)
}

/// Builds the side-by-side-mode file header line.
pub fn file_side_by_side_header_line(
    file: &DiffFile,
    selected: bool,
    width: usize,
) -> Line<'static> {
    header_line(width, &file.path, file, selected)
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

pub(super) struct InlineDiffLine<'a> {
    diff_line: &'a DiffLine,
    width: usize,
}

impl<'a> InlineDiffLine<'a> {
    pub(super) fn new(diff_line: &'a DiffLine, width: usize) -> Self {
        Self { diff_line, width }
    }

    pub(super) fn render(self, highlighter: &mut FileHighlighter<'static>) -> Line<'static> {
        InlineDiffStyle::from(self.diff_line).render(self.width, highlighter)
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

/// Builds the shared file header line with file label and change counts.
fn header_line(width: usize, label: &str, file: &DiffFile, selected: bool) -> Line<'static> {
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
    let divider_style = Style::default().fg(Color::DarkGray);

    let suffix = format!("+{additions}, -{deletions}");
    let available_label_width = width.saturating_sub(suffix.chars().count());
    let label = fit_text(&format!(" {label}"), available_label_width.max(1))
        .trim_end()
        .to_string();
    let rendered_width = label.chars().count() + suffix.chars().count();
    let trailing = " ".repeat(width.saturating_sub(rendered_width));

    Line::from(vec![
        Span::styled(label, title_style),
        Span::styled("  ".to_string(), divider_style),
        Span::styled(format!("+{additions}"), additions_style),
        Span::styled(", ".to_string(), divider_style),
        Span::styled(format!("-{deletions}"), deletions_style),
        Span::styled(trailing, divider_style),
    ])
}

struct InlineDiffStyle<'a> {
    prefix: &'static str,
    old_lineno: Option<usize>,
    new_lineno: Option<usize>,
    text: &'a str,
    color: Option<Color>,
    diff_kind: DiffKind,
}

// FIXME: No docs?
impl<'a> From<&'a DiffLine> for InlineDiffStyle<'a> {
    fn from(diff_line: &'a DiffLine) -> Self {
        match diff_line {
            DiffLine::Context {
                old_lineno,
                new_lineno,
                text,
            } => Self {
                prefix: " ",
                old_lineno: Some(*old_lineno),
                new_lineno: Some(*new_lineno),
                text,
                color: None,
                diff_kind: DiffKind::Context,
            },
            DiffLine::Added { new_lineno, text } => Self {
                prefix: "+",
                old_lineno: None,
                new_lineno: Some(*new_lineno),
                text,
                color: Some(Color::Green),
                diff_kind: DiffKind::Added,
            },
            DiffLine::Removed { old_lineno, text } => Self {
                prefix: "-",
                old_lineno: Some(*old_lineno),
                new_lineno: None,
                text,
                color: Some(Color::Red),
                diff_kind: DiffKind::Removed,
            },
        }
    }
}

impl InlineDiffStyle<'_> {
    /// Builds a unified diff line with prefix, line numbers, syntax highlighting, and padding.
    fn render(self, width: usize, highlighter: &mut FileHighlighter<'static>) -> Line<'static> {
        let background = diff_background(self.diff_kind);
        let style = self
            .color
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
            Span::styled(format!("{:>1} ", self.prefix), style),
            Span::styled(
                format!("{:>4} ", format_lineno(self.old_lineno)),
                line_number_style,
            ),
            Span::styled(
                format!("{:>4} ", format_lineno(self.new_lineno)),
                line_number_style,
            ),
        ];
        let prefix_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
        let available_width = width.saturating_sub(prefix_width);
        let mut code_spans = highlighter.highlight_line(self.text, self.diff_kind);
        let rendered_code_width: usize = code_spans
            .iter()
            .map(|span| span.content.chars().count())
            .sum();
        if rendered_code_width > available_width {
            code_spans = vec![Span::styled(
                truncate_text(self.text, available_width),
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
