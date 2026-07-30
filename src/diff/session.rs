use std::path::Path;

use git2::{Diff as GitDiff, DiffDelta, Patch, Repository};

use super::{
    DiffFile, DiffFilter, DiffHunk, DiffLine, DiffSession, DiffTarget,
    target::ResolvedDiff,
    walk::{SyntheticRead, for_each_patch, read_synthetic_added_file},
};

impl DiffSession {
    pub fn load_from_repo(
        path: impl AsRef<Path>,
        target: &DiffTarget,
        diff_filter: Option<&DiffFilter>,
    ) -> Result<Self, git2::Error> {
        Self::load_from_repo_cancellable(path, target, diff_filter, || false).map(|session| {
            session.expect("a session load with a false cancellation callback must complete")
        })
    }

    /// If the caller cancels the load, `load_from_repo_cancellable` returns `None`.
    ///
    /// `load_from_repo_cancellable` discards an incomplete session.
    pub fn load_from_repo_cancellable(
        path: impl AsRef<Path>,
        target: &DiffTarget,
        diff_filter: Option<&DiffFilter>,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, git2::Error> {
        if is_cancelled() {
            return Ok(None);
        }
        let repo = Repository::discover(path)?;
        if is_cancelled() {
            return Ok(None);
        }
        let workdir = repo.workdir().map(Path::to_path_buf);
        let resolved = ResolvedDiff::resolve(&repo, target)?;
        if is_cancelled() {
            return Ok(None);
        }
        let diff = resolved.load(&repo)?;
        if is_cancelled() {
            return Ok(None);
        }
        let Some(files) = collect_diff_files(
            &diff,
            workdir.as_deref(),
            resolved.allows_worktree_fallback(),
            diff_filter,
            &mut is_cancelled,
        ) else {
            return Ok(None);
        };

        Ok(Some(Self { files }))
    }
}

fn collect_diff_files(
    diff: &GitDiff<'_>,
    workdir: Option<&Path>,
    allow_worktree_fallback: bool,
    diff_filter: Option<&DiffFilter>,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Option<Vec<DiffFile>> {
    let mut files = Vec::new();
    let complete = for_each_patch(
        diff,
        |status| diff_filter.is_none_or(|filter| filter.matches(status)),
        is_cancelled,
        |delta, patch, is_cancelled| {
            let Some(file) =
                build_diff_file(delta, patch, workdir, allow_worktree_fallback, is_cancelled)
            else {
                return false;
            };
            files.push(file);
            true
        },
    );

    complete.then_some(files)
}

fn build_diff_file(
    delta: &DiffDelta<'_>,
    patch: &Patch<'_>,
    workdir: Option<&Path>,
    allow_worktree_fallback: bool,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Option<DiffFile> {
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
    let mut file = DiffFile {
        path,
        old_path,
        new_path,
        hunks: Vec::new(),
    };

    for hunk_index in 0..patch.num_hunks() {
        if is_cancelled() {
            return None;
        }
        let Ok((hunk, line_count)) = patch.hunk(hunk_index) else {
            continue;
        };
        let mut lines = Vec::new();

        for line_index in 0..line_count {
            if is_cancelled() {
                return None;
            }
            let Ok(line) = patch.line_in_hunk(hunk_index, line_index) else {
                continue;
            };

            match line.origin() {
                ' ' => lines.push(DiffLine::Context {
                    old_lineno: line.old_lineno().unwrap_or(0) as usize,
                    new_lineno: line.new_lineno().unwrap_or(0) as usize,
                    text: diff_line_text(line.content()),
                }),
                '+' => lines.push(DiffLine::Added {
                    new_lineno: line.new_lineno().unwrap_or(0) as usize,
                    text: diff_line_text(line.content()),
                }),
                '-' => lines.push(DiffLine::Removed {
                    old_lineno: line.old_lineno().unwrap_or(0) as usize,
                    text: diff_line_text(line.content()),
                }),
                _ => {}
            }
        }

        file.hunks.push(DiffHunk {
            header: String::from_utf8_lossy(hunk.header())
                .trim_end()
                .to_string(),
            lines,
        });
    }

    if allow_worktree_fallback && file.hunks.is_empty() {
        match read_synthetic_added_file(
            workdir,
            delta,
            Vec::new(),
            |lines, new_lineno, text| {
                lines.push(DiffLine::Added {
                    new_lineno,
                    text: diff_line_text(text),
                });
            },
            is_cancelled,
        ) {
            SyntheticRead::Complete(lines) if !lines.is_empty() => {
                file.hunks.push(DiffHunk {
                    header: format!("@@ -0,0 +1,{} @@", lines.len()),
                    lines,
                });
            }
            SyntheticRead::Cancelled => return None,
            SyntheticRead::Unavailable | SyntheticRead::Complete(_) => {}
        }
    }

    Some(file)
}

fn diff_line_text(text: &[u8]) -> String {
    String::from_utf8_lossy(text)
        .trim_end_matches('\n')
        .to_string()
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
