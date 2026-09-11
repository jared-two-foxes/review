use std::path::PathBuf;

#[derive(Debug)]
pub enum RepoError {
    PathEscape { requested: PathBuf, root: PathBuf },
    Git(git2::Error),
    Other(String),
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::PathEscape { requested, root } => write!(
                f,
                "path
 {} escapes repository root {}",
                requested.display(),
                root.display()
            ),
            Self::Git(e) => write!(f, "git error: {}", e),
            Self::Other(msg) => write!(f, "error: {}", msg),
        }
    }
}

impl std::error::Error for RepoError {}

impl From<git2::Error> for RepoError {
    fn from(e: git2::Error) -> Self {
        Self::Git(e)
    }
}
