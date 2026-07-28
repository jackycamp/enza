use std::time::{Duration, Instant};

pub const AGENT_ACTIVITY_INTERVAL: Duration = Duration::from_millis(400);

pub type NoteId = u64;
pub type RunId = u64;

#[derive(Clone, Debug)]
pub struct Note {
    pub id: NoteId,
    pub target: NoteTarget,
    content: NoteContent,
}

#[derive(Clone, Debug)]
pub enum NoteContent {
    Personal(String),
    AgentThread(AgentThread),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteMessage {
    pub author: MessageAuthor,
    pub body: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageAuthor {
    User,
    Agent,
}

#[derive(Clone, Debug)]
pub struct AgentThread {
    provider: AgentProvider,
    messages: Vec<NoteMessage>,
    state: AgentRunState,
}

#[derive(Clone, Debug)]
pub enum AgentRunState {
    Queued {
        run_id: RunId,
        session_id: Option<String>,
    },
    Running {
        run_id: RunId,
        session_id: Option<String>,
        started_at: Instant,
        slow: bool,
    },
    Ready {
        session_id: String,
    },
    Failed {
        session_id: Option<String>,
        failure: AgentFailure,
    },
    Cancelled {
        session_id: Option<String>,
    },
}

impl AgentRunState {
    pub fn active_run_id(&self) -> Option<RunId> {
        match self {
            Self::Queued { run_id, .. } | Self::Running { run_id, .. } => Some(*run_id),
            Self::Ready { .. } | Self::Failed { .. } | Self::Cancelled { .. } => None,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Queued { session_id, .. }
            | Self::Running { session_id, .. }
            | Self::Failed { session_id, .. }
            | Self::Cancelled { session_id } => session_id.as_deref(),
            Self::Ready { session_id } => Some(session_id),
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Queued { .. } | Self::Running { .. })
    }

    pub fn can_reply(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub fn can_retry(&self) -> bool {
        matches!(
            self,
            Self::Failed {
                failure: AgentFailure {
                    retryable: true,
                    ..
                },
                ..
            } | Self::Cancelled { .. }
        )
    }
}

#[derive(Clone, Debug)]
pub struct AgentFailure {
    pub kind: AgentFailureKind,
    pub message: String,
    pub retryable: bool,
}

impl AgentFailure {
    pub fn new(kind: AgentFailureKind, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind,
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentFailureKind {
    ExecutableNotFound,
    Authentication,
    ProcessExit,
    Timeout,
    OutputRead,
    InvalidResponse,
    MissingResponse,
    RuntimeDisconnected,
    Cancelled,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProvider {
    Codex,
    Claude,
}

impl AgentProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }

    pub fn mention(self) -> &'static str {
        match self {
            Self::Codex => "@codex",
            Self::Claude => "@claude",
        }
    }
}

#[derive(Clone, Debug)]
pub enum NoteTarget {
    File {
        file_path: String,
    },
    Hunk {
        file_path: String,
        hunk_header: String,
    },
    Line {
        file_path: String,
        old_lineno: Option<usize>,
        new_lineno: Option<usize>,
    },
    Range {
        file_path: String,
        start_old_lineno: Option<usize>,
        start_new_lineno: Option<usize>,
        end_old_lineno: Option<usize>,
        end_new_lineno: Option<usize>,
    },
}

impl Note {
    pub fn new(id: NoteId, target: NoteTarget, body: String) -> Self {
        Self {
            id,
            target,
            content: NoteContent::Personal(body),
        }
    }

    pub fn new_agent(
        id: NoteId,
        target: NoteTarget,
        provider: AgentProvider,
        body: String,
        run_id: RunId,
    ) -> Self {
        Self {
            id,
            target,
            content: NoteContent::AgentThread(AgentThread::new(provider, body, run_id)),
        }
    }

    pub fn personal_body(&self) -> Option<&str> {
        let NoteContent::Personal(body) = &self.content else {
            return None;
        };
        Some(body)
    }

    pub fn set_personal_body(&mut self, body: String) -> bool {
        let NoteContent::Personal(current) = &mut self.content else {
            return false;
        };
        *current = body;
        true
    }

    pub fn agent_thread(&self) -> Option<&AgentThread> {
        let NoteContent::AgentThread(thread) = &self.content else {
            return None;
        };
        Some(thread)
    }

    pub fn agent_thread_mut(&mut self) -> Option<&mut AgentThread> {
        let NoteContent::AgentThread(thread) = &mut self.content else {
            return None;
        };
        Some(thread)
    }
}

impl AgentThread {
    fn new(provider: AgentProvider, body: String, run_id: RunId) -> Self {
        Self {
            provider,
            messages: vec![NoteMessage {
                author: MessageAuthor::User,
                body,
            }],
            state: AgentRunState::Queued {
                run_id,
                session_id: None,
            },
        }
    }

    pub fn provider(&self) -> AgentProvider {
        self.provider
    }

    pub fn messages(&self) -> &[NoteMessage] {
        &self.messages
    }

    pub fn state(&self) -> &AgentRunState {
        &self.state
    }

    pub fn last_user_message(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.author == MessageAuthor::User)
            .map(|message| message.body.as_str())
    }

    pub fn queue_retry(&mut self, run_id: RunId) -> bool {
        if !self.state.can_retry() {
            return false;
        }
        let session_id = self.state.session_id().map(str::to_string);
        self.state = AgentRunState::Queued { run_id, session_id };
        true
    }

    pub fn queue_reply(&mut self, run_id: RunId, body: String) -> bool {
        let AgentRunState::Ready { session_id } = &self.state else {
            return false;
        };
        let session_id = session_id.clone();
        self.messages.push(NoteMessage {
            author: MessageAuthor::User,
            body,
        });
        self.state = AgentRunState::Queued {
            run_id,
            session_id: Some(session_id),
        };
        true
    }

    pub fn mark_running(&mut self, run_id: RunId, started_at: Instant) -> bool {
        let AgentRunState::Queued {
            run_id: current_run_id,
            session_id,
        } = &self.state
        else {
            return false;
        };
        if *current_run_id != run_id {
            return false;
        }
        self.state = AgentRunState::Running {
            run_id,
            session_id: session_id.clone(),
            started_at,
            slow: false,
        };
        true
    }

    pub fn mark_slow(&mut self, run_id: RunId) -> bool {
        let AgentRunState::Running {
            run_id: current_run_id,
            slow,
            ..
        } = &mut self.state
        else {
            return false;
        };
        if *current_run_id != run_id {
            return false;
        }
        *slow = true;
        true
    }

    pub fn complete(&mut self, run_id: RunId, session_id: String, response: String) -> bool {
        if !matches!(
            self.state,
            AgentRunState::Running {
                run_id: current_run_id,
                ..
            } if current_run_id == run_id
        ) {
            return false;
        }
        self.messages.push(NoteMessage {
            author: MessageAuthor::Agent,
            body: response,
        });
        self.state = AgentRunState::Ready { session_id };
        true
    }

    pub fn fail(&mut self, run_id: RunId, failure: AgentFailure) -> bool {
        if self.state.active_run_id() != Some(run_id) {
            return false;
        }
        let session_id = self.state.session_id().map(str::to_string);
        self.state = AgentRunState::Failed {
            session_id,
            failure,
        };
        true
    }

    pub fn cancel(&mut self, run_id: RunId) -> bool {
        if self.state.active_run_id() != Some(run_id) {
            return false;
        }
        let session_id = self.state.session_id().map(str::to_string);
        self.state = AgentRunState::Cancelled { session_id };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> NoteTarget {
        NoteTarget::File {
            file_path: "test.rs".to_string(),
        }
    }

    #[test]
    fn run_lifecycle_owns_the_active_run_id() {
        let mut note = Note::new_agent(
            1,
            target(),
            AgentProvider::Codex,
            "Explain this".to_string(),
            7,
        );
        let thread = note.agent_thread_mut().unwrap();

        assert_eq!(thread.state().active_run_id(), Some(7));
        assert!(!thread.complete(7, "session-1".to_string(), "Too early".to_string()));
        assert!(thread.mark_running(7, Instant::now()));
        assert!(thread.complete(
            7,
            "session-1".to_string(),
            "It changes the queue.".to_string(),
        ));
        assert_eq!(thread.state().active_run_id(), None);
        assert!(thread.state().can_reply());
    }

    #[test]
    fn queue_reply_atomically_adds_the_message_and_run() {
        let mut note = Note::new_agent(
            1,
            target(),
            AgentProvider::Claude,
            "Explain this".to_string(),
            1,
        );
        let thread = note.agent_thread_mut().unwrap();
        assert!(thread.mark_running(1, Instant::now()));
        assert!(thread.complete(1, "session-1".to_string(), "First answer".to_string()));

        assert!(thread.queue_reply(2, "Follow up".to_string()));
        assert!(matches!(
            thread.state(),
            AgentRunState::Queued {
                run_id: 2,
                session_id: Some(session_id),
            } if session_id == "session-1"
        ));
        assert_eq!(thread.last_user_message(), Some("Follow up"));
        assert!(!thread.queue_reply(3, "Duplicate".to_string()));
    }
}
