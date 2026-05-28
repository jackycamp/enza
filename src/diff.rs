use std::{fs, path::Path};

use git2::{Delta, Diff as GitDiff, DiffOptions, Patch, Repository, Tree};

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
    pub fn load_from_repo(
        path: impl AsRef<Path>,
        target: &DiffTarget,
        diff_filter: Option<&DiffFilter>,
    ) -> Result<Self, git2::Error> {
        let repo = Repository::discover(path)?;
        let workdir = repo.workdir().map(|path| path.to_path_buf());

        let diff = load_git_diff(&repo, target)?;
        let files = collect_diff_files(
            &diff,
            workdir.as_deref(),
            matches!(target, DiffTarget::Worktree),
            diff_filter,
        );

        Ok(Self { files })
    }

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
        let letter = delta_filter_letter(delta);

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

fn load_git_diff<'repo>(
    repo: &'repo Repository,
    target: &DiffTarget,
) -> Result<GitDiff<'repo>, git2::Error> {
    match target {
        DiffTarget::Worktree => {
            let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
            let mut opts = DiffOptions::new();
            opts.include_untracked(true)
                .recurse_untracked_dirs(true)
                .include_typechange(true)
                .include_unmodified(false)
                .ignore_submodules(true);
            repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        }
        DiffTarget::Cached => {
            let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
            let index = repo.index()?;
            let mut opts = DiffOptions::new();
            opts.include_typechange(true)
                .include_unmodified(false)
                .ignore_submodules(true);
            repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut opts))
        }
        DiffTarget::Range { base, head } => {
            let base_tree = peel_revision_tree(repo, base)?;
            let head_tree = peel_revision_tree(repo, head)?;
            tree_to_tree_diff(repo, &base_tree, &head_tree)
        }
        DiffTarget::MergeBaseRange { base, head } => {
            let merge_base = repo.merge_base(
                revision_commit_id(repo, base)?,
                revision_commit_id(repo, head)?,
            )?;
            let merge_base_commit = repo.find_commit(merge_base)?;
            let merge_base_tree = merge_base_commit.tree()?;
            let head_tree = peel_revision_tree(repo, head)?;
            tree_to_tree_diff(repo, &merge_base_tree, &head_tree)
        }
    }
}

fn collect_diff_files(
    diff: &GitDiff<'_>,
    workdir: Option<&Path>,
    allow_worktree_fallback: bool,
    diff_filter: Option<&DiffFilter>,
) -> Vec<DiffFile> {
    let mut files = Vec::new();

    for (index, delta) in diff.deltas().enumerate() {
        if diff_filter.is_some_and(|filter| !filter.matches(delta.status())) {
            continue;
        }

        let Some(patch) = Patch::from_diff(diff, index).ok().flatten() else {
            continue;
        };

        let old_path = delta
            .old_file()
            .path()
            .map(path_to_string)
            .unwrap_or_else(|| "/dev/null".to_string());
        let new_path = delta
            .new_file()
            .path()
            .map(path_to_string)
            .unwrap_or_else(|| "/dev/null".to_string());
        let path = if new_path != "/dev/null" {
            new_path.clone()
        } else {
            old_path.clone()
        };

        let mut hunks = Vec::new();
        let hunk_count = patch.num_hunks();

        for hunk_index in 0..hunk_count {
            let Ok((hunk, line_count)) = patch.hunk(hunk_index) else {
                continue;
            };
            let header = String::from_utf8_lossy(hunk.header())
                .trim_end()
                .to_string();
            let mut lines = Vec::new();

            for line_index in 0..line_count {
                let Ok(line) = patch.line_in_hunk(hunk_index, line_index) else {
                    continue;
                };
                let text = String::from_utf8_lossy(line.content())
                    .trim_end_matches('\n')
                    .to_string();

                match line.origin() {
                    ' ' => lines.push(DiffLine::Context {
                        old_lineno: line.old_lineno().unwrap_or(0) as usize,
                        new_lineno: line.new_lineno().unwrap_or(0) as usize,
                        text,
                    }),
                    '+' => lines.push(DiffLine::Added {
                        new_lineno: line.new_lineno().unwrap_or(0) as usize,
                        text,
                    }),
                    '-' => lines.push(DiffLine::Removed {
                        old_lineno: line.old_lineno().unwrap_or(0) as usize,
                        text,
                    }),
                    _ => {}
                }
            }

            hunks.push(DiffHunk { header, lines });
        }

        if allow_worktree_fallback && hunks.is_empty() {
            if let Some(hunk) = synthetic_added_file_hunk(workdir, &delta, &new_path) {
                hunks.push(hunk);
            }
        }

        files.push(DiffFile {
            path,
            old_path,
            new_path,
            hunks,
        });
    }

    files
}

fn tree_to_tree_diff<'repo>(
    repo: &'repo Repository,
    old_tree: &Tree<'repo>,
    new_tree: &Tree<'repo>,
) -> Result<GitDiff<'repo>, git2::Error> {
    let mut opts = DiffOptions::new();
    opts.include_typechange(true)
        .include_unmodified(false)
        .ignore_submodules(true);
    repo.diff_tree_to_tree(Some(old_tree), Some(new_tree), Some(&mut opts))
}

fn peel_revision_tree<'repo>(
    repo: &'repo Repository,
    revision: &str,
) -> Result<Tree<'repo>, git2::Error> {
    repo.revparse_single(revision)?.peel_to_tree()
}

fn revision_commit_id(repo: &Repository, revision: &str) -> Result<git2::Oid, git2::Error> {
    Ok(repo.revparse_single(revision)?.peel_to_commit()?.id())
}

fn is_supported_filter_char(ch: char) -> bool {
    matches!(
        ch.to_ascii_uppercase(),
        'A' | 'C' | 'D' | 'M' | 'R' | 'T' | 'U' | 'X' | 'B'
    )
}

fn delta_filter_letter(delta: Delta) -> char {
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

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn synthetic_added_file_hunk(
    workdir: Option<&Path>,
    delta: &git2::DiffDelta<'_>,
    new_path: &str,
) -> Option<DiffHunk> {
    if !matches!(delta.status(), Delta::Added | Delta::Untracked) {
        return None;
    }

    let absolute_path = workdir?.join(new_path);
    let contents = fs::read_to_string(absolute_path).ok()?;
    let mut lines = Vec::new();

    for (index, line) in contents.lines().enumerate() {
        lines.push(DiffLine::Added {
            new_lineno: index + 1,
            text: line.to_string(),
        });
    }

    if lines.is_empty() {
        return None;
    }

    Some(DiffHunk {
        header: format!("@@ -0,0 +1,{} @@", lines.len()),
        lines,
    })
}
