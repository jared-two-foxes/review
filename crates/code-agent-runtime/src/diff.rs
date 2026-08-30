use crate::error::RepoError;
use crate::repo::GitRepo;
use git2::Oid;

#[derive(Debug, Clone, PartialEq)]
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

/// Enumerate changed files between base and head commits with per-file status.
pub fn changed_files(repo: &GitRepo, base: Oid, head: Oid) -> Result<Vec<FileChange>, RepoError> {
    let git_repo = repo.raw_repo();
    let base_tree = git_repo.find_commit(base)?.tree()?;
    let head_tree = git_repo.find_commit(head)?.tree()?;
    let diff = git_repo.diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None)?;

    // Enable rename detection so renamed files are reported as Renamed,
    // not as separate Added + Deleted entries.
    let mut diff = diff;
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
    base: Oid,
    head: Oid,
    path: &str,
    byte_limit: usize,
) -> Result<DiffOutput, RepoError> {
    let git_repo = repo.raw_repo();
    let base_tree = git_repo.find_commit(base)?.tree()?;
    let head_tree = git_repo.find_commit(head)?.tree()?;

    let mut opts = git2::DiffOptions::new();
    opts.pathspec(path);
    let diff = git_repo.diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))?;

    let mut content = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let text = String::from_utf8_lossy(line.content());
        content.push_str(&text);
        true
    })?;

    let truncated = content.len() > byte_limit;
    if truncated {
        content.truncate(byte_limit);
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