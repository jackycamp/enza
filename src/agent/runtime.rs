use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::agent::{ProviderOutput, claude, codex};
use crate::note::{AgentFailure, AgentFailureKind, AgentProvider, NoteId, RunId};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_RESPONSE_CHARS: usize = 64_000;

#[derive(Clone, Debug)]
pub struct AgentRequest {
    pub note_id: NoteId,
    pub run_id: RunId,
    pub provider: AgentProvider,
    pub repo_root: PathBuf,
    pub prompt: String,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    Started {
        note_id: NoteId,
        run_id: RunId,
        started_at: Instant,
    },
    Slow {
        note_id: NoteId,
        run_id: RunId,
    },
    Completed {
        note_id: NoteId,
        run_id: RunId,
        session_id: String,
        response: String,
    },
    Failed {
        note_id: NoteId,
        run_id: RunId,
        failure: AgentFailure,
    },
    Cancelled {
        note_id: NoteId,
        run_id: RunId,
    },
}

impl AgentEvent {
    fn run_id(&self) -> RunId {
        match self {
            Self::Started { run_id, .. }
            | Self::Slow { run_id, .. }
            | Self::Completed { run_id, .. }
            | Self::Failed { run_id, .. }
            | Self::Cancelled { run_id, .. } => *run_id,
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AgentRuntimeConfig {
    pub concurrency: usize,
    pub soft_timeout: Duration,
    pub hard_timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            concurrency: 2,
            soft_timeout: Duration::from_secs(2 * 60),
            hard_timeout: Duration::from_secs(10 * 60),
            max_output_bytes: 1024 * 1024,
        }
    }
}

struct AgentJob {
    request: AgentRequest,
    cancelled: Arc<AtomicBool>,
}

pub struct AgentRuntime {
    request_tx: Option<Sender<AgentJob>>,
    event_rx: Receiver<AgentEvent>,
    cancellations: Arc<Mutex<HashMap<RunId, Arc<AtomicBool>>>>,
    workers: Vec<JoinHandle<()>>,
}

impl fmt::Debug for AgentRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentRuntime")
            .field("workers", &self.workers.len())
            .field("active_runs", &self.cancellations.lock().unwrap().len())
            .finish()
    }
}

impl AgentRuntime {
    pub fn new() -> Self {
        Self::with_config(AgentRuntimeConfig::default())
    }

    pub fn with_config(config: AgentRuntimeConfig) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<AgentJob>();
        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>();
        let request_rx = Arc::new(Mutex::new(request_rx));
        let cancellations = Arc::new(Mutex::new(HashMap::new()));
        let mut workers = Vec::new();

        for _ in 0..config.concurrency.max(1) {
            let request_rx = Arc::clone(&request_rx);
            let event_tx = event_tx.clone();
            let worker_cancellations = Arc::clone(&cancellations);
            workers.push(thread::spawn(move || {
                worker_loop(request_rx, event_tx, worker_cancellations, config);
            }));
        }

        Self {
            request_tx: Some(request_tx),
            event_rx,
            cancellations,
            workers,
        }
    }

    pub fn submit(&self, request: AgentRequest) -> Result<(), AgentFailure> {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancellations
            .lock()
            .unwrap()
            .insert(request.run_id, Arc::clone(&cancelled));
        let Some(request_tx) = &self.request_tx else {
            self.cancellations.lock().unwrap().remove(&request.run_id);
            return Err(runtime_disconnected());
        };
        if request_tx
            .send(AgentJob {
                request: request.clone(),
                cancelled,
            })
            .is_err()
        {
            self.cancellations.lock().unwrap().remove(&request.run_id);
            return Err(runtime_disconnected());
        }
        Ok(())
    }

    pub fn cancel(&self, run_id: RunId) -> bool {
        let Some(cancelled) = self.cancellations.lock().unwrap().get(&run_id).cloned() else {
            return false;
        };
        cancelled.store(true, Ordering::Relaxed);
        true
    }

    pub fn drain_events(&self) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            if event.is_terminal() {
                self.cancellations.lock().unwrap().remove(&event.run_id());
            }
            events.push(event);
        }
        events
    }
}

impl Drop for AgentRuntime {
    fn drop(&mut self) {
        for cancelled in self.cancellations.lock().unwrap().values() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.request_tx.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker_loop(
    request_rx: Arc<Mutex<Receiver<AgentJob>>>,
    event_tx: Sender<AgentEvent>,
    cancellations: Arc<Mutex<HashMap<RunId, Arc<AtomicBool>>>>,
    config: AgentRuntimeConfig,
) {
    loop {
        let job = match request_rx.lock().unwrap().recv() {
            Ok(job) => job,
            Err(_) => return,
        };
        let mut terminal = TerminalEventGuard::new(&job.request, event_tx.clone());
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            execute_request(&job.request, &job.cancelled, &event_tx, config)
        }));
        let event = match outcome {
            Ok(Ok(output)) => AgentEvent::Completed {
                note_id: job.request.note_id,
                run_id: job.request.run_id,
                session_id: output.session_id,
                response: output.response,
            },
            Ok(Err(failure)) if failure.kind == AgentFailureKind::Cancelled => {
                AgentEvent::Cancelled {
                    note_id: job.request.note_id,
                    run_id: job.request.run_id,
                }
            }
            Ok(Err(failure)) => AgentEvent::Failed {
                note_id: job.request.note_id,
                run_id: job.request.run_id,
                failure,
            },
            Err(_) => AgentEvent::Failed {
                note_id: job.request.note_id,
                run_id: job.request.run_id,
                failure: AgentFailure::new(
                    AgentFailureKind::Internal,
                    "The agent worker stopped unexpectedly.",
                    true,
                ),
            },
        };
        cancellations.lock().unwrap().remove(&job.request.run_id);
        terminal.finish(event);
    }
}

fn execute_request(
    request: &AgentRequest,
    cancelled: &AtomicBool,
    event_tx: &Sender<AgentEvent>,
    config: AgentRuntimeConfig,
) -> Result<ProviderOutput, AgentFailure> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(cancelled_failure());
    }

    let mut command = provider_command(request);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(spawn_failure)?;
    let started_at = Instant::now();
    let _ = event_tx.send(AgentEvent::Started {
        note_id: request.note_id,
        run_id: request.run_id,
        started_at,
    });

    write_prompt(&mut child, &request.prompt)?;
    let stdout = child.stdout.take().ok_or_else(output_read_failure)?;
    let stderr = child.stderr.take().ok_or_else(output_read_failure)?;
    let stdout_reader = spawn_output_reader(stdout, config.max_output_bytes);
    let stderr_reader = spawn_output_reader(stderr, config.max_output_bytes);

    let mut slow_event_sent = false;
    let status = loop {
        if cancelled.load(Ordering::Relaxed) {
            stop_child(&mut child);
            join_output_reader(stdout_reader)?;
            join_output_reader(stderr_reader)?;
            return Err(cancelled_failure());
        }
        let elapsed = started_at.elapsed();
        if elapsed >= config.hard_timeout {
            stop_child(&mut child);
            join_output_reader(stdout_reader)?;
            join_output_reader(stderr_reader)?;
            return Err(AgentFailure::new(
                AgentFailureKind::Timeout,
                format!(
                    "{} did not respond within {} minutes.",
                    request.provider.label(),
                    config.hard_timeout.as_secs().div_ceil(60)
                ),
                true,
            ));
        }
        if !slow_event_sent && elapsed >= config.soft_timeout {
            slow_event_sent = true;
            let _ = event_tx.send(AgentEvent::Slow {
                note_id: request.note_id,
                run_id: request.run_id,
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => {
                stop_child(&mut child);
                join_output_reader(stdout_reader)?;
                join_output_reader(stderr_reader)?;
                return Err(AgentFailure::new(
                    AgentFailureKind::ProcessExit,
                    format!("Could not monitor {}: {error}", request.provider.label()),
                    true,
                ));
            }
        }
    };

    let stdout = join_output_reader(stdout_reader)?;
    let stderr = join_output_reader(stderr_reader)?;
    if !status.success() {
        return Err(exit_failure(request.provider, status, &stderr.text));
    }
    if stdout.truncated {
        return Err(AgentFailure::new(
            AgentFailureKind::InvalidResponse,
            format!(
                "{} returned more output than Enza can process.",
                request.provider.label()
            ),
            true,
        ));
    }

    let mut output = match request.provider {
        AgentProvider::Codex => codex::parse_output(&stdout.text, request.session_id.as_deref()),
        AgentProvider::Claude => claude::parse_output(&stdout.text, request.session_id.as_deref()),
    }?;
    output.response = truncate_agent_response(
        &sanitize_terminal_text(&output.response),
        MAX_RESPONSE_CHARS,
    );
    if output.response.is_empty() {
        return Err(AgentFailure::new(
            AgentFailureKind::MissingResponse,
            format!("{} returned an empty message.", request.provider.label()),
            true,
        ));
    }
    Ok(output)
}

fn provider_command(request: &AgentRequest) -> Command {
    match request.provider {
        AgentProvider::Codex => codex::command(&request.repo_root, request.session_id.as_deref()),
        AgentProvider::Claude => claude::command(&request.repo_root, request.session_id.as_deref()),
    }
}

fn write_prompt(child: &mut Child, prompt: &str) -> Result<(), AgentFailure> {
    let mut stdin = child.stdin.take().ok_or_else(output_read_failure)?;
    if let Err(error) = stdin.write_all(prompt.as_bytes()) {
        stop_child(child);
        return Err(AgentFailure::new(
            AgentFailureKind::OutputRead,
            format!("Could not send the note to the agent: {error}"),
            true,
        ));
    }
    drop(stdin);
    Ok(())
}

struct CapturedOutput {
    text: String,
    truncated: bool,
}

fn spawn_output_reader(
    mut reader: impl Read + Send + 'static,
    limit: usize,
) -> JoinHandle<io::Result<CapturedOutput>> {
    thread::spawn(move || {
        let mut retained = Vec::new();
        let mut buffer = [0u8; 8192];
        let mut truncated = false;
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let available = limit.saturating_sub(retained.len());
            let keep = available.min(read);
            retained.extend_from_slice(&buffer[..keep]);
            truncated |= keep < read;
        }
        Ok(CapturedOutput {
            text: String::from_utf8_lossy(&retained).into_owned(),
            truncated,
        })
    })
}

fn join_output_reader(
    reader: JoinHandle<io::Result<CapturedOutput>>,
) -> Result<CapturedOutput, AgentFailure> {
    reader
        .join()
        .map_err(|_| output_read_failure())?
        .map_err(|error| {
            AgentFailure::new(
                AgentFailureKind::OutputRead,
                format!("Could not read the agent response: {error}"),
                true,
            )
        })
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_failure(error: io::Error) -> AgentFailure {
    if error.kind() == io::ErrorKind::NotFound {
        return AgentFailure::new(
            AgentFailureKind::ExecutableNotFound,
            "The selected agent CLI is not installed or is not on PATH.",
            false,
        );
    }
    AgentFailure::new(
        AgentFailureKind::ProcessExit,
        format!("Could not start the agent: {error}"),
        true,
    )
}

fn exit_failure(provider: AgentProvider, status: ExitStatus, stderr: &str) -> AgentFailure {
    let message = sanitize_terminal_text(stderr);
    let lower = message.to_ascii_lowercase();
    let authentication = ["auth", "login", "credential", "api key"]
        .iter()
        .any(|needle| lower.contains(needle));
    let kind = if authentication {
        AgentFailureKind::Authentication
    } else {
        AgentFailureKind::ProcessExit
    };
    let message = if message.is_empty() {
        format!("{} exited with {status}.", provider.label())
    } else {
        truncate_chars(&message, 2_000)
    };
    AgentFailure::new(kind, message, true)
}

fn output_read_failure() -> AgentFailure {
    AgentFailure::new(
        AgentFailureKind::OutputRead,
        "Could not receive the agent response.",
        true,
    )
}

fn cancelled_failure() -> AgentFailure {
    AgentFailure::new(
        AgentFailureKind::Cancelled,
        "The agent request was cancelled.",
        true,
    )
}

fn runtime_disconnected() -> AgentFailure {
    AgentFailure::new(
        AgentFailureKind::RuntimeDisconnected,
        "The agent runtime is unavailable.",
        true,
    )
}

pub(crate) fn sanitize_terminal_text(text: &str) -> String {
    let mut cleaned = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            if characters.peek() == Some(&'[') {
                characters.next();
                for candidate in characters.by_ref() {
                    if ('@'..='~').contains(&candidate) {
                        break;
                    }
                }
            } else {
                characters.next();
            }
            continue;
        }
        if character == '\r' {
            continue;
        }
        if character.is_control() && !matches!(character, '\n' | '\t') {
            continue;
        }
        cleaned.push(character);
    }
    cleaned.trim().to_string()
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut value = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    value.push('…');
    value
}

fn truncate_agent_response(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }

    let marker = "\n\n[Response truncated by Enza.]";
    let body_limit = limit.saturating_sub(marker.chars().count());
    let mut value = text.chars().take(body_limit).collect::<String>();
    value.push_str(marker);
    value
}

struct TerminalEventGuard {
    note_id: NoteId,
    run_id: RunId,
    event_tx: Sender<AgentEvent>,
    finished: bool,
}

impl TerminalEventGuard {
    fn new(request: &AgentRequest, event_tx: Sender<AgentEvent>) -> Self {
        Self {
            note_id: request.note_id,
            run_id: request.run_id,
            event_tx,
            finished: false,
        }
    }

    fn finish(&mut self, event: AgentEvent) {
        self.finished = true;
        let _ = self.event_tx.send(event);
    }
}

impl Drop for TerminalEventGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = self.event_tx.send(AgentEvent::Failed {
            note_id: self.note_id,
            run_id: self.run_id,
            failure: AgentFailure::new(
                AgentFailureKind::Internal,
                "The agent stopped without returning a response.",
                true,
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_an_unfinished_run_emits_the_catch_all_failure() {
        let (event_tx, event_rx) = mpsc::channel();
        let request = AgentRequest {
            note_id: 4,
            run_id: 8,
            provider: AgentProvider::Codex,
            repo_root: PathBuf::from("/repo"),
            prompt: "question".to_string(),
            session_id: None,
        };
        drop(TerminalEventGuard::new(&request, event_tx));

        let AgentEvent::Failed {
            note_id,
            run_id,
            failure,
        } = event_rx.recv().unwrap()
        else {
            panic!("expected catch-all failure");
        };
        assert_eq!(note_id, 4);
        assert_eq!(run_id, 8);
        assert_eq!(failure.kind, AgentFailureKind::Internal);
    }

    #[test]
    fn terminal_sanitizer_removes_ansi_and_control_characters() {
        assert_eq!(
            sanitize_terminal_text("\u{1b}[31mred\u{1b}[0m\r\ntext\u{7}"),
            "red\ntext"
        );
    }

    #[test]
    fn truncation_is_unicode_safe() {
        assert_eq!(truncate_chars("one 🦆 three", 7), "one 🦆 …");
    }

    #[test]
    fn response_truncation_is_explicit() {
        let response = truncate_agent_response(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            40,
        );

        assert_eq!(response.chars().count(), 40);
        assert!(response.ends_with("[Response truncated by Enza.]"));
    }
}
