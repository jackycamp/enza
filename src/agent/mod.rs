//! Agent request lifecycle and provider integration.
//!
//! This subsystem queues provider work and reports each run through shared
//! lifecycle events. Consumers apply those events to their own state; the runtime
//! does not mutate application features directly. Provider adapters enforce
//! read-only repository access and preserve session IDs for resumed work.

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
