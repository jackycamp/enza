//! Background loading, cancellation, and cleanup for the landing screen.

use std::{
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

use crate::{
    core::diff::{DiffFilter, DiffSession, DiffTarget},
    git::{ReviewSuggestion, ReviewSuggestions, load_suggestions},
    log,
};

pub(super) struct LandingWorker {
    worker: BackgroundWorker<ReviewSuggestions>,
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

impl LandingWorker {
    pub(super) fn new(repo_path: &Path) -> Self {
        let repo_path = repo_path.to_path_buf();
        Self::spawn(move |cancelled| load_suggestions(&repo_path, cancelled))
    }

    fn spawn<F>(load: F) -> Self
    where
        F: FnOnce(&AtomicBool) -> Option<ReviewSuggestions> + Send + 'static,
    {
        Self {
            worker: BackgroundWorker::spawn(load),
        }
    }

    pub(super) fn take_result(&self) -> Option<ReviewSuggestions> {
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
        match crate::git::load_session_cancellable(repo_path, &target, diff_filter, || {
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

pub(super) fn loading_worktree_suggestion() -> ReviewSuggestion {
    ReviewSuggestion {
        title: "Review your working tree".to_string(),
        command: "enza diff".to_string(),
        detail: "Calculating repository changes...".to_string(),
        target: DiffTarget::Worktree,
        diff_filter: None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{atomic::Ordering, mpsc},
        thread,
        time::Duration,
    };

    use super::{BackgroundWorker, LandingWorker, ReviewSuggestion, ReviewSuggestions};
    use crate::core::diff::{DiffStats, DiffTarget};

    #[test]
    fn landing_worker_publishes_loaded_data() {
        let worker = LandingWorker::spawn(|_| {
            Some(ReviewSuggestions {
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

    fn suggestion(command: &str, target: DiffTarget) -> ReviewSuggestion {
        ReviewSuggestion {
            title: "Review changes".to_string(),
            command: command.to_string(),
            detail: "Details".to_string(),
            target,
            diff_filter: None,
        }
    }
}
