use std::{
    collections::HashSet,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

use git2::{BranchType, Repository};

use crate::diff::{DiffFilter, DiffStats, DiffStatsBreakdown, DiffStatsLoader, DiffTarget};

pub(super) struct LandingData {
    pub(super) suggestions: Vec<LandingSuggestion>,
    pub(super) worktree_stats: DiffStats,
}

pub(super) struct LandingWorker {
    result_rx: Receiver<LandingData>,
    cancelled: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

struct SuggestionCollector<'a, 'repo> {
    suggestions: Vec<LandingSuggestion>,
    seen: HashSet<String>,
    stats_loader: &'a mut DiffStatsLoader<'repo>,
    cancelled: &'a AtomicBool,
}

#[derive(Clone)]
pub(super) struct LandingSuggestion {
    pub(super) title: String,
    pub(super) command: String,
    pub(super) detail: String,
    pub(super) target: DiffTarget,
    pub(super) diff_filter: Option<DiffFilter>,
}

impl LandingWorker {
    pub(super) fn new(repo_path: &Path) -> Self {
        let repo_path = repo_path.to_path_buf();
        Self::spawn(move |cancelled| load_landing_data(&repo_path, cancelled))
    }

    fn spawn<F>(load: F) -> Self
    where
        F: FnOnce(&AtomicBool) -> Option<LandingData> + Send + 'static,
    {
        let (result_tx, result_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);

        let handle = thread::spawn(move || {
            let Some(data) = load(&worker_cancelled) else {
                return;
            };
            if !worker_cancelled.load(Ordering::Relaxed) {
                let _ = result_tx.send(data);
            }
        });

        Self {
            result_rx,
            cancelled,
            handle: Some(handle),
        }
    }

    pub(super) fn take_result(&self) -> Option<LandingData> {
        self.result_rx.try_recv().ok()
    }

    pub(super) fn cancel_and_join(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LandingWorker {
    fn drop(&mut self) {
        self.cancel_and_join();
    }
}

fn load_landing_data(repo_path: &Path, cancelled: &AtomicBool) -> Option<LandingData> {
    let Ok(repo) = Repository::discover(repo_path) else {
        let worktree_stats = DiffStats::default();
        let mut suggestions = Vec::new();
        let mut seen = HashSet::new();
        push_worktree_suggestion(&mut suggestions, &mut seen, worktree_stats);
        return Some(LandingData {
            suggestions,
            worktree_stats,
        });
    };
    // Load the worktree once. The M, A and D suggestions reuse these counts.
    let mut stats_loader = DiffStatsLoader::new(&repo);
    let worktree_stats_breakdown =
        match stats_loader.load(&DiffTarget::Worktree, || cancelled.load(Ordering::Relaxed)) {
            Ok(Some(stats)) => stats,
            Ok(None) => return None,
            Err(_) => DiffStatsBreakdown::default(),
        };
    let worktree_stats = worktree_stats_breakdown.stats(None);
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }

    let suggestions = build_suggestions(&repo, &mut stats_loader, worktree_stats, cancelled);
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }

    Some(LandingData {
        suggestions,
        worktree_stats,
    })
}

fn build_suggestions(
    repo: &Repository,
    stats_loader: &mut DiffStatsLoader<'_>,
    worktree_stats: DiffStats,
    cancelled: &AtomicBool,
) -> Vec<LandingSuggestion> {
    let mut collector = SuggestionCollector {
        suggestions: Vec::new(),
        seen: HashSet::new(),
        stats_loader,
        cancelled,
    };
    push_worktree_suggestion(
        &mut collector.suggestions,
        &mut collector.seen,
        worktree_stats,
    );
    push_changed_suggestion(
        &mut collector,
        "Review staged changes".to_string(),
        "enza diff --cached".to_string(),
        DiffTarget::Cached,
        None,
        "Staged for the next commit",
    );

    if let Some(upstream) = current_upstream(repo) {
        push_changed_suggestion(
            &mut collector,
            "Review branch changes since upstream".to_string(),
            format!("enza diff {upstream}...HEAD"),
            DiffTarget::MergeBaseRange {
                base: upstream.clone(),
                head: "HEAD".to_string(),
            },
            None,
            "Compared from the merge base with upstream",
        );
        push_changed_suggestion(
            &mut collector,
            "Review unpushed commits".to_string(),
            format!("enza diff {upstream}..HEAD"),
            DiffTarget::Range {
                base: upstream.clone(),
                head: "HEAD".to_string(),
            },
            None,
            "Commits on this branch that are not upstream",
        );
        push_changed_suggestion(
            &mut collector,
            "Review incoming upstream changes".to_string(),
            format!("enza diff HEAD..{upstream}"),
            DiffTarget::Range {
                base: "HEAD".to_string(),
                head: upstream,
            },
            None,
            "Upstream commits not in this branch",
        );
    }

    for base in ["main", "master"] {
        if revision_exists(repo, base) {
            push_changed_suggestion(
                &mut collector,
                format!("Review branch changes since {base}"),
                format!("enza diff {base}...HEAD"),
                DiffTarget::MergeBaseRange {
                    base: base.to_string(),
                    head: "HEAD".to_string(),
                },
                None,
                &format!("Compared from the merge base with {base}"),
            );
        }
    }

    for (base, title, context) in [
        (
            "HEAD~1",
            "Review the last commit",
            "Changes introduced by the most recent commit",
        ),
        (
            "HEAD~5",
            "Review recent commits",
            "Changes across the last five commits",
        ),
    ] {
        if revision_exists(repo, base) {
            push_changed_suggestion(
                &mut collector,
                title.to_string(),
                format!("enza diff {base}..HEAD"),
                DiffTarget::Range {
                    base: base.to_string(),
                    head: "HEAD".to_string(),
                },
                None,
                context,
            );
        }
    }

    for (filter, title, context) in [
        (
            "M",
            "Review modified files only",
            "Working tree files changed in place",
        ),
        (
            "A",
            "Review added files only",
            "New or untracked working tree files",
        ),
        (
            "D",
            "Review deleted files only",
            "Working tree files removed from disk",
        ),
    ] {
        let Some(diff_filter) = DiffFilter::parse(filter) else {
            continue;
        };
        push_changed_suggestion(
            &mut collector,
            title.to_string(),
            format!("enza diff --diff-filter {filter}"),
            DiffTarget::Worktree,
            Some(diff_filter),
            context,
        );
    }

    collector.suggestions
}

fn push_worktree_suggestion(
    suggestions: &mut Vec<LandingSuggestion>,
    seen: &mut HashSet<String>,
    stats: DiffStats,
) {
    let command = "enza diff".to_string();
    if !seen.insert(command.clone()) {
        return;
    }

    let detail = if stats.has_changes() {
        format!("Changes not yet staged. {}", stats.summary())
    } else {
        "No working tree changes".to_string()
    };

    suggestions.push(LandingSuggestion {
        title: "Review your working tree".to_string(),
        command,
        detail,
        target: DiffTarget::Worktree,
        diff_filter: None,
    });
}

pub(super) fn loading_worktree_suggestion() -> LandingSuggestion {
    LandingSuggestion {
        title: "Review your working tree".to_string(),
        command: "enza diff".to_string(),
        detail: "Calculating repository changes...".to_string(),
        target: DiffTarget::Worktree,
        diff_filter: None,
    }
}

fn push_changed_suggestion(
    collector: &mut SuggestionCollector<'_, '_>,
    title: String,
    command: String,
    target: DiffTarget,
    diff_filter: Option<DiffFilter>,
    context: &str,
) {
    if collector.cancelled.load(Ordering::Relaxed) {
        return;
    }

    if !collector.seen.insert(command.clone()) {
        return;
    }

    let cancelled = collector.cancelled;
    let Ok(Some(stats)) = collector
        .stats_loader
        .load(&target, || cancelled.load(Ordering::Relaxed))
    else {
        return;
    };
    let stats = stats.stats(diff_filter.as_ref());
    if collector.cancelled.load(Ordering::Relaxed) {
        return;
    }

    if !stats.has_changes() {
        return;
    }

    collector.suggestions.push(LandingSuggestion {
        title,
        command,
        detail: format!("{context}. {}", stats.summary()),
        target,
        diff_filter,
    });
}

fn current_upstream(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }

    let branch_name = head.shorthand()?;
    let branch = repo.find_branch(branch_name, BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;
    upstream.name().ok().flatten().map(str::to_string)
}

fn revision_exists(repo: &Repository, revision: &str) -> bool {
    repo.revparse_single(revision).is_ok()
}

impl DiffStats {
    fn has_changes(self) -> bool {
        self.files > 0 || self.additions > 0 || self.deletions > 0
    }

    fn summary(self) -> String {
        format!(
            "{} {}, +{}, -{}",
            self.files,
            plural(self.files, "file", "files"),
            self.additions,
            self.deletions
        )
    }
}

fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{atomic::Ordering, mpsc},
        thread,
        time::Duration,
    };

    use super::{LandingData, LandingSuggestion, LandingWorker};
    use crate::diff::{DiffStats, DiffTarget};

    #[test]
    fn landing_worker_publishes_loaded_data() {
        let worker = LandingWorker::spawn(|_| {
            Some(LandingData {
                suggestions: vec![suggestion("enza diff", DiffTarget::Worktree)],
                worktree_stats: DiffStats::default(),
            })
        });

        let data = worker
            .result_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert_eq!(data.suggestions.len(), 1);
        assert_eq!(data.suggestions[0].command, "enza diff");
    }

    #[test]
    fn dropping_landing_worker_cancels_and_joins_its_thread() {
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let worker = LandingWorker::spawn(move |cancelled| {
            started_tx.send(()).unwrap();
            while !cancelled.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            stopped_tx.send(()).unwrap();
            None
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(worker);

        stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    fn suggestion(command: &str, target: DiffTarget) -> LandingSuggestion {
        LandingSuggestion {
            title: "Review changes".to_string(),
            command: command.to_string(),
            detail: "Details".to_string(),
            target,
            diff_filter: None,
        }
    }
}
