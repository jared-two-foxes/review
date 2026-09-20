use crate::error::RepoError;
use crate::repo::GitRepo;
use crate::target::ReviewTarget;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub status: FileStatus,
}

/// Compute a git diff between two review targets, optionally filtered by pathspec
pub fn compute_diff<'a>(
    repo: &'a GitRepo,
    base: &ReviewTarget,
    head: &ReviewTarget,
    pathspec: Option<&str>,
) -> Result<git2::Diff<'a>, RepoError> {
    let git_repo = repo.raw_repo();
    let mut opts = git2::DiffOptions::new();
    if let Some(path) = pathspec {
        opts.pathspec(path);
    }

    let diff = match (base, head) {
        (ReviewTarget::Commit(base_oid), ReviewTarget::Commit(head_oid)) => {
            let base_tree = git_repo.find_commit(*base_oid)?.tree()?;
            let head_tree = git_repo.find_commit(*head_oid)?.tree()?;
            git_repo.diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))?
        }
        (ReviewTarget::Commit(base_oid), ReviewTarget::WorkingDirectory) => {
            let base_tree = git_repo.find_commit(*base_oid)?.tree()?;
            opts.include_untracked(true);
            opts.recurse_untracked_dirs(true);
            git_repo.diff_tree_to_workdir(Some(&base_tree), Some(&mut opts))?
        }
        (ReviewTarget::Commit(base_oid), ReviewTarget::Index) => {
            let base_tree = git_repo.find_commit(*base_oid)?.tree()?;
            let index = git_repo.index()?;
            git_repo.diff_tree_to_index(Some(&base_tree), Some(&index), Some(&mut opts))?
        }
        (ReviewTarget::WorkingDirectory, ReviewTarget::Commit(head_oid)) => {
            let head_tree = git_repo.find_commit(*head_oid)?.tree()?;
            opts.include_untracked(true);
            opts.recurse_untracked_dirs(true);
            opts.reverse(true);
            git_repo.diff_tree_to_workdir(Some(&head_tree), Some(&mut opts))?
        }
        (ReviewTarget::Index, ReviewTarget::Commit(head_oid)) => {
            let head_tree = git_repo.find_commit(*head_oid)?.tree()?;
            let index = git_repo.index()?;
            opts.reverse(true);
            git_repo.diff_tree_to_index(Some(&head_tree), Some(&index), Some(&mut opts))?
        }
        (ReviewTarget::Empty, ReviewTarget::WorkingDirectory) => {
            opts.include_untracked(true);
            opts.recurse_untracked_dirs(true);
            git_repo.diff_tree_to_workdir(None, Some(&mut opts))?
        }
        (ReviewTarget::Empty, ReviewTarget::Commit(head_oid)) => {
            let head_tree = git_repo.find_commit(*head_oid)?.tree()?;
            git_repo.diff_tree_to_tree(None, Some(&head_tree), Some(&mut opts))?
        }
        (ReviewTarget::Empty, ReviewTarget::Index) => {
            let index = git_repo.index()?;
            git_repo.diff_tree_to_index(None, Some(&index), Some(&mut opts))?
        }
        _ => {
            return Err(RepoError::Other(
                "unsupported diff combination: at least one side must be a commit".into(),
            ));
        }
    };
    Ok(diff)
}

/// Enumerate changed files between base and head commits with per-file status.
pub fn changed_files(
    repo: &GitRepo,
    base: &ReviewTarget,
    head: &ReviewTarget,
) -> Result<Vec<FileChange>, RepoError> {
    let mut diff = compute_diff(repo, base, head, None)?;

    // Enable rename detection so renamed files are reported as Renamed,
    // not as separate Added + Deleted entries.
    let mut find_opts = git2::DiffFindOptions::new();
    find_opts.renames(true);
    diff.find_similar(Some(&mut find_opts))?;

    let mut changes: Vec<FileChange> = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            let path = delta
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().to_string())
                .or_else(|| {
                    delta
                        .old_file()
                        .path()
                        .map(|p| p.to_string_lossy().to_string())
                })
                .unwrap_or_default();
            let status = match delta.status() {
                git2::Delta::Added => FileStatus::Added,
                git2::Delta::Deleted => FileStatus::Deleted,
                git2::Delta::Renamed => FileStatus::Renamed,
                git2::Delta::Untracked
                    if matches!(
                        (base, head),
                        (ReviewTarget::WorkingDirectory, ReviewTarget::Commit(_))
                            | (ReviewTarget::Index, ReviewTarget::Commit(_))
                    ) =>
                {
                    FileStatus::Deleted
                }
                git2::Delta::Untracked => FileStatus::Added,
                _ => FileStatus::Modified,
            };
            changes.push(FileChange { path, status });
            true
        },
        None,
        None,
        None,
    )?;
    Ok(changes)
}

/// Read the diff for a specific changed file as structured hunks with line numbers.
pub fn read_diff(
    repo: &GitRepo,
    base: &ReviewTarget,
    head: &ReviewTarget,
    path: &str,
    byte_limit: usize,
) -> Result<DiffOutput, RepoError> {
    let diff = compute_diff(repo, base, head, Some(path))?;

    let mut content = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let origin = line.origin();
        if origin == '+' || origin == '-' || origin == ' ' {
            content.push(origin);
        }
        let text = String::from_utf8_lossy(line.content());
        content.push_str(&text);
        true
    })?;

    let truncated = content.len() > byte_limit;
    if truncated {
        let mut limit = byte_limit;
        while limit > 0 && !content.is_char_boundary(limit) {
            limit -= 1;
        }
        content.truncate(limit);
        content.push_str(&format!("\n[truncated at {} bytes]", byte_limit));
    }

    Ok(DiffOutput {
        path: path.to_string(),
        content,
        truncated,
    })
}

#[derive(Debug, Clone)]
pub struct DiffOutput {
    pub path: String,
    pub content: String,
    pub truncated: bool,
}
