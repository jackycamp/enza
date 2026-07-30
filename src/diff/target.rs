use std::path::{Path, PathBuf};

use git2::{Diff as GitDiff, DiffOptions, Oid, Repository, Tree};

use super::DiffTarget;

/// `ResolvedDiff` stores the resolved object IDs and comparison type for a diff target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ResolvedDiff {
    Worktree { head: Option<Oid> },
    Cached { head: Option<Oid> },
    // Tree object IDs let different revision names use the same cached diff.
    Trees { old: Oid, new: Oid },
}

impl ResolvedDiff {
    /// Resolves revision names and merge bases.
    /// For worktree and cached targets, this function records the object ID of the current `HEAD` tree.
    pub(super) fn resolve(repo: &Repository, target: &DiffTarget) -> Result<Self, git2::Error> {
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

    /// Builds the Git diff from the resolved comparison.
    /// This function does not parse revision names again.
    pub(super) fn load<'repo>(
        &self,
        repo: &'repo Repository,
    ) -> Result<GitDiff<'repo>, git2::Error> {
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

    pub(super) fn allows_worktree_fallback(self) -> bool {
        matches!(self, Self::Worktree { .. })
    }
}

pub fn discover_repo_root(path: impl AsRef<Path>) -> Result<PathBuf, git2::Error> {
    let repo = Repository::discover(path)?;
    Ok(repo
        .workdir()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo.path().to_path_buf()))
}

fn head_tree_id(repo: &Repository) -> Option<Oid> {
    repo.head()
        .ok()
        .and_then(|head| head.peel_to_tree().ok())
        .map(|tree| tree.id())
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

fn revision_commit_id(repo: &Repository, revision: &str) -> Result<Oid, git2::Error> {
    Ok(repo.revparse_single(revision)?.peel_to_commit()?.id())
}
