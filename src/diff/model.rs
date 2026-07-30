use std::collections::HashMap;

use git2::Delta;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiffTarget {
    Worktree,
    Cached,
    Range { base: String, head: String },
    MergeBaseRange { base: String, head: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffFilter {
    include: Vec<char>,
    exclude: Vec<char>,
}

/// `DiffStats` contains file, addition, and deletion counts.
///
/// This type does not contain paths, hunks, or line text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiffStats {
    pub files: usize,
    pub additions: usize,
    pub deletions: usize,
}

/// `DiffStatsBreakdown` stores the total counts and the counts for each Git status.
///
/// `DiffFilter` can use these counts without another diff scan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiffStatsBreakdown {
    pub(super) all: DiffStats,
    pub(super) by_status: HashMap<char, DiffStats>,
}

#[derive(Clone, Debug, Default)]
pub struct DiffSession {
    pub files: Vec<DiffFile>,
}

#[derive(Clone, Debug)]
pub struct DiffFile {
    pub path: String,
    pub old_path: String,
    pub new_path: String,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileChangeKind {
    Added,
    Deleted,
    Modified,
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub enum DiffLine {
    Context {
        old_lineno: usize,
        new_lineno: usize,
        text: String,
    },
    Added {
        new_lineno: usize,
        text: String,
    },
    Removed {
        old_lineno: usize,
        text: String,
    },
}

impl DiffSession {
    pub fn num_files(&self) -> usize {
        self.files.len()
    }

    pub fn num_hunks(&self) -> usize {
        self.files
            .iter()
            .map(|file| file.hunks.len())
            .sum::<usize>()
    }

    pub fn num_lines(&self) -> usize {
        self.files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .map(|hunk| hunk.lines.len())
            .sum::<usize>()
    }
}

impl DiffFilter {
    pub fn parse(value: &str) -> Option<Self> {
        let mut include = Vec::new();
        let mut exclude = Vec::new();

        for ch in value.chars() {
            if !is_supported_filter_char(ch) {
                return None;
            }

            if ch.is_ascii_uppercase() {
                if !include.contains(&ch) {
                    include.push(ch);
                }
            } else {
                let upper = ch.to_ascii_uppercase();
                if !exclude.contains(&upper) {
                    exclude.push(upper);
                }
            }
        }

        Some(Self { include, exclude })
    }

    pub fn matches(&self, delta: Delta) -> bool {
        self.matches_letter(delta_filter_letter(delta))
    }

    pub(super) fn matches_letter(&self, letter: char) -> bool {
        if self.exclude.contains(&letter) {
            return false;
        }

        if self.include.is_empty() {
            return true;
        }

        self.include.contains(&letter)
    }
}

impl DiffFile {
    pub fn change_kind(&self) -> FileChangeKind {
        if self.new_path == "/dev/null" {
            return FileChangeKind::Deleted;
        }

        if self.old_path == "/dev/null" {
            return FileChangeKind::Added;
        }

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

    pub fn change_counts(&self) -> (usize, usize) {
        let mut additions = 0usize;
        let mut deletions = 0usize;

        for hunk in &self.hunks {
            for line in &hunk.lines {
                match line {
                    DiffLine::Added { .. } => additions += 1,
                    DiffLine::Removed { .. } => deletions += 1,
                    DiffLine::Context { .. } => {}
                }
            }
        }

        (additions, deletions)
    }
}

impl DiffStatsBreakdown {
    /// If no filter exists, `stats` returns all counts.
    ///
    /// If a filter exists, `stats` returns counts for the Git statuses that the filter accepts.
    pub fn stats(&self, diff_filter: Option<&DiffFilter>) -> DiffStats {
        let Some(diff_filter) = diff_filter else {
            return self.all;
        };

        self.by_status
            .iter()
            .filter(|(letter, _)| diff_filter.matches_letter(**letter))
            .fold(DiffStats::default(), |mut total, (_, stats)| {
                total.add(*stats);
                total
            })
    }
}

impl DiffStats {
    pub(super) fn add(&mut self, other: Self) {
        self.files += other.files;
        self.additions += other.additions;
        self.deletions += other.deletions;
    }
}

pub(super) fn delta_filter_letter(delta: Delta) -> char {
    match delta {
        Delta::Added | Delta::Untracked => 'A',
        Delta::Copied => 'C',
        Delta::Deleted => 'D',
        Delta::Modified => 'M',
        Delta::Renamed => 'R',
        Delta::Typechange => 'T',
        Delta::Conflicted => 'U',
        Delta::Unreadable => 'X',
        _ => 'B',
    }
}

fn is_supported_filter_char(ch: char) -> bool {
    matches!(
        ch.to_ascii_uppercase(),
        'A' | 'C' | 'D' | 'M' | 'R' | 'T' | 'U' | 'X' | 'B'
    )
}
