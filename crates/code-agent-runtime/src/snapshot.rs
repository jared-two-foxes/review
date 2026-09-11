use crate::error::RepoError;
use crate::repo::GitRepo;
use crate::target::ReviewTarget;

/// Resolve base and head refs to review targets.
pub fn resolve_targets(
    repo: &GitRepo,
    base_ref: &str,
    head_ref: &str,
) -> Result<(ReviewTarget, ReviewTarget), RepoError> {
    let base = ReviewTarget::parse(repo, base_ref)?;
    let head = ReviewTarget::parse(repo, head_ref)?;
    Ok((base, head))
}

/// Compute a deterministic snapshot ID from the base/head pair.
pub fn snapshot_id(repo: &GitRepo, base: &ReviewTarget, head: &ReviewTarget) -> String {
    format!("sha256:{}:{}", base.label(repo), head.label(repo))
}
