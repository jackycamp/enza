use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::diff::DiffSession;
use crate::layout::base_layout::BaseLayout;
use crate::layout::lines::{combined_side_line, split_side_by_side_width};
use crate::layout::plan::plan_row_contexts;
use crate::layout::primitives::{
    HunkRange, LayoutWidths, NoteInsertion, RenderRow, RowContext, RowKind,
};
use crate::layout::text::{fit_text, truncate_with_ellipsis, wrap_text};
use crate::note::{Note, NoteTarget};

pub(super) struct NoteOverlay {
    pub insertions: Vec<NoteInsertion>,
    pub hunk_ranges: Vec<HunkRange>,
    pub row_count: usize,
}

#[derive(Clone, Copy)]
pub(super) struct NoteOverlayRequest<'a> {
    pub session: &'a DiffSession,
    pub base: &'a BaseLayout,
    pub notes: &'a [Note],
    pub expanded_note_ids: &'a [u64],
    pub widths: LayoutWidths,
}

impl NoteOverlay {
    pub(super) fn new(request: NoteOverlayRequest<'_>) -> Self {
        let overlay = NoteInsertions::new(request);

        Self {
            hunk_ranges: adjust_hunk_ranges_for_insertions(
                request.base.hunk_ranges.clone(),
                &overlay.inserted_before_or_at_base,
            ),
            row_count: request.base.plan.row_count + overlay.inserted_total,
            insertions: overlay.insertions,
        }
    }
}

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

struct NoteInsertions {
    insertions: Vec<NoteInsertion>,
    inserted_before_or_at_base: Vec<usize>,
    inserted_total: usize,
}

impl NoteInsertions {
    fn new(request: NoteOverlayRequest<'_>) -> Self {
        if request.notes.is_empty() {
            return Self {
                insertions: Vec::new(),
                inserted_before_or_at_base: vec![0usize; request.base.plan.row_count + 1],
                inserted_total: 0,
            };
        }

        let base_row_contexts = plan_row_contexts(request.session, &request.base.plan);
        let note_anchors = build_note_anchors(request.session, request.notes, &base_row_contexts);
        let mut insertions = Vec::new();
        let mut inserted_before_or_at_base = vec![0usize; request.base.plan.row_count + 1];
        let mut inserted_total = 0usize;
        let note_wrap_width = request.widths.inline.min(request.widths.side_by_side);

        for base_index in 0..request.base.plan.row_count {
            for note in note_anchors
                .iter()
                .filter(|(anchor_index, _)| *anchor_index == base_index)
                .map(|(_, note)| note)
            {
                let expanded = request.expanded_note_ids.contains(&note.id);
                let note_rows = build_note_rows(note, note_wrap_width, expanded);
                let context = RowContext::note(base_row_contexts[base_index], note.id);

                let insertion = build_note_insertion(NoteInsertionRequest {
                    base_index,
                    note_rows: &note_rows,
                    note,
                    context,
                    widths: request.widths,
                });
                inserted_total += insertion.len();
                insertions.push(insertion);
            }
            inserted_before_or_at_base[base_index] = inserted_total;
        }
        inserted_before_or_at_base[request.base.plan.row_count] = inserted_total;

        Self {
            insertions,
            inserted_before_or_at_base,
            inserted_total,
        }
    }
}

struct NoteInsertionRequest<'a> {
    base_index: usize,
    note_rows: &'a [String],
    note: &'a Note,
    context: RowContext,
    widths: LayoutWidths,
}

fn build_note_insertion(request: NoteInsertionRequest<'_>) -> NoteInsertion {
    NoteInsertion {
        base_index: request.base_index,
        inline_rows: render_note_rows(request.note_rows, request.widths.inline)
            .into_iter()
            .map(RenderRow::note)
            .collect(),
        side_by_side_rows: render_side_by_side_note_rows(
            request.note_rows,
            request.widths.side_by_side,
            request.note,
        )
        .into_iter()
        .map(RenderRow::note)
        .collect(),
        context: request.context,
    }
}

fn adjust_hunk_ranges_for_insertions(
    hunk_ranges: Vec<HunkRange>,
    inserted_before_or_at_base: &[usize],
) -> Vec<HunkRange> {
    hunk_ranges
        .into_iter()
        .map(|range| HunkRange {
            file_index: range.file_index,
            hunk_index: range.hunk_index,
            start: range.start + inserted_before_or_at_base[range.start],
        })
        .collect()
}

// FIXME: No test coverage here?
