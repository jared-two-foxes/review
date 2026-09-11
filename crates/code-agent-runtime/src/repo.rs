use crate::error::RepoError;
use git2::{Oid, Repository};
use std::path::{Component, Path, PathBuf};

pub struct GitRepo {
    repo: Repository,
    root: PathBuf,
    canonical_root: PathBuf,
}

impl GitRepo {
    /// Open a git repository. Uses `discover` to walk up from the given
    /// path to find the nearest `.git` directory, so a subdirectory of
    /// the repository works as well as the repository root itself.
    pub fn open(path: &Path) -> Result<Self, RepoError> {
        let repo = Repository::discover(path)?;
        let root = repo
            .workdir()
            .or_else(|| repo.path().parent())
            .unwrap()
            .to_path_buf();
        let canonical_root = root.canonicalize().unwrap_or(root.clone());
        Ok(Self {
            repo,
            root,
            canonical_root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn revparse_single(&self, refname: &str) -> Result<git2::Object<'_>, RepoError> {
        Ok(self.repo.revparse_single(refname)?)
    }

    /// Canonicalize a relative path against the repository root.
    ///
    /// Paths containing `..` that resolve within the repository root
    /// are allowed (e.g. `src/../src/main.rs` resolves to `src/main.rs`).
    /// Paths that escape the root are rejected with `RepoError::PathEscape`.
    pub fn canonicalize_path(&self, relative: &str) -> Result<PathBuf, RepoError> {
        // Normalize the path by resolving components without filesystem access
        let candidate = self.root.join(relative);
        let normalized = normalize_path(&candidate);

        // Check the normalized path is still within the root
        if !normalized.starts_with(&self.root) {
            return Err(RepoError::PathEscape {
                requested: PathBuf::from(relative),
                root: self.root.clone(),
            });
        }

        let canonical = normalized
            .canonicalize()
            .map_err(|_| RepoError::PathEscape {
                requested: PathBuf::from(relative),
                root: self.root.clone(),
            })?;
        if !canonical.starts_with(&self.canonical_root) {
            return Err(RepoError::PathEscape {
                requested: PathBuf::from(relative),
                root: self.root.clone(),
            });
        }
        Ok(canonical)
    }

    pub fn find_commit(&self, oid: Oid) -> Result<git2::Commit<'_>, RepoError> {
        Ok(self.repo.find_commit(oid)?)
    }

    /// Return the repository's current index (staged changes).
    pub fn index(&self) -> Result<git2::Index, RepoError> {
        Ok(self.repo.index()?)
    }

    pub fn raw_repo(&self) -> &git2::Repository {
        &self.repo
    }

    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }
}

/// Resolve `.` and `..` components lexically without filesystem access.
/// Returns the normalized path.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {
                // Skip `.` components
            }
            Component::ParentDir => {
                // Pop the last component (if any) for `..`
                // Don't pop past the root
                if components.len() > 1 {
                    components.pop();
                }
            }
            other => components.push(other),
        }
    }
    components.iter().collect()
}
