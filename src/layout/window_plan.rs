//! Loaded hunk window planning.
//!
//! This module decides which hunk render caches should be resident for a target
//! viewport and in which order missing hunks should be requested.

use std::collections::HashSet;

use crate::diff::DiffSession;
use crate::layout::window::HunkWindowTarget;
use crate::layout::worker::HunkBuildKey;

pub(super) struct LoadedHunkPlan {
    desired: HashSet<(usize, usize)>,
    pub ordered: Vec<HunkBuildKey>,
}

// FIXME: Again, why not have LoadedHunkPlan::new ?

impl LoadedHunkPlan {
    /// Returns whether a hunk should have rendered rows loaded.
    pub fn contains(&self, file_index: usize, hunk_index: usize) -> bool {
        self.desired.contains(&(file_index, hunk_index))
    }
}

/// Chooses which hunks should have rendered rows loaded.
///
/// The selected hunk is always included. Rows after the selected hunk cover the
/// viewport plus overscan; rows before it cover overscan only.
pub(super) fn plan_loaded_hunks(session: &DiffSession, target: HunkWindowTarget) -> LoadedHunkPlan {
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
