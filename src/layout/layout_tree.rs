//! Rendered layout tree construction.
//!
//! `LayoutTree` stores always-resident file rows and lazily loaded hunk row
//! caches. It owns render-cache construction but not base row planning or worker
//! scheduling.

use ratatui::text::Line;

use crate::diff::{DiffFile, DiffHunk, DiffSession};
use crate::highlight::FileHighlighter;
use crate::layout::lines::{
    InlineDiffLine, build_combined_side_line, file_header_line, file_separator_line,
    file_side_by_side_header_line, hunk_header_line, side_by_side_hunk_header_line,
};
use crate::layout::primitives::{LayoutWidths, RenderRow, RowContext};

#[derive(Clone, Debug)]
pub struct LayoutTree {
    pub files: Vec<FileNode>,
}

impl LayoutTree {
    pub(super) fn new(session: &DiffSession, widths: LayoutWidths) -> Self {
        let files = session
            .files
            .iter()
            .enumerate()
            .map(|(file_index, file)| FileNode::new(file_index, file, widths))
            .collect();

        Self { files }
    }
}

#[derive(Clone, Debug)]
pub struct FileNode {
    pub header: CachedRows,
    pub hunks: Vec<HunkNode>,
}

impl FileNode {
    fn new(file_index: usize, file: &DiffFile, widths: LayoutWidths) -> Self {
        let header = CachedRows {
            inline_rows: vec![
                RenderRow::static_line(file_separator_line(widths.inline)),
                RenderRow::file_header(
                    file_index,
                    file_header_line(file, false, widths.inline),
                    file_header_line(file, true, widths.inline),
                ),
            ],
            side_by_side_rows: vec![
                RenderRow::static_line(file_separator_line(widths.side_by_side)),
                RenderRow::file_header(
                    file_index,
                    file_side_by_side_header_line(file, false, widths.side_by_side),
                    file_side_by_side_header_line(file, true, widths.side_by_side),
                ),
            ],
            row_contexts: vec![
                RowContext::separator(file_index),
                RowContext::file_header(file_index),
            ],
        };

        let hunks = file
            .hunks
            .iter()
            .enumerate()
            .map(|(hunk_index, _)| HunkNode::unbuilt(file_index, hunk_index))
            .collect();

        Self { header, hunks }
    }
}

#[derive(Clone, Debug)]
pub struct HunkNode {
    pub file_index: usize,
    pub hunk_index: usize,
    pub status: NodeStatus,
    pub rows: CachedRows,
}

impl HunkNode {
    fn unbuilt(file_index: usize, hunk_index: usize) -> Self {
        Self {
            file_index,
            hunk_index,
            status: NodeStatus::Unbuilt,
            rows: CachedRows::default(),
        }
    }

    pub(super) fn ready(
        file_index: usize,
        hunk_index: usize,
        path: &str,
        hunk: &DiffHunk,
        widths: LayoutWidths,
    ) -> Self {
        let mut highlighter = FileHighlighter::new(path);
        let mut inline_rows = vec![RenderRow::hunk_header(
            file_index,
            hunk_index,
            hunk_header_line(&hunk.header, false),
            hunk_header_line(&hunk.header, true),
        )];
        let mut side_by_side_rows = vec![RenderRow::hunk_header(
            file_index,
            hunk_index,
            side_by_side_hunk_header_line(&hunk.header, false, widths.side_by_side),
            side_by_side_hunk_header_line(&hunk.header, true, widths.side_by_side),
        )];
        let mut row_contexts = vec![RowContext::hunk_header(file_index, hunk_index)];

        for diff_line in &hunk.lines {
            inline_rows.push(RenderRow::static_line(
                InlineDiffLine::new(diff_line, widths.inline).render(&mut highlighter),
            ));
            side_by_side_rows.push(RenderRow::static_line(build_combined_side_line(
                diff_line,
                widths.side_by_side,
                &mut highlighter,
            )));
            row_contexts.push(RowContext::diff_line(
                file_index,
                hunk_index,
                diff_line.old_lineno(),
                diff_line.new_lineno(),
            ));
        }

        inline_rows.push(RenderRow::static_line(Line::default()));
        side_by_side_rows.push(RenderRow::static_line(Line::default()));
        row_contexts.push(RowContext::spacer(file_index, hunk_index));

        Self {
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
}

#[derive(Clone, Debug, Default)]
pub struct CachedRows {
    /// Rendered rows for loaded hunks and always-loaded file header/separator rows.
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub row_contexts: Vec<RowContext>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeStatus {
    Ready,
    Loading,
    Unbuilt,
}
