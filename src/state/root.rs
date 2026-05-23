use crate::cache::{DiffCache, RowContext};
use crate::diff::{DiffFile, DiffSession};
use crate::note::{Note, NoteTarget};
use crate::state::{
    DiffMode, DiffViewState, FocusPane, GlobalState, NoteInputResult, NoteState, SidebarEntry,
    SidebarState, note_target_for_range, note_target_for_row,
};

#[derive(Debug)]
pub struct App {
    pub session: DiffSession,
    pub cache: Option<DiffCache>,
    pub global: GlobalState,
    pub diff_view: DiffViewState,
    pub sidebar: SidebarState,
    pub notes: NoteState,
}

impl App {
    pub fn new(session: DiffSession) -> Self {
        Self {
            session,
            cache: None,
            global: GlobalState {
                running: true,
                mode: DiffMode::SideBySide,
                focus: FocusPane::Main,
            },
            diff_view: DiffViewState {
                selected_file: 0,
                selected_hunk: 0,
                cursor_row: 0,
                selection_anchor: None,
                scroll: 0,
            },
            sidebar: SidebarState {
                open: true,
                cursor: 0,
                collapsed_dirs: Vec::new(),
            },
            notes: NoteState {
                items: Vec::new(),
                expanded_ids: Vec::new(),
                draft: None,
                editing_id: None,
            },
        }
    }

    pub fn quit(&mut self) {
        self.global.running = false;
    }

    pub fn toggle_mode(&mut self) {
        self.global.mode = self.global.mode.toggle();
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar.open = !self.sidebar.open;
        if !self.sidebar.open && self.global.focus == FocusPane::Files {
            self.global.focus = FocusPane::Main;
        }
    }

    pub fn focus_next(&mut self) {
        if self.sidebar.open {
            self.global.focus = self.global.focus.next();
        } else {
            self.global.focus = FocusPane::Main;
        }
    }

    pub fn focus_previous(&mut self) {
        if self.sidebar.open {
            self.global.focus = self.global.focus.previous();
        } else {
            self.global.focus = FocusPane::Main;
        }
    }

    pub fn file_cursor_down(&mut self) {
        self.sidebar.cursor_down(&self.session.files);
    }

    pub fn file_cursor_up(&mut self) {
        self.sidebar.cursor_up();
    }

    pub fn jump_to_file_cursor(&mut self) {
        self.activate_sidebar_cursor();
    }

    pub fn collapse_sidebar_directory(&mut self) {
        self.sidebar.collapse_directory(&self.session.files);
    }

    pub fn expand_sidebar_directory(&mut self) {
        self.sidebar.expand_directory(&self.session.files);
    }

    pub fn activate_sidebar_cursor(&mut self) {
        if let Some(file_index) = self.sidebar.activate_cursor(&self.session.files) {
            self.diff_view.selected_file = file_index;
            self.diff_view.selected_hunk = 0;
        }
    }

    pub fn next_hunk(&mut self) {
        let Some(current_file) = self.current_file() else {
            return;
        };
        if self.diff_view.selected_hunk + 1 < current_file.hunks.len() {
            self.diff_view.selected_hunk += 1;
        } else if self.diff_view.selected_file + 1 < self.session.files.len() {
            self.diff_view.selected_file += 1;
            self.diff_view.selected_hunk = 0;
        }
    }

    pub fn previous_hunk(&mut self) {
        if self.diff_view.selected_hunk > 0 {
            self.diff_view.selected_hunk -= 1;
        } else if self.diff_view.selected_file > 0 {
            self.diff_view.selected_file -= 1;
            self.diff_view.selected_hunk = self
                .current_file()
                .map(|file| file.hunks.len().saturating_sub(1))
                .unwrap_or(0);
        }
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.session.files.get(self.diff_view.selected_file)
    }

    pub fn move_cursor_down(&mut self, amount: usize, max_row: usize) {
        self.diff_view.cursor_row = (self.diff_view.cursor_row + amount).min(max_row);
    }

    pub fn move_cursor_up(&mut self, amount: usize) {
        self.diff_view.cursor_row = self.diff_view.cursor_row.saturating_sub(amount);
    }

    pub fn clamp_cursor_row(&mut self, max_row: usize) {
        self.diff_view.cursor_row = self.diff_view.cursor_row.min(max_row);
        if let Some(anchor) = self.diff_view.selection_anchor {
            self.diff_view.selection_anchor = Some(anchor.min(max_row));
        }
    }

    pub fn toggle_selection_anchor(&mut self) {
        if self.diff_view.selection_anchor == Some(self.diff_view.cursor_row) {
            self.diff_view.selection_anchor = None;
        } else {
            self.diff_view.selection_anchor = Some(self.diff_view.cursor_row);
        }
    }

    pub fn clear_selection(&mut self) {
        self.diff_view.selection_anchor = None;
    }

    pub fn selected_row_range(&self) -> Option<(usize, usize)> {
        self.diff_view.selection_anchor.map(|anchor| {
            if anchor <= self.diff_view.cursor_row {
                (anchor, self.diff_view.cursor_row)
            } else {
                (self.diff_view.cursor_row, anchor)
            }
        })
    }

    pub fn sync_selection_to_cursor(&mut self) {
        let Some(cache) = &self.cache else {
            return;
        };

        let Some(RowContext {
            file_index: Some(file_index),
            hunk_index,
            ..
        }) = cache.row_contexts.get(self.diff_view.cursor_row).copied()
        else {
            return;
        };

        self.diff_view.selected_file = file_index;
        if let Some(hunk_index) = hunk_index {
            self.diff_view.selected_hunk = hunk_index;
        } else if let Some(range) = cache
            .hunk_ranges
            .iter()
            .find(|range| range.file_index == file_index)
        {
            self.diff_view.selected_hunk = range.hunk_index;
        } else {
            self.diff_view.selected_hunk = 0;
        }
    }

    pub fn add_note(&mut self, target: NoteTarget, body: String) {
        let id = self.notes.items.len() as u64 + 1;
        self.notes.items.push(Note::new(id, target, body));
        self.cache = None;
    }

    pub fn toggle_current_note_expanded(&mut self) {
        let Some(cache) = &self.cache else {
            return;
        };
        let Some(note_id) = cache
            .row_contexts
            .get(self.diff_view.cursor_row)
            .and_then(|context| context.note_id)
        else {
            return;
        };

        self.notes.toggle_expanded(note_id);
        self.cache = None;
    }

    pub fn note_input_active(&self) -> bool {
        self.notes.input_active()
    }

    pub fn start_note_input(&mut self) {
        let current_note = self.current_note_id().and_then(|note_id| {
            self.notes
                .items
                .iter()
                .find(|note| note.id == note_id)
                .cloned()
        });
        self.notes.start_input(current_note);
    }

    pub fn cancel_note_input(&mut self) {
        self.notes.cancel_input();
    }

    pub fn insert_note_text(&mut self, text: &str) {
        self.notes.insert_text(text);
    }

    pub fn backspace_note_text(&mut self) {
        self.notes.backspace_text();
    }

    pub fn submit_note_input(&mut self) {
        let Some(result) = self.notes.finish_input() else {
            return;
        };

        match result {
            NoteInputResult::Edit { note_id, body } => {
                if let Some(note) = self.notes.items.iter_mut().find(|note| note.id == note_id) {
                    note.body = body;
                    self.cache = None;
                }
            }
            NoteInputResult::Create { body } => {
                if let Some(target) = self.current_note_target() {
                    self.add_note(target, body);
                    self.clear_selection();
                }
            }
        }
    }

    pub fn note_anchor_row(&self) -> usize {
        self.selected_row_range()
            .map(|(start, _)| start)
            .unwrap_or(self.diff_view.cursor_row)
    }

    pub fn current_note_target(&self) -> Option<NoteTarget> {
        let Some(cache) = &self.cache else {
            return None;
        };

        if let Some((start, end)) = self.selected_row_range()
            && start != end
        {
            return note_target_for_range(
                &self.session.files,
                &cache.row_contexts,
                start,
                end,
                self.diff_view.cursor_row,
            );
        }

        note_target_for_row(
            &self.session.files,
            &cache.row_contexts,
            self.diff_view.cursor_row,
        )
    }

    pub fn current_note_id(&self) -> Option<u64> {
        self.cache
            .as_ref()?
            .row_contexts
            .get(self.diff_view.cursor_row)?
            .note_id
    }

    pub fn composer_note_target(&self) -> Option<NoteTarget> {
        if let Some(note_id) = self.notes.editing_id {
            return self
                .notes
                .items
                .iter()
                .find(|note| note.id == note_id)
                .map(|note| note.target.clone());
        }

        self.current_note_target()
    }

    pub fn sync_sidebar_cursor_to_selected_file(&mut self) {
        self.sidebar
            .sync_cursor_to_file(&self.session.files, self.diff_view.selected_file);
    }

    pub fn visible_sidebar_entries(&self) -> Vec<SidebarEntry> {
        self.sidebar.visible_entries(&self.session.files)
    }
}
