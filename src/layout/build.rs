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

impl Layout {
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
                let note_rows = build_note_rows(note, note_wrap_width, expanded);
                let inline_note_lines = render_note_rows(&note_rows, inline_width);
                let side_note_lines =
                    render_side_by_side_note_rows(&note_rows, side_by_side_width, note);
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
