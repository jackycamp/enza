//! Terminal line rendering helpers.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::core::diff::{DiffFile, DiffLine};
use crate::tui::highlight::{DiffKind, FileHighlighter};
use crate::tui::layout::text::{fit_text, format_lineno, pad_to_width, truncate_text};

const INLINE_CODE_INDENT: usize = 12;
const SIDE_BY_SIDE_CODE_INDENT: usize = 7;
const FILE_HEADER_BACKGROUND: Color = Color::Rgb(24, 30, 36);
const SELECTED_FILE_HEADER_BACKGROUND: Color = Color::Rgb(35, 42, 50);

/// Builds the horizontal separator row before a file header.
pub fn file_separator_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(8)),
        Style::default().fg(Color::DarkGray),
    ))
}

/// File labels and counts shared by all cached header variants.
pub(super) struct FileHeaderRenderer<'a> {
    inline_label: &'a str,
    side_by_side_label: &'a str,
    additions: usize,
    deletions: usize,
    viewed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FileHeaderSelection {
    Normal,
    Selected,
}

impl<'a> FileHeaderRenderer<'a> {
    pub fn new(file: &'a DiffFile) -> Self {
        let (additions, deletions) = file.change_counts();
        Self {
            inline_label: if file.new_path != "/dev/null" {
                &file.new_path
            } else {
                &file.old_path
            },
            side_by_side_label: &file.path,
            additions,
            deletions,
            viewed: false,
        }
    }

    pub fn with_viewed_marker(mut self) -> Self {
        self.viewed = true;
        self
    }

    pub fn inline(&self, width: usize, selection: FileHeaderSelection) -> Line<'static> {
        self.render(self.inline_label, width, selection)
    }

    pub fn side_by_side(&self, width: usize, selection: FileHeaderSelection) -> Line<'static> {
        self.render(self.side_by_side_label, width, selection)
    }

    fn render(&self, label: &str, width: usize, selection: FileHeaderSelection) -> Line<'static> {
        let (additions, deletions) = (self.additions, self.deletions);
        let background = if selection == FileHeaderSelection::Selected {
            SELECTED_FILE_HEADER_BACKGROUND
        } else {
            FILE_HEADER_BACKGROUND
        };
        let title_style = if selection == FileHeaderSelection::Selected {
            Style::default()
                .fg(Color::White)
                .bg(background)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Gray)
                .bg(background)
                .add_modifier(Modifier::BOLD)
        };

        let additions_style = Style::default()
            .fg(Color::Green)
            .bg(background)
            .add_modifier(Modifier::BOLD);
        let deletions_style = Style::default()
            .fg(Color::Red)
            .bg(background)
            .add_modifier(Modifier::BOLD);
        let divider_style = Style::default().fg(Color::DarkGray).bg(background);

        let suffix = format!("+{additions}, -{deletions}");
        let marker = if self.viewed { " ✓" } else { "" };
        let available_label_width =
            width.saturating_sub(suffix.chars().count() + marker.chars().count() + 2);
        let label = fit_text(&format!(" {label}"), available_label_width.max(1))
            .trim_end()
            .to_string();
        let rendered_width =
            label.chars().count() + marker.chars().count() + suffix.chars().count() + 2;
        let trailing = " ".repeat(width.saturating_sub(rendered_width));

        Line::from(vec![
            Span::styled(marker, additions_style),
            Span::styled(label, title_style),
            Span::styled("  ".to_string(), divider_style),
            Span::styled(format!("+{additions}"), additions_style),
            Span::styled(", ".to_string(), divider_style),
            Span::styled(format!("-{deletions}"), deletions_style),
            Span::styled(trailing, divider_style),
        ])
    }
}

/// Builds the inline-mode hunk header line.
pub fn hunk_header_line(header: &str, _selected: bool) -> Line<'static> {
    let style = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled(" ".repeat(INLINE_CODE_INDENT), Style::default()),
        Span::styled(display_hunk_header(header), style),
    ])
}

/// Builds the side-by-side hunk header with the center gutter preserved.
pub fn side_by_side_hunk_header_line(header: &str, _selected: bool, width: usize) -> Line<'static> {
    let style = Style::default().fg(Color::DarkGray);
    let divider_style = Style::default().fg(Color::DarkGray);
    let (left_width, right_width) = split_side_by_side_width(width);
    let left = fit_text(
        &format!(
            "{}{}",
            " ".repeat(SIDE_BY_SIDE_CODE_INDENT),
            display_hunk_header(header)
        ),
        left_width,
    );
    let right = fit_text("", right_width);

    Line::from(vec![
        Span::styled(left, style),
        Span::styled(" │ ".to_string(), divider_style),
        Span::styled(right, style),
    ])
}

fn display_hunk_header(header: &str) -> String {
    let Some(rest) = header.strip_prefix("@@ ") else {
        return header.to_string();
    };
    let Some((_, context)) = rest.split_once(" @@") else {
        return header.to_string();
    };

    context.trim().to_string()
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

struct InlineDiffStyle<'a> {
    prefix: &'static str,
    old_lineno: Option<usize>,
    new_lineno: Option<usize>,
    text: &'a str,
    color: Option<Color>,
    diff_kind: DiffKind,
}

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

#[cfg(test)]
mod tests {
    use super::{
        FILE_HEADER_BACKGROUND, FileHeaderRenderer, FileHeaderSelection, INLINE_CODE_INDENT,
        SELECTED_FILE_HEADER_BACKGROUND, SIDE_BY_SIDE_CODE_INDENT, display_hunk_header,
        hunk_header_line, side_by_side_hunk_header_line,
    };
    use crate::core::diff::{DiffFile, DiffHunk, DiffLine};
    use ratatui::text::Line;

    #[test]
    fn hunk_headers_render_only_trailing_context() {
        assert_eq!(display_hunk_header("@@ -12,7 +12,9 @@ fn main"), "fn main");
        assert_eq!(display_hunk_header("@@ -0,0 +1,3 @@"), "");
        assert_eq!(display_hunk_header("@@ -4 +4 @@"), "");
    }

    #[test]
    fn hunk_headers_align_context_with_code_column() {
        assert_eq!(
            plain_text(hunk_header_line("@@ -12,7 +12,9 @@ fn main", false)),
            format!("{}fn main", " ".repeat(INLINE_CODE_INDENT))
        );

        let side_by_side = plain_text(side_by_side_hunk_header_line(
            "@@ -12,7 +12,9 @@ fn main",
            false,
            40,
        ));
        assert!(
            side_by_side.starts_with(&format!("{}fn main", " ".repeat(SIDE_BY_SIDE_CODE_INDENT)))
        );
    }

    #[test]
    fn selected_hunk_headers_do_not_change_text_style() {
        let normal = hunk_header_line("@@ -12,7 +12,9 @@ fn main", false);
        let selected = hunk_header_line("@@ -12,7 +12,9 @@ fn main", true);
        assert_eq!(normal, selected);

        let normal_side = side_by_side_hunk_header_line("@@ -12,7 +12,9 @@ fn main", false, 40);
        let selected_side = side_by_side_hunk_header_line("@@ -12,7 +12,9 @@ fn main", true, 40);
        assert_eq!(normal_side, selected_side);
    }

    #[test]
    fn file_headers_apply_background_to_every_span() {
        let file = sample_file();
        let normal = FileHeaderRenderer::new(&file).inline(40, FileHeaderSelection::Normal);
        assert!(
            normal
                .spans
                .iter()
                .all(|span| span.style.bg == Some(FILE_HEADER_BACKGROUND))
        );

        let selected = FileHeaderRenderer::new(&file).inline(40, FileHeaderSelection::Selected);
        assert!(
            selected
                .spans
                .iter()
                .all(|span| span.style.bg == Some(SELECTED_FILE_HEADER_BACKGROUND))
        );
    }

    #[test]
    fn file_header_variants_preserve_labels_counts_marker_and_width() {
        let mut file = sample_file();
        file.path = "old.rs → new.rs".into();
        file.new_path = "new.rs".into();
        file.old_path = "old.rs".into();
        let renderer = FileHeaderRenderer::new(&file).with_viewed_marker();
        assert!(
            plain_text(renderer.inline(40, FileHeaderSelection::Normal)).starts_with(" ✓ new.rs")
        );
        assert!(
            plain_text(renderer.side_by_side(40, FileHeaderSelection::Normal))
                .starts_with(" ✓ old.rs → new.rs")
        );
        for selection in [FileHeaderSelection::Normal, FileHeaderSelection::Selected] {
            let background = if selection == FileHeaderSelection::Selected {
                SELECTED_FILE_HEADER_BACKGROUND
            } else {
                FILE_HEADER_BACKGROUND
            };
            for width in [16, 32, 80] {
                for line in [
                    renderer.inline(width, selection),
                    renderer.side_by_side(width, selection),
                ] {
                    assert_eq!(line.width(), width);
                    assert!(
                        line.spans
                            .iter()
                            .all(|span| span.style.bg == Some(background))
                    );
                    assert_eq!(line.spans[0].style.fg, Some(ratatui::style::Color::Green));
                    assert!(plain_text(line).contains("+1, -1"));
                }
            }
        }
        file.new_path = "/dev/null".into();
        assert!(
            plain_text(FileHeaderRenderer::new(&file).inline(40, FileHeaderSelection::Normal))
                .starts_with(" old.rs")
        );
    }

    fn plain_text(line: Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn sample_file() -> DiffFile {
        DiffFile {
            path: "src/main.rs".to_string(),
            old_path: "src/main.rs".to_string(),
            new_path: "src/main.rs".to_string(),
            hunks: vec![DiffHunk {
                header: "@@ -1,1 +1,1 @@".to_string(),
                lines: vec![
                    DiffLine::Removed {
                        old_lineno: 1,
                        text: "old".to_string(),
                    },
                    DiffLine::Added {
                        new_lineno: 1,
                        text: "new".to_string(),
                    },
                ],
            }],
        }
    }
}
