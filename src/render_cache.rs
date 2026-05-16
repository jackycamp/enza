use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::diff::{DiffFile, DiffLine, DiffSession};
use crate::highlight::{DiffKind, FileHighlighter};

#[derive(Clone, Debug)]
pub struct HunkRange {
    pub file_index: usize,
    pub hunk_index: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
pub struct RenderSession {
    pub inline_width: usize,
    pub side_by_side_width: usize,
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub hunk_ranges: Vec<HunkRange>,
}

#[derive(Clone, Debug)]
pub enum RenderRow {
    Static(Line<'static>),
    FileHeader {
        file_index: usize,
        normal: Line<'static>,
        selected: Line<'static>,
    },
    HunkHeader {
        file_index: usize,
        hunk_index: usize,
        normal: Line<'static>,
        selected: Line<'static>,
    },
}

impl RenderSession {
    pub fn build(session: &DiffSession, inline_width: usize, side_by_side_width: usize) -> Self {
        let mut inline_rows = Vec::new();
        let mut side_by_side_rows = Vec::new();
        let mut hunk_ranges = Vec::new();
        let mut cursor = 0usize;

        for (file_index, file) in session.files.iter().enumerate() {
            let mut highlighter = FileHighlighter::new(&file.path);

            let inline_separator = file_separator_line(inline_width);
            let side_separator = file_separator_line(side_by_side_width);
            inline_rows.push(RenderRow::Static(inline_separator.clone()));
            side_by_side_rows.push(RenderRow::Static(side_separator));

            inline_rows.push(file_header_row(
                file_index,
                file_header_line(file, false, inline_width),
                file_header_line(file, true, inline_width),
            ));
            side_by_side_rows.push(file_header_row(
                file_index,
                file_side_by_side_header_line(file, false, side_by_side_width),
                file_side_by_side_header_line(file, true, side_by_side_width),
            ));

            cursor += 2;

            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                let start = cursor;

                inline_rows.push(hunk_header_row(
                    file_index,
                    hunk_index,
                    hunk_header_line(&hunk.header, false),
                    hunk_header_line(&hunk.header, true),
                ));
                side_by_side_rows.push(hunk_header_row(
                    file_index,
                    hunk_index,
                    side_by_side_hunk_header_line(&hunk.header, false, side_by_side_width),
                    side_by_side_hunk_header_line(&hunk.header, true, side_by_side_width),
                ));

                for diff_line in &hunk.lines {
                    inline_rows.push(RenderRow::Static(build_inline_line(
                        diff_line,
                        inline_width,
                        &mut highlighter,
                    )));
                    side_by_side_rows.push(RenderRow::Static(build_combined_side_line(
                        diff_line,
                        side_by_side_width,
                        &mut highlighter,
                    )));
                }

                inline_rows.push(RenderRow::Static(Line::default()));
                side_by_side_rows.push(RenderRow::Static(Line::default()));

                cursor += 1 + hunk.lines.len() + 1;
                hunk_ranges.push(HunkRange {
                    file_index,
                    hunk_index,
                    start,
                    end: cursor,
                });
            }

            inline_rows.push(RenderRow::Static(Line::default()));
            side_by_side_rows.push(RenderRow::Static(Line::default()));
            cursor += 1;
        }

        Self {
            inline_width,
            side_by_side_width,
            inline_rows,
            side_by_side_rows,
            hunk_ranges,
        }
    }

    pub fn line_count_for_mode(&self, side_by_side: bool) -> usize {
        if side_by_side {
            self.side_by_side_rows.len()
        } else {
            self.inline_rows.len()
        }
    }
}

pub fn materialize_rows(
    rows: &[RenderRow],
    scroll: u16,
    selected_file: usize,
    selected_hunk: usize,
) -> Vec<Line<'static>> {
    rows.iter()
        .skip(scroll as usize)
        .map(|row| match row {
            RenderRow::Static(line) => line.clone(),
            RenderRow::FileHeader {
                file_index,
                normal,
                selected,
            } => {
                if *file_index == selected_file {
                    selected.clone()
                } else {
                    normal.clone()
                }
            }
            RenderRow::HunkHeader {
                file_index,
                hunk_index,
                normal,
                selected,
            } => {
                if *file_index == selected_file && *hunk_index == selected_hunk {
                    selected.clone()
                } else {
                    normal.clone()
                }
            }
        })
        .collect()
}

fn file_header_row(file_index: usize, normal: Line<'static>, selected: Line<'static>) -> RenderRow {
    RenderRow::FileHeader {
        file_index,
        normal,
        selected,
    }
}

fn hunk_header_row(
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

fn build_inline_line(
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

fn build_combined_side_line(
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

fn file_header_line(file: &DiffFile, selected: bool, width: usize) -> Line<'static> {
    let label = if file.new_path != "/dev/null" {
        file.new_path.as_str()
    } else {
        file.old_path.as_str()
    };

    chrome_line(width, label, file, selected)
}

fn file_side_by_side_header_line(file: &DiffFile, selected: bool, width: usize) -> Line<'static> {
    chrome_line(width, &file.path, file, selected)
}

fn file_separator_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(8)),
        Style::default().fg(Color::DarkGray),
    ))
}

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

fn side_by_side_hunk_header_line(header: &str, selected: bool, width: usize) -> Line<'static> {
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

fn combined_side_line(left: Line<'static>, right: Line<'static>) -> Line<'static> {
    let divider_style = Style::default().fg(Color::DarkGray);
    let mut spans = left.spans;
    spans.push(Span::styled(" │ ".to_string(), divider_style));
    spans.extend(right.spans);
    Line::from(spans)
}

fn split_side_by_side_width(width: usize) -> (usize, usize) {
    let gutter = 3;
    let usable = width.saturating_sub(gutter);
    let left = usable / 2;
    let right = usable.saturating_sub(left);
    (left, right)
}

fn hunk_header_line(header: &str, selected: bool) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Line::from(Span::styled(header.to_string(), style))
}

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

fn diff_background(diff_kind: DiffKind) -> Option<Color> {
    match diff_kind {
        DiffKind::Context => None,
        DiffKind::Added => Some(Color::Rgb(18, 48, 24)),
        DiffKind::Removed => Some(Color::Rgb(60, 24, 24)),
    }
}

fn truncate_text(text: &str, max_width: usize) -> String {
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }

    let mut truncated = String::new();
    for ch in text.chars().take(max_width - 1) {
        truncated.push(ch);
    }
    truncated.push('…');
    truncated
}

fn pad_to_width(text: &str, width: usize) -> String {
    let current = text.chars().count();
    if current >= width {
        return text.to_string();
    }

    format!("{text}{:width$}", "", width = width - current)
}

fn fit_text(text: &str, width: usize) -> String {
    pad_to_width(&truncate_text(text, width), width)
}

fn format_lineno(lineno: Option<usize>) -> String {
    lineno
        .map(|value| value.to_string())
        .unwrap_or_else(|| "·".to_string())
}
