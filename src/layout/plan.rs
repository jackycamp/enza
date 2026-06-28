//! Compact logical row planning.
//!
//! A `LayoutPlan` stores file and hunk spans plus the total logical row count.
//! It intentionally does not store one planned row or one rendered row per diff
//! line. Callers resolve a row index into a `RowContext` or `RenderRow` only when
//! they need that row, which keeps unloaded hunks addressable without allocating
//! placeholder rows for the whole diff.

use ratatui::text::Line;

use crate::diff::DiffSession;
use crate::layout::lines::{hunk_header_line, hunk_header_row, side_by_side_hunk_header_line};
use crate::layout::model::{
    CachedRows, HunkRange, LayoutPlan, LayoutTree, NodeStatus, PlannedFile, PlannedHunk, RenderRow,
    RowContext, RowId, RowKind,
};

/// Builds the compact base layout plan from the parsed diff.
///
/// The returned row indexes include file chrome, hunk headers, diff lines, and
/// spacer rows, but store only file and hunk spans instead of per-row records.
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

        let trailing_spacer = row_count;
        row_count += 1;

        files.push(PlannedFile {
            file_index,
            start: file_start,
            trailing_spacer,
            hunks,
        });
    }

    LayoutPlan {
        files,
        hunk_ranges,
        row_count,
    }
}

/// Resolves a base-layout row index into semantic row identity.
///
/// This is the lookup counterpart to `build_layout_plan`; callers use it for
/// navigation, selection, and note anchoring without requiring cached render
/// rows to exist.
pub(super) fn row_context_for_plan_row(
    session: &DiffSession,
    plan: &LayoutPlan,
    row_index: usize,
) -> Option<RowContext> {
    match row_id_for_plan_row(plan, row_index)? {
        RowId::FileSeparator { file_index } => Some(RowContext {
            file_index: Some(file_index),
            hunk_index: None,
            kind: RowKind::Separator,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        }),
        RowId::FileHeader { file_index } => Some(RowContext {
            file_index: Some(file_index),
            hunk_index: None,
            kind: RowKind::FileHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        }),
        RowId::HunkHeader {
            file_index,
            hunk_index,
        } => Some(RowContext {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: RowKind::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        }),
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
            Some(RowContext {
                file_index: Some(file_index),
                hunk_index: Some(hunk_index),
                kind: RowKind::DiffLine,
                old_lineno: line.old_lineno(),
                new_lineno: line.new_lineno(),
                note_id: None,
            })
        }
        RowId::HunkSpacer {
            file_index,
            hunk_index,
        } => Some(RowContext {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: RowKind::Spacer,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        }),
        RowId::FileSpacer { file_index } => Some(RowContext {
            file_index: Some(file_index),
            hunk_index: None,
            kind: RowKind::Spacer,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        }),
    }
}

/// Converts one base-layout row into inline and side-by-side render rows.
///
/// Resident hunk rows are read from `LayoutTree`; unloaded diff lines render as
/// blank placeholders while unloaded hunk headers can still render from
/// `DiffSession` so navigation has visible anchors.
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

/// Materializes all base row contexts for code that explicitly needs a slice.
///
/// This is intentionally not stored on `Layout`; use it only at boundaries such
/// as note anchoring or tests that compare logical rows.
pub(super) fn plan_row_contexts(session: &DiffSession, plan: &LayoutPlan) -> Vec<RowContext> {
    (0..plan.row_count)
        .filter_map(|row| row_context_for_plan_row(session, plan, row))
        .collect()
}

/// Finds the base-layout row index for a non-note row context.
///
/// Note rows are overlay rows and are handled by `layout::access`; this function
/// only maps identities that belong to the compact base plan.
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
                Some(file.trailing_spacer)
            }
        }
        RowKind::Note => None,
    }
}

/// Classifies a base row index using file and hunk spans.
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
    if row_index == file.trailing_spacer {
        return Some(RowId::FileSpacer {
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

/// Finds the planned file span containing a base row index.
fn file_for_row(plan: &LayoutPlan, row_index: usize) -> Option<&PlannedFile> {
    let index = plan.files.partition_point(|file| file.start <= row_index);
    let file = plan.files.get(index.checked_sub(1)?)?;
    (row_index <= file.trailing_spacer).then_some(file)
}

/// Finds a planned file by diff file index.
fn file_for_index(plan: &LayoutPlan, file_index: usize) -> Option<&PlannedFile> {
    plan.files.get(file_index)
}

/// Finds the planned hunk span containing a base row index within a file.
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
        RenderRow::Static(Line::default()),
        RenderRow::Static(Line::default()),
    )
}
