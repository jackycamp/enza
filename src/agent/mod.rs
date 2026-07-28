mod claude;
mod codex;
mod mention;
mod prompt;
mod runtime;

pub use mention::{MentionParseError, ParsedNoteInput, parse_note_input};
pub use prompt::{ReviewContext, build_agent_prompt};
pub use runtime::{AgentEvent, AgentRequest, AgentRuntime};

#[derive(Debug)]
pub(super) struct ProviderOutput {
    pub session_id: String,
    pub response: String,
}
