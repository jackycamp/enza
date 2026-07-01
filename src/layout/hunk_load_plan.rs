//! Hunk load planning.
//!
//! This module decides which hunk render caches should be resident for a target
//! viewport and in which order missing hunks should be requested.

use std::collections::HashSet;

use crate::layout::primitives::LayoutPlan;
use crate::layout::window::HunkWindowTarget;
use crate::layout::worker::HunkBuildKey;

pub(super) struct HunkLoadPlan {
    desired: HashSet<HunkBuildKey>,
    pub ordered: Vec<HunkBuildKey>,
}

impl HunkLoadPlan {
    /// Chooses which hunks should have rendered rows loaded.
    ///
    /// Initial builds use the selected hunk as the anchor. Incremental updates
    /// use the visible base-row window so scheduling follows what is actually
    /// on screen.
    pub(super) fn new(layout_plan: &LayoutPlan, target: HunkWindowTarget) -> Self {
        let all_hunks = planned_hunks(layout_plan);
        if all_hunks.is_empty() {
            return Self::empty();
        }

        if let Some(visible_start_row) = target.visible_start_row {
            return Self::for_visible_rows(
                &all_hunks,
                layout_plan.row_count,
                target,
                visible_start_row,
            );
        }

        Self::for_selected_hunk(&all_hunks, target)
    }

    fn for_visible_rows(
        all_hunks: &[PlannedHunkWindow],
        row_count: usize,
        target: HunkWindowTarget,
        visible_start_row: usize,
    ) -> Self {
        let visible_start = visible_start_row.min(row_count);
        let visible_end = visible_start
            .saturating_add(target.viewport_rows)
            .min(row_count);
        let overscan_start = visible_start.saturating_sub(target.overscan_rows);
        let overscan_end = visible_end
            .saturating_add(target.overscan_rows)
            .min(row_count);
        let mut plan = Self::empty();

        add_intersecting_hunks(&mut plan, all_hunks, visible_start, visible_end);
        add_intersecting_hunks(&mut plan, all_hunks, overscan_start, visible_start);
        add_intersecting_hunks(&mut plan, all_hunks, visible_end, overscan_end);

        if plan.ordered.is_empty() {
            add_selected_hunk(&mut plan, all_hunks, target);
        }

        plan
    }

    fn for_selected_hunk(all_hunks: &[PlannedHunkWindow], target: HunkWindowTarget) -> Self {
        let selected_key = HunkBuildKey {
            file_index: target.selected_file,
            hunk_index: target.selected_hunk,
        };
        let selected_index = all_hunks
            .iter()
            .position(|hunk| hunk.key == selected_key)
            .unwrap_or(0);
        let mut plan = Self::empty();
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
                all_hunks,
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

            progressed |= try_extend_down(
                all_hunks,
                &mut plan,
                &mut overscan_after_rows,
                overscan_after_target,
                &mut next_index,
            );
            progressed |= try_extend_up(
                all_hunks,
                &mut plan,
                &mut before_rows,
                before_target,
                &mut previous_index,
            );

            if !progressed {
                break;
            }
        }

        plan
    }

    /// Returns whether a hunk should have rendered rows loaded.
    pub fn contains(&self, file_index: usize, hunk_index: usize) -> bool {
        self.desired.contains(&HunkBuildKey {
            file_index,
            hunk_index,
        })
    }

    fn empty() -> Self {
        Self {
            desired: HashSet::new(),
            ordered: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct PlannedHunkWindow {
    key: HunkBuildKey,
    start: usize,
    end: usize,
}

impl PlannedHunkWindow {
    fn row_count(self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

fn planned_hunks(plan: &LayoutPlan) -> Vec<PlannedHunkWindow> {
    plan.files
        .iter()
        .flat_map(|file| {
            file.hunks.iter().map(|hunk| PlannedHunkWindow {
                key: HunkBuildKey {
                    file_index: hunk.file_index,
                    hunk_index: hunk.hunk_index,
                },
                start: hunk.start,
                end: hunk.start + hunk.line_count + 2,
            })
        })
        .collect()
}

fn add_intersecting_hunks(
    plan: &mut HunkLoadPlan,
    hunks: &[PlannedHunkWindow],
    start: usize,
    end: usize,
) {
    if start >= end {
        return;
    }

    for hunk in hunks {
        if hunk.end > start && hunk.start < end {
            add_desired_key(plan, hunk.key);
        }
    }
}

fn add_selected_hunk(
    plan: &mut HunkLoadPlan,
    hunks: &[PlannedHunkWindow],
    target: HunkWindowTarget,
) {
    let selected_key = HunkBuildKey {
        file_index: target.selected_file,
        hunk_index: target.selected_hunk,
    };
    if let Some(hunk) = hunks.iter().find(|hunk| hunk.key == selected_key) {
        add_desired_key(plan, hunk.key);
    }
}

fn add_desired_hunk(plan: &mut HunkLoadPlan, hunk: PlannedHunkWindow) {
    add_desired_key(plan, hunk.key);
}

fn add_desired_key(plan: &mut HunkLoadPlan, key: HunkBuildKey) {
    if plan.desired.insert(key) {
        plan.ordered.push(key);
    }
}

/// Adds the next hunk after the selected range if more downward coverage is needed.
fn try_extend_down(
    all_hunks: &[PlannedHunkWindow],
    plan: &mut HunkLoadPlan,
    covered_rows: &mut usize,
    target_rows: usize,
    next_index: &mut usize,
) -> bool {
    if *covered_rows >= target_rows || *next_index >= all_hunks.len() {
        return false;
    }

    let next = all_hunks[*next_index];
    add_desired_hunk(plan, next);
    *covered_rows += next.row_count();
    *next_index += 1;
    true
}

/// Adds the previous hunk before the selected range if more upward coverage is needed.
fn try_extend_up(
    all_hunks: &[PlannedHunkWindow],
    plan: &mut HunkLoadPlan,
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
    *covered_rows += previous.row_count();
    *previous_index = index.checked_sub(1);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffFile, DiffHunk, DiffLine, DiffSession};
    use crate::layout::primitives::LayoutPlan;

    #[test]
    fn planner_expands_after_the_selected_hunk_to_cover_the_viewport() {
        let session = session_with_hunks(4);
        let layout_plan = LayoutPlan::new(&session);
        let plan = HunkLoadPlan::new(
            &layout_plan,
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
        let layout_plan = LayoutPlan::new(&session);
        let plan = HunkLoadPlan::new(
            &layout_plan,
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
        let layout_plan = LayoutPlan::new(&session);
        let plan = HunkLoadPlan::new(
            &layout_plan,
            HunkWindowTarget {
                selected_hunk: 2,
                viewport_rows: 1,
                overscan_rows: 3,
                ..default_target()
            },
        );

        assert_eq!(ordered_hunks(&plan), vec![(0, 2), (0, 3), (0, 4), (0, 1)]);
    }

    #[test]
    fn planner_prioritizes_hunks_intersecting_the_visible_row_window() {
        let session = session_with_hunks(5);
        let layout_plan = LayoutPlan::new(&session);
        let visible_start = layout_plan.files[0].hunks[1].start;
        let plan = HunkLoadPlan::new(
            &layout_plan,
            HunkWindowTarget {
                selected_hunk: 4,
                visible_start_row: Some(visible_start),
                viewport_rows: 1,
                overscan_rows: 0,
                ..default_target()
            },
        );

        assert_eq!(ordered_hunks(&plan), vec![(0, 1)]);
    }

    fn desired_hunks(plan: &HunkLoadPlan) -> Vec<(usize, usize)> {
        let mut desired: Vec<_> = plan
            .desired
            .iter()
            .map(|key| (key.file_index, key.hunk_index))
            .collect();
        desired.sort_unstable();
        desired
    }

    fn ordered_hunks(plan: &HunkLoadPlan) -> Vec<(usize, usize)> {
        plan.ordered
            .iter()
            .map(|key| (key.file_index, key.hunk_index))
            .collect()
    }

    fn default_target() -> HunkWindowTarget {
        HunkWindowTarget {
            selected_file: 0,
            selected_hunk: 0,
            visible_start_row: None,
            viewport_rows: 0,
            overscan_rows: 0,
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
