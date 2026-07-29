//! Git diff loading and the shared diff model.
//!
//! `DiffSession::load_from_repo` supports working tree, staged, two-revision and
//! merge-base comparisons. It converts `git2` patches into the files, hunks and
//! lines used by the rest of the application. `DiffFilter` follows Git's
//! diff-filter convention: uppercase letters include statuses and lowercase
//! letters exclude them.
//!
//! `DiffStatsLoader` calculates only file and line totals. It caches
//! worktree and index scans and reuses results for revision comparisons that
//! resolve to the same pair of trees.
//!
//! Working tree comparisons include untracked files. If `git2` does not provide
//! a patch for a readable added file, this module creates an all-added hunk from
//! its contents.

use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
};

use git2::{Delta, Diff as GitDiff, DiffOptions, Oid, Patch, Repository, Tree};

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiffStats {
    pub files: usize,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiffStatsBreakdown {
    all: DiffStats,
    by_status: HashMap<char, DiffStats>,
}

pub struct DiffStatsLoader<'repo> {
    repo: &'repo Repository,
    workdir: Option<PathBuf>,
    cache: HashMap<DiffStatsKey, DiffStatsBreakdown>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum DiffStatsKey {
    Worktree,
    Cached,
    Trees { old: Oid, new: Oid },
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

impl DiffStatsBreakdown {
    pub fn stats(&self, diff_filter: Option<&DiffFilter>) -> DiffStats {
        let Some(diff_filter) = diff_filter else {
            return self.all;
        };

        self.by_status
            .iter()
            .filter(|(letter, _)| diff_filter.matches_letter(**letter))
            .fold(DiffStats::default(), |mut total, (_, summary)| {
                total.add(*summary);
                total
            })
    }
}

impl<'repo> DiffStatsLoader<'repo> {
    pub fn new(repo: &'repo Repository) -> Self {
        Self {
            repo,
            workdir: repo.workdir().map(Path::to_path_buf),
            cache: HashMap::new(),
        }
    }

    pub fn load(&mut self, target: &DiffTarget) -> Result<DiffStatsBreakdown, git2::Error> {
        let key = diff_stats_key(self.repo, target)?;
        if let Some(stats) = self.cache.get(&key) {
            return Ok(stats.clone());
        }

        let diff = match key {
            DiffStatsKey::Worktree | DiffStatsKey::Cached => load_git_diff(self.repo, target)?,
            DiffStatsKey::Trees { old, new } => {
                let old_tree = self.repo.find_tree(old)?;
                let new_tree = self.repo.find_tree(new)?;
                tree_to_tree_diff(self.repo, &old_tree, &new_tree)?
            }
        };
        let stats = collect_diff_stats(
            &diff,
            self.workdir.as_deref(),
            matches!(target, DiffTarget::Worktree),
        );
        self.cache.insert(key, stats.clone());
        Ok(stats)
    }
}

impl DiffStats {
    fn add(&mut self, other: Self) {
        self.files += other.files;
        self.additions += other.additions;
        self.deletions += other.deletions;
    }
}

pub fn discover_repo_root(path: impl AsRef<Path>) -> Result<PathBuf, git2::Error> {
    let repo = Repository::discover(path)?;
    Ok(repo
        .workdir()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo.path().to_path_buf()))
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

    fn matches_letter(&self, letter: char) -> bool {
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

fn diff_stats_key(repo: &Repository, target: &DiffTarget) -> Result<DiffStatsKey, git2::Error> {
    match target {
        DiffTarget::Worktree => Ok(DiffStatsKey::Worktree),
        DiffTarget::Cached => Ok(DiffStatsKey::Cached),
        DiffTarget::Range { base, head } => Ok(DiffStatsKey::Trees {
            old: peel_revision_tree(repo, base)?.id(),
            new: peel_revision_tree(repo, head)?.id(),
        }),
        DiffTarget::MergeBaseRange { base, head } => {
            let merge_base = repo.merge_base(
                revision_commit_id(repo, base)?,
                revision_commit_id(repo, head)?,
            )?;
            Ok(DiffStatsKey::Trees {
                old: repo.find_commit(merge_base)?.tree_id(),
                new: peel_revision_tree(repo, head)?.id(),
            })
        }
    }
}

fn collect_diff_stats(
    diff: &GitDiff<'_>,
    workdir: Option<&Path>,
    allow_worktree_fallback: bool,
) -> DiffStatsBreakdown {
    let mut stats = DiffStatsBreakdown::default();

    for (index, delta) in diff.deltas().enumerate() {
        let Some(patch) = Patch::from_diff(diff, index).ok().flatten() else {
            continue;
        };

        let (_, mut additions, deletions) = patch.line_stats().unwrap_or_default();
        if allow_worktree_fallback
            && patch.num_hunks() == 0
            && let Some(lines) = synthetic_added_file_line_count(workdir, &delta)
        {
            additions = lines;
        }

        let summary = DiffStats {
            files: 1,
            additions,
            deletions,
        };
        stats.all.add(summary);
        stats
            .by_status
            .entry(delta_filter_letter(delta.status()))
            .or_default()
            .add(summary);
    }

    stats
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

        if allow_worktree_fallback
            && hunks.is_empty()
            && let Some(hunk) = synthetic_added_file_hunk(workdir, &delta, &new_path)
        {
            hunks.push(hunk);
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

fn synthetic_added_file_line_count(
    workdir: Option<&Path>,
    delta: &git2::DiffDelta<'_>,
) -> Option<usize> {
    if !matches!(delta.status(), Delta::Added | Delta::Untracked) {
        return None;
    }

    let absolute_path = workdir?.join(delta.new_file().path()?);
    count_utf8_lines(BufReader::new(File::open(absolute_path).ok()?)).ok()
}

fn count_utf8_lines(mut reader: impl BufRead) -> io::Result<usize> {
    let mut count = 0;
    let mut line = String::new();

    loop {
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(count);
        }
        count += 1;
        line.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Cursor,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use git2::{IndexAddOption, Repository, Signature};

    use super::*;

    static NEXT_TEST_REPO: AtomicU64 = AtomicU64::new(0);

    struct TestRepo {
        path: PathBuf,
    }

    impl TestRepo {
        fn new() -> Self {
            let id = NEXT_TEST_REPO.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("enza-diff-summary-{}-{id}", process::id()));
            fs::create_dir(&path).expect("create test repository directory");

            let repo = Repository::init(&path).expect("initialize test repository");
            fs::write(path.join("modified.txt"), "before\n").expect("write modified fixture");
            fs::write(path.join("deleted.txt"), "deleted\n").expect("write deleted fixture");
            commit_all(&repo, "initial");
            drop(repo);

            Self { path }
        }

        fn open(&self) -> Repository {
            Repository::open(&self.path).expect("open test repository")
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).expect("remove test repository");
        }
    }

    fn commit_all(repo: &Repository, message: &str) -> Oid {
        let mut index = repo.index().expect("open index");
        index
            .add_all(["*"], IndexAddOption::DEFAULT, None)
            .expect("add fixture files");
        index.write().expect("write index");

        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let signature = Signature::now("Enza tests", "enza@example.com").expect("signature");
        let commit_id = repo
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                message,
                &tree,
                &[],
            )
            .expect("create initial commit");
        repo.set_head("refs/heads/main").expect("set HEAD");
        commit_id
    }

    fn stats_for_session(session: &DiffSession) -> DiffStats {
        let mut summary = DiffStats {
            files: session.files.len(),
            ..DiffStats::default()
        };
        for file in &session.files {
            let (additions, deletions) = file.change_counts();
            summary.additions += additions;
            summary.deletions += deletions;
        }
        summary
    }

    #[test]
    fn deleted_files_are_classified_separately_from_modified_files() {
        let file = DiffFile {
            path: "src/removed.rs".to_string(),
            old_path: "src/removed.rs".to_string(),
            new_path: "/dev/null".to_string(),
            hunks: vec![DiffHunk {
                header: "@@ -1 +0,0 @@".to_string(),
                lines: vec![DiffLine::Removed {
                    old_lineno: 1,
                    text: "removed".to_string(),
                }],
            }],
        };

        assert_eq!(file.change_kind(), FileChangeKind::Deleted);
    }

    #[test]
    fn stats_line_count_matches_diff_line_splitting() {
        assert_eq!(count_utf8_lines(Cursor::new(b"")).unwrap(), 0);
        assert_eq!(count_utf8_lines(Cursor::new(b"one")).unwrap(), 1);
        assert_eq!(count_utf8_lines(Cursor::new(b"one\n")).unwrap(), 1);
        assert_eq!(count_utf8_lines(Cursor::new(b"one\n\n")).unwrap(), 2);
        assert!(count_utf8_lines(Cursor::new(vec![0xff, b'\n'])).is_err());
    }

    #[test]
    fn worktree_stats_match_full_diff_session() {
        let fixture = TestRepo::new();
        fs::write(fixture.path().join("modified.txt"), "before\nafter\n").expect("modify fixture");
        fs::remove_file(fixture.path().join("deleted.txt")).expect("delete fixture");
        fs::write(fixture.path().join("untracked.txt"), "first\nsecond\n")
            .expect("write untracked fixture");

        let repo = fixture.open();
        let mut loader = DiffStatsLoader::new(&repo);
        let stats = loader
            .load(&DiffTarget::Worktree)
            .expect("load worktree stats");

        let full_session = DiffSession::load_from_repo(fixture.path(), &DiffTarget::Worktree, None)
            .expect("load full worktree diff");
        assert_eq!(stats.stats(None), stats_for_session(&full_session));

        for filter in ["M", "A", "D"].map(|value| DiffFilter::parse(value).unwrap()) {
            let filtered_session =
                DiffSession::load_from_repo(fixture.path(), &DiffTarget::Worktree, Some(&filter))
                    .expect("load filtered worktree diff");
            assert_eq!(
                stats.stats(Some(&filter)),
                stats_for_session(&filtered_session)
            );
        }

        assert_eq!(
            stats.stats(None),
            DiffStats {
                files: 3,
                additions: 3,
                deletions: 1,
            }
        );
    }

    #[test]
    fn stats_loader_reuses_worktree_and_equivalent_tree_diffs() {
        let fixture = TestRepo::new();
        let repo = fixture.open();
        let mut loader = DiffStatsLoader::new(&repo);

        loader
            .load(&DiffTarget::Worktree)
            .expect("load worktree summary");
        loader
            .load(&DiffTarget::Worktree)
            .expect("reuse worktree summary");
        assert_eq!(loader.cache.len(), 1);

        let head_to_head = DiffTarget::Range {
            base: "HEAD".to_string(),
            head: "HEAD".to_string(),
        };
        let main_to_head = DiffTarget::Range {
            base: "main".to_string(),
            head: "HEAD".to_string(),
        };
        loader
            .load(&head_to_head)
            .expect("load first equivalent tree summary");
        loader
            .load(&main_to_head)
            .expect("reuse equivalent tree summary");
        assert_eq!(loader.cache.len(), 2);
    }
}
