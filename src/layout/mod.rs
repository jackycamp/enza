//! Diff layout pipeline.
//!
//! The source diff lives in `DiffSession`; layout should not duplicate that data.
//! `plan` records where each file and hunk starts, `window` decides which hunks
//! have rendered rows loaded, and `lines` renders only the rows visible on
//! screen. Notes are inserted before base rows instead of changing the base row
//! map. Keep these layers separate so unloaded hunks still keep stable row
//! numbers without storing rendered rows for the whole diff.

use std::time::Instant;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::diff::DiffSession;
use crate::layout::base_layout::BaseLayout;
use crate::layout::layout_tree::HunkNode;
use crate::layout::notes::{NoteOverlay, NoteOverlayRequest};
use crate::layout::plan::{
    plan_row_index_for_context, plan_row_to_render_rows, row_context_for_plan_row,
};
use crate::layout::primitives::{HunkRange, LayoutRowLocation, NoteInsertion, RenderRow};
use crate::layout::window::{LoadedHunkLimits, LoadedHunkWindowRequest, apply_loaded_hunk_window};
use crate::log;
use crate::note::Note;

mod base_layout;
mod layout_tree;
mod lines;
mod notes;
mod plan;
mod primitives;
#[cfg(test)]
mod tests;
mod text;
mod window;
mod window_plan;
mod worker;

pub use layout_tree::NodeStatus;
pub(crate) use primitives::LayoutWidths;
pub use primitives::{RowContext, RowKind, RowViewState};
pub(crate) use window::{HunkWindowTarget, LayoutBuildOptions};
pub use worker::LayoutWorker;

#[derive(Clone, Debug)]
pub struct Layout {
    pub inline_width: usize,
    pub side_by_side_width: usize,
    target_state: LayoutTargetState,
    pub base: BaseLayout,
    pub hunk_ranges: Vec<HunkRange>,
    pub note_insertions: Vec<NoteInsertion>,
    pub row_count: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LayoutTargetState {
    generation: u64,
    generation_ready: bool,
    file: usize,
    hunk: usize,
    visible_start_row: Option<usize>,
    viewport_rows: usize,
    overscan_rows: usize,
}

impl Layout {
    /// Builds a fresh layout for the current diff, notes, dimensions, and target hunk.
    ///
    /// This synchronously builds rows for the selected area so the first frame
    /// has visible content. Later movement should use
    /// `ensure_hunk_window` so already-loaded hunk rows can be reused.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let target = HunkWindowTarget {
    ///     selected_file: 0,
    ///     selected_hunk: 2,
    ///     visible_start_row: None,
    ///     viewport_rows: 40,
    ///     overscan_rows: 80,
    /// };
    /// let layout = Layout::build(
    ///     &app.session,
    ///     &app.notes.items,
    ///     &app.notes.expanded_ids,
    ///     LayoutBuildOptions {
    ///         widths: LayoutWidths {
    ///             inline: 80,
    ///             side_by_side: 120,
    ///         },
    ///         target,
    ///     },
    /// );
    /// // -> Layout { inline_width: 80, side_by_side_width: 120, ... }
    /// ```
    pub fn build(
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
        options: LayoutBuildOptions,
    ) -> Self {
        let mut timer = log::timer("layout_build");
        timer.field("files", session.files.len());
        timer.field("notes", notes.len());
        timer.field("inline_width", options.widths.inline);
        timer.field("side_width", options.widths.side_by_side);
        timer.field("selected_file", options.target.selected_file);
        timer.field("selected_hunk", options.target.selected_hunk);
        timer.field("viewport_rows", options.target.viewport_rows);
        timer.field("overscan_rows", options.target.overscan_rows);

        let base = BaseLayout::new(session, options.widths, options.target);
        let mut layout = Self {
            inline_width: options.widths.inline,
            side_by_side_width: options.widths.side_by_side,
            target_state: LayoutTargetState {
                generation: 0,
                generation_ready: false,
                file: options.target.selected_file,
                hunk: options.target.selected_hunk,
                visible_start_row: options.target.visible_start_row,
                viewport_rows: options.target.viewport_rows,
                overscan_rows: options.target.overscan_rows,
            },
            base,
            hunk_ranges: Vec::new(),
            note_insertions: Vec::new(),
            row_count: 0,
        };
        layout.refresh_notes(session, notes, expanded_note_ids);
        timer.field("base_rows", layout.base.plan.row_count);
        timer.field("rows", layout.row_count);
        layout
    }

    /// Updates loaded hunk rows for a new viewport or selected hunk.
    ///
    /// Returns `true` when hunk rows were built, queued, applied, or unloaded and
    /// inserted note rows were refreshed. Target movement changes scheduling
    /// priority without invalidating compatible in-flight hunk builds.
    pub fn ensure_hunk_window(
        &mut self,
        worker: &LayoutWorker,
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
        target: HunkWindowTarget,
    ) -> bool {
        if !self.target_state.generation_ready {
            self.target_state.generation = worker.next_generation();
            self.target_state.generation_ready = true;
            worker.set_generation(self.target_state.generation);
        }
        if self.target_window_changed(target) {
            self.store_target_window(target);
        }
        let expand_start = Instant::now();
        let widths = self.widths();
        let window = apply_loaded_hunk_window(
            &mut self.base.tree,
            LoadedHunkWindowRequest {
                plan: &self.base.plan,
                worker,
                generation: self.target_state.generation,
                session,
                widths,
                target,
                limits: LoadedHunkLimits {
                    max_builds: 2,
                    max_evictions: 1,
                },
            },
        );
        if !window.changed {
            return false;
        }

        let flatten_start = Instant::now();
        let flatten_ms = flatten_start.elapsed().as_millis();

        let note_start = Instant::now();
        self.refresh_notes(session, notes, expanded_note_ids);
        let note_ms = note_start.elapsed().as_millis();
        let mut fields = vec![
            ("elapsed_ms", expand_start.elapsed().as_millis().to_string()),
            ("selected_file", target.selected_file.to_string()),
            ("selected_hunk", target.selected_hunk.to_string()),
            (
                "visible_start_row",
                target
                    .visible_start_row
                    .map(|row| row.to_string())
                    .unwrap_or_else(|| "selected".to_string()),
            ),
            ("viewport_rows", target.viewport_rows.to_string()),
            ("overscan_rows", target.overscan_rows.to_string()),
            ("built_hunks", window.built_hunks.to_string()),
            ("evicted_hunks", window.evicted_hunks.to_string()),
            ("built_rows", window.built_rows.to_string()),
            ("build_ms", window.build_ms.to_string()),
            ("flatten_ms", flatten_ms.to_string()),
            ("note_ms", note_ms.to_string()),
            ("missing_hunks", window.missing_hunks.to_string()),
            ("extra_hunks", window.extra_hunks.to_string()),
            ("queued_hunks", window.queued_hunks.to_string()),
            ("applied_hunks", window.applied_hunks.to_string()),
            ("base_rows", self.base.plan.row_count.to_string()),
            ("rows", self.row_count.to_string()),
        ];
        if let Some(rss_mb) = log::current_rss_mb() {
            fields.push(("rss_mb", rss_mb));
        }
        log::add_event("layout_expand", &fields);
        true
    }

    fn target_window_changed(&self, target: HunkWindowTarget) -> bool {
        self.target_state.file != target.selected_file
            || self.target_state.hunk != target.selected_hunk
            || self.target_state.visible_start_row != target.visible_start_row
            || self.target_state.viewport_rows != target.viewport_rows
            || self.target_state.overscan_rows != target.overscan_rows
    }

    fn store_target_window(&mut self, target: HunkWindowTarget) {
        self.target_state.file = target.selected_file;
        self.target_state.hunk = target.selected_hunk;
        self.target_state.visible_start_row = target.visible_start_row;
        self.target_state.viewport_rows = target.viewport_rows;
        self.target_state.overscan_rows = target.overscan_rows;
    }

    /// Synchronously builds the selected hunk if its rows are not loaded.
    ///
    /// This is used for explicit reveal/jump actions where the UI needs the
    /// target hunk available immediately instead of waiting for the worker.
    pub fn ensure_selected_hunk_ready_sync(
        &mut self,
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
        selected_file: usize,
        selected_hunk: usize,
    ) -> bool {
        let Some(file) = session.files.get(selected_file) else {
            return false;
        };
        let Some(hunk) = file.hunks.get(selected_hunk) else {
            return false;
        };
        let widths = self.widths();
        let Some(file_node) = self.base.tree.files.get_mut(selected_file) else {
            return false;
        };
        let Some(hunk_node) = file_node.hunks.get_mut(selected_hunk) else {
            return false;
        };
        if hunk_node.status == NodeStatus::Ready {
            return false;
        }

        *hunk_node = HunkNode::ready(selected_file, selected_hunk, &file.path, hunk, widths);

        self.refresh_notes(session, notes, expanded_note_ids);
        true
    }

    /// Rebuilds inserted note rows and adjusts hunk ranges for those insertions.
    ///
    /// Base layout rows are left untouched; only note insertions and derived
    /// row counts/ranges are refreshed.
    pub fn refresh_notes(
        &mut self,
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
    ) {
        let mut timer = log::timer("layout_refresh_notes");
        timer.field("notes", notes.len());
        timer.field("expanded_notes", expanded_note_ids.len());
        timer.field("base_rows", self.base.plan.row_count);
        let overlay = NoteOverlay::new(NoteOverlayRequest {
            session,
            base: &self.base,
            notes,
            expanded_note_ids,
            widths: self.widths(),
        });

        self.hunk_ranges = overlay.hunk_ranges;
        self.note_insertions = overlay.insertions;
        self.row_count = overlay.row_count;
        timer.field("rows", self.row_count);
    }

    /// Renders the rows visible on screen.
    ///
    /// This is intentionally bounded by `max_rows`; it must not render the whole
    /// layout just to draw one frame.
    pub fn render_visible_rows(
        &self,
        session: &DiffSession,
        side_by_side: bool,
        view: &RowViewState,
        max_rows: usize,
    ) -> Vec<Line<'static>> {
        let start = view.scroll as usize;
        let end = self.row_count.min(start.saturating_add(max_rows));

        (start..end)
            .map(|absolute_row| {
                let line = self.render_line_for_row(session, side_by_side, view, absolute_row);
                let in_selection = view
                    .selected_rows
                    .is_some_and(|(start, end)| absolute_row >= start && absolute_row <= end);

                if absolute_row == view.cursor_row {
                    highlight_cursor_line(line, view.cursor_focused, in_selection)
                } else if in_selection {
                    highlight_selected_line(line)
                } else {
                    line
                }
            })
            .collect()
    }

    /// Renders one absolute row index, applying selection-aware row variants.
    fn render_line_for_row(
        &self,
        session: &DiffSession,
        side_by_side: bool,
        view: &RowViewState,
        row_index: usize,
    ) -> Line<'static> {
        let row = match self.locate_row(row_index) {
            Some(LayoutRowLocation::Note {
                insertion_index,
                row_offset,
            }) => {
                let Some(insertion) = self.note_insertions.get(insertion_index) else {
                    return Line::default();
                };
                let rows = if side_by_side {
                    &insertion.side_by_side_rows
                } else {
                    &insertion.inline_rows
                };
                rows.get(row_offset).cloned()
            }
            Some(LayoutRowLocation::Base { base_index }) => {
                let (inline, side_by_side_row) = plan_row_to_render_rows(
                    session,
                    &self.base.tree,
                    &self.base.plan,
                    base_index,
                    self.side_by_side_width,
                );
                Some(if side_by_side {
                    side_by_side_row
                } else {
                    inline
                })
            }
            None => None,
        };

        row.unwrap_or_else(|| RenderRow::static_line(Line::default()))
            .into_line(view)
    }

    /// Returns the number of rows the UI can scroll through.
    ///
    /// Inline and side-by-side modes have the same row count; only row contents
    /// differ.
    pub fn line_count_for_mode(&self, _side_by_side: bool) -> usize {
        self.row_count
    }

    /// Returns what a rendered row represents.
    ///
    /// Base rows are looked up through `LayoutPlan`; inserted note rows are
    /// looked up through `note_insertions`.
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
    pub fn row_contexts(&self, session: &DiffSession) -> Vec<RowContext> {
        (0..self.row_count)
            .filter_map(|row| self.row_context(session, row))
            .collect()
    }

    /// Finds the rendered row index for a row description.
    ///
    /// Base rows are mapped through `LayoutPlan`; note rows are mapped through
    /// insertion positions.
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

    /// Builds a hunk loading target from the currently visible rendered rows.
    pub(crate) fn hunk_window_target(
        &self,
        selected_file: usize,
        selected_hunk: usize,
        scroll: u16,
        viewport_rows: usize,
        overscan_rows: usize,
    ) -> HunkWindowTarget {
        HunkWindowTarget {
            selected_file,
            selected_hunk,
            visible_start_row: Some(self.base_row_for_rendered_row(scroll as usize)),
            viewport_rows,
            overscan_rows,
        }
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

    fn base_row_for_rendered_row(&self, row: usize) -> usize {
        match self.locate_row(row.min(self.row_count.saturating_sub(1))) {
            Some(LayoutRowLocation::Base { base_index }) => base_index,
            Some(LayoutRowLocation::Note {
                insertion_index, ..
            }) => self
                .note_insertions
                .get(insertion_index)
                .map(|insertion| insertion.base_index)
                .unwrap_or_else(|| self.base.plan.row_count.saturating_sub(1)),
            None => self.base.plan.row_count.saturating_sub(1),
        }
    }

    fn widths(&self) -> LayoutWidths {
        LayoutWidths {
            inline: self.inline_width,
            side_by_side: self.side_by_side_width,
        }
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

/// Applies cursor background styling to an already-rendered line.
fn highlight_cursor_line(line: Line<'static>, focused: bool, in_selection: bool) -> Line<'static> {
    let cursor_style = if focused {
        if in_selection {
            Style::default().bg(Color::Rgb(58, 58, 58))
        } else {
            Style::default().bg(Color::Rgb(46, 46, 46))
        }
    } else {
        Style::default().bg(Color::Rgb(34, 34, 34))
    };
    patch_line_background(line, cursor_style)
}

/// Applies selection background styling to an already-rendered line.
fn highlight_selected_line(line: Line<'static>) -> Line<'static> {
    patch_line_background(line, Style::default().bg(Color::Rgb(40, 40, 40)))
}

/// Patches every span in a line with the provided background style.
fn patch_line_background(line: Line<'static>, patch: Style) -> Line<'static> {
    let spans = line
        .spans
        .into_iter()
        .map(|span| {
            let style = span.style.patch(patch);
            Span::styled(span.content, style)
        })
        .collect::<Vec<_>>();

    Line::from(spans)
}
