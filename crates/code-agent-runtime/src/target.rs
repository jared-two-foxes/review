use crate::error::RepoError;
use crate::repo::GitRepo;
use git2::Oid;

/// Identifies what to review against - a committed snapshot, the working
/// directory, or the git index (staged changes).
#[derive(Debug, Clone)]
pub enum ReviewTarget {
    /// Review against a specific commit.
    Commit(Oid),
    /// Review against the working directory (uncommitted changes).
    WorkingDirectory,
    /// Review against the git index (staged changes).
    Index,
}

impl ReviewTarget {
    /// Parse a ref string into a review target.
    ///
    /// - `:working` -> working directory
    /// - `:staged` -> git index
    /// - any other string -> resolved as a git ref (e.g. `HEAD`, `HEAD~1`, a branch name).
    pub fn parse(repo: &GitRepo, ref_str: &str) -> Result<Self, RepoError> {
        match ref_str {
            ":working" => Ok(ReviewTarget::WorkingDirectory),
            ":staged" => Ok(ReviewTarget::Index),
            _ => {
                let oid = repo.revparse_single(ref_str)?.id();
                Ok(ReviewTarget::Commit(oid))
            }
        }
    }

    /// A stable label for snapshot ID computation.
    /// For commits this is the OID hex
    pub fn label(&self, repo: &GitRepo) -> String {
        match self {
            ReviewTarget::Commit(oid) => oid.to_string(),
            ReviewTarget::WorkingDirectory => "working".to_string(),
            ReviewTarget::Index => match repo.index().and_then(|mut i| Ok(i.write_tree()?)) {
                Ok(oid) => oid.to_string(),
                Err(_) => "index-unknown".to_string(),
            },
        }
    }

    /// Read file content at the given path from this target.
    pub fn read_file(&self, repo: &GitRepo, path: &str) -> Result<Vec<u8>, RepoError> {
        match self {
            ReviewTarget::Commit(oid) => {
                let tree = repo.find_commit(*oid)?.tree()?;
                let entry = tree.get_path(std::path::Path::new(path))?;
                let blob = repo.raw_repo().find_blob(entry.id())?;
                Ok(blob.content().to_vec())
            }
            ReviewTarget::WorkingDirectory => {
                let full_path = repo.canonicalize_path(path)?;
                std::fs::read(&full_path)
                    .map_err(|e| RepoError::Other(format!("failed to read file: {}", e)))
            }
            ReviewTarget::Index => {
                let index = repo.index()?;
                let entry = index
                    .get_path(std::path::Path::new(path), 0)
                    .ok_or_else(|| {
                        RepoError::Other(format!("file not found in index: {}", path))
                    })?;
                let blob = repo.raw_repo().find_blob(entry.id)?;
                Ok(blob.content().to_vec())
            }
        }
    }
}
