use crate::error::RepoError;
use crate::repo::GitRepo;
use git2::Oid;
use sha2::{Digest, Sha256};

/// Resolve base and head refs to commit identifiers.
pub fn resolve_commits(
    repo: &GitRepo,
    base_ref: &str,
    head_ref: &str,
) -> Result<(Oid, Oid), RepoError> {
    let base = repo.revparse_single(base_ref)?.id();
    let head = repo.revparse_single(head_ref)?.id();
    Ok((base, head))
}

/// Compute a deterministic snapshot ID from the base/head pair.
/// Format: sha256:{base_hex}:{head_hex}
/// The OID hex strings are embedded directly so the snapshot ID
/// is human-readable and traceable to the exact commit range.
pub fn snapshot_id(base: Oid, head: Oid) -> String {
    format!("sha256:{}:{}", base, head)
}
