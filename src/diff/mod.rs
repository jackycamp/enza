use std::{fs, path::Path};

use git2::{Delta, DiffOptions, Patch, Repository};

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
    pub fn load_from_repo(path: impl AsRef<Path>) -> Result<Self, git2::Error> {
        let repo = Repository::discover(path)?;
        let workdir = repo.workdir().map(|path| path.to_path_buf());
        let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());

        let mut opts = DiffOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_typechange(true)
            .include_unmodified(false)
            .ignore_submodules(true);

        let diff = repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))?;

        let mut files = Vec::new();

        for (index, delta) in diff.deltas().enumerate() {
            let Some(patch) = Patch::from_diff(&diff, index)? else {
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
                let (hunk, line_count) = patch.hunk(hunk_index)?;
                let header = String::from_utf8_lossy(hunk.header())
                    .trim_end()
                    .to_string();
                let mut lines = Vec::new();

                for line_index in 0..line_count {
                    let line = patch.line_in_hunk(hunk_index, line_index)?;
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

            if hunks.is_empty() {
                if let Some(hunk) = synthetic_added_file_hunk(workdir.as_deref(), &delta, &new_path)
                {
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

        Ok(Self { files })
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
