use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use crate::diff::DiffHunk;
use crate::layout::build::build_hunk_node_for_worker;
use crate::layout::model::HunkNode;

#[derive(Clone, Debug)]
pub struct HunkBuildRequest {
    pub generation: u64,
    pub file_index: usize,
    pub hunk_index: usize,
    pub path: String,
    pub hunk: DiffHunk,
    pub inline_width: usize,
    pub side_by_side_width: usize,
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
    request_tx: Sender<HunkBuildRequest>,
    result_rx: Receiver<HunkBuildResult>,
    current_generation: Arc<AtomicU64>,
}

impl LayoutWorker {
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<HunkBuildRequest>();
        let (result_tx, result_rx) = mpsc::channel::<HunkBuildResult>();
        let current_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&current_generation);

        thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                if request.generation != worker_generation.load(Ordering::Relaxed) {
                    continue;
                }
                let build_start = Instant::now();
                let node = build_hunk_node_for_worker(
                    request.file_index,
                    request.hunk_index,
                    &request.path,
                    &request.hunk,
                    request.inline_width,
                    request.side_by_side_width,
                );
                let _ = result_tx.send(HunkBuildResult {
                    generation: request.generation,
                    file_index: request.file_index,
                    hunk_index: request.hunk_index,
                    inline_width: request.inline_width,
                    side_by_side_width: request.side_by_side_width,
                    build_ms: build_start.elapsed().as_millis(),
                    node,
                });
            }
        });

        Self {
            request_tx,
            result_rx,
            current_generation,
        }
    }

    pub fn set_generation(&self, generation: u64) {
        self.current_generation.store(generation, Ordering::Relaxed);
    }

    pub fn request_hunk(&self, request: HunkBuildRequest) {
        let _ = self.request_tx.send(request);
    }

    pub fn drain_completed(&self) -> Vec<HunkBuildResult> {
        let mut results = Vec::new();
        while let Ok(result) = self.result_rx.try_recv() {
            results.push(result);
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::diff::{DiffHunk, DiffLine};

    #[test]
    fn stale_requests_do_not_publish_results() {
        let worker = LayoutWorker::new();
        worker.set_generation(2);
        worker.request_hunk(HunkBuildRequest {
            generation: 1,
            file_index: 0,
            hunk_index: 0,
            path: "test.rs".to_string(),
            hunk: DiffHunk {
                header: "@@ stale @@".to_string(),
                lines: vec![DiffLine::Context {
                    old_lineno: 1,
                    new_lineno: 1,
                    text: "stale".to_string(),
                }],
            },
            inline_width: 80,
            side_by_side_width: 80,
        });
        worker.request_hunk(HunkBuildRequest {
            generation: 2,
            file_index: 0,
            hunk_index: 1,
            path: "test.rs".to_string(),
            hunk: DiffHunk {
                header: "@@ current @@".to_string(),
                lines: vec![DiffLine::Context {
                    old_lineno: 2,
                    new_lineno: 2,
                    text: "current".to_string(),
                }],
            },
            inline_width: 80,
            side_by_side_width: 80,
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
}
