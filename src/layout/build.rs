use std::time::Instant;

use crate::diff::DiffSession;
use crate::layout::cache::{build_hunk_node_for_worker, build_layout_tree};
use crate::layout::model::{
    BaseLayout, CachedRows, Layout, LayoutTargetState, LayoutTree, NodeStatus,
};
use crate::layout::note_overlay::build_note_overlay;
use crate::layout::plan::build_layout_plan;
use crate::layout::window::{
    HunkWindowTarget, LayoutBuildOptions, LayoutWidths, LoadedHunkLimits, apply_loaded_hunk_window,
    apply_loaded_hunk_window_sync,
};
use crate::layout::worker::LayoutWorker;
use crate::log;
use crate::note::Note;

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

        let base = build_base_layout(session, options.widths, options.target);
        let mut layout = Self {
            inline_width: options.widths.inline,
            side_by_side_width: options.widths.side_by_side,
            target_state: LayoutTargetState {
                generation: 0,
                generation_ready: false,
                file: options.target.selected_file,
                hunk: options.target.selected_hunk,
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
    /// inserted note rows were refreshed. Target changes advance the worker generation
    /// and discard stale loading hunks.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let changed = layout.ensure_hunk_window(
    ///     &app.layout_worker,
    ///     &app.session,
    ///     &app.notes.items,
    ///     &app.notes.expanded_ids,
    ///     HunkWindowTarget {
    ///         selected_file: 0,
    ///         selected_hunk: 3,
    ///         viewport_rows: 40,
    ///         overscan_rows: 80,
    ///     },
    /// );
    /// // -> true
    /// ```
    pub fn ensure_hunk_window(
        &mut self,
        worker: &LayoutWorker,
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
        target: HunkWindowTarget,
    ) -> bool {
        if !self.target_state.generation_ready || self.target_window_changed(target) {
            self.target_state.generation = worker.next_generation();
            self.target_state.generation_ready = true;
            self.store_target_window(target);
            reset_loading_hunks(&mut self.base.tree);
            worker.set_generation(self.target_state.generation);
        }
        let expand_start = Instant::now();
        let window = apply_loaded_hunk_window(
            &mut self.base.tree,
            worker,
            self.target_state.generation,
            session,
            LayoutWidths {
                inline: self.inline_width,
                side_by_side: self.side_by_side_width,
            },
            target,
            LoadedHunkLimits {
                max_builds: 2,
                max_evictions: 1,
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
            || self.target_state.viewport_rows != target.viewport_rows
            || self.target_state.overscan_rows != target.overscan_rows
    }

    fn store_target_window(&mut self, target: HunkWindowTarget) {
        self.target_state.file = target.selected_file;
        self.target_state.hunk = target.selected_hunk;
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
        let Some(file_node) = self.base.tree.files.get_mut(selected_file) else {
            return false;
        };
        let Some(hunk_node) = file_node.hunks.get_mut(selected_hunk) else {
            return false;
        };
        if hunk_node.status == NodeStatus::Ready {
            return false;
        }

        let node = build_hunk_node_for_worker(
            selected_file,
            selected_hunk,
            &file.path,
            hunk,
            self.inline_width,
            self.side_by_side_width,
        );
        *hunk_node = node;

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
        let overlay = build_note_overlay(
            session,
            &self.base,
            notes,
            expanded_note_ids,
            self.inline_width,
            self.side_by_side_width,
        );

        self.hunk_ranges = overlay.hunk_ranges;
        self.note_insertions = overlay.insertions;
        self.row_count = overlay.row_count;
        timer.field("rows", self.row_count);
    }
}

fn reset_loading_hunks(tree: &mut LayoutTree) {
    for file in &mut tree.files {
        for hunk in &mut file.hunks {
            if hunk.status == NodeStatus::Loading {
                hunk.status = NodeStatus::Unbuilt;
                hunk.rows = CachedRows::default();
            }
        }
    }
}

fn build_base_layout(
    session: &DiffSession,
    widths: LayoutWidths,
    target: HunkWindowTarget,
) -> BaseLayout {
    let mut timer = log::timer("layout_build_base");
    timer.field("files", session.files.len());
    let plan = build_layout_plan(session);
    let mut tree = build_layout_tree(session, widths.inline, widths.side_by_side);
    let window = apply_loaded_hunk_window_sync(&mut tree, session, widths, target);
    let hunk_ranges = plan.hunk_ranges.clone();

    let base = BaseLayout {
        tree,
        plan,
        hunk_ranges,
    };
    timer.field("base_rows", base.plan.row_count);
    timer.field("hunk_ranges", base.hunk_ranges.len());
    timer.field("built_hunks", window.built_hunks);
    timer.field("evicted_hunks", window.evicted_hunks);
    base
}
