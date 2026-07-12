use std::collections::HashSet;

use crate::diff::{DiffFile, FileChangeKind};

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
pub struct SidebarState {
    pub open: bool,
    pub cursor: usize,
    pub collapsed_dirs: Vec<String>,
}

impl SidebarState {
    pub fn cursor_down(&mut self, files: &[DiffFile]) {
        let max_index = self.visible_entries(files).len().saturating_sub(1);
        self.cursor = (self.cursor + 1).min(max_index);
    }

    pub fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn collapse_directory(&mut self, files: &[DiffFile]) {
        let Some(SidebarEntry {
            kind: SidebarEntryKind::Directory { path, collapsed },
            ..
        }) = self.current_entry(files)
        else {
            return;
        };

        if !collapsed {
            self.collapsed_dirs.push(path);
            self.clamp_cursor(files);
        }
    }

    pub fn expand_directory(&mut self, files: &[DiffFile]) {
        let Some(SidebarEntry {
            kind: SidebarEntryKind::Directory { path, collapsed },
            ..
        }) = self.current_entry(files)
        else {
            return;
        };

        if collapsed {
            self.collapsed_dirs.retain(|candidate| candidate != &path);
        }
    }

    pub fn activate_cursor(&mut self, files: &[DiffFile]) -> Option<usize> {
        let entry = self.current_entry(files)?;

        match entry.kind {
            SidebarEntryKind::File { file_index } => {
                Some(file_index.min(files.len().saturating_sub(1)))
            }
            SidebarEntryKind::Directory { path, collapsed } => {
                if collapsed {
                    self.collapsed_dirs.retain(|candidate| candidate != &path);
                } else {
                    self.collapsed_dirs.push(path);
                }
                self.clamp_cursor(files);
                None
            }
        }
    }

    pub fn sync_cursor_to_file(&mut self, files: &[DiffFile], selected_file: usize) {
        if let Some(index) = self
            .visible_entries(files)
            .iter()
            .position(|entry| matches!(entry.kind, SidebarEntryKind::File { file_index } if file_index == selected_file))
        {
            self.cursor = index;
        }
    }

    pub fn visible_entries(&self, files: &[DiffFile]) -> Vec<SidebarEntry> {
        let mut entries = Vec::new();
        let mut inserted_dirs = HashSet::new();

        for (file_index, file) in files.iter().enumerate() {
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
                    "{}{} {} ",
                    "  ".repeat(parts.len().saturating_sub(1)),
                    file_name,
                    change_label
                ),
                kind: SidebarEntryKind::File { file_index },
            });
        }

        entries
    }

    fn current_entry(&self, files: &[DiffFile]) -> Option<SidebarEntry> {
        self.visible_entries(files).get(self.cursor).cloned()
    }

    fn clamp_cursor(&mut self, files: &[DiffFile]) {
        let max_index = self.visible_entries(files).len().saturating_sub(1);
        self.cursor = self.cursor.min(max_index);
    }
}
