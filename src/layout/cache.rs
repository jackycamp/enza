//! Cached layout row construction.
//!
//! This module builds the always-resident file rows and the lazily loaded hunk
//! row caches. It owns render-cache construction but not layout planning or
//! worker scheduling.

use ratatui::text::Line;

use crate::diff::{DiffFile, DiffHunk, DiffSession};
use crate::highlight::FileHighlighter;
use crate::layout::lines::{
    build_combined_side_line, build_inline_line, file_header_line, file_header_row,
    file_separator_line, file_side_by_side_header_line, hunk_header_line, hunk_header_row,
    side_by_side_hunk_header_line,
};
use crate::layout::model::{
    CachedRows, FileNode, HunkNode, LayoutTree, NodeStatus, RenderRow, RowContext, RowKind,
};

pub(super) fn build_layout_tree(
    session: &DiffSession,
    inline_width: usize,
    side_by_side_width: usize,
) -> LayoutTree {
    let files = session
        .files
        .iter()
        .enumerate()
        .map(|(file_index, file)| {
            build_file_node(file_index, file, inline_width, side_by_side_width)
        })
        .collect();

    LayoutTree { files }
}

fn build_file_node(
    file_index: usize,
    file: &DiffFile,
    inline_width: usize,
    side_by_side_width: usize,
) -> FileNode {
    let header = CachedRows {
        inline_rows: vec![
            RenderRow::Static(file_separator_line(inline_width)),
            file_header_row(
                file_index,
                file_header_line(file, false, inline_width),
                file_header_line(file, true, inline_width),
            ),
        ],
        side_by_side_rows: vec![
            RenderRow::Static(file_separator_line(side_by_side_width)),
            file_header_row(
                file_index,
                file_side_by_side_header_line(file, false, side_by_side_width),
                file_side_by_side_header_line(file, true, side_by_side_width),
            ),
        ],
        row_contexts: vec![
            RowContext {
                file_index: Some(file_index),
                hunk_index: None,
                kind: RowKind::Separator,
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            },
            RowContext {
                file_index: Some(file_index),
                hunk_index: None,
                kind: RowKind::FileHeader,
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            },
        ],
    };

    let hunks = file
        .hunks
        .iter()
        .enumerate()
        .map(|(hunk_index, _)| HunkNode {
            file_index,
            hunk_index,
            status: NodeStatus::Unbuilt,
            rows: CachedRows::default(),
        })
        .collect();

    FileNode { header, hunks }
}

fn build_hunk_node(
    file_index: usize,
    hunk_index: usize,
    hunk: &DiffHunk,
    inline_width: usize,
    side_by_side_width: usize,
    highlighter: &mut FileHighlighter<'static>,
) -> HunkNode {
    let mut inline_rows = vec![hunk_header_row(
        file_index,
        hunk_index,
        hunk_header_line(&hunk.header, false),
        hunk_header_line(&hunk.header, true),
    )];
    let mut side_by_side_rows = vec![hunk_header_row(
        file_index,
        hunk_index,
        side_by_side_hunk_header_line(&hunk.header, false, side_by_side_width),
        side_by_side_hunk_header_line(&hunk.header, true, side_by_side_width),
    )];
    let mut row_contexts = vec![RowContext {
        file_index: Some(file_index),
        hunk_index: Some(hunk_index),
        kind: RowKind::HunkHeader,
        old_lineno: None,
        new_lineno: None,
        note_id: None,
    }];

    for diff_line in &hunk.lines {
        inline_rows.push(RenderRow::Static(build_inline_line(
            diff_line,
            inline_width,
            highlighter,
        )));
        side_by_side_rows.push(RenderRow::Static(build_combined_side_line(
            diff_line,
            side_by_side_width,
            highlighter,
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

    HunkNode {
        file_index,
        hunk_index,
        status: NodeStatus::Ready,
        rows: CachedRows {
            inline_rows,
            side_by_side_rows,
            row_contexts,
        },
    }
}

/// Builds cached render rows for one hunk.
///
/// The worker calls this off-thread, while synchronous paths use it directly
/// when immediate hunk residency is required.
pub(crate) fn build_hunk_node_for_worker(
    file_index: usize,
    hunk_index: usize,
    path: &str,
    hunk: &DiffHunk,
    inline_width: usize,
    side_by_side_width: usize,
) -> HunkNode {
    let mut highlighter = FileHighlighter::new(path);
    build_hunk_node(
        file_index,
        hunk_index,
        hunk,
        inline_width,
        side_by_side_width,
        &mut highlighter,
    )
}
