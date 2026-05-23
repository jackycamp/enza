use crate::note::Note;

pub enum NoteInputResult {
    Create { body: String },
    Edit { note_id: u64, body: String },
}

#[derive(Debug)]
pub struct NoteState {
    pub items: Vec<Note>,
    pub expanded_ids: Vec<u64>,
    pub draft: Option<String>,
    pub editing_id: Option<u64>,
}

impl NoteState {
    pub fn input_active(&self) -> bool {
        self.draft.is_some()
    }

    pub fn start_input(&mut self, current_note: Option<Note>) {
        if self.draft.is_some() {
            return;
        }

        if let Some(note) = current_note {
            self.draft = Some(note.body.clone());
            self.editing_id = Some(note.id);
            return;
        }

        self.draft = Some(String::new());
        self.editing_id = None;
    }

    pub fn cancel_input(&mut self) {
        self.draft = None;
        self.editing_id = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        if let Some(draft) = &mut self.draft {
            draft.push_str(text);
        }
    }

    pub fn backspace_text(&mut self) {
        if let Some(draft) = &mut self.draft {
            draft.pop();
        }
    }

    pub fn finish_input(&mut self) -> Option<NoteInputResult> {
        let body = self.draft.take()?;
        let editing_note_id = self.editing_id.take();

        let body = body.trim().to_string();
        if body.is_empty() {
            return None;
        }

        match editing_note_id {
            Some(note_id) => Some(NoteInputResult::Edit { note_id, body }),
            None => Some(NoteInputResult::Create { body }),
        }
    }

    pub fn toggle_expanded(&mut self, note_id: u64) {
        if self.expanded_ids.contains(&note_id) {
            self.expanded_ids.retain(|candidate| candidate != &note_id);
        } else {
            self.expanded_ids.push(note_id);
        }
    }
}
