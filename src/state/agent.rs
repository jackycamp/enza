use crate::agent::{AgentEvent, AgentRequest, build_agent_prompt};
use crate::note::{
    AGENT_ACTIVITY_INTERVAL, AgentFailure, AgentProvider, AgentRunState, Note, NoteId, NoteTarget,
    RunId,
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
            let AgentRunState::Running { started_at, .. } = note.agent_thread()?.state() else {
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
            .and_then(Note::agent_thread)
            .and_then(|thread| thread.state().active_run_id())
        else {
            return;
        };
        if !self.agent_runtime.cancel(run_id) {
            return;
        }
        let changed = self
            .notes
            .items
            .iter_mut()
            .find(|note| note.id == note_id)
            .and_then(Note::agent_thread_mut)
            .is_some_and(|thread| thread.cancel(run_id));
        if changed {
            self.refresh_note_overlay();
        }
    }

    pub fn retry_current_agent(&mut self) {
        let Some(note_id) = self.current_note_id() else {
            return;
        };
        let Some((provider, session_id, target, message)) = self
            .notes
            .items
            .iter()
            .find(|note| note.id == note_id)
            .and_then(|note| {
                let thread = note.agent_thread()?;
                if !thread.state().can_retry() {
                    return None;
                }
                Some((
                    thread.provider(),
                    thread.state().session_id().map(str::to_string),
                    note.target.clone(),
                    thread.last_user_message()?.to_string(),
                ))
            })
        else {
            return;
        };

        let prompt = if session_id.is_some() {
            message
        } else {
            build_agent_prompt(&self.review, &self.session, &target, &message)
        };
        self.start_agent_retry(note_id, provider, session_id, prompt);
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
            .and_then(Note::agent_thread)
            .and_then(|thread| {
                if !thread.state().can_reply() {
                    return None;
                }
                Some((thread.provider(), thread.state().session_id()?.to_string()))
            })
        else {
            return;
        };

        let run_id = self.notes.allocate_run_id();
        let queued = self
            .notes
            .items
            .iter_mut()
            .find(|note| note.id == note_id)
            .and_then(Note::agent_thread_mut)
            .is_some_and(|thread| thread.queue_reply(run_id, body.clone()));
        if !queued {
            return;
        }
        self.refresh_note_overlay();
        self.submit_agent_request(note_id, run_id, provider, Some(session_id), body);
    }

    fn start_agent_retry(
        &mut self,
        note_id: NoteId,
        provider: AgentProvider,
        session_id: Option<String>,
        prompt: String,
    ) {
        let run_id = self.notes.allocate_run_id();
        let queued = self
            .notes
            .items
            .iter_mut()
            .find(|note| note.id == note_id)
            .and_then(Note::agent_thread_mut)
            .is_some_and(|thread| thread.queue_retry(run_id));
        if !queued {
            return;
        }
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
        let Some(thread) = self
            .notes
            .items
            .iter_mut()
            .find(|note| note.id == note_id)
            .and_then(Note::agent_thread_mut)
        else {
            return false;
        };

        match event {
            AgentEvent::Started { started_at, .. } => thread.mark_running(run_id, started_at),
            AgentEvent::Slow { .. } => thread.mark_slow(run_id),
            AgentEvent::Completed {
                session_id,
                response,
                ..
            } => thread.complete(run_id, session_id, response),
            AgentEvent::Failed { failure, .. } => thread.fail(run_id, failure),
            AgentEvent::Cancelled { .. } => thread.cancel(run_id),
        }
    }

    fn fail_agent_run(&mut self, note_id: NoteId, run_id: RunId, failure: AgentFailure) {
        if let Some(thread) = self
            .notes
            .items
            .iter_mut()
            .find(|note| note.id == note_id)
            .and_then(Note::agent_thread_mut)
        {
            thread.fail(run_id, failure);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffSession;

    #[test]
    fn completed_events_append_the_reply_and_session_id() {
        let mut app = App::new(DiffSession { files: Vec::new() });
        app.notes.items.push(Note::new_agent(
            1,
            NoteTarget::File {
                file_path: "test.rs".to_string(),
            },
            AgentProvider::Codex,
            "Explain this".to_string(),
            7,
        ));

        assert!(app.apply_agent_event(AgentEvent::Started {
            note_id: 1,
            run_id: 7,
            started_at: std::time::Instant::now(),
        }));
        assert!(app.apply_agent_event(AgentEvent::Completed {
            note_id: 1,
            run_id: 7,
            session_id: "thread-1".to_string(),
            response: "This changes the queue.".to_string(),
        }));

        let thread = app.notes.items[0].agent_thread().unwrap();
        assert!(matches!(
            thread.state(),
            AgentRunState::Ready { session_id } if session_id == "thread-1"
        ));
        assert_eq!(thread.messages().len(), 2);
        assert_eq!(thread.messages()[1].body, "This changes the queue.");
    }
}
