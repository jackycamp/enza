use std::{
    collections::HashSet,
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

use git2::{BranchType, Repository};

use crate::{
    diff::{DiffFilter, DiffSession, DiffStats, DiffStatsBreakdown, DiffStatsLoader, DiffTarget},
    log,
};

pub(super) struct LandingData {
    pub(super) suggestions: Vec<LandingSuggestion>,
    pub(super) worktree_stats: DiffStats,
}

pub(super) struct LandingWorker {
    worker: BackgroundWorker<LandingData>,
}

pub(crate) struct LoadedDiff {
    pub(crate) session: DiffSession,
    pub(crate) target: DiffTarget,
}

pub(super) struct DiffLoadWorker {
    worker: BackgroundWorker<LoadedDiff>,
}

struct BackgroundWorker<T> {
    result_rx: Receiver<T>,
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
        Self {
            worker: BackgroundWorker::spawn(load),
        }
    }

    pub(super) fn take_result(&self) -> Option<LandingData> {
        self.worker.take_result()
    }

    pub(super) fn load_diff(
        mut self,
        repo_path: &Path,
        target: DiffTarget,
        diff_filter: Option<DiffFilter>,
    ) -> DiffLoadWorker {
        let previous = self.worker.cancel_and_take_handle();
        let repo_path = repo_path.to_path_buf();

        DiffLoadWorker {
            worker: BackgroundWorker::spawn(move |cancelled| {
                if let Some(handle) = previous {
                    let _ = handle.join();
                }
                if cancelled.load(Ordering::Relaxed) {
                    return None;
                }

                load_full_diff(&repo_path, target, diff_filter.as_ref(), cancelled)
            }),
        }
    }
}

impl DiffLoadWorker {
    pub(super) fn take_result(&self) -> Option<LoadedDiff> {
        self.worker.take_result()
    }
}

impl<T: Send + 'static> BackgroundWorker<T> {
    fn spawn<F>(load: F) -> Self
    where
        F: FnOnce(&AtomicBool) -> Option<T> + Send + 'static,
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

    fn take_result(&self) -> Option<T> {
        self.result_rx.try_recv().ok()
    }

    fn cancel_and_take_handle(&mut self) -> Option<thread::JoinHandle<()>> {
        self.cancelled.store(true, Ordering::Relaxed);
        self.handle.take()
    }
}

impl<T> Drop for BackgroundWorker<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            reap_in_background(handle);
        }
    }
}

fn load_full_diff(
    repo_path: &Path,
    target: DiffTarget,
    diff_filter: Option<&DiffFilter>,
    cancelled: &AtomicBool,
) -> Option<LoadedDiff> {
    let mut diff_load = log::timer("diff_load");
    let session =
        match DiffSession::load_from_repo_cancellable(repo_path, &target, diff_filter, || {
            cancelled.load(Ordering::Relaxed)
        }) {
            Ok(Some(session)) => session,
            Ok(None) => return None,
            Err(_) => DiffSession::default(),
        };

    diff_load.field("files", session.num_files());
    diff_load.field("hunks", session.num_hunks());
    diff_load.field("lines", session.num_lines());

    if cancelled.load(Ordering::Relaxed) {
        None
    } else {
        Some(LoadedDiff { session, target })
    }
}

fn reap_in_background(handle: thread::JoinHandle<()>) {
    static REAPER: OnceLock<mpsc::Sender<thread::JoinHandle<()>>> = OnceLock::new();

    let reaper = REAPER.get_or_init(|| {
        let (handle_tx, handle_rx) = mpsc::channel::<thread::JoinHandle<()>>();
        let _ = thread::Builder::new()
            .name("enza-worker-reaper".to_string())
            .spawn(move || {
                while let Ok(handle) = handle_rx.recv() {
                    let _ = handle.join();
                }
            });
        handle_tx
    });

    if let Err(mpsc::SendError(handle)) = reaper.send(handle) {
        let _ = thread::Builder::new()
            .name("enza-worker-cleanup".to_string())
            .spawn(move || {
                let _ = handle.join();
            });
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
    // The loader scans the worktree one time. The `M`, `A`, and `D` suggestions reuse these counts.
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

    use super::{BackgroundWorker, LandingData, LandingSuggestion, LandingWorker};
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
            .worker
            .result_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert_eq!(data.suggestions.len(), 1);
        assert_eq!(data.suggestions[0].command, "enza diff");
    }

    #[test]
    fn dropping_landing_worker_cancels_without_waiting_for_its_thread() {
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = LandingWorker::spawn(move |cancelled| {
            started_tx.send(()).unwrap();
            while !cancelled.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            let _ = release_rx.recv();
            stopped_tx.send(()).unwrap();
            None
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let (dropped_tx, dropped_rx) = mpsc::channel();
        thread::spawn(move || {
            drop(worker);
            dropped_tx.send(()).unwrap();
        });

        let dropped_without_waiting = dropped_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        assert!(stopped_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        stopped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(dropped_without_waiting);
    }

    #[test]
    fn replacement_work_starts_after_the_cancelled_worker_stops() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut previous = BackgroundWorker::spawn(move |cancelled| {
            started_tx.send(()).unwrap();
            while !cancelled.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            let _ = release_rx.recv();
            None::<()>
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let previous_handle = previous.cancel_and_take_handle();
        let (replacement_tx, replacement_rx) = mpsc::channel();
        let replacement = BackgroundWorker::spawn(move |_| {
            if let Some(handle) = previous_handle {
                let _ = handle.join();
            }
            replacement_tx.send(()).unwrap();
            Some(())
        });

        assert!(replacement_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        replacement_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        drop(replacement);
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
