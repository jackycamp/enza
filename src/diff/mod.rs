#[derive(Clone, Debug)]
pub struct DiffSession {
    pub files: Vec<DiffFile>,
}

#[derive(Clone, Debug)]
pub struct DiffFile {
    pub path: &'static str,
    pub old_path: &'static str,
    pub new_path: &'static str,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileChangeKind {
    Added,
    Modified,
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub header: &'static str,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub enum DiffLine {
    Context {
        old_lineno: usize,
        new_lineno: usize,
        text: &'static str,
    },
    Added {
        new_lineno: usize,
        text: &'static str,
    },
    Removed {
        old_lineno: usize,
        text: &'static str,
    },
}

impl DiffSession {
    pub fn demo() -> Self {
        Self {
            files: vec![
                DiffFile {
                    path: "src/main.rs",
                    old_path: "src/main.rs",
                    new_path: "src/main.rs",
                    hunks: vec![
                        DiffHunk {
                            header: "@@ -1,8 +1,12 @@",
                            lines: vec![
                                ctx(1, 1, "use std::{io, time::Duration};"),
                                del(2, "use crossterm::event::{self, Event, KeyCode};"),
                                add(
                                    2,
                                    "use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};",
                                ),
                                ctx(3, 3, ""),
                                ctx(4, 4, "use crate::{app::App, cli::Cli};"),
                                add(5, "use crate::ui::max_scroll;"),
                                ctx(5, 6, ""),
                                ctx(6, 7, "fn main() -> io::Result<()> {"),
                                add(8, "    let _cli = Cli::parse();"),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -24,6 +28,14 @@",
                            lines: vec![
                                ctx(24, 28, "while app.running {"),
                                add(29, "    let viewport_area = terminal.get_frame().area();"),
                                add(
                                    30,
                                    "    app.scroll = app.scroll.min(max_scroll(&app, viewport_area));",
                                ),
                                add(31, "    terminal.draw(|frame| ui::render(frame, &app))?;"),
                                ctx(25, 32, ""),
                                del(26, "    if event::poll(Duration::from_millis(1000))? {"),
                                add(33, "    if event::poll(Duration::from_millis(250))?"),
                                add(34, "        && let Event::Key(key) = event::read()?"),
                                add(35, "    {"),
                                add(36, "        handle_key_event(&mut app, key);"),
                                add(37, "    }"),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -40,0 +50,7 @@",
                            lines: vec![
                                add(50, "fn handle_key_event(app: &mut App, key: KeyEvent) {"),
                                add(51, "    match (key.code, key.modifiers) {"),
                                add(
                                    52,
                                    "        (KeyCode::Char('j') | KeyCode::Down, _) => app.scroll_down(1),",
                                ),
                                add(
                                    53,
                                    "        (KeyCode::Char('k') | KeyCode::Up, _) => app.scroll_up(1),",
                                ),
                                add(54, "        (KeyCode::Char(']'), _) => app.next_hunk(),"),
                                add(
                                    55,
                                    "        (KeyCode::Char('['), _) => app.previous_hunk(),",
                                ),
                            ],
                        },
                    ],
                },
                DiffFile {
                    path: "src/app/state.rs",
                    old_path: "src/app/state.rs",
                    new_path: "src/app/state.rs",
                    hunks: vec![
                        DiffHunk {
                            header: "@@ -8,9 +8,14 @@",
                            lines: vec![
                                ctx(8, 8, "pub struct App {"),
                                add(9, "    pub session: DiffSession,"),
                                ctx(9, 10, "    pub mode: DiffMode,"),
                                del(10, "    pub files: Vec<&'static str>,"),
                                add(11, "    pub selected_file: usize,"),
                                add(12, "    pub selected_hunk: usize,"),
                                add(13, "    pub scroll: u16,"),
                                ctx(11, 14, "}"),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -30,6 +35,23 @@",
                            lines: vec![
                                ctx(30, 35, "pub fn toggle_sidebar(&mut self) {"),
                                ctx(31, 36, "    self.sidebar_open = !self.sidebar_open;"),
                                ctx(32, 37, "}"),
                                add(38, ""),
                                add(39, "pub fn next_hunk(&mut self) {"),
                                add(40, "    let current_file = self.current_file();"),
                                add(
                                    41,
                                    "    if self.selected_hunk + 1 < current_file.hunks.len() {",
                                ),
                                add(42, "        self.selected_hunk += 1;"),
                                add(
                                    43,
                                    "    } else if self.selected_file + 1 < self.session.files.len() {",
                                ),
                                add(44, "        self.selected_file += 1;"),
                                add(45, "        self.selected_hunk = 0;"),
                                add(46, "    }"),
                                add(47, "}"),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -55,0 +78,18 @@",
                            lines: vec![
                                add(78, "pub fn total_hunks(&self) -> usize {"),
                                add(
                                    79,
                                    "    self.session.files.iter().map(|file| file.hunks.len()).sum()",
                                ),
                                add(80, "}"),
                                add(81, ""),
                                add(82, "pub fn selected_hunk_global_index(&self) -> usize {"),
                                add(83, "    let prior_hunks: usize = self"),
                                add(84, "        .session"),
                                add(85, "        .files"),
                                add(86, "        .iter()"),
                                add(87, "        .take(self.selected_file)"),
                                add(88, "        .map(|file| file.hunks.len())"),
                                add(89, "        .sum();"),
                                add(90, "    prior_hunks + self.selected_hunk + 1"),
                                add(91, "}"),
                            ],
                        },
                    ],
                },
                DiffFile {
                    path: "src/ui/mod.rs",
                    old_path: "src/ui/mod.rs",
                    new_path: "src/ui/mod.rs",
                    hunks: vec![
                        DiffHunk {
                            header: "@@ -1,6 +1,11 @@",
                            lines: vec![
                                ctx(1, 1, "use ratatui::{"),
                                add(2, "    style::{Color, Modifier, Style, Stylize},"),
                                ctx(2, 3, "    text::{Line, Span},"),
                                del(3, "    widgets::{Block, Borders, List, Paragraph},"),
                                add(
                                    4,
                                    "    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},",
                                ),
                                add(5, "};"),
                                add(6, ""),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -18,0 +25,29 @@",
                            lines: vec![
                                add(25, "pub fn max_scroll(app: &App, area: Rect) -> u16 {"),
                                add(26, "    let root = Layout::default()"),
                                add(27, "        .direction(Direction::Vertical)"),
                                add(
                                    28,
                                    "        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(2)])",
                                ),
                                add(29, "        .split(area);"),
                                add(30, "    let body = if app.sidebar_open {"),
                                add(31, "        Layout::default()"),
                                add(32, "            .direction(Direction::Horizontal)"),
                                add(
                                    33,
                                    "            .constraints([Constraint::Length(28), Constraint::Min(1)])",
                                ),
                                add(34, "            .split(root[1])[1]"),
                                add(35, "    } else {"),
                                add(36, "        root[1]"),
                                add(37, "    };"),
                                add(
                                    38,
                                    "    let visible_lines = body.height.saturating_sub(2) as usize;",
                                ),
                                add(39, "    let total_lines = document_line_count(app);"),
                                add(40, "    total_lines.saturating_sub(visible_lines) as u16"),
                                add(41, "}"),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -90,0 +132,30 @@",
                            lines: vec![
                                add(
                                    132,
                                    "fn side_by_side_session_lines<'a>(app: &'a App, scroll: u16, width: usize) -> Vec<Line<'a>> {",
                                ),
                                add(133, "    let mut lines = Vec::new();"),
                                add(
                                    134,
                                    "    let (left_width, right_width) = split_diff_width(width);",
                                ),
                                add(
                                    135,
                                    "    for (file_index, file) in app.session.files.iter().enumerate() {",
                                ),
                                add(136, "        lines.push(file_separator_line(width));"),
                                add(
                                    137,
                                    "        lines.push(file_header_line(file, file_index == app.selected_file));",
                                ),
                                add(
                                    138,
                                    "        for (hunk_index, hunk) in file.hunks.iter().enumerate() {",
                                ),
                                add(
                                    139,
                                    "            let selected = file_index == app.selected_file && hunk_index == app.selected_hunk;",
                                ),
                                add(
                                    140,
                                    "            lines.push(hunk_header_line(hunk.header, selected));",
                                ),
                                add(
                                    141,
                                    "            lines.push(side_by_side_column_header(left_width, right_width));",
                                ),
                                add(142, "        }"),
                                add(143, "    }"),
                                add(144, "    lines.into_iter().skip(scroll as usize).collect()"),
                                add(145, "}"),
                            ],
                        },
                    ],
                },
                DiffFile {
                    path: "src/diff/mod.rs",
                    old_path: "src/diff/mod.rs",
                    new_path: "src/diff/mod.rs",
                    hunks: vec![
                        DiffHunk {
                            header: "@@ -1,0 +1,22 @@",
                            lines: vec![
                                add(1, "#[derive(Clone, Debug)]"),
                                add(2, "pub struct DiffSession {"),
                                add(3, "    pub files: Vec<DiffFile>,"),
                                add(4, "}"),
                                add(5, ""),
                                add(6, "#[derive(Clone, Debug)]"),
                                add(7, "pub struct DiffFile {"),
                                add(8, "    pub path: &'static str,"),
                                add(9, "    pub old_path: &'static str,"),
                                add(10, "    pub new_path: &'static str,"),
                                add(11, "    pub hunks: Vec<DiffHunk>,"),
                                add(12, "}"),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -24,0 +30,18 @@",
                            lines: vec![
                                add(30, "impl DiffSession {"),
                                add(31, "    pub fn demo() -> Self {"),
                                add(32, "        Self {"),
                                add(33, "            files: vec!["),
                                add(34, "                DiffFile {"),
                                add(35, "                    path: \"src/main.rs\","),
                                add(36, "                    old_path: \"src/main.rs\","),
                                add(37, "                    new_path: \"src/main.rs\","),
                                add(38, "                    hunks: vec!["),
                                add(39, "                        DiffHunk {"),
                                add(
                                    40,
                                    "                            header: \"@@ -1,8 +1,12 @@\",",
                                ),
                            ],
                        },
                    ],
                },
                DiffFile {
                    path: "src/notes/store.rs",
                    old_path: "src/notes/store.rs",
                    new_path: "src/notes/store.rs",
                    hunks: vec![
                        DiffHunk {
                            header: "@@ -1,0 +1,16 @@",
                            lines: vec![
                                add(1, "use std::path::PathBuf;"),
                                add(2, "use std::time::SystemTime;"),
                                add(3, ""),
                                add(4, "pub struct NoteStore {"),
                                add(5, "    path: PathBuf,"),
                                add(6, "    last_loaded_at: Option<SystemTime>,"),
                                add(7, "}"),
                                add(8, ""),
                                add(9, "impl NoteStore {"),
                                add(10, "    pub fn open(path: PathBuf) -> Self {"),
                                add(11, "        Self { path, last_loaded_at: None }"),
                                add(12, "    }"),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -18,0 +22,15 @@",
                            lines: vec![
                                add(
                                    22,
                                    "    pub fn append_note(&mut self, target_id: &str, body: &str) {",
                                ),
                                add(23, "        let _record = format!(\"{target_id}:{body}\");"),
                                add(24, "    }"),
                                add(25, ""),
                                add(26, "    pub fn reload_if_changed(&mut self) -> bool {"),
                                add(27, "        false"),
                                add(28, "    }"),
                            ],
                        },
                    ],
                },
                DiffFile {
                    path: "SPEC.md",
                    old_path: "SPEC.md",
                    new_path: "SPEC.md",
                    hunks: vec![
                        DiffHunk {
                            header: "@@ -10,8 +10,8 @@",
                            lines: vec![
                                ctx(
                                    10,
                                    10,
                                    "- Support for intel mac, mac m1, and debian linux x86_64",
                                ),
                                ctx(11, 11, "- Keyboard shortcuts:"),
                                ctx(12, 12, "  - j/k down/up"),
                                ctx(13, 13, "  - ][ next hunk/prev hunk"),
                                del(14, "  - Search for symbols"),
                                add(
                                    14,
                                    "  - Search for symbols with respect to current diff session (not repo wide)",
                                ),
                            ],
                        },
                        DiffHunk {
                            header: "@@ -28,6 +28,10 @@",
                            lines: vec![
                                ctx(
                                    28,
                                    28,
                                    "- Poll changes to jsonl file and update the TUI's rendered state if needed.",
                                ),
                                add(29, ""),
                                add(30, "## Development Plan"),
                                add(31, ""),
                                add(
                                    32,
                                    "1. Set up the tui infra. support side-by-side, inline views with file explorer sidebar",
                                ),
                            ],
                        },
                    ],
                },
            ],
        }
    }
}

impl DiffFile {
    pub fn change_kind(&self) -> FileChangeKind {
        let mut saw_non_added = false;

        for hunk in &self.hunks {
            for line in &hunk.lines {
                match line {
                    DiffLine::Added { .. } => {}
                    DiffLine::Context { .. } | DiffLine::Removed { .. } => {
                        saw_non_added = true;
                    }
                }
            }
        }

        if saw_non_added {
            FileChangeKind::Modified
        } else {
            FileChangeKind::Added
        }
    }
}

fn ctx(old_lineno: usize, new_lineno: usize, text: &'static str) -> DiffLine {
    DiffLine::Context {
        old_lineno,
        new_lineno,
        text,
    }
}

fn add(new_lineno: usize, text: &'static str) -> DiffLine {
    DiffLine::Added { new_lineno, text }
}

fn del(old_lineno: usize, text: &'static str) -> DiffLine {
    DiffLine::Removed { old_lineno, text }
}
