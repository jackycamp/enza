use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::diff::{DiffFile, DiffHunk, DiffLine, DiffSession};
use crate::layout::plan::plan_row_contexts;
use crate::layout::{
    HunkWindowTarget, Layout, LayoutBuildOptions, LayoutWidths, LayoutWorker, RowKind,
};

use super::layout_tree::NodeStatus;

#[test]
fn viewport_growth_after_a_resize_can_load_more_hunks() {
    let session = session_with_hunks(4);
    let worker = LayoutWorker::new(Arc::new(session.clone()));
    let mut layout = Layout::build(&session, &[], &[], build_options(0, 1, 0));

    layout.ensure_hunk_window(&worker, &session, &[], &[], window_target(0, 100, 0));

    wait_until(Duration::from_millis(250), || {
        layout.ensure_hunk_window(&worker, &session, &[], &[], window_target(0, 100, 0));
        layout.base.tree.files[0]
            .hunks
            .iter()
            .all(|hunk| hunk.status == NodeStatus::Ready)
    });

    assert!(
        layout.base.tree.files[0]
            .hunks
            .iter()
            .all(|hunk| hunk.status == NodeStatus::Ready)
    );
}

#[test]
fn moving_the_window_evicts_hunks_outside_it() {
    let session = session_with_hunks(4);
    let worker = LayoutWorker::new(Arc::new(session.clone()));
    let mut layout = Layout::build(&session, &[], &[], build_options(0, 1, 0));
    assert_eq!(layout.base.tree.files[0].hunks[0].status, NodeStatus::Ready);
    let original_contexts = plan_row_contexts(&session, &layout.base.plan);
    let original_hunk_starts = hunk_starts(&layout);

    wait_until(Duration::from_secs(1), || {
        layout.ensure_hunk_window(&worker, &session, &[], &[], window_target(3, 1, 0));
        layout.base.tree.files[0].hunks[3].status == NodeStatus::Ready
            && layout.base.tree.files[0].hunks[0].status == NodeStatus::Unbuilt
    });

    assert_eq!(layout.base.tree.files[0].hunks[3].status, NodeStatus::Ready);
    assert_eq!(
        layout.base.tree.files[0].hunks[0].status,
        NodeStatus::Unbuilt
    );
    assert!(
        layout.base.tree.files[0].hunks[0]
            .rows
            .row_contexts
            .is_empty()
    );
    assert_eq!(
        plan_row_contexts(&session, &layout.base.plan),
        original_contexts
    );
    assert_eq!(hunk_starts(&layout), original_hunk_starts);
}

#[test]
fn last_row_belongs_to_the_last_hunk_not_a_file_spacer() {
    let session = session_with_hunks(4);
    let layout = Layout::build(&session, &[], &[], build_options(3, 1, 0));

    let last_context = layout
        .row_context(&session, layout.row_count - 1)
        .expect("last row should have context");

    assert_eq!(last_context.file_index, Some(0));
    assert_eq!(last_context.hunk_index, Some(3));
    assert_eq!(last_context.kind, RowKind::Spacer);
}

#[test]
fn moving_the_window_target_does_not_advance_the_layout_generation() {
    let session = session_with_hunks(5);
    let worker = LayoutWorker::new(Arc::new(session.clone()));
    let mut layout = Layout::build(&session, &[], &[], build_options(0, 1, 0));

    layout.ensure_hunk_window(&worker, &session, &[], &[], window_target(1, 1, 0));
    let generation = layout.target_state.generation;
    assert!(generation > 0);

    layout.ensure_hunk_window(&worker, &session, &[], &[], window_target(2, 1, 0));

    assert_eq!(
        layout.target_state.generation, generation,
        "moving the viewport target should not invalidate compatible in-flight hunk builds"
    );
}

fn build_options(
    selected_hunk: usize,
    viewport_rows: usize,
    overscan_rows: usize,
) -> LayoutBuildOptions {
    LayoutBuildOptions {
        widths: LayoutWidths {
            inline: 80,
            side_by_side: 80,
        },
        target: window_target(selected_hunk, viewport_rows, overscan_rows),
    }
}

fn window_target(
    selected_hunk: usize,
    viewport_rows: usize,
    overscan_rows: usize,
) -> HunkWindowTarget {
    HunkWindowTarget {
        selected_file: 0,
        selected_hunk,
        visible_start_row: None,
        viewport_rows,
        overscan_rows,
    }
}

fn session_with_hunks(count: usize) -> DiffSession {
    DiffSession {
        files: vec![DiffFile {
            path: "test.rs".to_string(),
            old_path: "test.rs".to_string(),
            new_path: "test.rs".to_string(),
            hunks: (0..count)
                .map(|index| DiffHunk {
                    header: format!("@@ hunk {index} @@"),
                    lines: vec![DiffLine::Context {
                        old_lineno: index + 1,
                        new_lineno: index + 1,
                        text: format!("line {index}"),
                    }],
                })
                .collect(),
        }],
    }
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn hunk_starts(layout: &Layout) -> Vec<(usize, usize, usize)> {
    layout
        .base
        .hunk_ranges
        .iter()
        .map(|range| (range.file_index, range.hunk_index, range.start))
        .collect()
}
