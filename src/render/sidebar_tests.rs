use crate::note::NoteTarget;

#[test]
fn unread_file_dot_replaces_status_without_changing_other_columns() {
    use crate::diff::{DiffFile, DiffHunk, DiffLine, DiffSession, FileChangeKind};
    use crate::note::{AgentProvider, Note};
    use crate::state::{App, SidebarEntry, SidebarEntryKind};
    use ratatui::style::{Color, Modifier};

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
                    assert!(dot.style.add_modifier.contains(Modifier::BOLD));
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
