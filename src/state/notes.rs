use crate::note::{Note, NoteId, RunId};

pub enum NoteInputResult {
    Create { body: String },
    EditPersonal { note_id: NoteId, body: String },
    Reply { note_id: NoteId, body: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoteComposerMode {
    Create,
    EditPersonal { note_id: NoteId },
    Reply { note_id: NoteId },
}

#[derive(Debug)]
pub struct NoteState {
    pub items: Vec<Note>,
    pub expanded_ids: Vec<NoteId>,
    pub draft: Option<String>,
    pub composer_mode: Option<NoteComposerMode>,
    pub input_error: Option<String>,
    next_note_id: NoteId,
    next_run_id: RunId,
}

impl Default for NoteState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            expanded_ids: Vec::new(),
            draft: None,
            composer_mode: None,
            input_error: None,
            next_note_id: 1,
            next_run_id: 1,
        }
    }
}

impl NoteState {
    pub fn input_active(&self) -> bool {
        self.draft.is_some()
    }

    pub fn start_input(&mut self, current_note: Option<Note>) {
        if self.draft.is_some() {
            return;
        }
        self.input_error = None;

        match current_note {
            Some(note) if note.is_agent_thread() => {
                self.draft = Some(String::new());
                self.composer_mode = Some(NoteComposerMode::Reply { note_id: note.id });
                if let Some(agent) = self
                    .items
                    .iter_mut()
                    .find(|candidate| candidate.id == note.id)
                    .and_then(|candidate| candidate.agent.as_mut())
                {
                    agent.composer_open = true;
                }
            }
            Some(note) => {
                self.draft = Some(note.personal_body().unwrap_or_default().to_string());
                self.composer_mode = Some(NoteComposerMode::EditPersonal { note_id: note.id });
            }
            None => {
                self.draft = Some(String::new());
                self.composer_mode = Some(NoteComposerMode::Create);
            }
        }
    }

    pub fn cancel_input(&mut self) {
        self.close_reply_composer();
        self.draft = None;
        self.composer_mode = None;
        self.input_error = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        if let Some(draft) = &mut self.draft {
            draft.push_str(text);
            self.input_error = None;
        }
    }

    pub fn backspace_text(&mut self) {
        if let Some(draft) = &mut self.draft {
            draft.pop();
            self.input_error = None;
        }
    }

    pub fn finish_input(&mut self) -> Option<NoteInputResult> {
        let body = self.draft.take()?;
        let mode = self.composer_mode.take()?;
        if let NoteComposerMode::Reply { note_id } = mode
            && let Some(agent) = self
                .items
                .iter_mut()
                .find(|note| note.id == note_id)
                .and_then(|note| note.agent.as_mut())
        {
            agent.composer_open = false;
        }

        let body = body.trim().to_string();
        if body.is_empty() {
            return None;
        }

        match mode {
            NoteComposerMode::Create => Some(NoteInputResult::Create { body }),
            NoteComposerMode::EditPersonal { note_id } => {
                Some(NoteInputResult::EditPersonal { note_id, body })
            }
            NoteComposerMode::Reply { note_id } => Some(NoteInputResult::Reply { note_id, body }),
        }
    }

    pub fn allocate_note_id(&mut self) -> NoteId {
        let id = self.next_note_id;
        self.next_note_id += 1;
        id
    }

    pub fn restore_create_input(&mut self, body: String, error: String) {
        self.draft = Some(body);
        self.composer_mode = Some(NoteComposerMode::Create);
        self.input_error = Some(error);
    }

    pub fn allocate_run_id(&mut self) -> RunId {
        let id = self.next_run_id;
        self.next_run_id += 1;
        id
    }

    pub fn composer_note_id(&self) -> Option<NoteId> {
        match self.composer_mode? {
            NoteComposerMode::EditPersonal { note_id } | NoteComposerMode::Reply { note_id } => {
                Some(note_id)
            }
            NoteComposerMode::Create => None,
        }
    }

    pub fn toggle_expanded(&mut self, note_id: NoteId) {
        if self.expanded_ids.contains(&note_id) {
            self.expanded_ids.retain(|candidate| candidate != &note_id);
        } else {
            self.expanded_ids.push(note_id);
        }
    }

    fn close_reply_composer(&mut self) {
        let Some(NoteComposerMode::Reply { note_id }) = self.composer_mode else {
            return;
        };
        if let Some(agent) = self
            .items
            .iter_mut()
            .find(|note| note.id == note_id)
            .and_then(|note| note.agent.as_mut())
        {
            agent.composer_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::{AgentProvider, NoteTarget};

    fn target() -> NoteTarget {
        NoteTarget::File {
            file_path: "test.rs".to_string(),
        }
    }

    #[test]
    fn personal_notes_open_in_edit_mode() {
        let mut state = NoteState::default();
        state
            .items
            .push(Note::new(1, target(), "remember this".to_string()));

        state.start_input(state.items.first().cloned());

        assert_eq!(state.draft.as_deref(), Some("remember this"));
        assert_eq!(
            state.composer_mode,
            Some(NoteComposerMode::EditPersonal { note_id: 1 })
        );
    }

    #[test]
    fn agent_notes_open_an_empty_reply_composer() {
        let mut state = NoteState::default();
        state.items.push(Note::new_agent(
            1,
            target(),
            AgentProvider::Codex,
            "explain this".to_string(),
            1,
        ));
        state.items[0].agent.as_mut().unwrap().status = crate::note::AgentStatus::Complete;

        state.start_input(state.items.first().cloned());

        assert_eq!(state.draft.as_deref(), Some(""));
        assert_eq!(
            state.composer_mode,
            Some(NoteComposerMode::Reply { note_id: 1 })
        );
        assert!(state.items[0].agent.as_ref().unwrap().composer_open);

        state.cancel_input();
        assert!(!state.items[0].agent.as_ref().unwrap().composer_open);
    }
}
