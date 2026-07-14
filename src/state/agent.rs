use crate::agent::{AgentEvent, AgentRequest, build_agent_prompt};
use crate::note::{
    AGENT_ACTIVITY_INTERVAL, AgentFailure, AgentProvider, AgentStatus, MessageAuthor, Note, NoteId,
    NoteTarget, RunId,
};

use super::App;

impl App {
    pub fn drain_agent_events(&mut self) {
        let events = self.agent_runtime.drain_events();
        let mut changed = false;
        for event in events {
            changed |= self.apply_agent_event(event);
        }
        let animation_phase = self.notes.items.iter().find_map(|note| {
            let AgentStatus::Running { started_at, .. } = &note.agent.as_ref()?.status else {
                return None;
            };
            Some(started_at.elapsed().as_millis() / AGENT_ACTIVITY_INTERVAL.as_millis())
        });
        if animation_phase != self.agent_animation_phase {
            self.agent_animation_phase = animation_phase;
            changed |= animation_phase.is_some();
        }
        if changed {
            self.refresh_note_overlay();
        }
    }

    pub fn cancel_current_agent(&mut self) {
        let Some(note_id) = self.current_note_id() else {
            return;
        };
        let Some(run_id) = self
            .notes
            .items
            .iter()
            .find(|note| note.id == note_id)
            .and_then(|note| note.agent.as_ref())
            .and_then(|agent| agent.current_run_id)
        else {
            return;
        };
        if !self.agent_runtime.cancel(run_id) {
            return;
        }
        if let Some(agent) = self
            .notes
            .items
            .iter_mut()
            .find(|note| note.id == note_id)
            .and_then(|note| note.agent.as_mut())
        {
            agent.status = AgentStatus::Cancelled;
            agent.current_run_id = None;
            self.refresh_note_overlay();
        }
    }

    pub fn retry_current_agent(&mut self) {
        let Some(note_id) = self.current_note_id() else {
            return;
        };
        let Some((provider, session_id, target, message, retryable)) = self
            .notes
            .items
            .iter()
            .find(|note| note.id == note_id)
            .and_then(|note| {
                let agent = note.agent.as_ref()?;
                let retryable = match &agent.status {
                    AgentStatus::Failed(failure) => failure.retryable,
                    AgentStatus::Cancelled => true,
                    _ => false,
                };
                let message = note
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.author == MessageAuthor::User)?
                    .body
                    .clone();
                Some((
                    agent.provider,
                    agent.session_id.clone(),
                    note.target.clone(),
                    message,
                    retryable,
                ))
            })
        else {
            return;
        };
        if !retryable {
            return;
        }

        let prompt = if session_id.is_some() {
            message
        } else {
            build_agent_prompt(&self.review, &self.session, &target, &message)
        };
        self.start_agent_run(note_id, provider, session_id, prompt);
    }

    pub(super) fn add_agent_note(
        &mut self,
        target: NoteTarget,
        provider: AgentProvider,
        body: String,
    ) {
        let note_id = self.notes.allocate_note_id();
        let run_id = self.notes.allocate_run_id();
        let prompt = build_agent_prompt(&self.review, &self.session, &target, &body);
        self.notes
            .items
            .push(Note::new_agent(note_id, target, provider, body, run_id));
        self.refresh_note_overlay();
        self.submit_agent_request(note_id, run_id, provider, None, prompt);
    }

    pub(super) fn reply_to_agent_note(&mut self, note_id: NoteId, body: String) {
        let Some((provider, session_id)) = self
            .notes
            .items
            .iter()
            .find(|note| note.id == note_id)
            .and_then(|note| {
                let agent = note.agent.as_ref()?;
                if matches!(
                    agent.status,
                    AgentStatus::Queued | AgentStatus::Running { .. }
                ) {
                    return None;
                }
                Some((agent.provider, agent.session_id.clone()?))
            })
        else {
            return;
        };

        if let Some(note) = self.notes.items.iter_mut().find(|note| note.id == note_id) {
            note.push_user_message(body.clone());
        }
        self.start_agent_run(note_id, provider, Some(session_id), body);
    }

    fn start_agent_run(
        &mut self,
        note_id: NoteId,
        provider: AgentProvider,
        session_id: Option<String>,
        prompt: String,
    ) {
        let run_id = self.notes.allocate_run_id();
        let Some(note) = self.notes.items.iter_mut().find(|note| note.id == note_id) else {
            return;
        };
        let Some(agent) = &mut note.agent else {
            return;
        };
        agent.status = AgentStatus::Queued;
        agent.current_run_id = Some(run_id);
        self.refresh_note_overlay();
        self.submit_agent_request(note_id, run_id, provider, session_id, prompt);
    }

    fn submit_agent_request(
        &mut self,
        note_id: NoteId,
        run_id: RunId,
        provider: AgentProvider,
        session_id: Option<String>,
        prompt: String,
    ) {
        let request = AgentRequest {
            note_id,
            run_id,
            provider,
            repo_root: self.review.repo_root.clone(),
            prompt,
            session_id,
        };
        if let Err(failure) = self.agent_runtime.submit(request) {
            self.fail_agent_run(note_id, run_id, failure);
            self.refresh_note_overlay();
        }
    }

    pub(super) fn apply_agent_event(&mut self, event: AgentEvent) -> bool {
        let (note_id, run_id) = match &event {
            AgentEvent::Started {
                note_id, run_id, ..
            }
            | AgentEvent::Slow {
                note_id, run_id, ..
            }
            | AgentEvent::Completed {
                note_id, run_id, ..
            }
            | AgentEvent::Failed {
                note_id, run_id, ..
            }
            | AgentEvent::Cancelled {
                note_id, run_id, ..
            } => (*note_id, *run_id),
        };
        let Some(note) = self.notes.items.iter_mut().find(|note| note.id == note_id) else {
            return false;
        };
        let Some(agent) = &mut note.agent else {
            return false;
        };
        if agent.current_run_id != Some(run_id) {
            return false;
        }

        match event {
            AgentEvent::Started { started_at, .. } => {
                agent.status = AgentStatus::Running {
                    started_at,
                    slow: false,
                };
            }
            AgentEvent::Slow { .. } => match &mut agent.status {
                AgentStatus::Running { slow, .. } => *slow = true,
                _ => return false,
            },
            AgentEvent::Completed {
                session_id,
                response,
                ..
            } => {
                let provider = agent.provider;
                agent.session_id = Some(session_id);
                agent.status = AgentStatus::Complete;
                agent.current_run_id = None;
                note.push_agent_message(provider, response);
            }
            AgentEvent::Failed { failure, .. } => {
                agent.status = AgentStatus::Failed(failure);
                agent.current_run_id = None;
            }
            AgentEvent::Cancelled { .. } => {
                agent.status = AgentStatus::Cancelled;
                agent.current_run_id = None;
            }
        }
        true
    }

    fn fail_agent_run(&mut self, note_id: NoteId, run_id: RunId, failure: AgentFailure) {
        let Some(agent) = self
            .notes
            .items
            .iter_mut()
            .find(|note| note.id == note_id)
            .and_then(|note| note.agent.as_mut())
        else {
            return;
        };
        if agent.current_run_id == Some(run_id) {
            agent.status = AgentStatus::Failed(failure);
            agent.current_run_id = None;
        }
    }
}
