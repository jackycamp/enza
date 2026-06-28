//! Loaded hunk management.
//!
//! The planner chooses which hunks around the selected hunk should have rendered
//! rows loaded. The apply functions then build, queue, or unload hunk rows to
//! match that choice. Row numbers come from `LayoutPlan`; this module only
//! decides which hunk render rows are currently kept in memory.

use std::collections::HashSet;

use crate::diff::DiffSession;
use crate::layout::build::build_hunk_node_for_worker;
use crate::layout::model::{CachedRows, LayoutTree, NodeStatus};
use crate::layout::worker::{HunkBuildKey, HunkBuildWindowRequest, LayoutWorker};
use crate::state::NavDirection;

/// Render widths used when building hunk rows.
///
/// # Example
///
/// ```rust,ignore
/// let widths = LayoutWidths {
///     inline: 80,
///     side_by_side: 120,
/// };
/// // -> LayoutWidths { inline: 80, side_by_side: 120 }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LayoutWidths {
    pub inline: usize,
    pub side_by_side: usize,
}

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
///     nav_direction: Some(NavDirection::Down),
/// };
/// // -> HunkWindowTarget {
/// //      selected_file: 0,
/// //      selected_hunk: 2,
/// //      viewport_rows: 40,
/// //      overscan_rows: 80,
/// //      nav_direction: Some(NavDirection::Down),
/// //    }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HunkWindowTarget {
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub viewport_rows: usize,
    pub overscan_rows: usize,
    pub nav_direction: Option<NavDirection>,
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
///         nav_direction: None,
///     },
/// };
/// // -> LayoutBuildOptions {
/// //      widths: LayoutWidths { inline: 80, side_by_side: 120 },
/// //      target: HunkWindowTarget {
/// //          selected_file: 0,
/// //          selected_hunk: 2,
/// //          viewport_rows: 40,
/// //          overscan_rows: 80,
/// //          nav_direction: None,
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

struct LoadedHunkPlan {
    desired: HashSet<(usize, usize)>,
    ordered: Vec<HunkBuildKey>,
}

impl LoadedHunkPlan {
    /// Returns whether a hunk should have rendered rows loaded.
    fn contains(&self, file_index: usize, hunk_index: usize) -> bool {
        self.desired.contains(&(file_index, hunk_index))
    }
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

            let node = build_hunk_node_for_worker(
                file_index,
                hunk_index,
                &file.path,
                hunk,
                widths.inline,
                widths.side_by_side,
            );
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
        if result.inline_width != widths.inline || result.side_by_side_width != widths.side_by_side
        {
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
                (NodeStatus::Unbuilt | NodeStatus::Dirty | NodeStatus::Loading, true) => {
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

            if matches!(hunk_node.status, NodeStatus::Unbuilt | NodeStatus::Dirty) {
                hunk_node.status = NodeStatus::Loading;
                requested_hunks.push(*key);
                queued_hunks += 1;
                missing_hunks = missing_hunks.saturating_sub(1);
            }
        }

        if !requested_hunks.is_empty() {
            worker.request_window(HunkBuildWindowRequest {
                generation,
                inline_width: widths.inline,
                side_by_side_width: widths.side_by_side,
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

/// Chooses which hunks should have rendered rows loaded.
///
/// The selected hunk is always included. Rows after the selected hunk cover the
/// viewport plus overscan; rows before it cover overscan only.
fn plan_loaded_hunks(session: &DiffSession, target: HunkWindowTarget) -> LoadedHunkPlan {
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
        return LoadedHunkPlan {
            desired: HashSet::new(),
            ordered: Vec::new(),
        };
    }

    let selected_index = all_hunks
        .iter()
        .position(|&(file_index, hunk_index, _)| {
            file_index == target.selected_file && hunk_index == target.selected_hunk
        })
        .unwrap_or(0);
    let mut plan = LoadedHunkPlan {
        desired: HashSet::new(),
        ordered: Vec::new(),
    };
    let mut before_rows = 0usize;
    let mut visible_after_rows = 0usize;
    let mut overscan_after_rows = 0usize;
    let current = all_hunks[selected_index];
    add_desired_hunk(&mut plan, current);

    let before_target = target.overscan_rows;
    let visible_after_target = target.viewport_rows;
    let overscan_after_target = target.overscan_rows;
    let mut previous_index = selected_index.checked_sub(1);
    let mut next_index = selected_index + 1;

    while visible_after_rows < visible_after_target {
        if !try_extend_down(
            &all_hunks,
            &mut plan,
            &mut visible_after_rows,
            visible_after_target,
            &mut next_index,
        ) {
            break;
        }
    }

    while before_rows < before_target || overscan_after_rows < overscan_after_target {
        let mut progressed = false;
        let prefer_up = matches!(target.nav_direction, Some(NavDirection::Up));

        if prefer_up {
            progressed |= try_extend_up(
                &all_hunks,
                &mut plan,
                &mut before_rows,
                before_target,
                &mut previous_index,
            );
            progressed |= try_extend_down(
                &all_hunks,
                &mut plan,
                &mut overscan_after_rows,
                overscan_after_target,
                &mut next_index,
            );
        } else {
            progressed |= try_extend_down(
                &all_hunks,
                &mut plan,
                &mut overscan_after_rows,
                overscan_after_target,
                &mut next_index,
            );
            progressed |= try_extend_up(
                &all_hunks,
                &mut plan,
                &mut before_rows,
                before_target,
                &mut previous_index,
            );
        }

        if !progressed {
            break;
        }
    }

    plan
}

fn add_desired_hunk(plan: &mut LoadedHunkPlan, hunk: (usize, usize, usize)) {
    if plan.desired.insert((hunk.0, hunk.1)) {
        plan.ordered.push(HunkBuildKey {
            file_index: hunk.0,
            hunk_index: hunk.1,
        });
    }
}

/// Adds the next hunk after the selected range if more downward coverage is needed.
fn try_extend_down(
    all_hunks: &[(usize, usize, usize)],
    plan: &mut LoadedHunkPlan,
    covered_rows: &mut usize,
    target_rows: usize,
    next_index: &mut usize,
) -> bool {
    if *covered_rows >= target_rows || *next_index >= all_hunks.len() {
        return false;
    }

    let next = all_hunks[*next_index];
    add_desired_hunk(plan, next);
    *covered_rows += next.2;
    *next_index += 1;
    true
}

/// Adds the previous hunk before the selected range if more upward coverage is needed.
fn try_extend_up(
    all_hunks: &[(usize, usize, usize)],
    plan: &mut LoadedHunkPlan,
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
    add_desired_hunk(plan, previous);
    *covered_rows += previous.2;
    *previous_index = index.checked_sub(1);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffFile, DiffHunk, DiffLine, DiffSession};

    #[test]
    fn planner_expands_after_the_selected_hunk_to_cover_the_viewport() {
        let session = session_with_hunks(4);
        let plan = plan_loaded_hunks(
            &session,
            HunkWindowTarget {
                selected_hunk: 1,
                viewport_rows: 1,
                ..default_target()
            },
        );

        assert_eq!(desired_hunks(&plan), vec![(0, 1), (0, 2)]);
    }

    #[test]
    fn planner_uses_overscan_on_both_sides() {
        let session = session_with_hunks(4);
        let plan = plan_loaded_hunks(
            &session,
            HunkWindowTarget {
                selected_hunk: 1,
                viewport_rows: 0,
                overscan_rows: 3,
                ..default_target()
            },
        );

        assert_eq!(desired_hunks(&plan), vec![(0, 0), (0, 1), (0, 2)]);
    }

    #[test]
    fn planner_orders_visible_hunks_before_overscan() {
        let session = session_with_hunks(5);
        let plan = plan_loaded_hunks(
            &session,
            HunkWindowTarget {
                selected_hunk: 2,
                viewport_rows: 1,
                overscan_rows: 3,
                ..default_target()
            },
        );

        assert_eq!(ordered_hunks(&plan), vec![(0, 2), (0, 3), (0, 4), (0, 1)]);
    }

    fn desired_hunks(plan: &LoadedHunkPlan) -> Vec<(usize, usize)> {
        let mut desired: Vec<_> = plan.desired.iter().copied().collect();
        desired.sort_unstable();
        desired
    }

    fn ordered_hunks(plan: &LoadedHunkPlan) -> Vec<(usize, usize)> {
        plan.ordered
            .iter()
            .map(|key| (key.file_index, key.hunk_index))
            .collect()
    }

    fn default_target() -> HunkWindowTarget {
        HunkWindowTarget {
            selected_file: 0,
            selected_hunk: 0,
            viewport_rows: 0,
            overscan_rows: 0,
            nav_direction: None,
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
}
