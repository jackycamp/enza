use crate::note::NoteTarget;

#[test]
fn unread_file_dot_replaces_status_without_changing_other_columns() {
    use crate::diff::{DiffFile, DiffHunk, DiffLine, DiffSession, FileChangeKind};
    use crate::note::{AgentProvider, Note};
    use crate::state::{App, SidebarEntry, SidebarEntryKind};
    use ratatui::style::Color;

    for (kind, marker, color) in [
        (FileChangeKind::Added, "A", Color::Green),
        (FileChangeKind::Modified, "M", Color::Yellow),
        (FileChangeKind::Deleted, "D", Color::Red),
    ] {
        for name in ["file.rs", "very_long_file_name.rs"] {
            let path = format!("src/{name}");
            let mut app = App::new(DiffSession {
                files: vec![DiffFile {
                    path: path.clone(),
                    old_path: if kind == FileChangeKind::Added {
                        "/dev/null".into()
                    } else {
                        path.clone()
                    },
                    new_path: if kind == FileChangeKind::Deleted {
                        "/dev/null".into()
                    } else {
                        path.clone()
                    },
                    hunks: vec![DiffHunk {
                        header: "@@ context @@".into(),
                        lines: vec![DiffLine::Context {
                            old_lineno: 1,
                            new_lineno: 1,
                            text: "context".into(),
                        }],
                    }],
                }],
            });
            let cases: Vec<_> = [1, 20]
                .into_iter()
                .flat_map(|depth| {
                    let file = SidebarEntry {
                        depth,
                        label: name.into(),
                        kind: SidebarEntryKind::File { file_index: 0 },
                    };
                    [0, 1, 2, 3, 4, 8, 16, 24, 40]
                        .into_iter()
                        .map(move |width| (file.clone(), width))
                })
                .map(|(file, width)| {
                    let read = super::sidebar_entry_line(&app, &file, width);
                    (file, width, read)
                })
                .collect();
            let mut note = Note::new_agent(
                1,
                NoteTarget::File { file_path: path },
                AgentProvider::Codex,
                "prompt".into(),
                1,
            );
            let thread = note.agent_thread_mut().unwrap();
            assert!(thread.mark_running(1, std::time::Instant::now()));
            assert!(thread.complete(1, "s".into(), "reply".into()));
            app.notes.items.push(note);

            for (file, width, read) in &cases {
                let unread = super::sidebar_entry_line(&app, file, *width);
                assert!(unread.width() <= *width);
                assert_eq!(unread.width(), read.width());
                let read_text = read.to_string();
                let expected = if *width == 0 {
                    String::new()
                } else {
                    let (before, after) = read_text.rsplit_once(marker).unwrap();
                    format!("{before}●{after}")
                };
                assert_eq!(
                    unread.to_string(),
                    expected,
                    "kind {kind:?}, depth {}, width {width}",
                    file.depth
                );
                if *width > 0 {
                    let dot = unread
                        .spans
                        .iter()
                        .find(|span| span.content == "●")
                        .unwrap();
                    assert_eq!(dot.style.fg, Some(Color::Rgb(180, 120, 255)));
                }
            }
            app.notes.items[0].agent_thread_mut().unwrap().mark_read();
            for (file, width, before) in cases {
                let restored = super::sidebar_entry_line(&app, &file, width);
                assert_eq!(restored, before, "status and all spacing must be restored");
                if width > 0 {
                    let status = restored
                        .spans
                        .iter()
                        .find(|span| span.content == marker)
                        .unwrap();
                    assert_eq!(status.style.fg, Some(color));
                }
            }
        }
    }
}

#[test]
fn directory_unread_dots_remain_appended_and_clear_after_reading() {
    use crate::diff::DiffSession;
    use crate::note::{AgentProvider, Note};
    use crate::state::{App, SidebarEntry, SidebarEntryKind};
    use ratatui::style::Color;
    let mut app = App::new(DiffSession { files: vec![] });
    let directory = SidebarEntry {
        depth: 0,
        label: "▸ src".into(),
        kind: SidebarEntryKind::Directory {
            path: "src".into(),
            collapsed: true,
        },
    };
    let mut note = Note::new_agent(
        1,
        NoteTarget::File {
            file_path: "src/file.rs".into(),
        },
        AgentProvider::Codex,
        "prompt".into(),
        1,
    );
    let thread = note.agent_thread_mut().unwrap();
    assert!(thread.mark_running(1, std::time::Instant::now()));
    assert!(thread.complete(1, "s".into(), "reply".into()));
    app.notes.items.push(note);
    for width in [0, 1, 2, 4, 20] {
        let line = super::sidebar_entry_line(&app, &directory, width);
        assert!(line.width() <= width);
        if width >= 2 {
            assert!(line.to_string().ends_with(" ●"));
            assert_eq!(
                line.spans.last().unwrap().style.fg,
                Some(Color::Rgb(180, 120, 255))
            );
        }
    }
    assert_eq!(
        super::sidebar_entry_line(&app, &directory, 20).to_string(),
        "▸ src ●"
    );
    app.notes.items[0].agent_thread_mut().unwrap().mark_read();
    assert_eq!(
        super::sidebar_entry_line(&app, &directory, 20).to_string(),
        "▸ src"
    );
}

#[test]
fn sidebar_totals_cover_all_loaded_files_and_ignore_collapse() {
    use crate::diff::{DiffFile, DiffHunk, DiffLine, DiffSession};
    use crate::state::{App, FocusPane};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        layout::Rect,
        style::{Color, Modifier},
    };

    let mut app = App::new(DiffSession {
        files: vec![
            DiffFile {
                path: "src/one.rs".into(),
                old_path: "src/one.rs".into(),
                new_path: "src/one.rs".into(),
                hunks: vec![DiffHunk {
                    header: "@@ first @@".into(),
                    lines: vec![
                        DiffLine::Added {
                            new_lineno: 1,
                            text: "new".into(),
                        },
                        DiffLine::Removed {
                            old_lineno: 1,
                            text: "old".into(),
                        },
                        DiffLine::Context {
                            old_lineno: 2,
                            new_lineno: 2,
                            text: "same".into(),
                        },
                    ],
                }],
            },
            DiffFile {
                path: "src/two.rs".into(),
                old_path: "/dev/null".into(),
                new_path: "src/two.rs".into(),
                hunks: vec![DiffHunk {
                    header: "@@ second @@".into(),
                    lines: vec![DiffLine::Added {
                        new_lineno: 1,
                        text: "another".into(),
                    }],
                }],
            },
        ],
    });
    app.global.focus = FocusPane::Files;
    for collapsed in [false, true] {
        if collapsed {
            app.collapse_sidebar_directory();
            app.main_pane.file_states[0].collapsed = true;
        }
        let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();
        terminal
            .draw(|frame| super::render_sidebar_files(frame, Rect::new(0, 0, 30, 8), &app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let title: String = (0..30).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(title.contains(" Files +2, -1 "), "title: {title}");
        for (x, expected_color) in [(8, Color::Green), (12, Color::Red)] {
            let cell = &buffer[(x, 0)];
            assert_eq!(cell.fg, expected_color);
            assert!(cell.modifier.contains(Modifier::BOLD));
        }
        assert_eq!(app.sidebar.cursor, 0);
    }
    app.expand_sidebar_directory();
    app.file_cursor_down();
    app.activate_sidebar_cursor();
    assert_eq!(app.main_pane.selected_file, 0);
    assert_eq!(app.sidebar.cursor, 1);
}

#[test]
fn sidebar_empty_totals_and_narrow_bounds() {
    use crate::{diff::DiffSession, state::App};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    let app = App::new(DiffSession::default());
    let mut terminal = Terminal::new(TestBackend::new(32, 8)).unwrap();
    terminal
        .draw(|frame| super::render_sidebar_files(frame, Rect::new(0, 0, 30, 8), &app))
        .unwrap();
    let title: String = (0..30)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
        .collect();
    assert!(title.contains(" Files +0, -0 "), "title: {title}");
    for width in 0..15 {
        terminal
            .draw(|frame| super::render_sidebar_files(frame, Rect::new(0, 0, width, 8), &app))
            .unwrap();
        for y in 0..8 {
            for x in width..32 {
                assert_eq!(
                    terminal.backend().buffer()[(x, y)].symbol(),
                    " ",
                    "width {width}, cell {x},{y}"
                );
            }
        }
    }
}
