use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use crate::diff::DiffSession;
use crate::layout::cache::build_hunk_node_for_worker;
use crate::layout::model::HunkNode;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HunkBuildKey {
    pub file_index: usize,
    pub hunk_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HunkBuildWindowRequest {
    pub generation: u64,
    pub inline_width: usize,
    pub side_by_side_width: usize,
    pub hunks: Vec<HunkBuildKey>,
}

#[derive(Clone, Debug)]
pub struct HunkBuildResult {
    pub generation: u64,
    pub file_index: usize,
    pub hunk_index: usize,
    pub inline_width: usize,
    pub side_by_side_width: usize,
    pub build_ms: u128,
    pub node: HunkNode,
}

#[derive(Debug)]
pub struct LayoutWorker {
    state: Arc<WorkerState>,
    result_rx: Receiver<HunkBuildResult>,
}

impl LayoutWorker {
    pub fn new(session: Arc<DiffSession>) -> Self {
        let (result_tx, result_rx) = mpsc::channel::<HunkBuildResult>();
        let state = Arc::new(WorkerState::new());
        let worker_state = Arc::clone(&state);

        thread::spawn(move || {
            run_worker(session, worker_state, result_tx);
        });

        Self { state, result_rx }
    }

    pub fn next_generation(&self) -> u64 {
        self.state.next_generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn set_generation(&self, generation: u64) {
        self.state.set_generation(generation);
    }

    pub fn request_window(&self, request: HunkBuildWindowRequest) {
        self.state.request_window(request);
    }

    pub fn drain_completed(&self) -> Vec<HunkBuildResult> {
        let mut results = Vec::new();
        while let Ok(result) = self.result_rx.try_recv() {
            results.push(result);
        }
        results
    }
}

#[derive(Debug)]
struct WorkerState {
    current_generation: AtomicU64,
    next_generation: AtomicU64,
    pending: Mutex<PendingWorkerState>,
    pending_changed: Condvar,
}

#[derive(Debug, Default)]
struct PendingWorkerState {
    request: Option<HunkBuildWindowRequest>,
}

impl WorkerState {
    fn new() -> Self {
        Self {
            current_generation: AtomicU64::new(0),
            next_generation: AtomicU64::new(0),
            pending: Mutex::new(PendingWorkerState::default()),
            pending_changed: Condvar::new(),
        }
    }

    fn set_generation(&self, generation: u64) {
        let current = self.current_generation.load(Ordering::Relaxed);
        if generation <= current {
            return;
        }

        self.current_generation.store(generation, Ordering::Relaxed);
        self.pending.lock().unwrap().request = None;
        self.pending_changed.notify_one();
    }

    fn request_window(&self, request: HunkBuildWindowRequest) {
        let current = self.current_generation.load(Ordering::Relaxed);
        if request.generation < current {
            return;
        }

        self.current_generation
            .store(request.generation, Ordering::Relaxed);
        self.pending.lock().unwrap().request = Some(request);
        self.pending_changed.notify_one();
    }

    fn take_pending(&self) -> HunkBuildWindowRequest {
        let mut pending = self.pending.lock().unwrap();
        loop {
            if let Some(request) = pending.request.take() {
                return request;
            }
            pending = self.pending_changed.wait(pending).unwrap();
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.current_generation.load(Ordering::Relaxed) == generation
    }
}

fn run_worker(
    session: Arc<DiffSession>,
    state: Arc<WorkerState>,
    result_tx: Sender<HunkBuildResult>,
) {
    loop {
        let request = state.take_pending();
        for key in request.hunks {
            if !state.is_current(request.generation) {
                break;
            }

            let Some(file) = session.files.get(key.file_index) else {
                continue;
            };
            let Some(hunk) = file.hunks.get(key.hunk_index) else {
                continue;
            };

            let build_start = Instant::now();
            let node = build_hunk_node_for_worker(
                key.file_index,
                key.hunk_index,
                &file.path,
                hunk,
                request.inline_width,
                request.side_by_side_width,
            );

            if !state.is_current(request.generation) {
                continue;
            }

            if result_tx
                .send(HunkBuildResult {
                    generation: request.generation,
                    file_index: key.file_index,
                    hunk_index: key.hunk_index,
                    inline_width: request.inline_width,
                    side_by_side_width: request.side_by_side_width,
                    build_ms: build_start.elapsed().as_millis(),
                    node,
                })
                .is_err()
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::diff::{DiffFile, DiffHunk, DiffLine};

    #[test]
    fn latest_window_replaces_older_pending_work() {
        let state = WorkerState::new();
        state.request_window(window_request(1, &[0, 1]));
        state.request_window(window_request(2, &[2, 3]));

        assert_eq!(
            state.pending.lock().unwrap().request,
            Some(window_request(2, &[2, 3]))
        );
    }

    #[test]
    fn stale_requests_do_not_publish_results() {
        let worker = LayoutWorker::new(Arc::new(session_with_hunks(2)));
        worker.set_generation(2);
        worker.request_window(HunkBuildWindowRequest {
            generation: 1,
            inline_width: 80,
            side_by_side_width: 80,
            hunks: vec![HunkBuildKey {
                file_index: 0,
                hunk_index: 0,
            }],
        });
        worker.request_window(HunkBuildWindowRequest {
            generation: 2,
            inline_width: 80,
            side_by_side_width: 80,
            hunks: vec![HunkBuildKey {
                file_index: 0,
                hunk_index: 1,
            }],
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        let result = loop {
            if let Some(result) = worker.drain_completed().into_iter().next() {
                break result;
            }
            assert!(
                Instant::now() < deadline,
                "worker did not finish current request"
            );
            thread::sleep(Duration::from_millis(5));
        };

        assert_eq!(result.generation, 2);
        assert_eq!(result.hunk_index, 1);
        assert!(worker.drain_completed().is_empty());
    }

    #[test]
    fn pending_windows_store_hunk_ids_only() {
        let request = HunkBuildWindowRequest {
            generation: 1,
            inline_width: 80,
            side_by_side_width: 80,
            hunks: vec![HunkBuildKey {
                file_index: 3,
                hunk_index: 5,
            }],
        };

        assert_eq!(request.hunks[0].file_index, 3);
        assert_eq!(request.hunks[0].hunk_index, 5);
    }

    fn window_request(generation: u64, hunks: &[usize]) -> HunkBuildWindowRequest {
        HunkBuildWindowRequest {
            generation,
            inline_width: 80,
            side_by_side_width: 80,
            hunks: hunks
                .iter()
                .copied()
                .map(|hunk_index| HunkBuildKey {
                    file_index: 0,
                    hunk_index,
                })
                .collect(),
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
