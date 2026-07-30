//! This module loads Git diffs and defines the diff data types.
//!
//! `DiffSession::load_from_repo` supports worktree, staged, two-revision, and
//! merge-base comparisons. `DiffStatsLoader` uses the same target resolution and
//! patch walk. It stores only file, addition, and deletion counts.

mod model;
mod session;
mod stats;
mod target;
mod walk;

pub use model::{
    DiffFile, DiffFilter, DiffHunk, DiffLine, DiffSession, DiffStats, DiffStatsBreakdown,
    DiffTarget, FileChangeKind,
};
pub use stats::DiffStatsLoader;
pub use target::discover_repo_root;

#[cfg(test)]
mod tests;
