use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use git2::{Diff as GitDiff, Repository};

use super::{
    DiffStats, DiffStatsBreakdown, DiffTarget,
    model::delta_filter_letter,
    target::ResolvedDiff,
    walk::{SyntheticRead, for_each_patch, read_synthetic_added_file},
};

/// `DiffStatsLoader` loads diff counts.
///
/// This loader caches the counts until the caller drops the loader.
pub struct DiffStatsLoader<'repo> {
    repo: &'repo Repository,
    workdir: Option<PathBuf>,
    pub(super) cache: HashMap<ResolvedDiff, DiffStatsBreakdown>,
}

impl<'repo> DiffStatsLoader<'repo> {
    pub fn new(repo: &'repo Repository) -> Self {
        Self {
            repo,
            workdir: repo.workdir().map(std::path::Path::to_path_buf),
            cache: HashMap::new(),
        }
    }

    /// If the caller cancels the load, `load` returns `None`.
    ///
    /// `DiffStatsLoader` does not cache incomplete counts.
    pub fn load(
        &mut self,
        target: &DiffTarget,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<DiffStatsBreakdown>, git2::Error> {
        if is_cancelled() {
            return Ok(None);
        }

        let resolved = ResolvedDiff::resolve(self.repo, target)?;
        if is_cancelled() {
            return Ok(None);
        }
        if let Some(stats) = self.cache.get(&resolved) {
            return Ok(Some(stats.clone()));
        }

        let diff = resolved.load(self.repo)?;
        if is_cancelled() {
            return Ok(None);
        }
        let Some(stats) = collect_diff_stats(
            &diff,
            self.workdir.as_deref(),
            resolved.allows_worktree_fallback(),
            &mut is_cancelled,
        ) else {
            return Ok(None);
        };
        self.cache.insert(resolved, stats.clone());
        Ok(Some(stats))
    }
}

fn collect_diff_stats(
    diff: &GitDiff<'_>,
    workdir: Option<&Path>,
    allow_worktree_fallback: bool,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Option<DiffStatsBreakdown> {
    let mut stats = DiffStatsBreakdown::default();
    let complete = for_each_patch(
        diff,
        |_| true,
        is_cancelled,
        |delta, patch, is_cancelled| {
            let mut file_stats = DiffStats {
                files: 1,
                ..DiffStats::default()
            };
            let mut saw_hunk = false;

            for hunk_index in 0..patch.num_hunks() {
                if is_cancelled() {
                    return false;
                }
                let Ok((_, line_count)) = patch.hunk(hunk_index) else {
                    continue;
                };
                saw_hunk = true;

                for line_index in 0..line_count {
                    if is_cancelled() {
                        return false;
                    }
                    let Ok(line) = patch.line_in_hunk(hunk_index, line_index) else {
                        continue;
                    };
                    match line.origin() {
                        '+' => file_stats.additions += 1,
                        '-' => file_stats.deletions += 1,
                        _ => {}
                    }
                }
            }

            if allow_worktree_fallback && !saw_hunk {
                match read_synthetic_added_file(
                    workdir,
                    delta,
                    0,
                    |additions, _, _| *additions += 1,
                    is_cancelled,
                ) {
                    SyntheticRead::Complete(additions) => file_stats.additions = additions,
                    SyntheticRead::Cancelled => return false,
                    SyntheticRead::Unavailable => {}
                }
            }

            let status = delta_filter_letter(delta.status());
            stats.all.add(file_stats);
            stats.by_status.entry(status).or_default().add(file_stats);
            true
        },
    );

    complete.then_some(stats)
}
