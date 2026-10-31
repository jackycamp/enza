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
    assert!(app.layout.as_ref().unwrap().base.plan.files[1].collapsed);
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
