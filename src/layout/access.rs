//! Row lookup helpers for `Layout`.
//!
//! These methods translate between public row indexes and the compact base plan
//! plus note overlays. They are deliberately lookup-oriented: keeping a full
//! `Vec<RowContext>` on `Layout` would reintroduce the per-row storage that the
//! compact plan avoids.

use crate::diff::DiffSession;
use crate::layout::model::{Layout, LayoutRowLocation, NoteInsertion, RowContext, RowKind};
use crate::layout::plan::{plan_row_index_for_context, row_context_for_plan_row};

impl Layout {
    /// Returns the total rendered row count for the current layout mode.
    ///
    /// Inline and side-by-side modes share the same logical row count; only row
    /// contents differ.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Given layout.row_count == 128:
    /// let row_count = layout.line_count_for_mode(false);
    /// // -> 128
    /// let max_row = row_count.saturating_sub(1);
    /// // -> 127
    /// app.main_pane.cursor_row = app.main_pane.cursor_row.min(max_row);
    /// ```
    pub fn line_count_for_mode(&self, _side_by_side: bool) -> usize {
        self.row_count
    }

    /// Resolves a rendered row index into its semantic context.
    ///
    /// Base rows are looked up through the compact plan; note rows are resolved
    /// through overlay insertions.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(context) = layout.row_context(&session, 0) {
    ///     app.main_pane.cursor_target = Some(context);
    /// }
    /// // -> Some(RowContext {
    /// //      file_index: Some(0),
    /// //      hunk_index: None,
    /// //      kind: RowKind::Separator,
    /// //      old_lineno: None,
    /// //      new_lineno: None,
    /// //      note_id: None,
    /// //    })
    /// ```
    pub fn row_context(&self, session: &DiffSession, row: usize) -> Option<RowContext> {
        match self.locate_row(row)? {
            LayoutRowLocation::Base { base_index } => {
                row_context_for_plan_row(session, &self.base.plan, base_index)
            }
            LayoutRowLocation::Note {
                insertion_index, ..
            } => Some(self.note_insertions.get(insertion_index)?.context),
        }
    }

    /// Materializes all row contexts, including note overlays.
    ///
    /// Prefer `row_context` for point lookups. This exists for selection and note
    /// APIs that still need slice-style access.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let row_contexts = layout.row_contexts(&session);
    /// let target = note_target_for_range(&session.files, &row_contexts, start, end, cursor)?;
    /// // -> Vec<RowContext> with row_contexts.len() == layout.row_count
    /// ```
    pub fn row_contexts(&self, session: &DiffSession) -> Vec<RowContext> {
        (0..self.row_count)
            .filter_map(|row| self.row_context(session, row))
            .collect()
    }

    /// Finds the rendered row index for a semantic row context.
    ///
    /// Base contexts are mapped through `LayoutPlan`; note contexts are mapped
    /// through overlay insertion positions.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let context = RowContext {
    ///     file_index: Some(0),
    ///     hunk_index: Some(0),
    ///     kind: RowKind::HunkHeader,
    ///     old_lineno: None,
    ///     new_lineno: None,
    ///     note_id: None,
    /// };
    /// if let Some(row) = layout.row_index_for_context(&session, context) {
    ///     app.main_pane.cursor_row = row;
    /// }
    /// // -> app.main_pane.cursor_row == 2
    /// ```
    pub fn row_index_for_context(
        &self,
        session: &DiffSession,
        target: RowContext,
    ) -> Option<usize> {
        if target.kind == RowKind::Note {
            return note_row_index(&self.note_insertions, target);
        }

        let base_index = plan_row_index_for_context(session, &self.base.plan, target)?;
        let inserted_before_or_at = self
            .note_insertions
            .iter()
            .take_while(|insertion| insertion.base_index <= base_index)
            .map(NoteInsertion::len)
            .sum::<usize>();
        Some(base_index + inserted_before_or_at)
    }

    /// Splits a rendered row index into either a base-plan row or note row.
    pub(crate) fn locate_row(&self, row: usize) -> Option<LayoutRowLocation> {
        if row >= self.row_count {
            return None;
        }

        let mut inserted_before = 0usize;
        for (insertion_index, insertion) in self.note_insertions.iter().enumerate() {
            let insertion_start = insertion.base_index + inserted_before;
            if row < insertion_start {
                return Some(LayoutRowLocation::Base {
                    base_index: row - inserted_before,
                });
            }

            let insertion_end = insertion_start + insertion.len();
            if row < insertion_end {
                return Some(LayoutRowLocation::Note {
                    insertion_index,
                    row_offset: row - insertion_start,
                });
            }

            inserted_before += insertion.len();
        }

        Some(LayoutRowLocation::Base {
            base_index: row - inserted_before,
        })
    }
}

/// Finds the first rendered row for a note overlay context.
fn note_row_index(insertions: &[NoteInsertion], target: RowContext) -> Option<usize> {
    let mut inserted_before = 0usize;
    for insertion in insertions {
        let insertion_start = insertion.base_index + inserted_before;
        if insertion.context == target {
            return Some(insertion_start);
        }
        inserted_before += insertion.len();
    }
    None
}
