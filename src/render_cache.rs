use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::diff::{DiffFile, DiffLine, DiffSession};
use crate::highlight::{DiffKind, FileHighlighter};
use crate::notes::{Note, NoteTarget};

#[derive(Clone, Debug)]
pub struct HunkRange {
    pub file_index: usize,
    pub hunk_index: usize,
    pub start: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RowContext {
    pub file_index: Option<usize>,
    pub hunk_index: Option<usize>,
    pub kind: RowKind,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    pub note_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RowKind {
    #[default]
    Separator,
    FileHeader,
    HunkHeader,
    DiffLine,
    Note,
    Spacer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoteSide {
    Full,
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub struct RenderSession {
    pub inline_width: usize,
    pub side_by_side_width: usize,
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub hunk_ranges: Vec<HunkRange>,
    pub row_contexts: Vec<RowContext>,
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
    Note(Line<'static>),
}

impl RenderSession {
    pub fn build(
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
        inline_width: usize,
        side_by_side_width: usize,
    ) -> Self {
        let mut base_inline_rows = Vec::new();
        let mut base_side_by_side_rows = Vec::new();
        let mut base_hunk_ranges = Vec::new();
        let mut base_row_contexts = Vec::new();
        let mut cursor = 0usize;

        for (file_index, file) in session.files.iter().enumerate() {
            let mut highlighter = FileHighlighter::new(&file.path);

            let inline_separator = file_separator_line(inline_width);
            let side_separator = file_separator_line(side_by_side_width);
            base_inline_rows.push(RenderRow::Static(inline_separator.clone()));
            base_side_by_side_rows.push(RenderRow::Static(side_separator));
            base_row_contexts.push(RowContext {
                file_index: Some(file_index),
                hunk_index: None,
                kind: RowKind::Separator,
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            });

            base_inline_rows.push(file_header_row(
                file_index,
                file_header_line(file, false, inline_width),
                file_header_line(file, true, inline_width),
            ));
            base_side_by_side_rows.push(file_header_row(
                file_index,
                file_side_by_side_header_line(file, false, side_by_side_width),
                file_side_by_side_header_line(file, true, side_by_side_width),
            ));
            base_row_contexts.push(RowContext {
                file_index: Some(file_index),
                hunk_index: None,
                kind: RowKind::FileHeader,
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            });

            cursor += 2;

            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                let start = cursor;

                base_inline_rows.push(hunk_header_row(
                    file_index,
                    hunk_index,
                    hunk_header_line(&hunk.header, false),
                    hunk_header_line(&hunk.header, true),
                ));
                base_side_by_side_rows.push(hunk_header_row(
                    file_index,
                    hunk_index,
                    side_by_side_hunk_header_line(&hunk.header, false, side_by_side_width),
                    side_by_side_hunk_header_line(&hunk.header, true, side_by_side_width),
                ));
                base_row_contexts.push(RowContext {
                    file_index: Some(file_index),
                    hunk_index: Some(hunk_index),
                    kind: RowKind::HunkHeader,
                    old_lineno: None,
                    new_lineno: None,
                    note_id: None,
                });

                for diff_line in &hunk.lines {
                    base_inline_rows.push(RenderRow::Static(build_inline_line(
                        diff_line,
                        inline_width,
                        &mut highlighter,
                    )));
                    base_side_by_side_rows.push(RenderRow::Static(build_combined_side_line(
                        diff_line,
                        side_by_side_width,
                        &mut highlighter,
                    )));
                    base_row_contexts.push(RowContext {
                        file_index: Some(file_index),
                        hunk_index: Some(hunk_index),
                        kind: RowKind::DiffLine,
                        old_lineno: diff_line.old_lineno(),
                        new_lineno: diff_line.new_lineno(),
                        note_id: None,
                    });
                }

                base_inline_rows.push(RenderRow::Static(Line::default()));
                base_side_by_side_rows.push(RenderRow::Static(Line::default()));
                base_row_contexts.push(RowContext {
                    file_index: Some(file_index),
                    hunk_index: Some(hunk_index),
                    kind: RowKind::Spacer,
                    old_lineno: None,
                    new_lineno: None,
                    note_id: None,
                });

                cursor += 1 + hunk.lines.len() + 1;
                base_hunk_ranges.push(HunkRange {
                    file_index,
                    hunk_index,
                    start,
                });
            }

            base_inline_rows.push(RenderRow::Static(Line::default()));
            base_side_by_side_rows.push(RenderRow::Static(Line::default()));
            base_row_contexts.push(RowContext {
                file_index: Some(file_index),
                hunk_index: None,
                kind: RowKind::Spacer,
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            });
            cursor += 1;
        }

        let note_anchors = build_note_anchors(session, notes, &base_row_contexts);
        let mut inline_rows = Vec::new();
        let mut side_by_side_rows = Vec::new();
        let mut row_contexts = Vec::new();
        let mut inserted_before_base = vec![0usize; base_row_contexts.len() + 1];
        let mut inserted_total = 0usize;
        let note_wrap_width = inline_width.min(side_by_side_width);

        for base_index in 0..base_row_contexts.len() {
            inserted_before_base[base_index] = inserted_total;
            for note in note_anchors
                .iter()
                .filter(|(anchor_index, _)| *anchor_index == base_index)
                .map(|(_, note)| note)
            {
                let expanded = expanded_note_ids.contains(&note.id);
                let note_side = note_side(note);
                let note_rows = build_note_rows(note, note_wrap_width, expanded);
                let inline_note_lines = render_note_rows(&note_rows, inline_width);
                let side_note_lines =
                    render_side_by_side_note_rows(&note_rows, side_by_side_width, note_side);
                let note_context = RowContext {
                    file_index: base_row_contexts[base_index].file_index,
                    hunk_index: base_row_contexts[base_index].hunk_index,
                    kind: RowKind::Note,
                    old_lineno: None,
                    new_lineno: None,
                    note_id: Some(note.id),
                };

                for line in inline_note_lines {
                    inline_rows.push(RenderRow::Note(line));
                    row_contexts.push(note_context);
                }
                for line in side_note_lines {
                    side_by_side_rows.push(RenderRow::Note(line));
                }

                inserted_total += note_rows.len();
            }

            inline_rows.push(base_inline_rows[base_index].clone());
            side_by_side_rows.push(base_side_by_side_rows[base_index].clone());
            row_contexts.push(base_row_contexts[base_index]);
        }
        inserted_before_base[base_row_contexts.len()] =
            row_contexts.len().saturating_sub(base_row_contexts.len());

        let hunk_ranges = base_hunk_ranges
            .into_iter()
            .map(|range| HunkRange {
                file_index: range.file_index,
                hunk_index: range.hunk_index,
                start: range.start + inserted_before_base[range.start],
            })
            .collect();

        Self {
            inline_width,
            side_by_side_width,
            inline_rows,
            side_by_side_rows,
            hunk_ranges,
            row_contexts,
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

fn build_note_anchors<'a>(
    session: &DiffSession,
    notes: &'a [Note],
    row_contexts: &[RowContext],
) -> Vec<(usize, &'a Note)> {
    notes
        .iter()
        .filter_map(|note| note_anchor_row(session, row_contexts, note).map(|row| (row, note)))
        .collect()
}

fn note_anchor_row(
    session: &DiffSession,
    row_contexts: &[RowContext],
    note: &Note,
) -> Option<usize> {
    match &note.target {
        NoteTarget::File { file_path } => row_contexts.iter().position(|context| {
            matches!(context.kind, RowKind::FileHeader)
                && context
                    .file_index
                    .and_then(|index| session.files.get(index))
                    .is_some_and(|file| &file.path == file_path)
        }),
        NoteTarget::Hunk {
            file_path,
            hunk_header,
        } => row_contexts.iter().position(|context| {
            matches!(context.kind, RowKind::DiffLine | RowKind::HunkHeader)
                && context
                    .file_index
                    .and_then(|index| session.files.get(index))
                    .is_some_and(|file| {
                        &file.path == file_path
                            && context
                                .hunk_index
                                .and_then(|hunk_index| file.hunks.get(hunk_index))
                                .is_some_and(|hunk| &hunk.header == hunk_header)
                    })
        }),
        NoteTarget::Line {
            file_path,
            old_lineno,
            new_lineno,
        } => row_contexts.iter().position(|context| {
            matches!(context.kind, RowKind::DiffLine)
                && context.old_lineno == *old_lineno
                && context.new_lineno == *new_lineno
                && context
                    .file_index
                    .and_then(|index| session.files.get(index))
                    .is_some_and(|file| &file.path == file_path)
        }),
        NoteTarget::Range {
            file_path,
            start_old_lineno,
            start_new_lineno,
            ..
        } => row_contexts.iter().position(|context| {
            matches!(context.kind, RowKind::DiffLine)
                && context.old_lineno == *start_old_lineno
                && context.new_lineno == *start_new_lineno
                && context
                    .file_index
                    .and_then(|index| session.files.get(index))
                    .is_some_and(|file| &file.path == file_path)
        }),
    }
}

fn build_note_rows(note: &Note, width: usize, expanded: bool) -> Vec<String> {
    let body_width = width.saturating_sub(4).max(8);
    let mut rows = wrap_text(&note.body, body_width);
    if rows.is_empty() {
        rows.push(String::new());
    }

    if !expanded && rows.len() > 2 {
        rows.truncate(2);
        if let Some(last) = rows.last_mut() {
            *last = truncate_with_ellipsis(last, body_width);
        }
    }

    rows
}

fn note_side(note: &Note) -> NoteSide {
    match &note.target {
        NoteTarget::Line {
            old_lineno: Some(_),
            new_lineno: None,
            ..
        }
        | NoteTarget::Range {
            start_old_lineno: Some(_),
            start_new_lineno: None,
            ..
        } => NoteSide::Left,
        NoteTarget::Line {
            old_lineno: None,
            new_lineno: Some(_),
            ..
        }
        | NoteTarget::Range {
            start_old_lineno: None,
            start_new_lineno: Some(_),
            ..
        } => NoteSide::Right,
        _ => NoteSide::Full,
    }
}

fn render_note_rows(rows: &[String], width: usize) -> Vec<Line<'static>> {
    let inner_width = width.saturating_sub(2).max(4);
    let border_style = Style::default().fg(Color::DarkGray);
    let content_style = Style::default().fg(Color::White);
    let mut rendered = Vec::with_capacity(rows.len() + 2);

    rendered.push(Line::from(vec![
        Span::styled("┌".to_string(), border_style),
        Span::styled("─".repeat(inner_width), border_style),
        Span::styled("┐".to_string(), border_style),
    ]));

    for row in rows {
        rendered.push(Line::from(vec![
            Span::styled("│".to_string(), border_style),
            Span::styled(fit_text(&format!(" {row}"), inner_width), content_style),
            Span::styled("│".to_string(), border_style),
        ]));
    }

    rendered.push(Line::from(vec![
        Span::styled("└".to_string(), border_style),
        Span::styled("─".repeat(inner_width), border_style),
        Span::styled("┘".to_string(), border_style),
    ]));

    rendered
}

fn render_side_by_side_note_rows(
    rows: &[String],
    width: usize,
    side: NoteSide,
) -> Vec<Line<'static>> {
    if side == NoteSide::Full {
        return render_note_rows(rows, width);
    }

    let (left_width, right_width) = split_side_by_side_width(width);
    let note_width = match side {
        NoteSide::Left => left_width,
        NoteSide::Right => right_width,
        NoteSide::Full => width,
    };
    let note_rows = render_note_rows(rows, note_width);
    let divider_style = Style::default().fg(Color::DarkGray);

    note_rows
        .into_iter()
        .map(|note_row| match side {
            NoteSide::Left => combined_side_line(note_row, blank_note_side_line(right_width)),
            NoteSide::Right => {
                let mut spans = blank_note_side_line(left_width).spans;
                spans.push(Span::styled(" │ ".to_string(), divider_style));
                spans.extend(note_row.spans);
                Line::from(spans)
            }
            NoteSide::Full => unreachable!(),
        })
        .collect()
}

fn blank_note_side_line(width: usize) -> Line<'static> {
    Line::from(Span::raw(" ".repeat(width)))
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let current_len = current.chars().count();
        let word_len = word.chars().count();
        let separator = usize::from(!current.is_empty());

        if current_len + separator + word_len > width && !current.is_empty() {
            rows.push(current);
            current = word.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }

    if !current.is_empty() {
        rows.push(current);
    }

    rows
}

fn truncate_with_ellipsis(text: &str, width: usize) -> String {
    truncate_text(text, width.saturating_sub(1).max(1))
}

pub fn materialize_rows(
    rows: &[RenderRow],
    row_contexts: &[RowContext],
    scroll: u16,
    selected_file: usize,
    selected_hunk: usize,
    cursor_row: usize,
    cursor_focused: bool,
    selected_rows: Option<(usize, usize)>,
) -> Vec<Line<'static>> {
    rows.iter()
        .skip(scroll as usize)
        .zip(row_contexts.iter().skip(scroll as usize))
        .enumerate()
        .map(|(visible_index, (row, _context))| {
            let line = match row {
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
                RenderRow::Note(line) => line.clone(),
            };

            let absolute_row = scroll as usize + visible_index;
            let in_selection = selected_rows
                .is_some_and(|(start, end)| absolute_row >= start && absolute_row <= end);

            if absolute_row == cursor_row {
                highlight_cursor_line(line, cursor_focused, in_selection)
            } else if in_selection {
                highlight_selected_line(line)
            } else {
                line
            }
        })
        .collect()
}

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

fn highlight_selected_line(line: Line<'static>) -> Line<'static> {
    patch_line_background(line, Style::default().bg(Color::Rgb(40, 40, 40)))
}

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

impl DiffLine {
    fn old_lineno(&self) -> Option<usize> {
        match self {
            Self::Context { old_lineno, .. } | Self::Removed { old_lineno, .. } => {
                Some(*old_lineno)
            }
            Self::Added { .. } => None,
        }
    }

    fn new_lineno(&self) -> Option<usize> {
        match self {
            Self::Context { new_lineno, .. } | Self::Added { new_lineno, .. } => Some(*new_lineno),
            Self::Removed { .. } => None,
        }
    }
}
