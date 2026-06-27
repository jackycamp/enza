use crate::diff::DiffSession;
use crate::layout::model::{Layout, LayoutRowLocation, NoteInsertion, RowContext, RowKind};
use crate::layout::plan::{
    plan_row_contexts, plan_row_index_for_context, row_context_for_plan_row,
};

impl Layout {
    pub fn line_count_for_mode(&self, _side_by_side: bool) -> usize {
        self.row_count
    }

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

    pub fn row_contexts(&self, session: &DiffSession) -> Vec<RowContext> {
        (0..self.row_count)
            .filter_map(|row| self.row_context(session, row))
            .collect()
    }

    pub fn base_row_contexts(&self, session: &DiffSession) -> Vec<RowContext> {
        plan_row_contexts(session, &self.base.plan)
    }

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
