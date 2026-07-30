use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use git2::{IndexAddOption, Oid, Repository, Signature, build::CheckoutBuilder};

use super::*;

static NEXT_TEST_REPO: AtomicU64 = AtomicU64::new(0);

struct TestRepo {
    path: PathBuf,
}

impl TestRepo {
    fn new() -> Self {
        let id = NEXT_TEST_REPO.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("enza-diff-summary-{}-{id}", process::id()));
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
    let mut stats = DiffStats {
        files: session.files.len(),
        ..DiffStats::default()
    };
    for file in &session.files {
        let (additions, deletions) = file.change_counts();
        stats.additions += additions;
        stats.deletions += deletions;
    }
    stats
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
        let filtered_session = DiffSession::load_from_repo(fixture.path(), target, Some(&filter))
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
    fs::write(fixture.path().join("feature-added.txt"), "feature\n").expect("add feature fixture");
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
fn cancelled_session_load_discards_the_partial_model() {
    let fixture = TestRepo::new();
    fs::write(fixture.path().join("modified.txt"), "before\nafter\n")
        .expect("modify cancellation fixture");
    fs::write(fixture.path().join("untracked.txt"), "first\nsecond\n")
        .expect("write cancellation fixture");

    let checks = Cell::new(0);
    let cancelled = DiffSession::load_from_repo_cancellable(
        fixture.path(),
        &DiffTarget::Worktree,
        None,
        || {
            let next = checks.get() + 1;
            checks.set(next);
            next >= 9
        },
    )
    .expect("cancel session load");

    assert!(cancelled.is_none());
    assert!(checks.get() >= 9);

    let completed = DiffSession::load_from_repo(fixture.path(), &DiffTarget::Worktree, None)
        .expect("retry cancelled session load");
    assert_eq!(completed.files.len(), 2);
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

    load_stats(&mut loader, &DiffTarget::Worktree).expect("load worktree stats");
    load_stats(&mut loader, &DiffTarget::Worktree).expect("reuse worktree stats");
    assert_eq!(loader.cache.len(), 1);

    let head_to_head = DiffTarget::Range {
        base: "HEAD".to_string(),
        head: "HEAD".to_string(),
    };
    let main_to_head = DiffTarget::Range {
        base: "main".to_string(),
        head: "HEAD".to_string(),
    };
    load_stats(&mut loader, &head_to_head).expect("load first equivalent tree stats");
    load_stats(&mut loader, &main_to_head).expect("reuse equivalent tree stats");
    assert_eq!(loader.cache.len(), 2);

    let main_merge_base_to_head = DiffTarget::MergeBaseRange {
        base: "main".to_string(),
        head: "HEAD".to_string(),
    };
    load_stats(&mut loader, &main_merge_base_to_head)
        .expect("reuse range stats for equivalent merge-base trees");
    assert_eq!(loader.cache.len(), 2);
}
