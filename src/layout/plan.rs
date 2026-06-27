use ratatui::text::Line;

use crate::diff::DiffSession;
use crate::layout::lines::{hunk_header_line, hunk_header_row, side_by_side_hunk_header_line};
use crate::layout::model::{
    CachedRows, HunkRange, LayoutPlan, LayoutTree, NodeStatus, PlannedRow, RenderRow, RowContext,
    RowId, RowKind,
};

pub(super) struct LayoutRenderRows {
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub row_contexts: Vec<RowContext>,
}

pub(super) fn build_layout_plan(session: &DiffSession) -> LayoutPlan {
    let mut rows = Vec::new();
    let mut hunk_ranges = Vec::new();

    for (file_index, file) in session.files.iter().enumerate() {
        rows.push(PlannedRow {
            id: RowId::FileSeparator { file_index },
            context: RowContext {
                file_index: Some(file_index),
                hunk_index: None,
                kind: RowKind::Separator,
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            },
        });
        rows.push(PlannedRow {
            id: RowId::FileHeader { file_index },
            context: RowContext {
                file_index: Some(file_index),
                hunk_index: None,
                kind: RowKind::FileHeader,
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            },
        });

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            hunk_ranges.push(HunkRange {
                file_index,
                hunk_index,
                start: rows.len(),
            });
            rows.push(PlannedRow {
                id: RowId::HunkHeader {
                    file_index,
                    hunk_index,
                },
                context: RowContext {
                    file_index: Some(file_index),
                    hunk_index: Some(hunk_index),
                    kind: RowKind::HunkHeader,
                    old_lineno: None,
                    new_lineno: None,
                    note_id: None,
                },
            });
            rows.extend(
                hunk.lines
                    .iter()
                    .enumerate()
                    .map(|(line_index, line)| PlannedRow {
                        id: RowId::DiffLine {
                            file_index,
                            hunk_index,
                            line_index,
                        },
                        context: RowContext {
                            file_index: Some(file_index),
                            hunk_index: Some(hunk_index),
                            kind: RowKind::DiffLine,
                            old_lineno: line.old_lineno(),
                            new_lineno: line.new_lineno(),
                            note_id: None,
                        },
                    }),
            );
            rows.push(PlannedRow {
                id: RowId::HunkSpacer {
                    file_index,
                    hunk_index,
                },
                context: RowContext {
                    file_index: Some(file_index),
                    hunk_index: Some(hunk_index),
                    kind: RowKind::Spacer,
                    old_lineno: None,
                    new_lineno: None,
                    note_id: None,
                },
            });
        }

        rows.push(PlannedRow {
            id: RowId::FileSpacer { file_index },
            context: RowContext {
                file_index: Some(file_index),
                hunk_index: None,
                kind: RowKind::Spacer,
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            },
        });
    }

    LayoutPlan { rows, hunk_ranges }
}

pub(super) fn layout_plan_to_render_rows(
    session: &DiffSession,
    tree: &LayoutTree,
    plan: &LayoutPlan,
    side_by_side_width: usize,
) -> LayoutRenderRows {
    let mut inline_rows = Vec::with_capacity(plan.rows.len());
    let mut side_by_side_rows = Vec::with_capacity(plan.rows.len());

    for planned in &plan.rows {
        let (inline, side_by_side) =
            materialize_planned_row(session, tree, planned.id, side_by_side_width);
        inline_rows.push(inline);
        side_by_side_rows.push(side_by_side);
    }

    LayoutRenderRows {
        inline_rows,
        side_by_side_rows,
        row_contexts: plan.rows.iter().map(|row| row.context).collect(),
    }
}

fn materialize_planned_row(
    session: &DiffSession,
    tree: &LayoutTree,
    row_id: RowId,
    side_by_side_width: usize,
) -> (RenderRow, RenderRow) {
    match row_id {
        RowId::FileSeparator { file_index } => tree
            .files
            .get(file_index)
            .and_then(|file| cached_row_pair(&file.header, 0))
            .unwrap_or_else(blank_row_pair),
        RowId::FileHeader { file_index } => tree
            .files
            .get(file_index)
            .and_then(|file| cached_row_pair(&file.header, 1))
            .unwrap_or_else(blank_row_pair),
        RowId::HunkHeader {
            file_index,
            hunk_index,
        } => {
            let cached = tree
                .files
                .get(file_index)
                .and_then(|file| file.hunks.get(hunk_index))
                .filter(|hunk| hunk.status == NodeStatus::Ready)
                .and_then(|hunk| cached_row_pair(&hunk.rows, 0));
            cached.unwrap_or_else(|| {
                let Some(hunk) = session
                    .files
                    .get(file_index)
                    .and_then(|file| file.hunks.get(hunk_index))
                else {
                    return blank_row_pair();
                };
                (
                    hunk_header_row(
                        file_index,
                        hunk_index,
                        hunk_header_line(&hunk.header, false),
                        hunk_header_line(&hunk.header, true),
                    ),
                    hunk_header_row(
                        file_index,
                        hunk_index,
                        side_by_side_hunk_header_line(&hunk.header, false, side_by_side_width),
                        side_by_side_hunk_header_line(&hunk.header, true, side_by_side_width),
                    ),
                )
            })
        }
        RowId::DiffLine {
            file_index,
            hunk_index,
            line_index,
        } => tree
            .files
            .get(file_index)
            .and_then(|file| file.hunks.get(hunk_index))
            .filter(|hunk| hunk.status == NodeStatus::Ready)
            .and_then(|hunk| cached_row_pair(&hunk.rows, line_index + 1))
            .unwrap_or_else(blank_row_pair),
        RowId::HunkSpacer {
            file_index,
            hunk_index,
        } => {
            let line_count = session
                .files
                .get(file_index)
                .and_then(|file| file.hunks.get(hunk_index))
                .map(|hunk| hunk.lines.len())
                .unwrap_or(0);
            tree.files
                .get(file_index)
                .and_then(|file| file.hunks.get(hunk_index))
                .filter(|hunk| hunk.status == NodeStatus::Ready)
                .and_then(|hunk| cached_row_pair(&hunk.rows, line_count + 1))
                .unwrap_or_else(blank_row_pair)
        }
        RowId::FileSpacer { file_index } => tree
            .files
            .get(file_index)
            .and_then(|file| cached_row_pair(&file.trailing_spacer, 0))
            .unwrap_or_else(blank_row_pair),
    }
}

fn cached_row_pair(rows: &CachedRows, index: usize) -> Option<(RenderRow, RenderRow)> {
    Some((
        rows.inline_rows.get(index)?.clone(),
        rows.side_by_side_rows.get(index)?.clone(),
    ))
}

fn blank_row_pair() -> (RenderRow, RenderRow) {
    (
        RenderRow::Static(Line::default()),
        RenderRow::Static(Line::default()),
    )
}
