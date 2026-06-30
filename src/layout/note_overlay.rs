//! Note overlay construction.
//!
//! Notes are rendered as inserted rows before stable base rows. This module
//! resolves note anchors, builds rendered note insertions, and adjusts derived
//! row counts/ranges without mutating the base layout plan.

use crate::diff::DiffSession;
use crate::layout::model::{BaseLayout, HunkRange, NoteInsertion, RenderRow, RowContext, RowKind};
use crate::layout::notes::{
    build_note_anchors, build_note_rows, render_note_rows, render_side_by_side_note_rows,
};
use crate::layout::plan::plan_row_contexts;
use crate::note::Note;

pub(super) struct NoteOverlay {
    pub insertions: Vec<NoteInsertion>,
    pub hunk_ranges: Vec<HunkRange>,
    pub row_count: usize,
}

pub(super) fn build_note_overlay(
    session: &DiffSession,
    base: &BaseLayout,
    notes: &[Note],
    expanded_note_ids: &[u64],
    inline_width: usize,
    side_by_side_width: usize,
) -> NoteOverlay {
    let overlay = inject_notes(
        session,
        base,
        notes,
        expanded_note_ids,
        inline_width,
        side_by_side_width,
    );

    NoteOverlay {
        hunk_ranges: adjust_hunk_ranges_for_insertions(
            base.hunk_ranges.clone(),
            &overlay.inserted_before_or_at_base,
        ),
        row_count: base.plan.row_count + overlay.inserted_total,
        insertions: overlay.insertions,
    }
}

struct NoteInsertions {
    insertions: Vec<NoteInsertion>,
    inserted_before_or_at_base: Vec<usize>,
    inserted_total: usize,
}

fn inject_notes(
    session: &DiffSession,
    base: &BaseLayout,
    notes: &[Note],
    expanded_note_ids: &[u64],
    inline_width: usize,
    side_by_side_width: usize,
) -> NoteInsertions {
    if notes.is_empty() {
        return NoteInsertions {
            insertions: Vec::new(),
            inserted_before_or_at_base: vec![0usize; base.plan.row_count + 1],
            inserted_total: 0,
        };
    }

    let base_row_contexts = plan_row_contexts(session, &base.plan);
    let note_anchors = build_note_anchors(session, notes, &base_row_contexts);
    let mut insertions = Vec::new();
    let mut inserted_before_or_at_base = vec![0usize; base.plan.row_count + 1];
    let mut inserted_total = 0usize;
    let note_wrap_width = inline_width.min(side_by_side_width);

    for base_index in 0..base.plan.row_count {
        for note in note_anchors
            .iter()
            .filter(|(anchor_index, _)| *anchor_index == base_index)
            .map(|(_, note)| note)
        {
            let expanded = expanded_note_ids.contains(&note.id);
            let note_rows = build_note_rows(note, note_wrap_width, expanded);
            let note_context = RowContext {
                file_index: base_row_contexts[base_index].file_index,
                hunk_index: base_row_contexts[base_index].hunk_index,
                kind: RowKind::Note,
                old_lineno: None,
                new_lineno: None,
                note_id: Some(note.id),
            };

            let insertion = build_note_insertion(
                base_index,
                &note_rows,
                note,
                note_context,
                inline_width,
                side_by_side_width,
            );
            inserted_total += insertion.len();
            insertions.push(insertion);
        }
        inserted_before_or_at_base[base_index] = inserted_total;
    }
    inserted_before_or_at_base[base.plan.row_count] = inserted_total;

    NoteInsertions {
        insertions,
        inserted_before_or_at_base,
        inserted_total,
    }
}

fn build_note_insertion(
    base_index: usize,
    note_rows: &[String],
    note: &Note,
    note_context: RowContext,
    inline_width: usize,
    side_by_side_width: usize,
) -> NoteInsertion {
    NoteInsertion {
        base_index,
        inline_rows: render_note_rows(note_rows, inline_width)
            .into_iter()
            .map(RenderRow::Note)
            .collect(),
        side_by_side_rows: render_side_by_side_note_rows(note_rows, side_by_side_width, note)
            .into_iter()
            .map(RenderRow::Note)
            .collect(),
        context: note_context,
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
