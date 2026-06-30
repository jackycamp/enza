//! Base row planning.
//!
//! A `LayoutPlan` stores where each file and hunk starts and how many base rows
//! exist. It does not store one record or rendered row per diff line. Callers ask
//! for row details only when they need a row, so unloaded hunks still have row
//! numbers without allocating placeholders for the whole diff.

use ratatui::text::Line;

use crate::diff::DiffSession;
use crate::layout::layout_tree::{CachedRows, LayoutTree, NodeStatus};
use crate::layout::lines::{hunk_header_line, side_by_side_hunk_header_line};
use crate::layout::primitives::{
    HunkRange, LayoutPlan, PlannedFile, PlannedHunk, RenderRow, RowContext, RowId, RowKind,
};

// FIXME: At this point why isn't build_layout_plan just LayoutPlan::new ?

/// Builds base row metadata from the parsed diff.
///
/// Rows include file separators, file headers, hunk headers, diff lines, and
/// spacer rows. The plan stores only file/hunk start rows and row counts.
pub(super) fn build_layout_plan(session: &DiffSession) -> LayoutPlan {
    let mut files = Vec::new();
    let mut hunk_ranges = Vec::new();
    let mut row_count = 0usize;

    for (file_index, file) in session.files.iter().enumerate() {
        let file_start = row_count;
        row_count += 2;

        let mut hunks = Vec::with_capacity(file.hunks.len());
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let start = row_count;
            hunk_ranges.push(HunkRange {
                file_index,
                hunk_index,
                start,
            });
            hunks.push(PlannedHunk {
                file_index,
                hunk_index,
                start,
                line_count: hunk.lines.len(),
            });
            row_count += hunk.lines.len() + 2;
        }

        files.push(PlannedFile {
            file_index,
            start: file_start,
            end: row_count,
            hunks,
        });
    }

    LayoutPlan {
        files,
        hunk_ranges,
        row_count,
    }
}

/// Returns what a base row represents.
///
/// Example: row `0` is usually a file separator, row `1` is a file header, and
/// row `2` is the first hunk header. This works even when a hunk's rendered rows
/// are not loaded.
pub(super) fn row_context_for_plan_row(
    session: &DiffSession,
    plan: &LayoutPlan,
    row_index: usize,
) -> Option<RowContext> {
    match row_id_for_plan_row(plan, row_index)? {
        RowId::FileSeparator { file_index } => Some(RowContext::separator(file_index)),
        RowId::FileHeader { file_index } => Some(RowContext::file_header(file_index)),
        RowId::HunkHeader {
            file_index,
            hunk_index,
        } => Some(RowContext::hunk_header(file_index, hunk_index)),
        RowId::DiffLine {
            file_index,
            hunk_index,
            line_index,
        } => {
            let line = session
                .files
                .get(file_index)?
                .hunks
                .get(hunk_index)?
                .lines
                .get(line_index)?;
            Some(RowContext::diff_line(
                file_index,
                hunk_index,
                line.old_lineno(),
                line.new_lineno(),
            ))
        }
        RowId::HunkSpacer {
            file_index,
            hunk_index,
        } => Some(RowContext::spacer(file_index, hunk_index)),
    }
}

/// Converts one base row into inline and side-by-side render rows.
///
/// Loaded hunk rows come from `LayoutTree`. Unloaded diff lines render as blank
/// rows; unloaded hunk headers can still render from `DiffSession` so navigation
/// has visible anchors.
pub(super) fn plan_row_to_render_rows(
    session: &DiffSession,
    tree: &LayoutTree,
    plan: &LayoutPlan,
    row_index: usize,
    side_by_side_width: usize,
) -> (RenderRow, RenderRow) {
    let Some(row_id) = row_id_for_plan_row(plan, row_index) else {
        return blank_row_pair();
    };

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
                    RenderRow::hunk_header(
                        file_index,
                        hunk_index,
                        hunk_header_line(&hunk.header, false),
                        hunk_header_line(&hunk.header, true),
                    ),
                    RenderRow::hunk_header(
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
    }
}

/// Builds a `RowContext` vector for every base row.
///
/// This is intentionally not stored on `Layout`; use it only at boundaries such
/// as note anchoring or tests that compare row meanings.
pub(super) fn plan_row_contexts(session: &DiffSession, plan: &LayoutPlan) -> Vec<RowContext> {
    (0..plan.row_count)
        .filter_map(|row| row_context_for_plan_row(session, plan, row))
        .collect()
}

/// Finds the base row index for a non-note row context.
///
/// Note rows are inserted rows and are handled by `layout::access`; this
/// function only maps rows that belong to the base plan.
pub(super) fn plan_row_index_for_context(
    session: &DiffSession,
    plan: &LayoutPlan,
    target: RowContext,
) -> Option<usize> {
    if target.note_id.is_some() {
        return None;
    }

    let file_index = target.file_index?;
    let file = file_for_index(plan, file_index)?;

    match target.kind {
        RowKind::Separator => Some(file.start),
        RowKind::FileHeader => Some(file.start + 1),
        RowKind::HunkHeader => {
            let hunk = hunk_for_index(file, target.hunk_index?)?;
            Some(hunk.start)
        }
        RowKind::DiffLine => {
            let hunk_index = target.hunk_index?;
            let hunk = hunk_for_index(file, hunk_index)?;
            let line_index = session
                .files
                .get(file_index)?
                .hunks
                .get(hunk_index)?
                .lines
                .iter()
                .position(|line| {
                    line.old_lineno() == target.old_lineno && line.new_lineno() == target.new_lineno
                })?;
            Some(hunk.start + 1 + line_index)
        }
        RowKind::Spacer => {
            if let Some(hunk_index) = target.hunk_index {
                let hunk = hunk_for_index(file, hunk_index)?;
                Some(hunk.start + hunk.line_count + 1)
            } else {
                None
            }
        }
        RowKind::Note => None,
    }
}

/// Converts a base row number into the matching file/hunk row type.
fn row_id_for_plan_row(plan: &LayoutPlan, row_index: usize) -> Option<RowId> {
    let file = file_for_row(plan, row_index)?;
    if row_index == file.start {
        return Some(RowId::FileSeparator {
            file_index: file.file_index,
        });
    }
    if row_index == file.start + 1 {
        return Some(RowId::FileHeader {
            file_index: file.file_index,
        });
    }
    let hunk = hunk_for_row(file, row_index)?;
    let offset = row_index.saturating_sub(hunk.start);
    if offset == 0 {
        return Some(RowId::HunkHeader {
            file_index: hunk.file_index,
            hunk_index: hunk.hunk_index,
        });
    }
    if offset == hunk.line_count + 1 {
        return Some(RowId::HunkSpacer {
            file_index: hunk.file_index,
            hunk_index: hunk.hunk_index,
        });
    }

    Some(RowId::DiffLine {
        file_index: hunk.file_index,
        hunk_index: hunk.hunk_index,
        line_index: offset - 1,
    })
}

/// Finds the planned file containing a base row index.
fn file_for_row(plan: &LayoutPlan, row_index: usize) -> Option<&PlannedFile> {
    let index = plan.files.partition_point(|file| file.start <= row_index);
    let file = plan.files.get(index.checked_sub(1)?)?;
    (row_index < file.end).then_some(file)
}

/// Finds a planned file by diff file index.
fn file_for_index(plan: &LayoutPlan, file_index: usize) -> Option<&PlannedFile> {
    plan.files.get(file_index)
}

/// Finds the planned hunk containing a base row index within a file.
fn hunk_for_row(file: &PlannedFile, row_index: usize) -> Option<&PlannedHunk> {
    let index = file.hunks.partition_point(|hunk| hunk.start <= row_index);
    let hunk = file.hunks.get(index.checked_sub(1)?)?;
    (row_index <= hunk.start + hunk.line_count + 1).then_some(hunk)
}

/// Finds a planned hunk by diff hunk index within a file.
fn hunk_for_index(file: &PlannedFile, hunk_index: usize) -> Option<&PlannedHunk> {
    file.hunks.get(hunk_index)
}

/// Clones the inline and side-by-side cached rows at the same cache index.
fn cached_row_pair(rows: &CachedRows, index: usize) -> Option<(RenderRow, RenderRow)> {
    Some((
        rows.inline_rows.get(index)?.clone(),
        rows.side_by_side_rows.get(index)?.clone(),
    ))
}

/// Returns an empty inline/side-by-side row pair for unavailable render content.
fn blank_row_pair() -> (RenderRow, RenderRow) {
    (
        RenderRow::static_line(Line::default()),
        RenderRow::static_line(Line::default()),
    )
}
