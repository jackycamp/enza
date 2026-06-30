//! Loaded hunk management.
//!
//! The planner chooses which hunks around the selected hunk should have rendered
//! rows loaded. The apply functions then build, queue, or unload hunk rows to
//! match that choice. Row numbers come from `LayoutPlan`; this module only
//! decides which hunk render rows are currently kept in memory.

use crate::diff::DiffSession;
use crate::layout::layout_tree::{CachedRows, HunkNode, LayoutTree, NodeStatus};
use crate::layout::primitives::LayoutWidths;
use crate::layout::window_plan::plan_loaded_hunks;
use crate::layout::worker::{HunkBuildWindowRequest, LayoutWorker};

/// Selection and viewport inputs used to choose which hunks are loaded.
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
/// // -> HunkWindowTarget {
/// //      selected_file: 0,
/// //      selected_hunk: 2,
/// //      viewport_rows: 40,
/// //      overscan_rows: 80,
/// //    }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HunkWindowTarget {
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub viewport_rows: usize,
    pub overscan_rows: usize,
}

/// Inputs required to build a full layout from scratch.
///
/// # Example
///
/// ```rust,ignore
/// let options = LayoutBuildOptions {
///     widths: LayoutWidths {
///         inline: 80,
///         side_by_side: 120,
///     },
///     target: HunkWindowTarget {
///         selected_file: 0,
///         selected_hunk: 2,
///         viewport_rows: 40,
///         overscan_rows: 80,
///     },
/// };
/// // -> LayoutBuildOptions {
/// //      widths: LayoutWidths { inline: 80, side_by_side: 120 },
/// //      target: HunkWindowTarget {
/// //          selected_file: 0,
/// //          selected_hunk: 2,
/// //          viewport_rows: 40,
/// //          overscan_rows: 80,
/// //      },
/// //    }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LayoutBuildOptions {
    pub widths: LayoutWidths,
    pub target: HunkWindowTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LoadedHunkLimits {
    pub max_builds: usize,
    pub max_evictions: usize,
}

// FIXME: Add docs for this, what is it for?
// Should we have impl LoadedHunkUpdate with constructors/fns too? instead of the
// apply_loaded_hunk_window_sync etc?
#[derive(Debug)]
pub(super) struct LoadedHunkUpdate {
    pub changed: bool,
    pub built_hunks: usize,
    pub evicted_hunks: usize,
    pub built_rows: usize,
    pub build_ms: u128,
    pub missing_hunks: usize,
    pub extra_hunks: usize,
    pub queued_hunks: usize,
    pub applied_hunks: usize,
}

/// Synchronously builds every hunk needed for the first frame.
///
/// Used during full layout construction so the first viewport has ready content
/// without waiting for the worker thread.
pub(super) fn apply_loaded_hunk_window_sync(
    tree: &mut LayoutTree,
    session: &DiffSession,
    widths: LayoutWidths,
    target: HunkWindowTarget,
) -> LoadedHunkUpdate {
    let plan = plan_loaded_hunks(session, target);
    let mut changed = false;
    let mut built_hunks = 0usize;
    let mut built_rows = 0usize;

    for (file_index, file) in session.files.iter().enumerate() {
        let Some(file_node) = tree.files.get_mut(file_index) else {
            continue;
        };

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            if !plan.contains(file_index, hunk_index) {
                continue;
            }
            let Some(hunk_node) = file_node.hunks.get_mut(hunk_index) else {
                continue;
            };
            if hunk_node.status == NodeStatus::Ready {
                continue;
            }

            let node = HunkNode::ready(file_index, hunk_index, &file.path, hunk, widths);
            built_rows += node.rows.row_contexts.len();
            *hunk_node = node;
            built_hunks += 1;
            changed = true;
        }
    }

    LoadedHunkUpdate {
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

/// Updates loaded hunks incrementally using the worker.
///
/// This drains finished worker results, queues missing hunks up to `limits`, and
/// unloads hunks that are no longer needed once all needed hunks are ready or
/// queued.
pub(super) fn apply_loaded_hunk_window(
    tree: &mut LayoutTree,
    worker: &LayoutWorker,
    generation: u64,
    session: &DiffSession,
    widths: LayoutWidths,
    target: HunkWindowTarget,
    limits: LoadedHunkLimits,
) -> LoadedHunkUpdate {
    let plan = plan_loaded_hunks(session, target);
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
        if result.widths != widths {
            continue;
        }
        let should_be_ready = plan.contains(result.file_index, result.hunk_index);
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
            let should_be_ready = plan.contains(file_index, hunk_index);
            match (hunk_node.status, should_be_ready) {
                (NodeStatus::Unbuilt | NodeStatus::Loading, true) => {
                    missing_hunks += 1;
                }
                (NodeStatus::Ready, false) => extra_hunks += 1,
                _ => {}
            }
        }
    }

    if missing_hunks > 0 {
        let mut requested_hunks = Vec::new();
        for key in &plan.ordered {
            if queued_hunks >= limits.max_builds {
                break;
            }
            let Some(file_node) = tree.files.get_mut(key.file_index) else {
                continue;
            };
            let Some(hunk_node) = file_node.hunks.get_mut(key.hunk_index) else {
                continue;
            };

            if hunk_node.status == NodeStatus::Unbuilt {
                hunk_node.status = NodeStatus::Loading;
                requested_hunks.push(*key);
                queued_hunks += 1;
                missing_hunks = missing_hunks.saturating_sub(1);
            }
        }

        if !requested_hunks.is_empty() {
            worker.request_window(HunkBuildWindowRequest {
                generation,
                widths,
                hunks: requested_hunks,
            });
            changed = true;
        }
    }

    let remaining_missing = missing_hunks.saturating_sub(built_hunks);
    if remaining_missing == 0 {
        for file_node in &mut tree.files {
            for hunk_node in &mut file_node.hunks {
                let should_be_ready = plan.contains(hunk_node.file_index, hunk_node.hunk_index);
                if hunk_node.status == NodeStatus::Ready
                    && !should_be_ready
                    && evicted_hunks < limits.max_evictions
                {
                    hunk_node.status = NodeStatus::Unbuilt;
                    hunk_node.rows = CachedRows::default();
                    evicted_hunks += 1;
                    changed = true;
                }
            }
        }
    }

    LoadedHunkUpdate {
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
