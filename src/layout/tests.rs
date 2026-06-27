use std::thread;
use std::time::{Duration, Instant};

use crate::diff::{DiffFile, DiffHunk, DiffLine, DiffSession};
use crate::layout::plan::plan_row_contexts;
use crate::layout::{Layout, LayoutWorker};

use super::model::NodeStatus;

#[test]
fn viewport_growth_after_a_resize_can_load_more_hunks() {
    let session = session_with_hunks(4);
    let worker = LayoutWorker::new();
    let mut layout = Layout::build(&session, &[], &[], 80, 80, 0, 0, 1, 0, None);

    layout.ensure_hunk_window(&worker, &session, &[], &[], 0, 0, 100, 0, None);

    wait_until(Duration::from_millis(250), || {
        layout.ensure_hunk_window(&worker, &session, &[], &[], 0, 0, 100, 0, None);
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
    let worker = LayoutWorker::new();
    let mut layout = Layout::build(&session, &[], &[], 80, 80, 0, 0, 1, 0, None);
    assert_eq!(layout.base.tree.files[0].hunks[0].status, NodeStatus::Ready);
    let original_contexts = plan_row_contexts(&session, &layout.base.plan);
    let original_hunk_starts = hunk_starts(&layout);

    wait_until(Duration::from_secs(1), || {
        layout.ensure_hunk_window(&worker, &session, &[], &[], 0, 3, 1, 0, None);
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
