use std::{io, time::Duration};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::{
    render,
    state::{App, FocusPane},
};

pub enum NavAction {
    None,
    RevealSelectedHunk,
    SyncSelectionToScroll,
}

impl NavAction {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (_, Self::RevealSelectedHunk) | (Self::RevealSelectedHunk, _) => {
                Self::RevealSelectedHunk
            }
            (_, Self::SyncSelectionToScroll) | (Self::SyncSelectionToScroll, _) => {
                Self::SyncSelectionToScroll
            }
            _ => Self::None,
        }
    }
}

pub fn poll_and_handle_events(app: &mut App) -> io::Result<Option<NavAction>> {
    if !event::poll(Duration::from_millis(16))? {
        return Ok(None);
    }

    let mut action = NavAction::None;
    loop {
        action = action.merge(handle_event(app, event::read()?));
        if !event::poll(Duration::from_millis(0))? {
            break;
        }
    }

    Ok(Some(action))
}

fn handle_event(app: &mut App, event: Event) -> NavAction {
    match event {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_key_event(app, key)
        }
        Event::Mouse(mouse) => handle_mouse_event(app, mouse),
        _ => NavAction::None,
    }
}

fn handle_key_event(app: &mut App, key: KeyEvent) -> NavAction {
    if app.note_input_active() {
        return handle_note_input_key_event(app, key);
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => {
            app.quit();
            NavAction::None
        }
        (KeyCode::Char('j') | KeyCode::Down, _) => match app.global.focus {
            FocusPane::Main => {
                app.move_cursor_down(1, render::max_cursor_row(app));
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => {
                app.file_cursor_down();
                NavAction::None
            }
        },
        (KeyCode::Char('k') | KeyCode::Up, _) => match app.global.focus {
            FocusPane::Main => {
                app.move_cursor_up(1);
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => {
                app.file_cursor_up();
                NavAction::None
            }
        },
        (KeyCode::Char(']'), _) => match app.global.focus {
            FocusPane::Main => {
                app.next_hunk();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('['), _) => match app.global.focus {
            FocusPane::Main => {
                app.previous_hunk();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => match app.global.focus {
            FocusPane::Main => {
                app.move_cursor_down(10, render::max_cursor_row(app));
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => match app.global.focus {
            FocusPane::Main => {
                app.move_cursor_up(10);
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Left, _) => match app.global.focus {
            FocusPane::Files => {
                app.collapse_sidebar_directory();
                NavAction::None
            }
            FocusPane::Main => NavAction::None,
        },
        (KeyCode::Right, _) => match app.global.focus {
            FocusPane::Files => {
                app.expand_sidebar_directory();
                NavAction::None
            }
            FocusPane::Main => NavAction::None,
        },
        (KeyCode::Tab, KeyModifiers::SHIFT) | (KeyCode::BackTab, _) => {
            app.focus_previous();
            NavAction::None
        }
        (KeyCode::Tab, _) => {
            app.focus_next();
            NavAction::None
        }
        (KeyCode::Char('b'), _) => {
            app.toggle_sidebar();
            NavAction::RevealSelectedHunk
        }
        (KeyCode::Char('D'), _) => {
            app.toggle_debug_pane();
            NavAction::None
        }
        (KeyCode::Char('n'), _) => match app.global.focus {
            FocusPane::Main => {
                app.start_contextual_note_input();
                NavAction::None
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => match app.global.focus {
            FocusPane::Main => {
                app.toggle_current_file_collapsed();
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('c'), _) => match app.global.focus {
            FocusPane::Main => {
                app.cancel_current_agent();
                NavAction::None
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('r'), _) => match app.global.focus {
            FocusPane::Main => {
                app.retry_current_agent();
                NavAction::None
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('v'), _) => match app.global.focus {
            FocusPane::Main => {
                app.toggle_selection_anchor();
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Esc, _) => {
            app.clear_selection();
            NavAction::None
        }
        (KeyCode::Enter, _) => match app.global.focus {
            FocusPane::Files => {
                app.jump_to_file_cursor();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Main => {
                app.toggle_current_note_expanded();
                NavAction::SyncSelectionToScroll
            }
        },
        (KeyCode::Char('m'), _) => {
            app.toggle_mode();
            NavAction::RevealSelectedHunk
        }
        _ => NavAction::None,
    }
}

fn handle_mouse_event(app: &mut App, mouse: MouseEvent) -> NavAction {
    const WHEEL_SCROLL_LINES: u16 = 3;

    match mouse.kind {
        MouseEventKind::ScrollDown => {
            app.move_cursor_down(WHEEL_SCROLL_LINES as usize, render::max_cursor_row(app));
            NavAction::SyncSelectionToScroll
        }
        MouseEventKind::ScrollUp => {
            app.move_cursor_up(WHEEL_SCROLL_LINES as usize);
            NavAction::SyncSelectionToScroll
        }
        _ => NavAction::None,
    }
}

fn handle_note_input_key_event(app: &mut App, key: KeyEvent) -> NavAction {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            app.cancel_note_input();
            NavAction::None
        }
        (KeyCode::Enter, _) => {
            app.submit_note_input();
            NavAction::None
        }
        (KeyCode::Backspace, _) => {
            app.backspace_note_text();
            NavAction::None
        }
        (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            app.insert_note_text(&ch.to_string());
            NavAction::None
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.cancel_note_input();
            NavAction::None
        }
        _ => NavAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffFile, DiffHunk, DiffLine, DiffSession};
    use crate::layout::{RowContext, RowKind, RowViewState};
    use crate::note::NoteTarget;
    use ratatui::layout::Rect;

    const AREA: Rect = Rect::new(0, 0, 120, 30);

    fn app() -> App {
        let mut app = App::new(DiffSession {
            files: (0..3)
                .map(|file| DiffFile {
                    path: format!("file{file}.rs"),
                    old_path: format!("file{file}.rs"),
                    new_path: format!("file{file}.rs"),
                    hunks: (0..2)
                        .map(|hunk| DiffHunk {
                            header: format!("@@ -{} +{} @@ fn test", hunk + 1, hunk + 1),
                            lines: vec![DiffLine::Context {
                                old_lineno: hunk + 1,
                                new_lineno: hunk + 1,
                                text: "let x = 1;".into(),
                            }],
                        })
                        .collect(),
                })
                .collect(),
        });
        render::ensure_layout(&mut app, AREA);
        app
    }

    fn select(app: &mut App, context: RowContext) {
        app.main_pane.cursor_row = app
            .layout
            .as_ref()
            .unwrap()
            .row_index_for_context(&app.session, context)
            .unwrap();
        app.sync_selection_to_cursor();
    }

    fn ctrl_c(app: &mut App) {
        let action = handle_key_event(
            app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(matches!(action, NavAction::SyncSelectionToScroll));
        render::ensure_layout(app, AREA);
        app.sync_selection_to_cursor();
        render::ensure_cursor_visible(app, AREA);
    }

    #[test]
    fn ctrl_c_collapses_each_file_row_and_reopens_from_retained_header() {
        for context in [
            RowContext::separator(1),
            RowContext::file_header(1),
            RowContext::hunk_header(1, 1),
            RowContext::diff_line(1, 1, Some(2), Some(2)),
            RowContext::spacer(1, 1),
        ] {
            let mut app = app();
            let original = app.layout.as_ref().unwrap().row_contexts(&app.session);
            select(&mut app, context);
            app.toggle_selection_anchor();
            ctrl_c(&mut app);
            assert_eq!(
                app.layout
                    .as_ref()
                    .unwrap()
                    .row_context(&app.session, app.main_pane.cursor_row),
                Some(RowContext::file_header(1))
            );
            assert!(app.main_pane.selection_anchor.is_none());
            let rows = app.layout.as_ref().unwrap().row_contexts(&app.session);
            assert_eq!(
                rows.iter().filter(|row| row.file_index == Some(1)).count(),
                2
            );
            assert_eq!(rows.len(), original.len() - 6);
            assert!(!app.main_pane.file_states[0].collapsed);
            assert!(!app.main_pane.file_states[2].collapsed);
            ctrl_c(&mut app);
            assert_eq!(
                app.layout.as_ref().unwrap().row_contexts(&app.session),
                original
            );
            assert!(app.main_pane.file_states[1].viewed);
        }
    }

    #[test]
    fn collapse_hides_all_note_scopes_and_restores_note_rows() {
        let mut app = app();
        let path = "file1.rs".to_string();
        let header = app.session.files[1].hunks[0].header.clone();
        for target in [
            NoteTarget::File {
                file_path: path.clone(),
            },
            NoteTarget::Hunk {
                file_path: path.clone(),
                hunk_header: header,
            },
            NoteTarget::Line {
                file_path: path.clone(),
                old_lineno: Some(1),
                new_lineno: Some(1),
            },
            NoteTarget::Range {
                file_path: path,
                start_old_lineno: Some(1),
                start_new_lineno: Some(1),
                end_old_lineno: Some(2),
                end_new_lineno: Some(2),
            },
        ] {
            app.add_note(target, "keep this note".into());
        }
        let original = app.layout.as_ref().unwrap().row_contexts(&app.session);
        let note_row = original
            .iter()
            .copied()
            .find(|row| row.kind == RowKind::Note)
            .unwrap();
        select(&mut app, note_row);
        ctrl_c(&mut app);
        assert!(app.layout.as_ref().unwrap().note_insertions.is_empty());
        assert_eq!(app.notes.items.len(), 4);
        ctrl_c(&mut app);
        assert_eq!(
            app.layout.as_ref().unwrap().row_contexts(&app.session),
            original
        );
    }

    #[test]
    fn viewed_header_survives_rebuild_and_reopen_in_both_modes() {
        let mut app = app();
        select(&mut app, RowContext::file_header(1));
        ctrl_c(&mut app);
        render::ensure_layout(&mut app, Rect::new(0, 0, 100, 20));
        assert!(app.main_pane.file_states[1].collapsed);
        assert!(
            app.layout.as_ref().unwrap().base.plan.files[1]
                .hunks
                .is_empty()
        );
        for reopened in [false, true] {
            if reopened {
                ctrl_c(&mut app);
            }
            let layout = app.layout.as_ref().unwrap();
            let row = layout
                .row_index_for_context(&app.session, RowContext::file_header(1))
                .unwrap();
            for side_by_side in [false, true] {
                let lines = layout.render_visible_rows(
                    &app.session,
                    side_by_side,
                    &RowViewState {
                        scroll: row as u16,
                        selected_file: 1,
                        selected_hunk: 0,
                        cursor_row: row,
                        cursor_focused: true,
                        selected_rows: None,
                    },
                    1,
                );
                assert!(lines[0].to_string().contains('✓'));
            }
        }
    }

    #[test]
    fn navigation_skips_hidden_hunks_and_sidebar_reveals_collapsed_header() {
        let mut app = app();
        select(&mut app, RowContext::file_header(1));
        ctrl_c(&mut app);
        app.next_hunk();
        assert_eq!(
            (app.main_pane.selected_file, app.main_pane.selected_hunk),
            (2, 0)
        );
        app.previous_hunk();
        assert_eq!(
            (app.main_pane.selected_file, app.main_pane.selected_hunk),
            (0, 1)
        );
        app.sidebar.cursor = 1;
        app.jump_to_file_cursor();
        render::reveal_selected_hunk(&mut app, AREA);
        assert_eq!(
            app.layout
                .as_ref()
                .unwrap()
                .row_context(&app.session, app.main_pane.cursor_row),
            Some(RowContext::file_header(1))
        );
        assert!(app.main_pane.file_states[1].collapsed);
        for file in [0, 2] {
            select(&mut app, RowContext::file_header(file));
            ctrl_c(&mut app);
        }
        assert!(app.layout.as_ref().unwrap().hunk_ranges.is_empty());
        let selected = app.main_pane.selected_file;
        app.next_hunk();
        app.previous_hunk();
        assert_eq!(app.main_pane.selected_file, selected);
        assert_eq!(app.layout.as_ref().unwrap().row_count, 6);
    }

    #[test]
    fn plain_c_sidebar_focus_and_composer_do_not_toggle_files() {
        let mut app = app();
        select(&mut app, RowContext::file_header(0));
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        );
        app.global.focus = FocusPane::Files;
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        app.global.focus = FocusPane::Main;
        app.start_contextual_note_input();
        assert!(app.note_input_active());
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(app.note_input_active());
        assert!(
            app.main_pane
                .file_states
                .iter()
                .all(|state| !state.collapsed && !state.viewed)
        );
    }

    #[test]
    fn empty_diff_and_files_without_hunks_can_toggle_safely() {
        let mut app = App::new(DiffSession::default());
        render::ensure_layout(&mut app, AREA);
        ctrl_c(&mut app);
        let mut session = DiffSession::default();
        session.files.push(DiffFile {
            path: "binary".into(),
            old_path: "binary".into(),
            new_path: "binary".into(),
            hunks: vec![],
        });
        let mut app = App::new(session);
        render::ensure_layout(&mut app, AREA);
        render::reveal_selected_hunk(&mut app, AREA);
        assert_eq!(app.main_pane.cursor_row, 1);
        ctrl_c(&mut app);
        assert!(app.main_pane.file_states[0].collapsed);
        ctrl_c(&mut app);
        assert!(!app.main_pane.file_states[0].collapsed);
        assert_eq!(app.layout.as_ref().unwrap().row_count, 2);
    }
    #[test]
    fn zero_hunk_file_notes_follow_authoritative_collapse_state() {
        let mut app = App::new(DiffSession {
            files: vec![DiffFile {
                path: "binary".into(),
                old_path: "binary".into(),
                new_path: "binary".into(),
                hunks: vec![],
            }],
        });
        render::ensure_layout(&mut app, AREA);
        app.add_note(
            NoteTarget::File {
                file_path: "binary".into(),
            },
            "expanded file note".into(),
        );
        assert_eq!(app.layout.as_ref().unwrap().note_insertions.len(), 1);
        select(&mut app, RowContext::file_header(0));
        ctrl_c(&mut app);
        app.add_note(
            NoteTarget::File {
                file_path: "binary".into(),
            },
            "added while collapsed".into(),
        );
        assert!(app.layout.as_ref().unwrap().note_insertions.is_empty());
        render::ensure_layout(&mut app, Rect::new(0, 0, 100, 20));
        assert!(app.layout.as_ref().unwrap().note_insertions.is_empty());
        assert_eq!(app.layout.as_ref().unwrap().row_count, 2);
        ctrl_c(&mut app);
        assert_eq!(app.layout.as_ref().unwrap().note_insertions.len(), 2);
        assert!(
            app.layout.as_ref().unwrap().base.plan.files[0]
                .hunks
                .is_empty()
        );
    }
}
