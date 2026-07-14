use std::time::{Duration, Instant};

pub const AGENT_ACTIVITY_INTERVAL: Duration = Duration::from_millis(400);

pub type NoteId = u64;
pub type RunId = u64;

#[derive(Clone, Debug)]
pub struct Note {
    pub id: NoteId,
    pub target: NoteTarget,
    pub messages: Vec<NoteMessage>,
    pub agent: Option<AgentConversation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteMessage {
    pub author: MessageAuthor,
    pub body: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageAuthor {
    User,
    Agent(AgentProvider),
}

#[derive(Clone, Debug)]
pub struct AgentConversation {
    pub provider: AgentProvider,
    pub session_id: Option<String>,
    pub status: AgentStatus,
    pub current_run_id: Option<RunId>,
    pub composer_open: bool,
}

#[derive(Clone, Debug)]
pub enum AgentStatus {
    Queued,
    Running { started_at: Instant, slow: bool },
    Complete,
    Failed(AgentFailure),
    Cancelled,
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
            messages: vec![NoteMessage {
                author: MessageAuthor::User,
                body,
            }],
            agent: None,
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
            messages: vec![NoteMessage {
                author: MessageAuthor::User,
                body,
            }],
            agent: Some(AgentConversation {
                provider,
                session_id: None,
                status: AgentStatus::Queued,
                current_run_id: Some(run_id),
                composer_open: false,
            }),
        }
    }

    pub fn is_agent_thread(&self) -> bool {
        self.agent.is_some()
    }

    pub fn personal_body(&self) -> Option<&str> {
        if self.agent.is_some() {
            return None;
        }

        self.messages.first().map(|message| message.body.as_str())
    }

    pub fn set_personal_body(&mut self, body: String) -> bool {
        if self.agent.is_some() {
            return false;
        }

        let Some(message) = self.messages.first_mut() else {
            return false;
        };
        message.body = body;
        true
    }

    pub fn push_user_message(&mut self, body: String) {
        self.messages.push(NoteMessage {
            author: MessageAuthor::User,
            body,
        });
    }

    pub fn push_agent_message(&mut self, provider: AgentProvider, body: String) {
        self.messages.push(NoteMessage {
            author: MessageAuthor::Agent(provider),
            body,
        });
    }
}
