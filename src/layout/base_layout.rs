use crate::core::diff::DiffSession;
use crate::layout::layout_tree::LayoutTree;
use crate::layout::primitives::{HunkRange, LayoutPlan};
use crate::layout::window::{LayoutBuildOptions, apply_loaded_hunk_window_sync};
use crate::log;

#[derive(Clone, Debug)]
pub struct BaseLayout {
    pub tree: LayoutTree,
    pub plan: LayoutPlan,
    pub hunk_ranges: Vec<HunkRange>,
}

impl BaseLayout {
    pub(super) fn new(session: &DiffSession, options: LayoutBuildOptions<'_>) -> Self {
        let mut timer = log::timer("layout_build_base");
        timer.field("files", session.files.len());
        let plan = LayoutPlan::new(session, options.display.file_states);
        let mut tree = LayoutTree::new(session, options);
        let window = apply_loaded_hunk_window_sync(
            &mut tree,
            &plan,
            session,
            options.widths,
            options.target,
        );
        let hunk_ranges = plan.hunk_ranges.clone();

        let base = Self {
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
}
