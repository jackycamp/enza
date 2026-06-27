use crate::diff::{DiffFile, DiffSession};
use crate::layout::{Layout, LayoutWorker, RowContext};
use crate::note::{Note, NoteTarget};
use crate::state::{
    DiffMode, FocusPane, GlobalState, MainPaneState, NavDirection, NoteInputResult, NoteState,
    SidebarEntry, SidebarState, note_target_for_range, note_target_for_row,
};

#[derive(Debug)]
pub struct App {
    pub session: DiffSession,
    pub layout: Option<Layout>,
    pub layout_worker: LayoutWorker,
    pub global: GlobalState,
    pub main_pane: MainPaneState,
    pub sidebar: SidebarState,
    pub notes: NoteState,
}

impl App {
    pub fn new(session: DiffSession) -> Self {
        Self {
            session,
            layout: None,
            layout_worker: LayoutWorker::new(),
            global: GlobalState {
                running: true,
                mode: DiffMode::SideBySide,
                focus: FocusPane::Main,
                debug_pane_open: false,
                nav_direction: None,
            },
            main_pane: MainPaneState {
                selected_file: 0,
                selected_hunk: 0,
                cursor_row: 0,
                cursor_target: None,
                selection_anchor: None,
                scroll: 0,
            },
            sidebar: SidebarState {
                open: true,
                cursor: 0,
                scroll: 0,
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

    pub fn toggle_debug_pane(&mut self) {
        self.global.debug_pane_open = !self.global.debug_pane_open;
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
            self.main_pane.selected_file = file_index;
            self.main_pane.selected_hunk = 0;
            self.main_pane.cursor_target = Some(RowContext {
                file_index: Some(file_index),
                hunk_index: self
                    .session
                    .files
                    .get(file_index)
                    .and_then(|file| (!file.hunks.is_empty()).then_some(0)),
                kind: if self
                    .session
                    .files
                    .get(file_index)
                    .is_some_and(|file| !file.hunks.is_empty())
                {
                    crate::layout::RowKind::HunkHeader
                } else {
                    crate::layout::RowKind::FileHeader
                },
                old_lineno: None,
                new_lineno: None,
                note_id: None,
            });
        }
    }

    pub fn next_hunk(&mut self) {
        self.global.nav_direction = Some(NavDirection::Down);
        let Some(current_file) = self.current_file() else {
            return;
        };
        if self.main_pane.selected_hunk + 1 < current_file.hunks.len() {
            self.main_pane.selected_hunk += 1;
        } else if self.main_pane.selected_file + 1 < self.session.files.len() {
            self.main_pane.selected_file += 1;
            self.main_pane.selected_hunk = 0;
        }
        self.main_pane.cursor_target = Some(RowContext {
            file_index: Some(self.main_pane.selected_file),
            hunk_index: Some(self.main_pane.selected_hunk),
            kind: crate::layout::RowKind::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        });
    }

    pub fn previous_hunk(&mut self) {
        self.global.nav_direction = Some(NavDirection::Up);
        if self.main_pane.selected_hunk > 0 {
            self.main_pane.selected_hunk -= 1;
        } else if self.main_pane.selected_file > 0 {
            self.main_pane.selected_file -= 1;
            self.main_pane.selected_hunk = self
                .current_file()
                .map(|file| file.hunks.len().saturating_sub(1))
                .unwrap_or(0);
        }
        self.main_pane.cursor_target = Some(RowContext {
            file_index: Some(self.main_pane.selected_file),
            hunk_index: Some(self.main_pane.selected_hunk),
            kind: crate::layout::RowKind::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        });
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.session.files.get(self.main_pane.selected_file)
    }

    pub fn move_cursor_down(&mut self, amount: usize, max_row: usize) {
        self.global.nav_direction = Some(NavDirection::Down);
        for _ in 0..amount {
            if self.try_advance_to_next_hunk() {
                continue;
            }
            self.main_pane.cursor_row = (self.main_pane.cursor_row + 1).min(max_row);
            self.capture_cursor_target();
            self.sync_selection_to_cursor();
        }
    }

    pub fn move_cursor_up(&mut self, amount: usize) {
        self.global.nav_direction = Some(NavDirection::Up);
        for _ in 0..amount {
            if self.try_retreat_to_previous_hunk() {
                continue;
            }
            self.main_pane.cursor_row = self.main_pane.cursor_row.saturating_sub(1);
            self.capture_cursor_target();
            self.sync_selection_to_cursor();
        }
    }

    pub fn clamp_cursor_row(&mut self, max_row: usize) {
        self.main_pane.cursor_row = self.main_pane.cursor_row.min(max_row);
        if let Some(anchor) = self.main_pane.selection_anchor {
            self.main_pane.selection_anchor = Some(anchor.min(max_row));
        }
        self.capture_cursor_target();
    }

    pub fn toggle_selection_anchor(&mut self) {
        if self.main_pane.selection_anchor == Some(self.main_pane.cursor_row) {
            self.main_pane.selection_anchor = None;
        } else {
            self.main_pane.selection_anchor = Some(self.main_pane.cursor_row);
        }
    }

    pub fn clear_selection(&mut self) {
        self.main_pane.selection_anchor = None;
    }

    pub fn selected_row_range(&self) -> Option<(usize, usize)> {
        self.main_pane.selection_anchor.map(|anchor| {
            if anchor <= self.main_pane.cursor_row {
                (anchor, self.main_pane.cursor_row)
            } else {
                (self.main_pane.cursor_row, anchor)
            }
        })
    }

    pub fn sync_selection_to_cursor(&mut self) {
        let Some(layout) = &self.layout else {
            return;
        };

        let Some(
            context @ RowContext {
                file_index: Some(file_index),
                hunk_index,
                ..
            },
        ) = layout.row_contexts.get(self.main_pane.cursor_row).copied()
        else {
            return;
        };

        self.main_pane.cursor_target = Some(context);
        self.main_pane.selected_file = file_index;
        if let Some(hunk_index) = hunk_index {
            self.main_pane.selected_hunk = hunk_index;
        } else if let Some(range) = layout
            .hunk_ranges
            .iter()
            .find(|range| range.file_index == file_index)
        {
            self.main_pane.selected_hunk = range.hunk_index;
        } else {
            self.main_pane.selected_hunk = 0;
        }
    }

    pub fn add_note(&mut self, target: NoteTarget, body: String) {
        let id = self.notes.items.len() as u64 + 1;
        self.notes.items.push(Note::new(id, target, body));
        self.refresh_note_overlay();
    }

    pub fn toggle_current_note_expanded(&mut self) {
        let Some(layout) = &self.layout else {
            return;
        };
        let Some(note_id) = layout
            .row_contexts
            .get(self.main_pane.cursor_row)
            .and_then(|context| context.note_id)
        else {
            return;
        };

        self.notes.toggle_expanded(note_id);
        self.refresh_note_overlay();
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
                    self.refresh_note_overlay();
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
            .unwrap_or(self.main_pane.cursor_row)
    }

    pub fn current_note_target(&self) -> Option<NoteTarget> {
        let Some(layout) = &self.layout else {
            return None;
        };

        if let Some((start, end)) = self.selected_row_range()
            && start != end
        {
            return note_target_for_range(
                &self.session.files,
                &layout.row_contexts,
                start,
                end,
                self.main_pane.cursor_row,
            );
        }

        note_target_for_row(
            &self.session.files,
            &layout.row_contexts,
            self.main_pane.cursor_row,
        )
    }

    pub fn current_note_id(&self) -> Option<u64> {
        self.layout
            .as_ref()?
            .row_contexts
            .get(self.main_pane.cursor_row)?
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
            .sync_cursor_to_file(&self.session.files, self.main_pane.selected_file);
    }

    pub fn visible_sidebar_entries(&self) -> Vec<SidebarEntry> {
        self.sidebar.visible_entries(&self.session.files)
    }

    fn refresh_note_overlay(&mut self) {
        if let Some(layout) = &mut self.layout {
            layout.refresh_notes(&self.session, &self.notes.items, &self.notes.expanded_ids);
        }
    }

    fn capture_cursor_target(&mut self) {
        let Some(layout) = &self.layout else {
            return;
        };
        self.main_pane.cursor_target = layout.row_contexts.get(self.main_pane.cursor_row).copied();
    }

    fn try_advance_to_next_hunk(&mut self) -> bool {
        let Some(layout) = &self.layout else {
            return false;
        };
        let Some(current) = layout.row_contexts.get(self.main_pane.cursor_row).copied() else {
            return false;
        };
        let Some(next) = layout
            .row_contexts
            .get(self.main_pane.cursor_row.saturating_add(1))
            .copied()
        else {
            return false;
        };

        if current.hunk_index == next.hunk_index || next.kind != crate::layout::RowKind::Spacer {
            return false;
        }

        let Some((file_index, hunk_index)) = next_hunk_indices(
            &self.session,
            self.main_pane.selected_file,
            self.main_pane.selected_hunk,
        ) else {
            return false;
        };

        self.main_pane.selected_file = file_index;
        self.main_pane.selected_hunk = hunk_index;
        let target = RowContext {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: crate::layout::RowKind::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        };
        self.main_pane.cursor_target = Some(target);
        if let Some(index) = row_index_for_context(layout, target) {
            self.main_pane.cursor_row = index;
        }
        true
    }

    fn try_retreat_to_previous_hunk(&mut self) -> bool {
        let Some(layout) = &self.layout else {
            return false;
        };
        let Some(current) = layout.row_contexts.get(self.main_pane.cursor_row).copied() else {
            return false;
        };
        if self.main_pane.cursor_row == 0 {
            return false;
        }
        let Some(previous) = layout
            .row_contexts
            .get(self.main_pane.cursor_row - 1)
            .copied()
        else {
            return false;
        };

        if current.kind != crate::layout::RowKind::HunkHeader
            || previous.kind != crate::layout::RowKind::Spacer
        {
            return false;
        }

        let Some((file_index, hunk_index)) = previous_hunk_indices(
            &self.session,
            self.main_pane.selected_file,
            self.main_pane.selected_hunk,
        ) else {
            return false;
        };
        let Some(target) = last_meaningful_row_context(&self.session, file_index, hunk_index)
        else {
            return false;
        };

        self.main_pane.selected_file = file_index;
        self.main_pane.selected_hunk = hunk_index;
        self.main_pane.cursor_target = Some(target);
        if let Some(index) = row_index_for_context(layout, target) {
            self.main_pane.cursor_row = index;
        }
        true
    }
}

fn next_hunk_indices(
    session: &DiffSession,
    selected_file: usize,
    selected_hunk: usize,
) -> Option<(usize, usize)> {
    for (file_index, file) in session.files.iter().enumerate().skip(selected_file) {
        let start_hunk = if file_index == selected_file {
            selected_hunk + 1
        } else {
            0
        };
        if start_hunk < file.hunks.len() {
            return Some((file_index, start_hunk));
        }
    }
    None
}

fn previous_hunk_indices(
    session: &DiffSession,
    selected_file: usize,
    selected_hunk: usize,
) -> Option<(usize, usize)> {
    let mut file_index = selected_file;
    loop {
        let file = session.files.get(file_index)?;
        let candidate = if file_index == selected_file {
            selected_hunk.checked_sub(1)
        } else {
            file.hunks.len().checked_sub(1)
        };
        if let Some(hunk_index) = candidate {
            return Some((file_index, hunk_index));
        }
        if file_index == 0 {
            return None;
        }
        file_index -= 1;
    }
}

fn last_meaningful_row_context(
    session: &DiffSession,
    file_index: usize,
    hunk_index: usize,
) -> Option<RowContext> {
    let hunk = session.files.get(file_index)?.hunks.get(hunk_index)?;
    let last_line = hunk.lines.last();
    Some(match last_line {
        Some(line) => RowContext {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: crate::layout::RowKind::DiffLine,
            old_lineno: line.old_lineno(),
            new_lineno: line.new_lineno(),
            note_id: None,
        },
        None => RowContext {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: crate::layout::RowKind::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
        },
    })
}

fn row_index_for_context(layout: &Layout, target: RowContext) -> Option<usize> {
    layout.row_contexts.iter().position(|context| {
        context.file_index == target.file_index
            && context.hunk_index == target.hunk_index
            && context.kind == target.kind
            && context.old_lineno == target.old_lineno
            && context.new_lineno == target.new_lineno
            && context.note_id == target.note_id
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffHunk, DiffLine};
    use crate::layout::RowKind;

    fn app_with_hunks(count: usize) -> App {
        App::new(DiffSession {
            files: vec![DiffFile {
                path: "test.rs".to_string(),
                old_path: "test.rs".to_string(),
                new_path: "test.rs".to_string(),
                hunks: (0..count)
                    .map(|index| DiffHunk {
                        header: format!("@@ hunk {index} @@"),
                        lines: vec![DiffLine::Context {
                            old_lineno: index + 1,
                            new_lineno: index + 1,
                            text: format!("line {index}"),
                        }],
                    })
                    .collect(),
            }],
        })
    }

    fn row_for(layout: &Layout, hunk_index: usize, kind: RowKind) -> usize {
        layout
            .row_contexts
            .iter()
            .position(|context| context.hunk_index == Some(hunk_index) && context.kind == kind)
            .unwrap()
    }

    #[test]
    fn navigation_across_an_unloaded_hunk_preserves_the_target() {
        let mut app = app_with_hunks(4);
        let mut layout = Layout::build(&app.session, &[], &[], 80, 80, 0, 0, 1, 0, None);

        layout.target_hunk = 1;
        let boundary = row_for(&layout, 1, RowKind::Spacer);
        app.layout = Some(layout);
        app.main_pane.selected_hunk = 1;
        app.main_pane.cursor_row = boundary;

        let max_row = app.layout.as_ref().unwrap().row_contexts.len() - 1;
        app.move_cursor_down(1, max_row);
        assert_eq!(app.main_pane.selected_hunk, 2);

        app.layout.as_mut().unwrap().ensure_hunk_window(
            &app.layout_worker,
            &app.session,
            &[],
            &[],
            app.main_pane.selected_file,
            app.main_pane.selected_hunk,
            1,
            0,
            app.global.nav_direction,
        );
        let new_max_row = app.layout.as_ref().unwrap().row_contexts.len() - 1;
        app.clamp_cursor_row(new_max_row);
        app.sync_selection_to_cursor();

        assert_eq!(app.main_pane.selected_hunk, 2);
        assert_eq!(app.main_pane.cursor_target.unwrap().hunk_index, Some(2));
    }

    #[test]
    fn materializing_rows_preserves_both_selection_endpoints() {
        let mut app = app_with_hunks(3);
        app.layout = Some(Layout::build(
            &app.session,
            &[],
            &[],
            80,
            80,
            0,
            1,
            1,
            0,
            None,
        ));

        let selected_context = RowContext {
            file_index: Some(0),
            hunk_index: Some(1),
            kind: RowKind::DiffLine,
            old_lineno: Some(2),
            new_lineno: Some(2),
            note_id: None,
        };
        let original_index =
            row_index_for_context(app.layout.as_ref().unwrap(), selected_context).unwrap();
        app.main_pane.cursor_row = original_index;
        app.main_pane.cursor_target = Some(selected_context);
        app.main_pane.selection_anchor = Some(original_index);

        app.layout
            .as_mut()
            .unwrap()
            .ensure_selected_hunk_ready_sync(&app.session, &[], &[], 0, 0);
        let remapped_cursor =
            row_index_for_context(app.layout.as_ref().unwrap(), selected_context).unwrap();
        app.main_pane.cursor_row = remapped_cursor;

        let anchor = app.main_pane.selection_anchor.unwrap();
        let anchor_context = app.layout.as_ref().unwrap().row_contexts[anchor];
        assert_eq!(anchor_context.file_index, selected_context.file_index);
        assert_eq!(anchor_context.hunk_index, selected_context.hunk_index);
        assert_eq!(anchor_context.kind, selected_context.kind);
        assert_eq!(anchor_context.old_lineno, selected_context.old_lineno);
        assert_eq!(anchor_context.new_lineno, selected_context.new_lineno);
    }
}
