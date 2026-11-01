//! Unread agent replies and deliberate navigation to their latest response.

use crate::core::note::Note;
use crate::tui::layout::{Layout, LayoutDisplayContext};

use super::{App, FocusPane, SidebarEntry, SidebarEntryKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyDirection {
    Next,
    Previous,
}

impl App {
    pub fn sidebar_entry_has_unread(&self, entry: &SidebarEntry) -> bool {
        self.notes.items.iter().any(|note| {
            if !note
                .agent_thread()
                .is_some_and(|thread| thread.has_unread_reply())
            {
                return false;
            }
            let path = note.target.file_path();
            match &entry.kind {
                SidebarEntryKind::File { file_index } => self
                    .session
                    .files
                    .get(*file_index)
                    .is_some_and(|file| file.path == path),
                SidebarEntryKind::Directory {
                    path: directory, ..
                } => path
                    .strip_prefix(directory)
                    .is_some_and(|suffix| suffix.starts_with('/')),
            }
        })
    }

    /// Acknowledge only after deliberate input focuses the newest expanded reply.
    pub fn acknowledge_focused_reply(&mut self) {
        if self.global.focus != FocusPane::Main || self.note_input_active() {
            return;
        }
        let Some(note_id) = self.current_note_id() else {
            return;
        };
        let visited = self.notes.expanded_ids.contains(&note_id)
            && self
                .layout
                .as_ref()
                .and_then(|layout| layout.latest_reply_bounds(note_id))
                .is_some_and(|bounds| bounds.contains(&self.main_pane.cursor_row));
        if visited
            && let Some(thread) = self
                .notes
                .items
                .iter_mut()
                .find(|note| note.id == note_id)
                .and_then(Note::agent_thread_mut)
        {
            thread.mark_read();
        }
    }

    pub fn jump_to_unread_reply(&mut self, direction: ReplyDirection) -> bool {
        if self.note_input_active() {
            return false;
        }
        let Some(layout) = &self.layout else {
            return false;
        };
        let targets: Vec<_> = Layout::note_targets(&self.session, &self.notes.items)
            .into_iter()
            .enumerate()
            .map(|(order, (row, context))| (row, context, order))
            .collect();
        let current_context = layout.row_context(&self.session, self.main_pane.cursor_row);
        let current = current_context
            .and_then(|context| {
                targets
                    .iter()
                    .find(|(_, target, _)| target.note_id == context.note_id)
                    .filter(|_| context.note_id.is_some())
                    .map(|(row, _, order)| (*row, *order))
                    .or_else(|| {
                        Layout::navigation_base_row(&self.session, context)
                            .map(|row| (row, usize::MAX))
                    })
            })
            .unwrap_or_default();
        let unread: Vec<_> = targets
            .into_iter()
            .filter(|(_, context, _)| {
                self.notes
                    .items
                    .iter()
                    .find(|note| Some(note.id) == context.note_id)
                    .and_then(Note::agent_thread)
                    .is_some_and(|thread| thread.has_unread_reply())
            })
            .collect();
        let destination = if direction == ReplyDirection::Next {
            unread
                .iter()
                .find(|(row, _, order)| (*row, *order) > current)
                .or(unread.first())
        } else {
            unread
                .iter()
                .rev()
                .find(|(row, _, order)| (*row, *order) < current)
                .or(unread.last())
        };
        let Some((_, context, _)) = destination else {
            return false;
        };
        let Some(file_index) = context.file_index else {
            return false;
        };
        let Some(note_id) = context.note_id else {
            return false;
        };
        self.main_pane.file_states[file_index].collapsed = false;
        if !self.notes.expanded_ids.contains(&note_id) {
            self.notes.expanded_ids.push(note_id);
        }
        let display = LayoutDisplayContext {
            file_states: &self.main_pane.file_states,
            notes: &self.notes.items,
            expanded_note_ids: &self.notes.expanded_ids,
            reply_composer_note_id: None,
        };
        let Some(layout) = &mut self.layout else {
            return false;
        };
        layout.update_file_states(&self.session, display, None);
        let hunk_index = context.hunk_index.unwrap_or(0);
        layout.ensure_selected_hunk_ready_sync(&self.session, display, file_index, hunk_index);
        let Some(bounds) = layout.latest_reply_bounds(note_id) else {
            return false;
        };
        self.main_pane.cursor_row = bounds.start;
        self.main_pane.scroll = bounds.start as u16;
        self.main_pane.selection_anchor = None;
        self.main_pane.selected_file = file_index;
        self.main_pane.selected_hunk = hunk_index;
        self.global.focus = FocusPane::Main;
        let path = &self.session.files[file_index].path;
        self.sidebar.collapsed_dirs.retain(|directory| {
            !path
                .strip_prefix(directory.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
        });
        self.sync_sidebar_cursor_to_selected_file();
        self.acknowledge_focused_reply();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::diff::{DiffFile, DiffHunk, DiffLine, DiffSession};
    use crate::core::note::{AgentProvider, NoteTarget};
    use crate::tui::layout::RowContext;
    use crate::{
        tui::input::{self, NavAction},
        tui::render,
        tui::state::DiffMode,
    };
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    use std::time::Instant;

    const AREA: Rect = Rect::new(0, 0, 120, 24);

    fn app() -> App {
        let mut app = App::new(DiffSession {
            files: (0..3)
                .map(|file| DiffFile {
                    path: format!("src/nested/file{file}.rs"),
                    old_path: format!("src/nested/file{file}.rs"),
                    new_path: format!("src/nested/file{file}.rs"),
                    hunks: (0..10)
                        .map(|hunk| DiffHunk {
                            header: format!("@@ hunk {hunk} @@"),
                            lines: vec![DiffLine::Context {
                                old_lineno: hunk + 1,
                                new_lineno: hunk + 1,
                                text: "context".into(),
                            }],
                        })
                        .collect(),
                })
                .collect(),
        });
        render::ensure_layout(&mut app, AREA);
        app
    }

    fn reply(app: &mut App, id: u64, target: NoteTarget, long_history: bool) {
        let mut note = Note::new_agent(
            id,
            target,
            AgentProvider::Codex,
            "old prompt ".repeat(if long_history { 300 } else { 1 }),
            id,
        );
        let thread = note.agent_thread_mut().unwrap();
        assert!(thread.mark_running(id, Instant::now()));
        assert!(thread.complete(id, "session".into(), "old response ".repeat(100)));
        assert!(thread.queue_reply(id + 100, "follow up".into()));
        assert!(thread.mark_running(id + 100, Instant::now()));
        assert!(thread.complete(
            id + 100,
            "session".into(),
            "newest reply that needs attention".into()
        ));
        app.notes.items.push(note);
        app.refresh_note_overlay();
    }

    fn unread(app: &App, id: u64) -> bool {
        app.notes
            .items
            .iter()
            .find(|note| note.id == id)
            .unwrap()
            .agent_thread()
            .unwrap()
            .has_unread_reply()
    }

    fn file_target(index: usize) -> NoteTarget {
        NoteTarget::File {
            file_path: format!("src/nested/file{index}.rs"),
        }
    }

    #[test]
    fn jump_reveals_latest_response_in_collapsed_distant_hunk_and_survives_reflow() {
        for mode in [DiffMode::Inline, DiffMode::SideBySide] {
            let mut app = app();
            app.global.mode = mode;
            reply(
                &mut app,
                1,
                NoteTarget::Line {
                    file_path: "src/nested/file2.rs".into(),
                    old_lineno: Some(10),
                    new_lineno: Some(10),
                },
                true,
            );
            app.main_pane.file_states[2].collapsed = true;
            app.main_pane.file_states[1].collapsed = true;
            app.sidebar.collapsed_dirs = vec!["src".into(), "src/nested".into()];
            app.global.focus = FocusPane::Files;
            app.main_pane.selection_anchor = Some(RowContext::file_header(0));
            assert!(app.jump_to_unread_reply(ReplyDirection::Next));
            assert_eq!(app.global.focus, FocusPane::Main);
            assert!(!app.main_pane.file_states[2].collapsed);
            assert!(app.main_pane.file_states[1].collapsed);
            assert!(app.main_pane.selection_anchor.is_none());
            assert!(app.sidebar.collapsed_dirs.is_empty());
            assert!(!unread(&app, 1));
            assert_eq!(app.current_note_id(), Some(1));
            assert_eq!(
                (app.main_pane.selected_file, app.main_pane.selected_hunk),
                (2, 9)
            );
            input::complete_navigation(&mut app, AREA, NavAction::RevealAgentReply(1));
            for area in [AREA, Rect::new(0, 0, 85, 18), Rect::new(0, 0, 150, 30)] {
                input::complete_navigation(&mut app, area, NavAction::None);
                let mut terminal =
                    Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
                terminal.draw(|frame| render::render(frame, &app)).unwrap();
                let buffer = terminal.backend().buffer();
                let visible = (0..area.height)
                    .map(|row| {
                        (0..area.width)
                            .map(|column| buffer[(column, row)].symbol())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    visible.contains("newest"),
                    "{mode:?} at {area:?} must show newest response:\n{visible}"
                );
                assert_eq!(app.current_note_id(), Some(1));
                assert!(
                    app.layout
                        .as_ref()
                        .unwrap()
                        .latest_reply_bounds(1)
                        .unwrap()
                        .contains(&app.main_pane.cursor_row),
                    "resize must keep focus within the newest response"
                );
            }
        }
    }

    #[test]
    fn passive_resize_preserves_reply_focus_without_reading_or_moving_history_focus() {
        let mut app = app();
        reply(&mut app, 1, file_target(0), true);
        app.notes.expanded_ids.push(1);
        let narrow = Rect::new(0, 0, 85, 18);
        input::complete_navigation(&mut app, narrow, NavAction::None);
        let layout = app.layout.as_ref().unwrap();
        app.main_pane.cursor_row = layout.latest_reply_bounds(1).unwrap().end - 1;
        app.main_pane.scroll = app.main_pane.cursor_row as u16;
        input::complete_navigation(&mut app, Rect::new(0, 0, 150, 30), NavAction::None);
        assert!(
            app.layout
                .as_ref()
                .unwrap()
                .latest_reply_bounds(1)
                .unwrap()
                .contains(&app.main_pane.cursor_row)
        );
        assert!(unread(&app, 1), "resizing must not acknowledge a reply");

        let layout = app.layout.as_ref().unwrap();
        let history = layout
            .note_insertions
            .iter()
            .find(|note| note.context.note_id == Some(1))
            .unwrap()
            .context;
        app.main_pane.cursor_row = layout.row_index_for_context(&app.session, history).unwrap();
        app.main_pane.scroll = app.main_pane.cursor_row as u16;
        input::complete_navigation(&mut app, narrow, NavAction::None);
        assert_eq!(
            app.layout
                .as_ref()
                .unwrap()
                .row_context(&app.session, app.main_pane.cursor_row),
            Some(history)
        );
        assert!(unread(&app, 1));
    }

    #[test]
    fn navigation_wraps_in_diff_order_and_keeps_other_threads_unread() {
        let mut app = app();
        // Creation order differs from diff order, with two threads at the same anchor.
        reply(&mut app, 3, file_target(2), false);
        reply(&mut app, 2, file_target(0), false);
        reply(&mut app, 1, file_target(0), false);
        assert!(app.jump_to_unread_reply(ReplyDirection::Previous));
        assert_eq!(app.current_note_id(), Some(3));
        assert!(app.jump_to_unread_reply(ReplyDirection::Next));
        assert_eq!(app.current_note_id(), Some(2));
        let file = app
            .visible_sidebar_entries()
            .into_iter()
            .find(|entry| matches!(entry.kind, SidebarEntryKind::File { file_index: 0 }))
            .unwrap();
        assert!(app.sidebar_entry_has_unread(&file));
        assert!(app.jump_to_unread_reply(ReplyDirection::Next));
        assert_eq!(app.current_note_id(), Some(1));
        assert!(!app.sidebar_entry_has_unread(&file));
        let before = app.main_pane.cursor_row;
        assert!(!app.jump_to_unread_reply(ReplyDirection::Next));
        assert_eq!(app.main_pane.cursor_row, before);
    }

    #[test]
    fn only_latest_expanded_reply_focus_acknowledges_and_directories_aggregate() {
        let mut app = app();
        reply(&mut app, 1, file_target(0), false);
        app.sidebar.collapsed_dirs.push("src".into());
        let directory = app.visible_sidebar_entries().remove(0);
        assert!(app.sidebar_entry_has_unread(&directory));
        app.acknowledge_focused_reply();
        assert!(unread(&app, 1));
        app.notes.expanded_ids.push(1);
        app.refresh_note_overlay();
        let layout = app.layout.as_ref().unwrap();
        let context = layout.note_insertions[0].context;
        app.main_pane.cursor_row = layout
            .row_index_for_context(&app.session, context.with_note_row_offset(2))
            .unwrap();
        app.acknowledge_focused_reply();
        assert!(
            unread(&app, 1),
            "old history does not acknowledge newest reply"
        );
        app.main_pane.cursor_row = app
            .layout
            .as_ref()
            .unwrap()
            .latest_reply_bounds(1)
            .unwrap()
            .start;
        app.global.focus = FocusPane::Files;
        app.acknowledge_focused_reply();
        assert!(unread(&app, 1));
        app.global.focus = FocusPane::Main;
        app.acknowledge_focused_reply();
        assert!(!unread(&app, 1));
        assert!(!app.sidebar_entry_has_unread(&directory));
    }

    #[test]
    fn all_note_scopes_and_files_without_hunks_are_navigable() {
        let mut app = app();
        for (id, target) in [
            (1, file_target(0)),
            (
                2,
                NoteTarget::Hunk {
                    file_path: "src/nested/file0.rs".into(),
                    hunk_header: "@@ hunk 5 @@".into(),
                },
            ),
            (
                3,
                NoteTarget::Range {
                    file_path: "src/nested/file0.rs".into(),
                    start_old_lineno: Some(8),
                    start_new_lineno: Some(8),
                    end_old_lineno: Some(9),
                    end_new_lineno: Some(9),
                },
            ),
        ] {
            reply(&mut app, id, target, false);
        }
        for id in [1, 2, 3] {
            assert!(app.jump_to_unread_reply(ReplyDirection::Next));
            assert_eq!(app.current_note_id(), Some(id));
        }
        let mut empty = App::new(DiffSession { files: vec![] });
        assert!(!empty.jump_to_unread_reply(ReplyDirection::Next));
        let mut no_hunks = App::new(DiffSession {
            files: vec![DiffFile {
                path: "src/nested/file0.rs".into(),
                old_path: "src/nested/file0.rs".into(),
                new_path: "src/nested/file0.rs".into(),
                hunks: vec![],
            }],
        });
        render::ensure_layout(&mut no_hunks, AREA);
        reply(&mut no_hunks, 1, file_target(0), false);
        no_hunks.main_pane.file_states[0].collapsed = true;
        assert!(no_hunks.jump_to_unread_reply(ReplyDirection::Next));
        assert_eq!(no_hunks.current_note_id(), Some(1));
    }
}
