use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::diff::{DiffFile, DiffLine, DiffSession, FileChangeKind};
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
    pub side_width: usize,
    pub inline_rows: Vec<RenderRow>,
    pub old_rows: Vec<RenderRow>,
    pub new_rows: Vec<RenderRow>,
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

#[derive(Clone, Copy)]
enum DiffSide {
    Old,
    New,
}

impl RenderSession {
    pub fn build(session: &DiffSession, inline_width: usize, side_width: usize) -> Self {
        let mut inline_rows = Vec::new();
        let mut old_rows = Vec::new();
        let mut new_rows = Vec::new();
        let mut hunk_ranges = Vec::new();
        let mut cursor = 0usize;

        for (file_index, file) in session.files.iter().enumerate() {
            let mut highlighter = FileHighlighter::new(&file.path);

            let inline_separator = file_separator_line(inline_width);
            let side_separator = file_separator_line(side_width);
            inline_rows.push(RenderRow::Static(inline_separator.clone()));
            old_rows.push(RenderRow::Static(side_separator.clone()));
            new_rows.push(RenderRow::Static(side_separator));

            inline_rows.push(file_header_row(
                file_index,
                file_header_line(file, false, inline_width),
                file_header_line(file, true, inline_width),
            ));
            old_rows.push(file_header_row(
                file_index,
                file_side_header_line(file, false, side_width, DiffSide::Old),
                file_side_header_line(file, true, side_width, DiffSide::Old),
            ));
            new_rows.push(file_header_row(
                file_index,
                file_side_header_line(file, false, side_width, DiffSide::New),
                file_side_header_line(file, true, side_width, DiffSide::New),
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
                old_rows.push(hunk_header_row(
                    file_index,
                    hunk_index,
                    hunk_header_line(&hunk.header, false),
                    hunk_header_line(&hunk.header, true),
                ));
                new_rows.push(hunk_header_row(
                    file_index,
                    hunk_index,
                    hunk_header_line(&hunk.header, false),
                    hunk_header_line(&hunk.header, true),
                ));

                for diff_line in &hunk.lines {
                    inline_rows.push(RenderRow::Static(build_inline_line(
                        diff_line,
                        inline_width,
                        &mut highlighter,
                    )));
                    old_rows.push(RenderRow::Static(build_side_line(
                        diff_line,
                        side_width,
                        DiffSide::Old,
                        &mut highlighter,
                    )));
                    new_rows.push(RenderRow::Static(build_side_line(
                        diff_line,
                        side_width,
                        DiffSide::New,
                        &mut highlighter,
                    )));
                }

                inline_rows.push(RenderRow::Static(Line::default()));
                old_rows.push(RenderRow::Static(Line::default()));
                new_rows.push(RenderRow::Static(Line::default()));

                cursor += 1 + hunk.lines.len() + 1;
                hunk_ranges.push(HunkRange {
                    file_index,
                    hunk_index,
                    start,
                    end: cursor,
                });
            }

            inline_rows.push(RenderRow::Static(Line::default()));
            old_rows.push(RenderRow::Static(Line::default()));
            new_rows.push(RenderRow::Static(Line::default()));
            cursor += 1;
        }

        Self {
            inline_width,
            side_width,
            inline_rows,
            old_rows,
            new_rows,
            hunk_ranges,
        }
    }

    pub fn line_count_for_mode(&self, side_by_side: bool) -> usize {
        if side_by_side {
            self.old_rows.len()
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

fn build_side_line(
    diff_line: &DiffLine,
    width: usize,
    side: DiffSide,
    highlighter: &mut FileHighlighter<'static>,
) -> Line<'static> {
    match diff_line {
        DiffLine::Context {
            old_lineno,
            new_lineno,
            text,
        } => match side {
            DiffSide::Old => highlighted_side_line(
                " ",
                Some(*old_lineno),
                text,
                width,
                None,
                highlighter,
                DiffKind::Context,
            ),
            DiffSide::New => highlighted_side_line(
                " ",
                Some(*new_lineno),
                text,
                width,
                None,
                highlighter,
                DiffKind::Context,
            ),
        },
        DiffLine::Added { new_lineno, text } => match side {
            DiffSide::Old => side_line(" ", None, "", width, Some(Color::DarkGray)),
            DiffSide::New => highlighted_side_line(
                "+",
                Some(*new_lineno),
                text,
                width,
                Some(Color::Green),
                highlighter,
                DiffKind::Added,
            ),
        },
        DiffLine::Removed { old_lineno, text } => match side {
            DiffSide::Old => highlighted_side_line(
                "-",
                Some(*old_lineno),
                text,
                width,
                Some(Color::Red),
                highlighter,
                DiffKind::Removed,
            ),
            DiffSide::New => side_line(" ", None, "", width, Some(Color::DarkGray)),
        },
    }
}

fn file_header_line(file: &DiffFile, selected: bool, width: usize) -> Line<'static> {
    let status = match file.change_kind() {
        FileChangeKind::Added => "added",
        FileChangeKind::Modified => "modified",
    };

    chrome_line(width, &file.path, status, selected)
}

fn file_side_header_line(
    file: &DiffFile,
    selected: bool,
    width: usize,
    side: DiffSide,
) -> Line<'static> {
    let status = match file.change_kind() {
        FileChangeKind::Added => "added",
        FileChangeKind::Modified => "modified",
    };

    match side {
        DiffSide::Old => shared_chrome_left(width, &file.path, status, selected),
        DiffSide::New => shared_chrome_right(width, selected),
    }
}

fn file_separator_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(8)),
        Style::default().fg(Color::DarkGray),
    ))
}

fn chrome_line(width: usize, label: &str, badge: &str, selected: bool) -> Line<'static> {
    let title_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD)
    };
    let badge_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    };
    let chrome_style = Style::default().fg(Color::DarkGray);
    let action_style = Style::default().fg(Color::DarkGray);

    let left = format!(" {label}  ");
    let badge = format!("[{badge}]");
    let actions = "[note] [read]";
    let used = left.chars().count() + badge.chars().count() + 2 + actions.chars().count();
    let spacer = " ".repeat(width.saturating_sub(used));

    Line::from(vec![
        Span::styled(left, title_style),
        Span::styled(badge, badge_style),
        Span::styled("  ".to_string(), chrome_style),
        Span::styled(spacer, chrome_style),
        Span::styled(actions.to_string(), action_style),
    ])
}

fn shared_chrome_left(width: usize, label: &str, badge: &str, selected: bool) -> Line<'static> {
    let title_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD)
    };
    let badge_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    };
    let chrome_style = Style::default().fg(Color::DarkGray);

    let left = format!("▌ {label}  ");
    let badge = format!("[{badge}]");
    let used = left.chars().count() + badge.chars().count();
    let spacer = " ".repeat(width.saturating_sub(used));

    Line::from(vec![
        Span::styled(left, title_style),
        Span::styled(badge, badge_style),
        Span::styled(spacer, chrome_style),
    ])
}

fn shared_chrome_right(width: usize, _selected: bool) -> Line<'static> {
    let chrome_style = Style::default().fg(Color::DarkGray);
    let actions = "[note] [read]";
    let spacer = " ".repeat(width.saturating_sub(actions.chars().count()));

    Line::from(vec![
        Span::styled(spacer, chrome_style),
        Span::styled(actions.to_string(), chrome_style),
    ])
}

fn hunk_header_line(header: &str, selected: bool) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::Yellow)
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
    if let Some(background) = background {
        if rendered_width < width {
            spans.push(Span::styled(
                " ".repeat(width - rendered_width),
                Style::default().bg(background),
            ));
        }
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

    Line::from(Span::styled(pad_to_width(&body, width), style))
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

fn format_lineno(lineno: Option<usize>) -> String {
    lineno
        .map(|value| value.to_string())
        .unwrap_or_else(|| "·".to_string())
}
