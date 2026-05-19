use std::collections::HashSet;

use crate::diff::{DiffFile, DiffSession, FileChangeKind};
use crate::notes::{Note, NoteTarget};
use crate::render_cache::{RenderSession, RowContext, RowKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffMode {
    SideBySide,
    Inline,
}

impl DiffMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::SideBySide => Self::Inline,
            Self::Inline => Self::SideBySide,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusPane {
    Files,
    Main,
}

impl FocusPane {
    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::Main,
            Self::Main => Self::Files,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Files => Self::Main,
            Self::Main => Self::Files,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SidebarEntryKind {
    Directory { path: String, collapsed: bool },
    File { file_index: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidebarEntry {
    pub depth: usize,
    pub label: String,
    pub kind: SidebarEntryKind,
}

#[derive(Debug)]
pub struct App {
    pub running: bool,
    pub session: DiffSession,
    pub render_session: Option<RenderSession>,
    pub notes: Vec<Note>,
    pub expanded_note_ids: Vec<u64>,
    pub mode: DiffMode,
    pub focus: FocusPane,
    pub sidebar_open: bool,
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub sidebar_cursor: usize,
    pub collapsed_dirs: Vec<String>,
    pub cursor_row: usize,
    pub selection_anchor: Option<usize>,
    pub note_draft: Option<String>,
    pub editing_note_id: Option<u64>,
    pub scroll: u16,
}

impl App {
    pub fn new(session: DiffSession) -> Self {
        Self {
            running: true,
            session,
            render_session: None,
            notes: Vec::new(),
            expanded_note_ids: Vec::new(),
            mode: DiffMode::SideBySide,
            focus: FocusPane::Main,
            sidebar_open: true,
            selected_file: 0,
            selected_hunk: 0,
            sidebar_cursor: 0,
            collapsed_dirs: Vec::new(),
            cursor_row: 0,
            selection_anchor: None,
            note_draft: None,
            editing_note_id: None,
            scroll: 0,
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.toggle();
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_open = !self.sidebar_open;
        if !self.sidebar_open && self.focus == FocusPane::Files {
            self.focus = FocusPane::Main;
        }
    }

    pub fn focus_next(&mut self) {
        if self.sidebar_open {
            self.focus = self.focus.next();
        } else {
            self.focus = FocusPane::Main;
        }
    }

    pub fn focus_previous(&mut self) {
        if self.sidebar_open {
            self.focus = self.focus.previous();
        } else {
            self.focus = FocusPane::Main;
        }
    }

    pub fn file_cursor_down(&mut self) {
        let max_index = self.visible_sidebar_entries().len().saturating_sub(1);
        self.sidebar_cursor = (self.sidebar_cursor + 1).min(max_index);
    }

    pub fn file_cursor_up(&mut self) {
        self.sidebar_cursor = self.sidebar_cursor.saturating_sub(1);
    }

    pub fn jump_to_file_cursor(&mut self) {
        self.activate_sidebar_cursor();
    }

    pub fn collapse_sidebar_directory(&mut self) {
        let Some(SidebarEntry {
            kind: SidebarEntryKind::Directory { path, collapsed },
            ..
        }) = self.current_sidebar_entry()
        else {
            return;
        };

        if !collapsed {
            self.collapsed_dirs.push(path);
            self.clamp_sidebar_cursor();
        }
    }

    pub fn expand_sidebar_directory(&mut self) {
        let Some(SidebarEntry {
            kind: SidebarEntryKind::Directory { path, collapsed },
            ..
        }) = self.current_sidebar_entry()
        else {
            return;
        };

        if collapsed {
            self.collapsed_dirs.retain(|candidate| candidate != &path);
        }
    }

    pub fn activate_sidebar_cursor(&mut self) {
        let Some(entry) = self.current_sidebar_entry() else {
            return;
        };

        match entry.kind {
            SidebarEntryKind::File { file_index } => {
                self.selected_file = file_index.min(self.session.files.len().saturating_sub(1));
                self.selected_hunk = 0;
            }
            SidebarEntryKind::Directory { path, collapsed } => {
                if collapsed {
                    self.collapsed_dirs.retain(|candidate| candidate != &path);
                } else {
                    self.collapsed_dirs.push(path);
                }
                self.clamp_sidebar_cursor();
            }
        }
    }

    pub fn next_hunk(&mut self) {
        let Some(current_file) = self.current_file() else {
            return;
        };
        if self.selected_hunk + 1 < current_file.hunks.len() {
            self.selected_hunk += 1;
        } else if self.selected_file + 1 < self.session.files.len() {
            self.selected_file += 1;
            self.selected_hunk = 0;
        }
    }

    pub fn previous_hunk(&mut self) {
        if self.selected_hunk > 0 {
            self.selected_hunk -= 1;
        } else if self.selected_file > 0 {
            self.selected_file -= 1;
            self.selected_hunk = self
                .current_file()
                .map(|file| file.hunks.len().saturating_sub(1))
                .unwrap_or(0);
        }
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.session.files.get(self.selected_file)
    }

    pub fn move_cursor_down(&mut self, amount: usize, max_row: usize) {
        self.cursor_row = (self.cursor_row + amount).min(max_row);
    }

    pub fn move_cursor_up(&mut self, amount: usize) {
        self.cursor_row = self.cursor_row.saturating_sub(amount);
    }

    pub fn clamp_cursor_row(&mut self, max_row: usize) {
        self.cursor_row = self.cursor_row.min(max_row);
        if let Some(anchor) = self.selection_anchor {
            self.selection_anchor = Some(anchor.min(max_row));
        }
    }

    pub fn toggle_selection_anchor(&mut self) {
        if self.selection_anchor == Some(self.cursor_row) {
            self.selection_anchor = None;
        } else {
            self.selection_anchor = Some(self.cursor_row);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    pub fn selected_row_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor.map(|anchor| {
            if anchor <= self.cursor_row {
                (anchor, self.cursor_row)
            } else {
                (self.cursor_row, anchor)
            }
        })
    }

    pub fn sync_selection_to_cursor(&mut self) {
        let Some(cache) = &self.render_session else {
            return;
        };

        let Some(RowContext {
            file_index: Some(file_index),
            hunk_index,
            ..
        }) = cache.row_contexts.get(self.cursor_row).copied()
        else {
            return;
        };

        self.selected_file = file_index;
        if let Some(hunk_index) = hunk_index {
            self.selected_hunk = hunk_index;
        } else if let Some(range) = cache
            .hunk_ranges
            .iter()
            .find(|range| range.file_index == file_index)
        {
            self.selected_hunk = range.hunk_index;
        } else {
            self.selected_hunk = 0;
        }
    }

    pub fn add_note(&mut self, target: NoteTarget, body: String) {
        let id = self.notes.len() as u64 + 1;
        self.notes.push(Note::new(id, target, body));
        self.render_session = None;
    }

    pub fn toggle_current_note_expanded(&mut self) {
        let Some(cache) = &self.render_session else {
            return;
        };
        let Some(note_id) = cache
            .row_contexts
            .get(self.cursor_row)
            .and_then(|context| context.note_id)
        else {
            return;
        };

        if self.expanded_note_ids.contains(&note_id) {
            self.expanded_note_ids
                .retain(|candidate| candidate != &note_id);
        } else {
            self.expanded_note_ids.push(note_id);
        }

        self.render_session = None;
    }

    pub fn note_input_active(&self) -> bool {
        self.note_draft.is_some()
    }

    pub fn start_note_input(&mut self) {
        if self.note_draft.is_some() {
            return;
        }

        if let Some(note_id) = self.current_note_id()
            && let Some(note) = self.notes.iter().find(|note| note.id == note_id)
        {
            self.note_draft = Some(note.body.clone());
            self.editing_note_id = Some(note_id);
            return;
        }

        self.note_draft = Some(String::new());
        self.editing_note_id = None;
    }

    pub fn cancel_note_input(&mut self) {
        self.note_draft = None;
        self.editing_note_id = None;
    }

    pub fn insert_note_text(&mut self, text: &str) {
        if let Some(draft) = &mut self.note_draft {
            draft.push_str(text);
        }
    }

    pub fn backspace_note_text(&mut self) {
        if let Some(draft) = &mut self.note_draft {
            draft.pop();
        }
    }

    pub fn submit_note_input(&mut self) {
        let Some(body) = self.note_draft.take() else {
            return;
        };
        let editing_note_id = self.editing_note_id.take();

        let body = body.trim().to_string();
        if body.is_empty() {
            return;
        }

        if let Some(note_id) = editing_note_id {
            if let Some(note) = self.notes.iter_mut().find(|note| note.id == note_id) {
                note.body = body;
                self.render_session = None;
            }
            return;
        }

        if let Some(target) = self.current_note_target() {
            self.add_note(target, body);
            self.clear_selection();
        }
    }

    pub fn note_anchor_row(&self) -> usize {
        self.selected_row_range()
            .map(|(start, _)| start)
            .unwrap_or(self.cursor_row)
    }

    pub fn current_note_target(&self) -> Option<NoteTarget> {
        let Some(cache) = &self.render_session else {
            return None;
        };

        if let Some((start, end)) = self.selected_row_range()
            && start != end
        {
            return self.range_note_target(cache, start, end);
        }

        self.row_note_target(cache, self.cursor_row)
    }

    pub fn current_note_id(&self) -> Option<u64> {
        self.render_session
            .as_ref()?
            .row_contexts
            .get(self.cursor_row)?
            .note_id
    }

    pub fn composer_note_target(&self) -> Option<NoteTarget> {
        if let Some(note_id) = self.editing_note_id {
            return self
                .notes
                .iter()
                .find(|note| note.id == note_id)
                .map(|note| note.target.clone());
        }

        self.current_note_target()
    }

    pub fn sync_sidebar_cursor_to_selected_file(&mut self) {
        if let Some(index) = self
            .visible_sidebar_entries()
            .iter()
            .position(|entry| matches!(entry.kind, SidebarEntryKind::File { file_index } if file_index == self.selected_file))
        {
            self.sidebar_cursor = index;
        }
    }

    pub fn visible_sidebar_entries(&self) -> Vec<SidebarEntry> {
        let mut entries = Vec::new();
        let mut inserted_dirs = HashSet::new();

        for (file_index, file) in self.session.files.iter().enumerate() {
            let parts: Vec<&str> = file.path.split('/').collect();
            let mut hidden_by_collapsed_parent = false;

            for depth in 0..parts.len().saturating_sub(1) {
                let path = parts[..=depth].join("/");
                if inserted_dirs.insert(path.clone()) {
                    let collapsed = self
                        .collapsed_dirs
                        .iter()
                        .any(|candidate| candidate == &path);
                    let chevron = if collapsed { "▸" } else { "▾" };
                    entries.push(SidebarEntry {
                        depth,
                        label: format!("{}{} {}/", "  ".repeat(depth), chevron, parts[depth]),
                        kind: SidebarEntryKind::Directory {
                            path: path.clone(),
                            collapsed,
                        },
                    });
                }

                if self
                    .collapsed_dirs
                    .iter()
                    .any(|candidate| candidate == &path)
                {
                    hidden_by_collapsed_parent = true;
                    break;
                }
            }

            if hidden_by_collapsed_parent {
                continue;
            }

            let file_name = parts.last().copied().unwrap_or(file.path.as_str());
            let change_label = match file.change_kind() {
                FileChangeKind::Added => "A",
                FileChangeKind::Modified => "M",
            };

            entries.push(SidebarEntry {
                depth: parts.len().saturating_sub(1),
                label: format!(
                    "{}{}  {}",
                    "  ".repeat(parts.len().saturating_sub(1) + 1),
                    file_name,
                    change_label
                ),
                kind: SidebarEntryKind::File { file_index },
            });
        }

        entries
    }

    fn current_sidebar_entry(&self) -> Option<SidebarEntry> {
        self.visible_sidebar_entries()
            .get(self.sidebar_cursor)
            .cloned()
    }

    fn clamp_sidebar_cursor(&mut self) {
        let max_index = self.visible_sidebar_entries().len().saturating_sub(1);
        self.sidebar_cursor = self.sidebar_cursor.min(max_index);
    }

    fn row_note_target(&self, cache: &RenderSession, row: usize) -> Option<NoteTarget> {
        let context = cache.row_contexts.get(row)?;
        let file_index = context.file_index?;
        let file = self.session.files.get(file_index)?;
        let file_path = file.path.clone();

        match context.kind {
            RowKind::FileHeader | RowKind::Separator | RowKind::Spacer => {
                Some(NoteTarget::File { file_path })
            }
            RowKind::Note => Some(NoteTarget::File { file_path }),
            RowKind::HunkHeader => {
                let hunk_index = context.hunk_index?;
                let hunk = file.hunks.get(hunk_index)?;
                Some(NoteTarget::Hunk {
                    file_path,
                    hunk_header: hunk.header.clone(),
                })
            }
            RowKind::DiffLine => Some(NoteTarget::Line {
                file_path,
                old_lineno: context.old_lineno,
                new_lineno: context.new_lineno,
            }),
        }
    }

    fn range_note_target(
        &self,
        cache: &RenderSession,
        start: usize,
        end: usize,
    ) -> Option<NoteTarget> {
        let start_context = cache.row_contexts.get(start)?;
        let end_context = cache.row_contexts.get(end)?;
        let file_index = start_context.file_index?;
        if end_context.file_index != Some(file_index) {
            return self.row_note_target(cache, self.cursor_row);
        }

        let file_path = self.session.files.get(file_index)?.path.clone();
        Some(NoteTarget::Range {
            file_path,
            start_old_lineno: start_context.old_lineno,
            start_new_lineno: start_context.new_lineno,
            end_old_lineno: end_context.old_lineno,
            end_new_lineno: end_context.new_lineno,
        })
    }
}
