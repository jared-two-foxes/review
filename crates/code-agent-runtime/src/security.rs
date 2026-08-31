use crate::repo::GitRepo;
use glob::{MatchOptions, Pattern};
use std::path::{Component, Path, PathBuf};

const DENY_PATTERNS: &[&str] = &[
    ".git/**",
    ".env*",
    "**/.aws/**",
    "**/id_rsa*",
    "**/*.pem",
    "**/*.key",
];

pub const DEFAULT_ENTRY_LIMIT: usize = 100;
pub const DEFAULT_MATCH_LIMIT: usize = 50;

/// Result of rejecting an input at the repository security boundary.
#[derive(Debug, PartialEq, Eq)]
pub struct SecurityError {
    pub requested: PathBuf,
}

/// The bounded representation of raw file output.
#[derive(Debug, PartialEq, Eq)]
pub struct BoundedOutput {
    pub content: String,
    pub truncated: bool,
    pub completeness: bool,
}

/// Bounds raw file bytes and reports whether the result is complete.
pub fn bound_bytes(raw: &[u8], limit: usize) -> BoundedOutput {
    if raw.len() <= limit {
        return BoundedOutput {
            content: String::from_utf8_lossy(raw).into_owned(),
            truncated: false,
            completeness: true,
        };
    }
    let mut kept = limit;
    while kept > 0 && (raw[kept] & 0xC0) == 0x80 {
        kept -= 1;
    }
    let mut content = String::from_utf8_lossy(&raw[..kept]).into_owned();
    content.push_str(&format!("\n[truncated: {} of {} bytes]", kept, raw.len()));
    BoundedOutput {
        content,
        truncated: true,
        completeness: false,
    }
}

pub fn bound_entries(entries: &[String]) -> BoundedOutput {
    bound_entries_with_limit(entries, DEFAULT_ENTRY_LIMIT)
}

pub fn bound_entries_with_limit(entries: &[String], limit: usize) -> BoundedOutput {
    if entries.len() <= limit {
        return BoundedOutput {
            content: entries.join("\n"),
            truncated: false,
            completeness: true,
        };
    }
    let mut content = entries[..limit].join("\n");
    content.push_str(&format!(
        "\n[truncated: {} of {} entries]",
        limit,
        entries.len()
    ));
    BoundedOutput {
        content,
        truncated: true,
        completeness: false,
    }
}

/// Bounds search matches using the default match limit.
pub fn bound_matches(matches: &[String]) -> BoundedOutput {
    bound_matches_with_limit(matches, DEFAULT_MATCH_LIMIT)
}

pub fn bound_matches_with_limit(matches: &[String], limit: usize) -> BoundedOutput {
    if matches.len() <= limit {
        return BoundedOutput {
            content: matches.join("\n"),
            truncated: false,
            completeness: true,
        };
    }
    let mut content = matches[..limit].join("\n");
    content.push_str(&format!(
        "\n[truncated: {} matches, search incomplete]",
        limit
    ));
    BoundedOutput {
        content,
        truncated: true,
        completeness: false,
    }
}

/// Repository path security policy.
#[derive(Debug)]
pub struct SecurityPolicy {
    bytes_limit: usize,
    entries_limit: usize,
    matches_limit: usize,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityPolicy {
    pub fn new() -> Self {
        Self::with_limits(10240, DEFAULT_ENTRY_LIMIT, DEFAULT_MATCH_LIMIT)
    }

    pub fn with_bytes_limit(limit: usize) -> Self {
        Self::with_limits(limit, DEFAULT_ENTRY_LIMIT, DEFAULT_MATCH_LIMIT)
    }

    pub fn with_entries_limit(limit: usize) -> Self {
        Self::with_limits(10240, limit, DEFAULT_MATCH_LIMIT)
    }

    pub fn with_matches_limit(limit: usize) -> Self {
        Self::with_limits(10240, DEFAULT_ENTRY_LIMIT, limit)
    }

    pub fn with_limits(bytes_limit: usize, entries_limit: usize, matches_limit: usize) -> Self {
        Self {
            bytes_limit,
            entries_limit,
            matches_limit,
        }
    }

    /// Allow existing, non-sensitive repository-relative paths only; reject rooted,
    /// escaping, broken, or externally resolved symlink paths before filesystem access.
    pub fn validate_path(
        &self,
        repo: &GitRepo,
        requested: impl AsRef<str>,
    ) -> Result<PathBuf, SecurityError> {
        let requested = requested.as_ref();
        let path = Path::new(requested);
        if matches!(
            path.components().next(),
            Some(Component::Prefix(_)) | Some(Component::RootDir)
        ) {
            return Err(SecurityError {
                requested: PathBuf::from(requested),
            });
        }
        if is_denied(requested) {
            return Err(SecurityError {
                requested: PathBuf::from(requested),
            });
        }
        let candidate = repo.root().join(requested);
        let canonical_root = repo.canonical_root();
        if has_broken_or_escaping_symlink(&candidate, canonical_root) {
            return Err(SecurityError {
                requested: PathBuf::from(requested),
            });
        }

        let canonical = repo
            .canonicalize_path(requested)
            .map_err(|_| SecurityError {
                requested: PathBuf::from(requested),
            })?;

        if let Ok(rel) = canonical.strip_prefix(repo.canonical_root()) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if is_denied(&rel_str) {
                return Err(SecurityError {
                    requested: PathBuf::from(requested),
                });
            }
        }

        Ok(canonical)
    }

    /// Read a repository file through the security boundary.
    pub fn read_file(
        &self,
        repo: &GitRepo,
        requested: impl AsRef<str>,
    ) -> Result<BoundedOutput, SecurityError> {
        let requested = requested.as_ref();
        let path = self.validate_path(repo, requested)?;
        std::fs::read(&path)
            .map(|raw| bound_bytes(&raw, self.bytes_limit))
            .map_err(|_| SecurityError {
                requested: PathBuf::from(requested),
            })
    }

    /// List a repository directory through the security boundary.
    pub fn list_directory(
        &self,
        repo: &GitRepo,
        requested: impl AsRef<str>,
    ) -> Result<BoundedOutput, SecurityError> {
        let requested = requested.as_ref();
        let path = self.validate_path(repo, requested)?;
        let mut entries: Vec<String> = std::fs::read_dir(&path)
            .map_err(|_| SecurityError {
                requested: PathBuf::from(requested),
            })?
            .map(|entry| {
                let entry = entry.map_err(|_| SecurityError {
                    requested: PathBuf::from(requested),
                })?;
                Ok::<_, SecurityError>(entry.file_name().to_string_lossy().into_owned())
            })
            .collect::<Result<_, _>>()?;
        entries.sort();
        Ok(bound_entries_with_limit(&entries, self.entries_limit))
    }

    /// Search a repository path for a text query.
    ///
    /// This additive declaration is scaffolding for the search operation test.
    ///
    /// Retained deliberately (deferred, not removed): the ticket's match-limit
    /// criterion describes search-tool behavior ("searching ... returns 50 matches
    /// with [truncated: ...]"), so the operation must exist somewhere; this
    /// implementation will be absorbed by the dedicated read-only-tools ticket.
    pub fn search(
        &self,
        repo: &GitRepo,
        query: impl AsRef<str>,
    ) -> Result<BoundedOutput, SecurityError> {
        let query = query.as_ref();
        let root = self.validate_path(repo, ".")?;
        let mut matches: Vec<(String, usize, String)> = Vec::new();
        let repo_root = repo
            .root()
            .canonicalize()
            .unwrap_or_else(|_| repo.root().to_path_buf());
        collect_matches(&root, &repo_root, query, &mut matches)?;
        matches.sort(); // deterministic: path order, then line order
        let lines: Vec<String> = matches
            .into_iter()
            .map(|(path, line_number, line)| format!("{}:{}:{}", path, line_number, line))
            .collect();
        Ok(bound_matches_with_limit(&lines, self.matches_limit))
    }

    pub fn entries_limit(&self) -> usize {
        self.entries_limit
    }

    pub fn is_path_denied(&self, rel_path: &str) -> bool {
        is_denied(rel_path)
    }

    pub fn matches_limit(&self) -> usize {
        self.matches_limit
    }
}

fn collect_matches(
    dir: &Path,
    repo_root: &Path,
    query: &str,
    matches: &mut Vec<(String, usize, String)>,
) -> Result<(), SecurityError> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Err(SecurityError {
            requested: dir.to_path_buf(),
        });
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let Ok(rel) = path.strip_prefix(repo_root) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if is_denied(&rel_str) {
            continue; // never search sensitive files
        }
        // Follow the documented symlink policy: allow in-repository
        // targets, reject broken or escaping links (matching `validate_path`).
        if file_type.is_symlink() {
            match path.canonicalize() {
                Ok(target) if target.starts_with(repo_root) => {
                    if target.is_dir() {
                        collect_matches(&target, repo_root, query, matches)?;
                    } else if let Ok(content) = std::fs::read_to_string(&target) {
                        for (idx, line) in content.lines().enumerate() {
                            if line.contains(query) {
                                matches.push((rel_str.clone(), idx + 1, line.to_string()));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SecurityError {
                        requested: PathBuf::from(rel_str),
                    });
                }
            }
            continue;
        }
        if file_type.is_dir() {
            collect_matches(&path, repo_root, query, matches)?;
        } else if let Ok(content) = std::fs::read_to_string(&path) {
            for (idx, line) in content.lines().enumerate() {
                if line.contains(query) {
                    matches.push((rel_str.clone(), idx + 1, line.to_string()));
                }
            }
        }
    }
    Ok(())
}

fn is_denied(requested: &str) -> bool {
    let normalized = requested.replace('\\', "/");
    let mut components: Vec<&str> = Vec::new();
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return true; // a `..` that escaapes the repository root
                }
            }
            other => components.push(other),
        }
    }
    let options = MatchOptions {
        require_literal_separator: true,
        ..MatchOptions::default()
    };

    components.iter().enumerate().any(|(index, _)| {
        let suffix = components[index..].join("/");
        DENY_PATTERNS.iter().any(|pattern| {
            let p = Pattern::new(pattern).expect("deny patterns must be valid globs");
            if p.matches_with(&suffix, options) {
                return true;
            }
            // Also deny the directory itself (e.g. `.git` for pattern `.git/**`)
            if let Some(prefix) = pattern.strip_suffix("/**")
                && let Ok(p2) = Pattern::new(prefix)
                && p2.matches_with(&suffix, options)
            {
                return true;
            }
            false
        })
    })
}

fn has_broken_or_escaping_symlink(candidate: &Path, canonical_root: &Path) -> bool {
    let mut prefix = PathBuf::new();
    for component in candidate.components() {
        prefix.push(component);
        let Ok(meta) = std::fs::symlink_metadata(&prefix) else {
            return false;
        };
        if meta.file_type().is_symlink()
            && !matches!(prefix.canonicalize(), Ok(target) if target.starts_with(canonical_root))
        {
            return true;
        }
    }
    false
}
