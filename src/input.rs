use std::{io, time::Duration};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::{
    render,
    state::{App, FocusPane, ReplyDirection},
};

pub enum NavAction {
    None,
    RevealSelectedHunk,
    RevealAgentReply(u64),
    SyncSelectionToScroll,
}

impl NavAction {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (_, Self::RevealAgentReply(id)) => Self::RevealAgentReply(id),
            (Self::RevealAgentReply(id), Self::None) => Self::RevealAgentReply(id),
            (Self::RevealAgentReply(_), later) => later,
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

/// Applies pending navigation and keeps the cursor, viewport and sidebar aligned.
pub fn complete_navigation(app: &mut App, viewport_area: ratatui::layout::Rect, action: NavAction) {
    render::ensure_layout(app, viewport_area);

    app.clamp_cursor_row(render::max_cursor_row(app));
    app.main_pane.scroll = app
        .main_pane
        .scroll
        .min(render::max_scroll(app, viewport_area));

    match action {
        NavAction::RevealSelectedHunk => render::reveal_selected_hunk(app, viewport_area),
        NavAction::RevealAgentReply(note_id) => {
            render::reveal_agent_reply(app, viewport_area, note_id);
        }
        NavAction::SyncSelectionToScroll => {
            app.sync_selection_to_cursor();
            render::ensure_cursor_visible(app, viewport_area);
        }
        NavAction::None => {}
    }

    app.sync_selection_to_cursor();
    render::ensure_cursor_visible(app, viewport_area);

    if app.global.focus != FocusPane::Files {
        app.sync_sidebar_cursor_to_selected_file();
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
        Event::Mouse(mouse) => {
            app.thread_prefix_pending = false;
            let before = app.main_pane.cursor_row;
            let action = handle_mouse_event(app, mouse);
            if before != app.main_pane.cursor_row {
                app.acknowledge_focused_reply();
            }
            action
        }
        Event::Resize(_, _) | Event::FocusLost => {
            app.thread_prefix_pending = false;
            NavAction::None
        }
        _ => NavAction::None,
    }
}

fn handle_key_event(app: &mut App, key: KeyEvent) -> NavAction {
    let before = (app.main_pane.cursor_row, app.global.focus);
    if app.note_input_active() {
        app.thread_prefix_pending = false;
        return handle_note_input_key_event(app, key);
    }
    if std::mem::take(&mut app.thread_prefix_pending)
        && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
        && matches!(key.code, KeyCode::Char('a' | 'A'))
    {
        return if app.jump_to_unread_reply(if key.code == KeyCode::Char('a') {
            ReplyDirection::Next
        } else {
            ReplyDirection::Previous
        }) {
            NavAction::RevealAgentReply(
                app.current_note_id()
                    .expect("successful reply jump selects a note"),
            )
        } else {
            NavAction::None
        };
    }
    if key.code == KeyCode::Char('g') && key.modifiers == KeyModifiers::NONE {
        app.thread_prefix_pending = true;
        return NavAction::None;
    }
    let action = handle_navigation_key(app, key);
    let focus_moved = before != (app.main_pane.cursor_row, app.global.focus);
    // Deferred reveals have not established their destination yet.
    if matches!(action, NavAction::None | NavAction::SyncSelectionToScroll)
        && (focus_moved || key.code == KeyCode::Enter)
    {
        app.acknowledge_focused_reply();
    }
    // Opening a reply composer moves the cursor without requesting a reveal.
    // Preserve that destination when an earlier event requested a reply jump.
    if focus_moved && matches!(action, NavAction::None) {
        NavAction::SyncSelectionToScroll
    } else {
        action
    }
}

fn handle_navigation_key(app: &mut App, key: KeyEvent) -> NavAction {
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

    fn add_unread_reply(app: &mut App) {
        let mut note = crate::note::Note::new_agent(
            1,
            NoteTarget::File {
                file_path: "file2.rs".into(),
            },
            crate::note::AgentProvider::Codex,
            "prompt".into(),
            1,
        );
        let thread = note.agent_thread_mut().unwrap();
        assert!(thread.mark_running(1, std::time::Instant::now()));
        assert!(thread.complete(1, "s".into(), "new reply".into()));
        app.notes.items.push(note);
        app.layout = None;
        render::ensure_layout(app, AREA);
    }

    fn key(app: &mut App, ch: char) -> NavAction {
        handle_key_event(
            app,
            KeyEvent::new(
                KeyCode::Char(ch),
                if ch.is_uppercase() {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                },
            ),
        )
    }

    #[test]
    fn reply_keys_work_from_both_panes_and_do_not_intercept_the_composer() {
        for pane in [FocusPane::Main, FocusPane::Files] {
            for direction in ['a', 'A'] {
                let mut app = app();
                add_unread_reply(&mut app);
                app.global.focus = pane;
                key(&mut app, 'g');
                assert!(matches!(
                    key(&mut app, direction),
                    NavAction::RevealAgentReply(1)
                ));
                assert_eq!(app.current_note_id(), Some(1));
                assert_eq!(app.global.focus, FocusPane::Main);
                assert!(
                    !app.notes.items[0]
                        .agent_thread()
                        .unwrap()
                        .has_unread_reply()
                );
            }
        }
        let mut app = app();
        add_unread_reply(&mut app);
        app.start_new_note_input();
        key(&mut app, 'g');
        key(&mut app, 'a');
        assert_eq!(app.notes.composer.as_ref().unwrap().draft, "ga");
        assert!(
            app.notes.items[0]
                .agent_thread()
                .unwrap()
                .has_unread_reply()
        );
    }

    #[test]
    fn reply_prefix_cancels_and_hunk_keys_remain_immediate() {
        let mut app = app();
        add_unread_reply(&mut app);
        key(&mut app, 'g');
        handle_key_event(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.thread_prefix_pending);
        assert!(matches!(key(&mut app, 'a'), NavAction::None));
        key(&mut app, 'g');
        handle_key_event(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(!app.thread_prefix_pending);
        key(&mut app, 'a');
        assert!(
            app.notes.items[0]
                .agent_thread()
                .unwrap()
                .has_unread_reply()
        );
        app.global.focus = FocusPane::Main;
        assert!(matches!(key(&mut app, ']'), NavAction::RevealSelectedHunk));
        assert_eq!(app.main_pane.selected_hunk, 1);
        assert!(matches!(key(&mut app, '['), NavAction::RevealSelectedHunk));
        assert_eq!(app.main_pane.selected_hunk, 0);
        key(&mut app, 'g');
        assert!(matches!(key(&mut app, 'n'), NavAction::None));
        assert!(!app.thread_prefix_pending);
        assert!(app.note_input_active());
    }

    #[test]
    fn frame_sync_and_unrelated_keys_do_not_acknowledge_new_arrivals() {
        let mut app = app();
        add_unread_reply(&mut app);
        app.notes.expanded_ids.push(1);
        app.layout = None;
        render::ensure_layout(&mut app, AREA);
        let bounds = app.layout.as_ref().unwrap().latest_reply_bounds(1).unwrap();
        app.main_pane.cursor_row = bounds.start;
        app.sync_selection_to_cursor();
        render::ensure_layout(&mut app, AREA);
        app.sync_selection_to_cursor();
        key(&mut app, 'x');
        assert!(
            app.notes.items[0]
                .agent_thread()
                .unwrap()
                .has_unread_reply()
        );
        key(&mut app, 'j');
        assert!(
            !app.notes.items[0]
                .agent_thread()
                .unwrap()
                .has_unread_reply()
        );
    }

    #[test]
    fn later_cursor_movement_supersedes_a_reply_jump_in_the_same_batch() {
        let mut app = app();
        add_unread_reply(&mut app);
        key(&mut app, 'g');
        let jump = key(&mut app, 'a');
        let expected = app.main_pane.cursor_row + 1;
        let movement = key(&mut app, 'j');
        complete_navigation(&mut app, AREA, jump.merge(movement));
        assert_eq!(app.main_pane.cursor_row, expected);
    }

    #[test]
    fn later_hunk_movement_supersedes_a_reply_jump_in_the_same_batch() {
        let mut app = app();
        add_unread_reply(&mut app);
        key(&mut app, 'g');
        let jump = key(&mut app, 'a');
        let movement = key(&mut app, ']');
        complete_navigation(&mut app, AREA, jump.merge(movement));
        assert_eq!(
            (app.main_pane.selected_file, app.main_pane.selected_hunk),
            (2, 1)
        );
    }

    #[test]
    fn final_reply_jump_supersedes_earlier_hunk_movement_in_the_same_batch() {
        let mut app = app();
        add_unread_reply(&mut app);
        let movement = key(&mut app, ']');
        key(&mut app, 'g');
        let jump = key(&mut app, 'a');
        complete_navigation(&mut app, AREA, movement.merge(jump));
        assert_eq!(app.current_note_id(), Some(1));
        let bounds = app.layout.as_ref().unwrap().latest_reply_bounds(1).unwrap();
        assert_eq!(app.main_pane.cursor_row, bounds.start);
    }

    #[test]
    fn later_composer_input_keeps_its_cursor_after_a_reply_jump() {
        let mut app = app();
        add_unread_reply(&mut app);
        key(&mut app, 'g');
        let jump = key(&mut app, 'a');
        let composer = key(&mut app, 'n');
        let typing = key(&mut app, 'x');
        let expected = app.reply_composer_bounds().unwrap().start;
        complete_navigation(&mut app, AREA, jump.merge(composer).merge(typing));
        assert!(app.note_input_active());
        assert_eq!(app.notes.composer.as_ref().unwrap().draft, "x");
        assert_eq!(app.main_pane.cursor_row, expected);
    }

    #[test]
    fn later_thread_collapse_supersedes_a_reply_jump_in_the_same_batch() {
        let mut app = app();
        add_unread_reply(&mut app);
        key(&mut app, 'g');
        let jump = key(&mut app, 'a');
        let collapse =
            handle_key_event(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let expected = app.main_pane.cursor_row;
        complete_navigation(&mut app, AREA, jump.merge(collapse));
        assert!(!app.notes.expanded_ids.contains(&1));
        assert_eq!(app.main_pane.cursor_row, expected);
    }

    #[test]
    fn deferred_hunk_reveals_do_not_acknowledge_the_previous_cursor() {
        for command in ['b', '\n'] {
            let mut app = app();
            add_unread_reply(&mut app);
            app.notes.expanded_ids.push(1);
            app.layout = None;
            render::ensure_layout(&mut app, AREA);
            let bounds = app.layout.as_ref().unwrap().latest_reply_bounds(1).unwrap();
            app.main_pane.cursor_row = bounds.start;
            app.sync_selection_to_cursor();
            app.global.focus = FocusPane::Files;
            let action = if command == '\n' {
                handle_key_event(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            } else {
                key(&mut app, command)
            };
            assert!(matches!(action, NavAction::RevealSelectedHunk));
            assert!(
                app.notes.items[0]
                    .agent_thread()
                    .unwrap()
                    .has_unread_reply(),
                "{command:?} has not visited the reply before its deferred reveal"
            );
            complete_navigation(&mut app, AREA, action);
            assert_eq!(
                app.global.focus,
                if command == 'b' {
                    FocusPane::Main
                } else {
                    FocusPane::Files
                }
            );
            assert_ne!(app.current_note_id(), Some(1));
            assert!(
                app.notes.items[0]
                    .agent_thread()
                    .unwrap()
                    .has_unread_reply(),
                "{command:?} reveals a hunk header without visiting the reply"
            );
        }
    }
}
