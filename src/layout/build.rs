use ratatui::text::Line;

use crate::diff::DiffSession;
use crate::highlight::FileHighlighter;
use crate::layout::lines::{
    build_combined_side_line, build_inline_line, file_header_line, file_header_row,
    file_separator_line, file_side_by_side_header_line, hunk_header_line, hunk_header_row,
    side_by_side_hunk_header_line,
};
use crate::layout::model::{HunkRange, Layout, RenderRow, RowContext, RowKind};
use crate::layout::notes::{
    build_note_anchors, build_note_rows, render_note_rows, render_side_by_side_note_rows,
};
use crate::note::Note;

struct BaseLayout {
    inline_rows: Vec<RenderRow>,
    side_by_side_rows: Vec<RenderRow>,
    hunk_ranges: Vec<HunkRange>,
    row_contexts: Vec<RowContext>,
}

struct NoteOverlay {
    inline_rows: Vec<RenderRow>,
    side_by_side_rows: Vec<RenderRow>,
    row_contexts: Vec<RowContext>,
    inserted_before_base: Vec<usize>,
}

impl Layout {
    pub fn build(
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
        inline_width: usize,
        side_by_side_width: usize,
    ) -> Self {
        let base = build_base_layout(session, inline_width, side_by_side_width);
        let overlay = inject_notes(
            session,
            &base,
            notes,
            expanded_note_ids,
            inline_width,
            side_by_side_width,
        );
        let hunk_ranges =
            adjust_hunk_ranges_for_insertions(base.hunk_ranges, &overlay.inserted_before_base);

        Self {
            inline_width,
            side_by_side_width,
            inline_rows: overlay.inline_rows,
            side_by_side_rows: overlay.side_by_side_rows,
            hunk_ranges,
            row_contexts: overlay.row_contexts,
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

fn build_base_layout(
    session: &DiffSession,
    inline_width: usize,
    side_by_side_width: usize,
) -> BaseLayout {
    let mut inline_rows = Vec::new();
    let mut side_by_side_rows = Vec::new();
    let mut hunk_ranges = Vec::new();
    let mut row_contexts = Vec::new();
    let mut cursor = 0usize;

    for (file_index, file) in session.files.iter().enumerate() {
        cursor += build_file_layout(
            file_index,
            file,
            inline_width,
            side_by_side_width,
            &mut inline_rows,
            &mut side_by_side_rows,
            &mut row_contexts,
            &mut hunk_ranges,
            cursor,
        );
    }

    BaseLayout {
        inline_rows,
        side_by_side_rows,
        hunk_ranges,
        row_contexts,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_file_layout(
    file_index: usize,
    file: &crate::diff::DiffFile,
    inline_width: usize,
    side_by_side_width: usize,
    inline_rows: &mut Vec<RenderRow>,
    side_by_side_rows: &mut Vec<RenderRow>,
    row_contexts: &mut Vec<RowContext>,
    hunk_ranges: &mut Vec<HunkRange>,
    start_cursor: usize,
) -> usize {
    let mut highlighter = FileHighlighter::new(&file.path);
    let mut cursor = start_cursor;

    let inline_separator = file_separator_line(inline_width);
    let side_separator = file_separator_line(side_by_side_width);
    inline_rows.push(RenderRow::Static(inline_separator.clone()));
    side_by_side_rows.push(RenderRow::Static(side_separator));
    row_contexts.push(RowContext {
        file_index: Some(file_index),
        hunk_index: None,
        kind: RowKind::Separator,
        old_lineno: None,
        new_lineno: None,
        note_id: None,
    });

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
    row_contexts.push(RowContext {
        file_index: Some(file_index),
        hunk_index: None,
        kind: RowKind::FileHeader,
        old_lineno: None,
        new_lineno: None,
        note_id: None,
    });

    cursor += 2;

    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        let hunk_start = cursor;

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
        row_contexts.push(RowContext {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: RowKind::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        });

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
            row_contexts.push(RowContext {
                file_index: Some(file_index),
                hunk_index: Some(hunk_index),
                kind: RowKind::DiffLine,
                old_lineno: diff_line.old_lineno(),
                new_lineno: diff_line.new_lineno(),
                note_id: None,
            });
        }

        inline_rows.push(RenderRow::Static(Line::default()));
        side_by_side_rows.push(RenderRow::Static(Line::default()));
        row_contexts.push(RowContext {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: RowKind::Spacer,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        });

        cursor += 1 + hunk.lines.len() + 1;
        hunk_ranges.push(HunkRange {
            file_index,
            hunk_index,
            start: hunk_start,
        });
    }

    inline_rows.push(RenderRow::Static(Line::default()));
    side_by_side_rows.push(RenderRow::Static(Line::default()));
    row_contexts.push(RowContext {
        file_index: Some(file_index),
        hunk_index: None,
        kind: RowKind::Spacer,
        old_lineno: None,
        new_lineno: None,
        note_id: None,
    });

    cursor + 1 - start_cursor
}

fn inject_notes(
    session: &DiffSession,
    base: &BaseLayout,
    notes: &[Note],
    expanded_note_ids: &[u64],
    inline_width: usize,
    side_by_side_width: usize,
) -> NoteOverlay {
    let note_anchors = build_note_anchors(session, notes, &base.row_contexts);
    let mut inline_rows = Vec::new();
    let mut side_by_side_rows = Vec::new();
    let mut row_contexts = Vec::new();
    let mut inserted_before_base = vec![0usize; base.row_contexts.len() + 1];
    let mut inserted_total = 0usize;
    let note_wrap_width = inline_width.min(side_by_side_width);

    for base_index in 0..base.row_contexts.len() {
        inserted_before_base[base_index] = inserted_total;
        for note in note_anchors
            .iter()
            .filter(|(anchor_index, _)| *anchor_index == base_index)
            .map(|(_, note)| note)
        {
            let expanded = expanded_note_ids.contains(&note.id);
            let note_rows = build_note_rows(note, note_wrap_width, expanded);
            let note_context = RowContext {
                file_index: base.row_contexts[base_index].file_index,
                hunk_index: base.row_contexts[base_index].hunk_index,
                kind: RowKind::Note,
                old_lineno: None,
                new_lineno: None,
                note_id: Some(note.id),
            };

            append_note_rows(
                &mut inline_rows,
                &mut side_by_side_rows,
                &mut row_contexts,
                &note_rows,
                note,
                note_context,
                inline_width,
                side_by_side_width,
            );

            inserted_total += note_rows.len();
        }

        inline_rows.push(base.inline_rows[base_index].clone());
        side_by_side_rows.push(base.side_by_side_rows[base_index].clone());
        row_contexts.push(base.row_contexts[base_index]);
    }
    inserted_before_base[base.row_contexts.len()] =
        row_contexts.len().saturating_sub(base.row_contexts.len());

    NoteOverlay {
        inline_rows,
        side_by_side_rows,
        row_contexts,
        inserted_before_base,
    }
}

fn append_note_rows(
    inline_rows: &mut Vec<RenderRow>,
    side_by_side_rows: &mut Vec<RenderRow>,
    row_contexts: &mut Vec<RowContext>,
    note_rows: &[String],
    note: &Note,
    note_context: RowContext,
    inline_width: usize,
    side_by_side_width: usize,
) {
    for line in render_note_rows(note_rows, inline_width) {
        inline_rows.push(RenderRow::Note(line));
        row_contexts.push(note_context);
    }

    for line in render_side_by_side_note_rows(note_rows, side_by_side_width, note) {
        side_by_side_rows.push(RenderRow::Note(line));
    }
}

fn adjust_hunk_ranges_for_insertions(
    hunk_ranges: Vec<HunkRange>,
    inserted_before_base: &[usize],
) -> Vec<HunkRange> {
    hunk_ranges
        .into_iter()
        .map(|range| HunkRange {
            file_index: range.file_index,
            hunk_index: range.hunk_index,
            start: range.start + inserted_before_base[range.start],
        })
        .collect()
}
