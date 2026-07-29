//! Git diff loading and the shared diff model.
//!
//! `DiffSession::load_from_repo` supports working tree, staged, two-revision and
//! merge-base comparisons. It converts `git2` patches into the files, hunks and
//! lines used by the rest of the application. `DiffFilter` follows Git's
//! diff-filter convention: uppercase letters include statuses and lowercase
//! letters exclude them.
//!
//! `DiffSession` and `DiffStatsLoader` resolve targets and walk patches through
//! the same path. The stats loader keeps only file and line totals. It caches
//! worktree and index scans and reuses revision comparisons that resolve to the
//! same pair of trees.
//!
//! Working tree comparisons include untracked files. If `git2` does not provide
//! text hunks for a readable added file, the shared walker emits an all-added
//! hunk.

use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
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

/// File and line counts for a diff, without its paths, hunks or line text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiffStats {
    pub files: usize,
    pub additions: usize,
    pub deletions: usize,
}

/// Holds total counts and counts by Git status so filters can reuse one scan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiffStatsBreakdown {
    all: DiffStats,
    by_status: HashMap<char, DiffStats>,
}

/// Loads diff counts and reuses each result for the lifetime of the loader.
pub struct DiffStatsLoader<'repo> {
    repo: &'repo Repository,
    workdir: Option<PathBuf>,
    cache: HashMap<ResolvedDiff, DiffStatsBreakdown>,
}

/// A diff target reduced to the repository objects used for its comparison.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ResolvedDiff {
    Worktree { head: Option<Oid> },
    Cached { head: Option<Oid> },
    // Resolved trees let different revision names share the same cached diff.
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

enum WalkedDiffLine<'a> {
    Context {
        old_lineno: usize,
        new_lineno: usize,
        text: &'a [u8],
    },
    Added {
        new_lineno: usize,
        text: &'a [u8],
    },
    Removed {
        old_lineno: usize,
        text: &'a [u8],
    },
}

/// Receives the canonical patch walk and decides which data to retain.
trait DiffCollector {
    type Output;

    fn includes(&self, status: Delta) -> bool;
    fn begin_file(&mut self, delta: &git2::DiffDelta<'_>);
    fn begin_hunk(&mut self, header: &[u8]);
    fn line(&mut self, line: WalkedDiffLine<'_>);
    fn begin_synthetic_hunk(&mut self);
    fn finish_synthetic_hunk(&mut self);
    fn discard_synthetic_hunk(&mut self);
    fn finish_file(&mut self);
    fn finish(self) -> Self::Output;
}

struct DiffSessionCollector<'filter> {
    files: Vec<DiffFile>,
    diff_filter: Option<&'filter DiffFilter>,
}

#[derive(Default)]
struct DiffStatsCollector {
    stats: DiffStatsBreakdown,
    current: Option<(char, DiffStats)>,
    synthetic_additions_start: Option<usize>,
}

impl DiffSession {
    pub fn load_from_repo(
        path: impl AsRef<Path>,
        target: &DiffTarget,
        diff_filter: Option<&DiffFilter>,
    ) -> Result<Self, git2::Error> {
        let repo = Repository::discover(path)?;
        let workdir = repo.workdir().map(|path| path.to_path_buf());

        let resolved = ResolvedDiff::resolve(&repo, target)?;
        let diff = resolved.load(&repo)?;
        let mut never_cancelled = || false;
        let files = walk_diff(
            &diff,
            workdir.as_deref(),
            resolved.allows_worktree_fallback(),
            DiffSessionCollector {
                files: Vec::new(),
                diff_filter,
            },
            &mut never_cancelled,
        )
        .expect("non-cancellable diff walk must complete");

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
    /// Returns all counts, or combines the statuses accepted by the filter.
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

    /// Returns `None` when cancelled and never caches partial counts.
    pub fn load(
        &mut self,
        target: &DiffTarget,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<DiffStatsBreakdown>, git2::Error> {
        if is_cancelled() {
            return Ok(None);
        }

        let resolved = ResolvedDiff::resolve(self.repo, target)?;
        if is_cancelled() {
            return Ok(None);
        }
        if let Some(stats) = self.cache.get(&resolved) {
            return Ok(Some(stats.clone()));
        }

        let diff = resolved.load(self.repo)?;
        if is_cancelled() {
            return Ok(None);
        }
        let Some(stats) = walk_diff(
            &diff,
            self.workdir.as_deref(),
            resolved.allows_worktree_fallback(),
            DiffStatsCollector::default(),
            &mut is_cancelled,
        ) else {
            return Ok(None);
        };
        self.cache.insert(resolved, stats.clone());
        Ok(Some(stats))
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

impl ResolvedDiff {
    /// Resolves revision names and merge bases once.
    ///
    /// Worktree and cached targets capture the current HEAD tree.
    fn resolve(repo: &Repository, target: &DiffTarget) -> Result<Self, git2::Error> {
        match target {
            DiffTarget::Worktree => Ok(Self::Worktree {
                head: head_tree_id(repo),
            }),
            DiffTarget::Cached => Ok(Self::Cached {
                head: head_tree_id(repo),
            }),
            DiffTarget::Range { base, head } => Ok(Self::Trees {
                old: peel_revision_tree(repo, base)?.id(),
                new: peel_revision_tree(repo, head)?.id(),
            }),
            DiffTarget::MergeBaseRange { base, head } => {
                let merge_base = repo.merge_base(
                    revision_commit_id(repo, base)?,
                    revision_commit_id(repo, head)?,
                )?;
                Ok(Self::Trees {
                    old: repo.find_commit(merge_base)?.tree_id(),
                    new: peel_revision_tree(repo, head)?.id(),
                })
            }
        }
    }

    /// Builds the git diff without parsing the original target again.
    fn load<'repo>(&self, repo: &'repo Repository) -> Result<GitDiff<'repo>, git2::Error> {
        match *self {
            Self::Worktree { head } => {
                let head_tree = head.map(|id| repo.find_tree(id)).transpose()?;
                let mut opts = DiffOptions::new();
                opts.include_untracked(true)
                    .recurse_untracked_dirs(true)
                    .include_typechange(true)
                    .include_unmodified(false)
                    .ignore_submodules(true);
                repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
            }
            Self::Cached { head } => {
                let head_tree = head.map(|id| repo.find_tree(id)).transpose()?;
                let index = repo.index()?;
                let mut opts = DiffOptions::new();
                opts.include_typechange(true)
                    .include_unmodified(false)
                    .ignore_submodules(true);
                repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut opts))
            }
            Self::Trees { old, new } => {
                let old_tree = repo.find_tree(old)?;
                let new_tree = repo.find_tree(new)?;
                tree_to_tree_diff(repo, &old_tree, &new_tree)
            }
        }
    }

    fn allows_worktree_fallback(self) -> bool {
        matches!(self, Self::Worktree { .. })
    }
}

fn head_tree_id(repo: &Repository) -> Option<Oid> {
    repo.head()
        .ok()
        .and_then(|head| head.peel_to_tree().ok())
        .map(|tree| tree.id())
}

fn walk_diff<C: DiffCollector>(
    diff: &GitDiff<'_>,
    workdir: Option<&Path>,
    allow_worktree_fallback: bool,
    mut collector: C,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Option<C::Output> {
    if is_cancelled() {
        return None;
    }

    for (index, delta) in diff.deltas().enumerate() {
        if is_cancelled() {
            return None;
        }
        if !collector.includes(delta.status()) {
            continue;
        }

        let Some(patch) = Patch::from_diff(diff, index).ok().flatten() else {
            continue;
        };

        collector.begin_file(&delta);
        let mut saw_hunk = false;

        for hunk_index in 0..patch.num_hunks() {
            if is_cancelled() {
                return None;
            }
            let Ok((hunk, line_count)) = patch.hunk(hunk_index) else {
                continue;
            };
            saw_hunk = true;
            collector.begin_hunk(hunk.header());

            for line_index in 0..line_count {
                if is_cancelled() {
                    return None;
                }
                let Ok(line) = patch.line_in_hunk(hunk_index, line_index) else {
                    continue;
                };

                let walked_line = match line.origin() {
                    ' ' => WalkedDiffLine::Context {
                        old_lineno: line.old_lineno().unwrap_or(0) as usize,
                        new_lineno: line.new_lineno().unwrap_or(0) as usize,
                        text: line.content(),
                    },
                    '+' => WalkedDiffLine::Added {
                        new_lineno: line.new_lineno().unwrap_or(0) as usize,
                        text: line.content(),
                    },
                    '-' => WalkedDiffLine::Removed {
                        old_lineno: line.old_lineno().unwrap_or(0) as usize,
                        text: line.content(),
                    },
                    _ => continue,
                };
                collector.line(walked_line);
            }
        }

        if allow_worktree_fallback
            && !saw_hunk
            && !walk_synthetic_added_file(workdir, &delta, &mut collector, is_cancelled)
        {
            return None;
        }

        collector.finish_file();
    }

    if is_cancelled() {
        None
    } else {
        Some(collector.finish())
    }
}

fn walk_synthetic_added_file<C: DiffCollector>(
    workdir: Option<&Path>,
    delta: &git2::DiffDelta<'_>,
    collector: &mut C,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> bool {
    if is_cancelled() {
        return false;
    }
    if !matches!(delta.status(), Delta::Added | Delta::Untracked) {
        return true;
    }

    let Some(absolute_path) = workdir
        .zip(delta.new_file().path())
        .map(|(workdir, path)| workdir.join(path))
    else {
        return true;
    };
    let Ok(file) = File::open(absolute_path) else {
        return true;
    };

    collector.begin_synthetic_hunk();
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut new_lineno = 0;

    loop {
        if is_cancelled() {
            collector.discard_synthetic_hunk();
            return false;
        }
        match reader.read_line(&mut line) {
            Ok(0) => {
                collector.finish_synthetic_hunk();
                return true;
            }
            Ok(_) => {
                new_lineno += 1;
                let text = line
                    .strip_suffix('\n')
                    .map(|text| text.strip_suffix('\r').unwrap_or(text))
                    .unwrap_or(&line);
                collector.line(WalkedDiffLine::Added {
                    new_lineno,
                    text: text.as_bytes(),
                });
                line.clear();
            }
            Err(_) => {
                collector.discard_synthetic_hunk();
                return true;
            }
        }
    }
}

impl DiffCollector for DiffSessionCollector<'_> {
    type Output = Vec<DiffFile>;

    fn includes(&self, status: Delta) -> bool {
        self.diff_filter.is_none_or(|filter| filter.matches(status))
    }

    fn begin_file(&mut self, delta: &git2::DiffDelta<'_>) {
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

        self.files.push(DiffFile {
            path,
            old_path,
            new_path,
            hunks: Vec::new(),
        });
    }

    fn begin_hunk(&mut self, header: &[u8]) {
        let header = String::from_utf8_lossy(header).trim_end().to_string();
        self.files
            .last_mut()
            .expect("diff file must exist before its hunks")
            .hunks
            .push(DiffHunk {
                header,
                lines: Vec::new(),
            });
    }

    fn line(&mut self, line: WalkedDiffLine<'_>) {
        let line = match line {
            WalkedDiffLine::Context {
                old_lineno,
                new_lineno,
                text,
            } => DiffLine::Context {
                old_lineno,
                new_lineno,
                text: diff_line_text(text),
            },
            WalkedDiffLine::Added { new_lineno, text } => DiffLine::Added {
                new_lineno,
                text: diff_line_text(text),
            },
            WalkedDiffLine::Removed { old_lineno, text } => DiffLine::Removed {
                old_lineno,
                text: diff_line_text(text),
            },
        };
        self.files
            .last_mut()
            .and_then(|file| file.hunks.last_mut())
            .expect("diff hunk must exist before its lines")
            .lines
            .push(line);
    }

    fn begin_synthetic_hunk(&mut self) {
        self.files
            .last_mut()
            .expect("diff file must exist before its hunks")
            .hunks
            .push(DiffHunk {
                header: String::new(),
                lines: Vec::new(),
            });
    }

    fn finish_synthetic_hunk(&mut self) {
        let file = self
            .files
            .last_mut()
            .expect("diff file must exist before its hunks");
        let Some(hunk) = file.hunks.last_mut() else {
            return;
        };
        if hunk.lines.is_empty() {
            file.hunks.pop();
        } else {
            hunk.header = format!("@@ -0,0 +1,{} @@", hunk.lines.len());
        }
    }

    fn discard_synthetic_hunk(&mut self) {
        self.files
            .last_mut()
            .expect("diff file must exist before its hunks")
            .hunks
            .pop();
    }

    fn finish_file(&mut self) {}

    fn finish(self) -> Self::Output {
        self.files
    }
}

impl DiffCollector for DiffStatsCollector {
    type Output = DiffStatsBreakdown;

    fn includes(&self, _status: Delta) -> bool {
        true
    }

    fn begin_file(&mut self, delta: &git2::DiffDelta<'_>) {
        self.current = Some((
            delta_filter_letter(delta.status()),
            DiffStats {
                files: 1,
                ..DiffStats::default()
            },
        ));
    }

    fn begin_hunk(&mut self, _header: &[u8]) {}

    fn line(&mut self, line: WalkedDiffLine<'_>) {
        let Some((_, stats)) = self.current.as_mut() else {
            return;
        };
        match line {
            WalkedDiffLine::Added { .. } => stats.additions += 1,
            WalkedDiffLine::Removed { .. } => stats.deletions += 1,
            WalkedDiffLine::Context { .. } => {}
        }
    }

    fn begin_synthetic_hunk(&mut self) {
        self.synthetic_additions_start = self.current.map(|(_, stats)| stats.additions);
    }

    fn finish_synthetic_hunk(&mut self) {
        self.synthetic_additions_start = None;
    }

    fn discard_synthetic_hunk(&mut self) {
        if let (Some(start), Some((_, stats))) =
            (self.synthetic_additions_start.take(), self.current.as_mut())
        {
            stats.additions = start;
        }
    }

    fn finish_file(&mut self) {
        let Some((status, stats)) = self.current.take() else {
            return;
        };
        self.stats.all.add(stats);
        self.stats.by_status.entry(status).or_default().add(stats);
    }

    fn finish(self) -> Self::Output {
        self.stats
    }
}

fn diff_line_text(text: &[u8]) -> String {
    String::from_utf8_lossy(text)
        .trim_end_matches('\n')
        .to_string()
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

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use git2::{IndexAddOption, Repository, Signature, build::CheckoutBuilder};

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
            fs::write(path.join("binary.bin"), [0, 1, 0, 2]).expect("write binary fixture");
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
            .update_all(["*"], None)
            .expect("update tracked fixture files");
        index
            .add_all(["*"], IndexAddOption::DEFAULT, None)
            .expect("add fixture files");
        index.write().expect("write index");

        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let signature = Signature::now("Enza tests", "enza@example.com").expect("signature");
        let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        let parents = parent.iter().collect::<Vec<_>>();
        let update_ref = if parent.is_some() {
            "HEAD"
        } else {
            "refs/heads/main"
        };
        let commit_id = repo
            .commit(
                Some(update_ref),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .expect("create commit");
        if parent.is_none() {
            repo.set_head("refs/heads/main").expect("set HEAD");
        }
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

    fn load_stats(
        loader: &mut DiffStatsLoader<'_>,
        target: &DiffTarget,
    ) -> Result<DiffStatsBreakdown, git2::Error> {
        Ok(loader
            .load(target, || false)?
            .expect("non-cancellable test load must complete"))
    }

    fn assert_target_stats_match(fixture: &TestRepo, target: &DiffTarget) -> DiffStatsBreakdown {
        let repo = fixture.open();
        let mut loader = DiffStatsLoader::new(&repo);
        let stats = load_stats(&mut loader, target).expect("load diff stats");

        let full_session =
            DiffSession::load_from_repo(fixture.path(), target, None).expect("load full diff");
        assert_eq!(stats.stats(None), stats_for_session(&full_session));

        for value in ["M", "A", "D", "AD", "m", "a", "d"] {
            let filter = DiffFilter::parse(value).expect("parse test filter");
            let filtered_session =
                DiffSession::load_from_repo(fixture.path(), target, Some(&filter))
                    .expect("load filtered diff");
            assert_eq!(
                stats.stats(Some(&filter)),
                stats_for_session(&filtered_session),
                "stats differ for --diff-filter={value}"
            );
        }

        stats
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
    fn worktree_stats_match_full_diff_session() {
        let fixture = TestRepo::new();
        fs::write(fixture.path().join("modified.txt"), "before\nafter\n").expect("modify fixture");
        fs::remove_file(fixture.path().join("deleted.txt")).expect("delete fixture");
        fs::write(fixture.path().join("untracked.txt"), "first\nsecond\n")
            .expect("write untracked fixture");

        let stats = assert_target_stats_match(&fixture, &DiffTarget::Worktree);

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
    fn cached_stats_match_full_diff_session() {
        let fixture = TestRepo::new();
        fs::write(fixture.path().join("modified.txt"), "before\nstaged\n")
            .expect("modify staged fixture");
        fs::write(fixture.path().join("staged-empty.txt"), "").expect("write empty staged fixture");

        let repo = fixture.open();
        let mut index = repo.index().expect("open index");
        index
            .add_path(Path::new("modified.txt"))
            .expect("stage modified fixture");
        index
            .add_path(Path::new("staged-empty.txt"))
            .expect("stage empty fixture");
        index.write().expect("write staged fixtures");
        drop(index);
        drop(repo);

        let stats = assert_target_stats_match(&fixture, &DiffTarget::Cached);
        assert_eq!(
            stats.stats(None),
            DiffStats {
                files: 2,
                additions: 1,
                deletions: 0,
            }
        );
    }

    #[test]
    fn range_stats_match_full_diff_session() {
        let fixture = TestRepo::new();
        fs::write(fixture.path().join("modified.txt"), "after\n").expect("modify range fixture");
        fs::write(fixture.path().join("range-added.txt"), "first\nsecond\n")
            .expect("add range fixture");

        let repo = fixture.open();
        commit_all(&repo, "second");
        drop(repo);

        let target = DiffTarget::Range {
            base: "main~1".to_string(),
            head: "main".to_string(),
        };
        let stats = assert_target_stats_match(&fixture, &target);
        assert_eq!(
            stats.stats(None),
            DiffStats {
                files: 2,
                additions: 3,
                deletions: 1,
            }
        );
    }

    #[test]
    fn merge_base_stats_match_full_diff_session() {
        let fixture = TestRepo::new();
        let repo = fixture.open();
        let initial = repo
            .head()
            .expect("read initial HEAD")
            .peel_to_commit()
            .expect("find initial commit");
        repo.branch("feature", &initial, false)
            .expect("create feature branch");

        fs::write(fixture.path().join("modified.txt"), "main\n").expect("modify main fixture");
        commit_all(&repo, "main change");

        repo.set_head("refs/heads/feature")
            .expect("switch HEAD to feature");
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .expect("check out feature");
        fs::write(fixture.path().join("modified.txt"), "feature\none\n")
            .expect("modify feature fixture");
        fs::write(fixture.path().join("feature-added.txt"), "feature\n")
            .expect("add feature fixture");
        commit_all(&repo, "feature change");
        drop(initial);
        drop(repo);

        let target = DiffTarget::MergeBaseRange {
            base: "main".to_string(),
            head: "feature".to_string(),
        };
        let stats = assert_target_stats_match(&fixture, &target);
        assert_eq!(
            stats.stats(None),
            DiffStats {
                files: 2,
                additions: 3,
                deletions: 1,
            }
        );
    }

    #[test]
    fn binary_empty_and_invalid_utf8_worktree_files_share_canonical_behavior() {
        let fixture = TestRepo::new();
        fs::write(fixture.path().join("binary.bin"), [0, 1, 0, 3]).expect("modify binary fixture");
        fs::write(fixture.path().join("empty.txt"), "").expect("write empty fixture");
        fs::write(
            fixture.path().join("invalid.txt"),
            [b'v', b'a', b'l', b'i', b'd', b'\n', b'f', 0xff, b'\n'],
        )
        .expect("write invalid UTF-8 fixture");

        let stats = assert_target_stats_match(&fixture, &DiffTarget::Worktree);
        assert_eq!(
            stats.stats(None),
            DiffStats {
                files: 3,
                additions: 0,
                deletions: 0,
            }
        );
    }

    #[test]
    fn untracked_fallback_preserves_line_content() {
        let fixture = TestRepo::new();
        fs::write(fixture.path().join("line-endings.txt"), "first\r\nsecond\r")
            .expect("write line-ending fixture");

        let session = DiffSession::load_from_repo(fixture.path(), &DiffTarget::Worktree, None)
            .expect("load untracked fallback");
        let texts = session.files[0].hunks[0]
            .lines
            .iter()
            .filter_map(|line| match line {
                DiffLine::Added { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(texts, ["first", "second\r"]);

        let stats = assert_target_stats_match(&fixture, &DiffTarget::Worktree);
        assert_eq!(
            stats.stats(None),
            DiffStats {
                files: 1,
                additions: 2,
                deletions: 0,
            }
        );
    }

    #[test]
    fn invalid_revision_errors_match() {
        let fixture = TestRepo::new();
        let target = DiffTarget::Range {
            base: "missing-revision".to_string(),
            head: "HEAD".to_string(),
        };

        let session_error = DiffSession::load_from_repo(fixture.path(), &target, None)
            .expect_err("full diff should reject an invalid revision");
        let repo = fixture.open();
        let mut loader = DiffStatsLoader::new(&repo);
        let stats_error =
            load_stats(&mut loader, &target).expect_err("stats should reject an invalid revision");

        assert_eq!(stats_error.code(), session_error.code());
        assert_eq!(stats_error.class(), session_error.class());
    }

    #[test]
    fn cancelled_stats_walk_discards_partial_results() {
        let fixture = TestRepo::new();
        fs::write(fixture.path().join("modified.txt"), "before\nafter\n")
            .expect("modify cancellation fixture");
        fs::write(fixture.path().join("untracked.txt"), "first\nsecond\n")
            .expect("write cancellation fixture");

        let repo = fixture.open();
        let mut loader = DiffStatsLoader::new(&repo);
        let checks = Cell::new(0);
        let cancelled = loader
            .load(&DiffTarget::Worktree, || {
                let next = checks.get() + 1;
                checks.set(next);
                next >= 6
            })
            .expect("cancel stats load");

        assert!(cancelled.is_none());
        assert!(checks.get() >= 6);
        assert!(loader.cache.is_empty());

        let completed =
            load_stats(&mut loader, &DiffTarget::Worktree).expect("retry cancelled stats load");
        assert_eq!(
            completed.stats(None),
            DiffStats {
                files: 2,
                additions: 3,
                deletions: 0,
            }
        );
        assert_eq!(loader.cache.len(), 1);
    }

    #[test]
    fn stats_loader_reuses_worktree_and_equivalent_tree_diffs() {
        let fixture = TestRepo::new();
        let repo = fixture.open();
        let mut loader = DiffStatsLoader::new(&repo);

        load_stats(&mut loader, &DiffTarget::Worktree).expect("load worktree summary");
        load_stats(&mut loader, &DiffTarget::Worktree).expect("reuse worktree summary");
        assert_eq!(loader.cache.len(), 1);

        let head_to_head = DiffTarget::Range {
            base: "HEAD".to_string(),
            head: "HEAD".to_string(),
        };
        let main_to_head = DiffTarget::Range {
            base: "main".to_string(),
            head: "HEAD".to_string(),
        };
        load_stats(&mut loader, &head_to_head).expect("load first equivalent tree summary");
        load_stats(&mut loader, &main_to_head).expect("reuse equivalent tree summary");
        assert_eq!(loader.cache.len(), 2);

        let main_merge_base_to_head = DiffTarget::MergeBaseRange {
            base: "main".to_string(),
            head: "HEAD".to_string(),
        };
        load_stats(&mut loader, &main_merge_base_to_head)
            .expect("reuse range summary for equivalent merge-base trees");
        assert_eq!(loader.cache.len(), 2);
    }
}
