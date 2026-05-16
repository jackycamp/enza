use std::collections::HashSet;

use crate::diff::{DiffFile, DiffSession, FileChangeKind};
use crate::render_cache::RenderSession;

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
    pub mode: DiffMode,
    pub focus: FocusPane,
    pub sidebar_open: bool,
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub sidebar_cursor: usize,
    pub collapsed_dirs: Vec<String>,
    pub scroll: u16,
}

impl App {
    pub fn new(session: DiffSession) -> Self {
        Self {
            running: true,
            session,
            render_session: None,
            mode: DiffMode::SideBySide,
            focus: FocusPane::Main,
            sidebar_open: true,
            selected_file: 0,
            selected_hunk: 0,
            sidebar_cursor: 0,
            collapsed_dirs: Vec::new(),
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

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_add(amount);
    }

    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.session.files.get(self.selected_file)
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
}
