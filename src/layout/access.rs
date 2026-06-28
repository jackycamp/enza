//! Row lookup helpers for `Layout`.
//!
//! These methods translate rendered row numbers to either base rows or inserted
//! note rows. They are lookup-oriented: storing a full `Vec<RowContext>` on
//! `Layout` would bring back one entry per rendered row.

use crate::diff::DiffSession;
use crate::layout::model::{Layout, LayoutRowLocation, NoteInsertion, RowContext, RowKind};
use crate::layout::plan::{plan_row_index_for_context, row_context_for_plan_row};

impl Layout {
    /// Returns the number of rows the UI can scroll through.
    ///
    /// Inline and side-by-side modes have the same row count; only row contents
    /// differ.
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

    /// Returns what a rendered row represents.
    ///
    /// Base rows are looked up through `LayoutPlan`; inserted note rows are
    /// looked up through `note_insertions`.
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

    /// Builds a `RowContext` vector for every rendered row.
    ///
    /// Prefer `row_context` for point lookups. This exists for selection and note
    /// APIs that still need slice-style access.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Given layout.row_count == 128:
    /// let row_contexts = layout.row_contexts(&session);
    /// let target = note_target_for_range(&session.files, &row_contexts, start, end, cursor)?;
    /// // -> Vec<RowContext> with row_contexts.len() == 128
    /// ```
    pub fn row_contexts(&self, session: &DiffSession) -> Vec<RowContext> {
        (0..self.row_count)
            .filter_map(|row| self.row_context(session, row))
            .collect()
    }

    /// Finds the rendered row index for a row description.
    ///
    /// Base rows are mapped through `LayoutPlan`; note rows are mapped through
    /// insertion positions.
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

    /// Splits a rendered row index into either a base row or inserted note row.
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

/// Finds the first rendered row for an inserted note row.
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
