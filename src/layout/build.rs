use ratatui::text::Line;
use std::time::Instant;

use crate::diff::DiffSession;
use crate::highlight::FileHighlighter;
use crate::layout::lines::{
    build_combined_side_line, build_inline_line, file_header_line, file_header_row,
    file_separator_line, file_side_by_side_header_line, hunk_header_line, hunk_header_row,
    side_by_side_hunk_header_line,
};
use crate::layout::model::{
    BaseLayout, CachedRows, FileNode, HunkNode, HunkRange, Layout, LayoutTree, NodeStatus,
    NoteInsertion, RenderRow, RowContext, RowKind,
};
use crate::layout::notes::{
    build_note_anchors, build_note_rows, render_note_rows, render_side_by_side_note_rows,
};
use crate::layout::plan::{build_layout_plan, plan_row_contexts};
use crate::layout::window::{
    HunkWindowTarget, LayoutBuildOptions, LayoutWidths, LoadedHunkLimits, apply_loaded_hunk_window,
    apply_loaded_hunk_window_sync,
};
use crate::layout::worker::LayoutWorker;
use crate::log;
use crate::note::Note;

struct NoteOverlay {
    insertions: Vec<NoteInsertion>,
    inserted_before_base: Vec<usize>,
    inserted_total: usize,
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
    ///     viewport_rows: 40,
    ///     overscan_rows: 80,
    ///     nav_direction: None,
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
    /// // -> Layout { inline_width: 80, side_by_side_width: 120, target_file: 0, target_hunk: 2, ... }
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
            target_generation: 0,
            target_file: options.target.selected_file,
            target_hunk: options.target.selected_hunk,
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
    ///         nav_direction: Some(NavDirection::Down),
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
        if self.target_file != target.selected_file || self.target_hunk != target.selected_hunk {
            self.target_generation = self.target_generation.wrapping_add(1);
            self.target_file = target.selected_file;
            self.target_hunk = target.selected_hunk;
            reset_loading_hunks(&mut self.base.tree);
            worker.set_generation(self.target_generation);
        }
        let expand_start = Instant::now();
        let window = apply_loaded_hunk_window(
            &mut self.base.tree,
            worker,
            self.target_generation,
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
        let overlay = inject_notes(
            session,
            &self.base,
            notes,
            expanded_note_ids,
            self.inline_width,
            self.side_by_side_width,
        );

        self.hunk_ranges = adjust_hunk_ranges_for_insertions(
            self.base.hunk_ranges.clone(),
            &overlay.inserted_before_base,
        );
        self.note_insertions = overlay.insertions;
        self.row_count = self.base.plan.row_count + overlay.inserted_total;
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

fn build_layout_tree(
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
    file: &crate::diff::DiffFile,
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

    let trailing_spacer = CachedRows {
        inline_rows: vec![RenderRow::Static(Line::default())],
        side_by_side_rows: vec![RenderRow::Static(Line::default())],
        row_contexts: vec![RowContext {
            file_index: Some(file_index),
            hunk_index: None,
            kind: RowKind::Spacer,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        }],
    };

    FileNode {
        file_index,
        status: NodeStatus::Ready,
        header,
        hunks,
        trailing_spacer,
    }
}

fn build_hunk_node(
    file_index: usize,
    hunk_index: usize,
    hunk: &crate::diff::DiffHunk,
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
    hunk: &crate::diff::DiffHunk,
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

fn inject_notes(
    session: &DiffSession,
    base: &BaseLayout,
    notes: &[Note],
    expanded_note_ids: &[u64],
    inline_width: usize,
    side_by_side_width: usize,
) -> NoteOverlay {
    if notes.is_empty() {
        return NoteOverlay {
            insertions: Vec::new(),
            inserted_before_base: vec![0usize; base.plan.row_count + 1],
            inserted_total: 0,
        };
    }

    let base_row_contexts = plan_row_contexts(session, &base.plan);
    let note_anchors = build_note_anchors(session, notes, &base_row_contexts);
    let mut insertions = Vec::new();
    let mut inserted_before_base = vec![0usize; base.plan.row_count + 1];
    let mut inserted_total = 0usize;
    let note_wrap_width = inline_width.min(side_by_side_width);

    for base_index in 0..base.plan.row_count {
        inserted_before_base[base_index] = inserted_total;
        for note in note_anchors
            .iter()
            .filter(|(anchor_index, _)| *anchor_index == base_index)
            .map(|(_, note)| note)
        {
            let expanded = expanded_note_ids.contains(&note.id);
            let note_rows = build_note_rows(note, note_wrap_width, expanded);
            let note_context = RowContext {
                file_index: base_row_contexts[base_index].file_index,
                hunk_index: base_row_contexts[base_index].hunk_index,
                kind: RowKind::Note,
                old_lineno: None,
                new_lineno: None,
                note_id: Some(note.id),
            };

            insertions.push(build_note_insertion(
                base_index,
                &note_rows,
                note,
                note_context,
                inline_width,
                side_by_side_width,
            ));

            inserted_total += note_rows.len();
        }
    }
    inserted_before_base[base.plan.row_count] = inserted_total;

    NoteOverlay {
        insertions,
        inserted_before_base,
        inserted_total,
    }
}

fn build_note_insertion(
    base_index: usize,
    note_rows: &[String],
    note: &Note,
    note_context: RowContext,
    inline_width: usize,
    side_by_side_width: usize,
) -> NoteInsertion {
    NoteInsertion {
        base_index,
        inline_rows: render_note_rows(note_rows, inline_width)
            .into_iter()
            .map(RenderRow::Note)
            .collect(),
        side_by_side_rows: render_side_by_side_note_rows(note_rows, side_by_side_width, note)
            .into_iter()
            .map(RenderRow::Note)
            .collect(),
        context: note_context,
    }
}

fn adjust_hunk_ranges_for_insertions(
    hunk_ranges: Vec<HunkRange>,
    inserted_before_base: &[usize],
) -> Vec<HunkRange> {
    hunk_ranges
        .into_iter()
        .map(|range| HunkRange {
            file_index: range.file_index,
            hunk_index: range.hunk_index,
            start: range.start + inserted_before_base[range.start],
        })
        .collect()
}
