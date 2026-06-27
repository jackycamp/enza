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
use crate::layout::worker::{HunkBuildRequest, LayoutWorker};
use crate::log;
use crate::note::Note;
use crate::state::NavDirection;

struct NoteOverlay {
    insertions: Vec<NoteInsertion>,
    inserted_before_base: Vec<usize>,
    inserted_total: usize,
}

impl Layout {
    // FIXME: Has way to many arguments
    pub fn build(
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
        inline_width: usize,
        side_by_side_width: usize,
        selected_file: usize,
        selected_hunk: usize,
        viewport_rows: usize,
        overscan_rows: usize,
        nav_direction: Option<NavDirection>,
    ) -> Self {
        let mut timer = log::timer("layout_build");
        timer.field("files", session.files.len());
        timer.field("notes", notes.len());
        timer.field("inline_width", inline_width);
        timer.field("side_width", side_by_side_width);
        timer.field("selected_file", selected_file);
        timer.field("selected_hunk", selected_hunk);
        timer.field("viewport_rows", viewport_rows);
        timer.field("overscan_rows", overscan_rows);
        let base = build_base_layout(
            session,
            inline_width,
            side_by_side_width,
            selected_file,
            selected_hunk,
            viewport_rows,
            overscan_rows,
            nav_direction,
        );
        let mut layout = Self {
            inline_width,
            side_by_side_width,
            target_generation: 0,
            target_file: selected_file,
            target_hunk: selected_hunk,
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

    // FIXME: Again, way too many arguments
    pub fn ensure_hunk_window(
        &mut self,
        worker: &LayoutWorker,
        session: &DiffSession,
        notes: &[Note],
        expanded_note_ids: &[u64],
        selected_file: usize,
        selected_hunk: usize,
        viewport_rows: usize,
        overscan_rows: usize,
        nav_direction: Option<NavDirection>,
    ) -> bool {
        if self.target_file != selected_file || self.target_hunk != selected_hunk {
            self.target_generation = self.target_generation.wrapping_add(1);
            self.target_file = selected_file;
            self.target_hunk = selected_hunk;
            reset_loading_hunks(&mut self.base.tree);
            worker.set_generation(self.target_generation);
        }
        let expand_start = Instant::now();
        let window = apply_resident_hunk_window(
            &mut self.base.tree,
            worker,
            self.target_generation,
            session,
            self.inline_width,
            self.side_by_side_width,
            selected_file,
            selected_hunk,
            viewport_rows,
            overscan_rows,
            nav_direction,
            2,
            1,
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
            ("selected_file", selected_file.to_string()),
            ("selected_hunk", selected_hunk.to_string()),
            ("viewport_rows", viewport_rows.to_string()),
            ("overscan_rows", overscan_rows.to_string()),
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

// FIXME: Why so many arguments
fn build_base_layout(
    session: &DiffSession,
    inline_width: usize,
    side_by_side_width: usize,
    selected_file: usize,
    selected_hunk: usize,
    viewport_rows: usize,
    overscan_rows: usize,
    nav_direction: Option<NavDirection>,
) -> BaseLayout {
    let mut timer = log::timer("layout_build_base");
    timer.field("files", session.files.len());
    let plan = build_layout_plan(session);
    let mut tree = build_layout_tree(session, inline_width, side_by_side_width);
    let window = apply_resident_hunk_window_sync(
        &mut tree,
        session,
        inline_width,
        side_by_side_width,
        selected_file,
        selected_hunk,
        viewport_rows,
        overscan_rows,
        nav_direction,
    );
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

struct WindowResult {
    changed: bool,
    built_hunks: usize,
    evicted_hunks: usize,
    built_rows: usize,
    build_ms: u128,
    missing_hunks: usize,
    extra_hunks: usize,
    queued_hunks: usize,
    applied_hunks: usize,
}

// FIXME: too many arguments
fn apply_resident_hunk_window_sync(
    tree: &mut LayoutTree,
    session: &DiffSession,
    inline_width: usize,
    side_by_side_width: usize,
    selected_file: usize,
    selected_hunk: usize,
    viewport_rows: usize,
    overscan_rows: usize,
    nav_direction: Option<NavDirection>,
) -> WindowResult {
    let desired = resident_hunk_window(
        session,
        selected_file,
        selected_hunk,
        viewport_rows,
        overscan_rows,
        nav_direction,
    );
    let mut changed = false;
    let mut built_hunks = 0usize;
    let mut built_rows = 0usize;

    for (file_index, file) in session.files.iter().enumerate() {
        let Some(file_node) = tree.files.get_mut(file_index) else {
            continue;
        };

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            if !desired.contains(&(file_index, hunk_index)) {
                continue;
            }
            let Some(hunk_node) = file_node.hunks.get_mut(hunk_index) else {
                continue;
            };
            if hunk_node.status == NodeStatus::Ready {
                continue;
            }

            let node = build_hunk_node_for_worker(
                file_index,
                hunk_index,
                &file.path,
                hunk,
                inline_width,
                side_by_side_width,
            );
            built_rows += node.rows.row_contexts.len();
            *hunk_node = node;
            built_hunks += 1;
            changed = true;
        }
    }

    WindowResult {
        changed,
        built_hunks,
        evicted_hunks: 0,
        built_rows,
        build_ms: 0,
        missing_hunks: 0,
        extra_hunks: 0,
        queued_hunks: 0,
        applied_hunks: 0,
    }
}

// FIXME: Too many arguments
fn apply_resident_hunk_window(
    tree: &mut LayoutTree,
    worker: &LayoutWorker,
    generation: u64,
    session: &DiffSession,
    inline_width: usize,
    side_by_side_width: usize,
    selected_file: usize,
    selected_hunk: usize,
    viewport_rows: usize,
    overscan_rows: usize,
    nav_direction: Option<NavDirection>,
    max_builds: usize,
    max_evictions: usize,
) -> WindowResult {
    let desired = resident_hunk_window(
        session,
        selected_file,
        selected_hunk,
        viewport_rows,
        overscan_rows,
        nav_direction,
    );
    let mut changed = false;
    let mut built_hunks = 0usize;
    let mut evicted_hunks = 0usize;
    let mut built_rows = 0usize;
    let mut build_ms = 0u128;
    let mut missing_hunks = 0usize;
    let mut extra_hunks = 0usize;
    let mut queued_hunks = 0usize;
    let mut applied_hunks = 0usize;

    for result in worker.drain_completed() {
        if result.generation != generation {
            continue;
        }
        if result.inline_width != inline_width || result.side_by_side_width != side_by_side_width {
            continue;
        }
        let should_be_ready = desired.contains(&(result.file_index, result.hunk_index));
        let Some(file_node) = tree.files.get_mut(result.file_index) else {
            continue;
        };
        let Some(hunk_node) = file_node.hunks.get_mut(result.hunk_index) else {
            continue;
        };
        if hunk_node.status != NodeStatus::Loading {
            continue;
        }

        if should_be_ready {
            built_rows += result.node.rows.row_contexts.len();
            build_ms += result.build_ms;
            *hunk_node = result.node;
            applied_hunks += 1;
            built_hunks += 1;
            changed = true;
        } else {
            hunk_node.status = NodeStatus::Unbuilt;
            hunk_node.rows = CachedRows::default();
            changed = true;
        }
    }

    for (file_index, file_node) in tree.files.iter().enumerate() {
        for (hunk_index, hunk_node) in file_node.hunks.iter().enumerate() {
            let should_be_ready = desired.contains(&(file_index, hunk_index));
            match (hunk_node.status, should_be_ready) {
                (NodeStatus::Unbuilt | NodeStatus::Dirty | NodeStatus::Loading, true) => {
                    if hunk_node.status != NodeStatus::Ready {
                        missing_hunks += 1;
                    }
                }
                (NodeStatus::Ready, false) => extra_hunks += 1,
                _ => {}
            }
        }
    }

    if missing_hunks > 0 {
        for (file_index, file) in session.files.iter().enumerate() {
            let Some(file_node) = tree.files.get_mut(file_index) else {
                continue;
            };

            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                if queued_hunks >= max_builds {
                    break;
                }
                let should_be_ready = desired.contains(&(file_index, hunk_index));
                let Some(hunk_node) = file_node.hunks.get_mut(hunk_index) else {
                    continue;
                };

                if should_be_ready
                    && matches!(hunk_node.status, NodeStatus::Unbuilt | NodeStatus::Dirty)
                {
                    worker.request_hunk(HunkBuildRequest {
                        generation,
                        file_index,
                        hunk_index,
                        path: file.path.clone(),
                        hunk: hunk.clone(),
                        inline_width,
                        side_by_side_width,
                    });
                    hunk_node.status = NodeStatus::Loading;
                    queued_hunks += 1;
                    missing_hunks = missing_hunks.saturating_sub(1);
                }
            }
            if queued_hunks >= max_builds {
                break;
            }
        }
    }

    let remaining_missing = missing_hunks.saturating_sub(built_hunks);
    if remaining_missing == 0 {
        for file_node in &mut tree.files {
            for hunk_node in &mut file_node.hunks {
                let should_be_ready =
                    desired.contains(&(hunk_node.file_index, hunk_node.hunk_index));
                if hunk_node.status == NodeStatus::Ready
                    && !should_be_ready
                    && evicted_hunks < max_evictions
                {
                    hunk_node.status = NodeStatus::Unbuilt;
                    hunk_node.rows = CachedRows::default();
                    evicted_hunks += 1;
                    changed = true;
                }
            }
        }
    }

    WindowResult {
        changed,
        built_hunks,
        evicted_hunks,
        built_rows,
        build_ms,
        missing_hunks: remaining_missing,
        extra_hunks: extra_hunks.saturating_sub(evicted_hunks),
        queued_hunks,
        applied_hunks,
    }
}

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

fn resident_hunk_window(
    session: &DiffSession,
    selected_file: usize,
    selected_hunk: usize,
    viewport_rows: usize,
    overscan_rows: usize,
    nav_direction: Option<NavDirection>,
) -> std::collections::HashSet<(usize, usize)> {
    let all_hunks: Vec<(usize, usize, usize)> = session
        .files
        .iter()
        .enumerate()
        .flat_map(|(file_index, file)| {
            file.hunks
                .iter()
                .enumerate()
                .map(move |(hunk_index, hunk)| (file_index, hunk_index, hunk.lines.len() + 2))
        })
        .collect();

    if all_hunks.is_empty() {
        return std::collections::HashSet::new();
    }

    let selected_index = all_hunks
        .iter()
        .position(|&(file_index, hunk_index, _)| {
            file_index == selected_file && hunk_index == selected_hunk
        })
        .unwrap_or(0);
    let mut desired = std::collections::HashSet::new();
    let mut before_rows = 0usize;
    let mut after_rows = 0usize;
    let current = all_hunks[selected_index];
    desired.insert((current.0, current.1));

    let before_target = overscan_rows;
    let after_target = viewport_rows.saturating_add(overscan_rows);
    let mut previous_index = selected_index.checked_sub(1);
    let mut next_index = selected_index + 1;

    while before_rows < before_target || after_rows < after_target {
        let mut progressed = false;
        let prefer_up = matches!(nav_direction, Some(NavDirection::Up));
        let prefer_down = matches!(nav_direction, Some(NavDirection::Down));

        if prefer_up {
            progressed |= try_extend_up(
                &all_hunks,
                &mut desired,
                &mut before_rows,
                before_target,
                &mut previous_index,
            );
            progressed |= try_extend_down(
                &all_hunks,
                &mut desired,
                &mut after_rows,
                after_target,
                &mut next_index,
            );
        } else if prefer_down {
            // FIXME: this if has identical blocks
            progressed |= try_extend_down(
                &all_hunks,
                &mut desired,
                &mut after_rows,
                after_target,
                &mut next_index,
            );
            progressed |= try_extend_up(
                &all_hunks,
                &mut desired,
                &mut before_rows,
                before_target,
                &mut previous_index,
            );
        } else {
            progressed |= try_extend_down(
                &all_hunks,
                &mut desired,
                &mut after_rows,
                after_target,
                &mut next_index,
            );
            progressed |= try_extend_up(
                &all_hunks,
                &mut desired,
                &mut before_rows,
                before_target,
                &mut previous_index,
            );
        }

        if !progressed {
            break;
        }
    }

    desired
}

fn try_extend_down(
    all_hunks: &[(usize, usize, usize)],
    desired: &mut std::collections::HashSet<(usize, usize)>,
    covered_rows: &mut usize,
    target_rows: usize,
    next_index: &mut usize,
) -> bool {
    if *covered_rows >= target_rows || *next_index >= all_hunks.len() {
        return false;
    }

    let next = all_hunks[*next_index];
    desired.insert((next.0, next.1));
    *covered_rows += next.2;
    *next_index += 1;
    true
}

fn try_extend_up(
    all_hunks: &[(usize, usize, usize)],
    desired: &mut std::collections::HashSet<(usize, usize)>,
    covered_rows: &mut usize,
    target_rows: usize,
    previous_index: &mut Option<usize>,
) -> bool {
    if *covered_rows >= target_rows {
        return false;
    }
    let Some(index) = *previous_index else {
        return false;
    };

    let previous = all_hunks[index];
    desired.insert((previous.0, previous.1));
    *covered_rows += previous.2;
    *previous_index = index.checked_sub(1);
    true
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
