use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::diff::DiffSession;
use crate::layout::lines::{combined_side_line, split_side_by_side_width};
use crate::layout::model::{RowContext, RowKind};
use crate::layout::text::{fit_text, truncate_with_ellipsis, wrap_text};
use crate::note::{Note, NoteTarget};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoteSide {
    Full,
    Left,
    Right,
}

pub fn build_note_anchors<'a>(
    session: &DiffSession,
    notes: &'a [Note],
    row_contexts: &[RowContext],
) -> Vec<(usize, &'a Note)> {
    notes
        .iter()
        .filter_map(|note| note_anchor_row(session, row_contexts, note).map(|row| (row, note)))
        .collect()
}

pub fn build_note_rows(note: &Note, width: usize, expanded: bool) -> Vec<String> {
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

pub fn render_note_rows(rows: &[String], width: usize) -> Vec<Line<'static>> {
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

pub fn render_side_by_side_note_rows(
    rows: &[String],
    width: usize,
    note: &Note,
) -> Vec<Line<'static>> {
    let side = note_side_impl(note);
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

fn blank_note_side_line(width: usize) -> Line<'static> {
    Line::from(Span::raw(" ".repeat(width)))
}

fn note_side_impl(note: &Note) -> NoteSide {
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
