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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteComposer {
    pub mode: NoteComposerMode,
    pub draft: String,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct NoteState {
    pub items: Vec<Note>,
    pub expanded_ids: Vec<NoteId>,
    pub composer: Option<NoteComposer>,
    next_note_id: NoteId,
    next_run_id: RunId,
}

impl Default for NoteState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            expanded_ids: Vec::new(),
            composer: None,
            next_note_id: 1,
            next_run_id: 1,
        }
    }
}

impl NoteState {
    pub fn input_active(&self) -> bool {
        self.composer.is_some()
    }

    pub fn start_input(&mut self, current_note: Option<Note>) {
        if self.composer.is_some() {
            return;
        }

        let (mode, draft) = match current_note {
            Some(note) if note.is_agent_thread() => {
                (NoteComposerMode::Reply { note_id: note.id }, String::new())
            }
            Some(note) => (
                NoteComposerMode::EditPersonal { note_id: note.id },
                note.personal_body().unwrap_or_default().to_string(),
            ),
            None => (NoteComposerMode::Create, String::new()),
        };
        self.composer = Some(NoteComposer {
            mode,
            draft,
            error: None,
        });
    }

    pub fn cancel_input(&mut self) {
        self.composer = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        if let Some(composer) = &mut self.composer {
            composer.draft.push_str(text);
            composer.error = None;
        }
    }

    pub fn backspace_text(&mut self) {
        if let Some(composer) = &mut self.composer {
            composer.draft.pop();
            composer.error = None;
        }
    }

    pub fn finish_input(&mut self) -> Option<NoteInputResult> {
        let composer = self.composer.take()?;
        let body = composer.draft.trim().to_string();
        if body.is_empty() {
            return None;
        }

        match composer.mode {
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
        self.composer = Some(NoteComposer {
            mode: NoteComposerMode::Create,
            draft: body,
            error: Some(error),
        });
    }

    pub fn allocate_run_id(&mut self) -> RunId {
        let id = self.next_run_id;
        self.next_run_id += 1;
        id
    }

    pub fn composer_mode(&self) -> Option<NoteComposerMode> {
        self.composer.as_ref().map(|composer| composer.mode)
    }

    pub fn composer_note_id(&self) -> Option<NoteId> {
        match self.composer_mode()? {
            NoteComposerMode::EditPersonal { note_id } | NoteComposerMode::Reply { note_id } => {
                Some(note_id)
            }
            NoteComposerMode::Create => None,
        }
    }

    pub fn reply_composer_note_id(&self) -> Option<NoteId> {
        let NoteComposerMode::Reply { note_id } = self.composer_mode()? else {
            return None;
        };
        Some(note_id)
    }

    pub fn toggle_expanded(&mut self, note_id: NoteId) {
        if self.expanded_ids.contains(&note_id) {
            self.expanded_ids.retain(|candidate| candidate != &note_id);
        } else {
            self.expanded_ids.push(note_id);
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

        assert_eq!(
            state.composer,
            Some(NoteComposer {
                mode: NoteComposerMode::EditPersonal { note_id: 1 },
                draft: "remember this".to_string(),
                error: None,
            })
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

        state.start_input(state.items.first().cloned());

        assert_eq!(
            state.composer,
            Some(NoteComposer {
                mode: NoteComposerMode::Reply { note_id: 1 },
                draft: String::new(),
                error: None,
            })
        );
        state.cancel_input();
        assert!(state.composer.is_none());
    }
}
