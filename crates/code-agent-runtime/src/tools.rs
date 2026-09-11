use agent_kernel::model::ToolDescription;
use agent_kernel::tools::{Tool, ToolResult, ToolStatus};
use serde_json::{Value, json};
use sha2::Digest;

use crate::diff::{FileStatus, changed_files, compute_diff, read_diff};
use crate::repo::GitRepo;
use crate::security::{SecurityPolicy, bound_bytes};
use crate::target::ReviewTarget;

/// Minimal API scaffold for the change-summary tool. The implementation is
/// intentionally incomplete; the behavior is supplied by the production work
/// driven by the integration test.
pub struct GetChangeSummaryTool {
    repo: GitRepo,
    base: ReviewTarget,
    head: ReviewTarget,
}

impl GetChangeSummaryTool {
    pub fn new(repo: GitRepo, base: ReviewTarget, head: ReviewTarget) -> Self {
        Self { repo, base, head }
    }
}

impl Tool for GetChangeSummaryTool {
    fn name(&self) -> &str {
        "get_change_summary"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().to_string(),
            description: "Return a complete summary of changes between two commits.".to_string(),
            input_schema: json!({"type": "object", "additionalProperties": false}),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.is_object() {
            Ok(())
        } else {
            Err("arguments must be an object".to_string())
        }
    }

    fn execute(&self, _arguments: &Value) -> ToolResult {
        let diff = match compute_diff(&self.repo, &self.base, &self.head, None) {
            Ok(d) => d,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "failed to compute diff"}),
                };
            }
        };
        let files = changed_files(&self.repo, &self.base, &self.head).unwrap_or_default();
        let file_entries: Vec<Value> = files
            .iter()
            .map(|f| {
                json!({
                    "path": f.path,
                    "status": serde_json::to_value(&f.status)
                        .expect("file status is serializable")
                        .as_str()
                        .expect("file status serializes as a string")
                        .to_lowercase(),
                })
            })
            .collect();

        let stats = diff.stats().unwrap();
        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "changed_file_count": files.len(),
                "total_insertions": stats.insertions(),
                "total_deletions": stats.deletions(),
                "files": file_entries,
                "snapshot_id": crate::snapshot::snapshot_id(&self.repo, &self.base, &self.head),
            }),
        }
    }
}

pub struct GetChangedFilesTool {
    repo: GitRepo,
    base: ReviewTarget,
    head: ReviewTarget,
}

impl GetChangedFilesTool {
    pub fn new(repo: GitRepo, base: ReviewTarget, head: ReviewTarget) -> Self {
        Self { repo, base, head }
    }
}

impl Tool for GetChangedFilesTool {
    fn name(&self) -> &str {
        "get_changed_files"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().to_string(),
            description: "List changed files and their head content identifiers.".to_string(),
            input_schema: json!({"type": "object", "additionalProperties": false}),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.is_object() {
            Ok(())
        } else {
            Err("arguments must be an object".to_string())
        }
    }

    fn execute(&self, _arguments: &Value) -> ToolResult {
        let changes = match changed_files(&self.repo, &self.base, &self.head) {
            Ok(c) => c,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "failed to enumerate changes"}),
                };
            }
        };
        let files: Vec<Value> = changes
            .iter()
            .map(|change| {
                let content_id = if change.status == FileStatus::Deleted {
                    Value::Null
                } else {
                    self.head
                        .read_file(&self.repo, &change.path)
                        .ok()
                        .map(|content| {
                            let hash = sha2::Sha256::digest(&content);
                            format!("sha256:{}", hex::encode(hash))
                        })
                        .map(Value::String)
                        .unwrap_or(Value::Null)
                };
                json!({
                    "path": change.path,
                    "status": format!("{:?}", change.status).to_lowercase(),
                    "content_id": content_id,
                })
            })
            .collect();

        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "files": files,
                "snapshot_id": crate::snapshot::snapshot_id(&self.repo, &self.base, &self.head),
            }),
        }
    }
}

/// Additive API scaffold for the read-diff tool exercised by the integration
/// test. Its behavior is intentionally stubbed until the tool implementation
/// is supplied.
pub struct ReadDiffTool {
    repo: GitRepo,
    base: ReviewTarget,
    head: ReviewTarget,
    byte_limit: usize,
}

impl ReadDiffTool {
    pub fn new(repo: GitRepo, base: ReviewTarget, head: ReviewTarget, byte_limit: usize) -> Self {
        Self {
            repo,
            base,
            head,
            byte_limit,
        }
    }
}

impl Tool for ReadDiffTool {
    fn name(&self) -> &str {
        "read_diff"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().to_string(),
            description: "Read the bounded diff for one changed file.".to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path"],
                "properties": {"path": {"type": "string"}}
            }),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.get("path").and_then(Value::as_str).is_some()
            && arguments
                .as_object()
                .is_some_and(|object| object.len() == 1)
        {
            Ok(())
        } else {
            Err("arguments must contain only a string path".to_string())
        }
    }

    fn execute(&self, arguments: &Value) -> ToolResult {
        let path = match arguments.get("path").and_then(Value::as_str) {
            Some(p) => p,
            None => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "missing path argument"}),
                };
            }
        };
        match read_diff(&self.repo, &self.base, &self.head, path, self.byte_limit) {
            Ok(diff) => {
                let content_id = format!(
                    "sha256:{}",
                    hex::encode(sha2::Sha256::digest(diff.content.as_bytes()))
                );
                ToolResult {
                    status: ToolStatus::Succeeded,
                    value: json!({
                        "path": path,
                        "content":diff.content,
                        "truncated": diff.truncated,
                        "content_id": content_id,
                        "snapshot_id": crate::snapshot::snapshot_id(&self.repo, &self.base, &self.head),
                    }),
                }
            }
            Err(_) => ToolResult {
                status: ToolStatus::Failed,
                value: json!({"error": "failed to produce diff"}),
            },
        }
    }
}

/// Additive compile-time scaffold for the read-file tool. Its behavior is
/// intentionally absent so the integration test fails at runtime until the
/// head-commit reader and metadata are implemented.
pub struct ReadFileTool {
    repo: GitRepo,
    head: ReviewTarget,
    byte_limit: usize,
    policy: SecurityPolicy,
}

impl ReadFileTool {
    pub fn new(
        repo: GitRepo,
        head: ReviewTarget,
        byte_limit: usize,
        policy: SecurityPolicy,
    ) -> Self {
        Self {
            repo,
            head,
            byte_limit,
            policy,
        }
    }
}

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().to_string(),
            description: "Read a file from the repository head.".to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path"],
                "properties": {"path": {"type": "string"}}
            }),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.get("path").and_then(Value::as_str).is_some()
            && arguments
                .as_object()
                .is_some_and(|object| object.len() == 1)
        {
            Ok(())
        } else {
            Err("arguments must contain only a string path".to_string())
        }
    }

    fn execute(&self, arguments: &Value) -> ToolResult {
        let path = match arguments.get("path").and_then(Value::as_str) {
            Some(p) => p,
            None => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "missing path argument"}),
                };
            }
        };
        if self.policy.is_path_denied(path) {
            return ToolResult {
                status: ToolStatus::Denied,
                value: json!({"error": "path denied"}),
            };
        }
        let content_bytes: Vec<u8> = match self.head.read_file(&self.repo, path) {
            Ok(t) => t,
            Err(e) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": format!("failed to read {} at {}: {}", path, self.head.label(&self.repo), e)}),
                };
            }
        };
        let content_id = format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(&content_bytes))
        );
        let (content, truncated, completeness) = if content_bytes.len() > self.byte_limit {
            let bounded = bound_bytes(&content_bytes, self.byte_limit);
            (bounded.content, bounded.truncated, bounded.completeness)
        } else {
            (
                String::from_utf8_lossy(&content_bytes).into_owned(),
                false,
                true,
            )
        };

        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "content": content,
                "truncated": truncated,
                "completeness": completeness,
                "content_id": content_id,
                "observed_head": self.head.label(&self.repo),
            }),
        }
    }
}

fn collect_tree_entries(
    tree: &git2::Tree,
    path: &str,
    policy: &SecurityPolicy,
) -> Vec<(String, String)> {
    tree.iter()
        .filter_map(|entry| {
            let name = entry.name()?.to_string();
            let rel = format!("{path}/{name}")
                .trim_start_matches("./")
                .to_string();
            if policy.is_path_denied(&rel) {
                return None;
            }
            let ty = match entry.kind() {
                Some(git2::ObjectType::Tree) => "dir".into(),
                Some(git2::ObjectType::Blob) => "file".into(),
                _ => return None,
            };
            Some((name, ty))
        })
        .collect()
}

fn collect_workdir_entries(
    repo: &GitRepo,
    path: &str,
    policy: &SecurityPolicy,
) -> Result<Vec<(String, String)>, crate::error::RepoError> {
    let base = if path == "." || path.is_empty() {
        repo.root().to_path_buf()
    } else {
        repo.canonicalize_path(path)?
    };
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(base)
        .map_err(|e| crate::error::RepoError::Other(format!("failed to read directory: {}", e)))?
    {
        let entry = entry.map_err(|e| {
            crate::error::RepoError::Other(format!("failed to read directory entry: {}", e))
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        let rel = if path == "." || path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", path, name)
        };
        if policy.is_path_denied(&rel) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => {
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }
        let ty = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            "dir".into()
        } else {
            "file".into()
        };
        entries.push((name, ty));
    }
    Ok(entries)
}

fn collect_index_entries(
    repo: &GitRepo,
    path: &str,
    policy: &SecurityPolicy,
) -> Result<Vec<(String, String)>, crate::error::RepoError> {
    let index = repo.index()?;
    let prefix = if path == "." || path.is_empty() {
        ""
    } else {
        path
    };
    let mut entries = Vec::new();
    let mut seen_dirs = std::collections::HashSet::new();
    for entry in index.iter() {
        let entry_path = String::from_utf8_lossy(&entry.path).to_string();
        if !prefix.is_empty() && !entry_path.starts_with(&format!("{}/", prefix)) {
            continue;
        }
        let rel = if prefix.is_empty() {
            entry_path.clone()
        } else {
            entry_path[prefix.len() + 1..].to_string()
        };
        if let Some(slash) = rel.find('/') {
            let dir_name = &rel[..slash];
            if seen_dirs.insert(dir_name.to_string()) {
                let full = if prefix.is_empty() {
                    dir_name.to_string()
                } else {
                    format!("{}/{}", prefix, dir_name)
                };
                if !policy.is_path_denied(&full) {
                    entries.push((dir_name.to_string(), "dir".to_string()));
                }
            }
        } else if !policy.is_path_denied(&rel) {
            entries.push((rel.clone(), "file".to_string()));
        }
    }
    Ok(entries)
}

/// Additive compile-time scaffold for the directory-listing tool. The tool
/// remains deliberately stubbed so its integration test fails until the
/// bounded, typed listing behavior is implemented.
pub struct ListDirectoryTool {
    repo: GitRepo,
    head: ReviewTarget,
    policy: crate::security::SecurityPolicy,
}

impl ListDirectoryTool {
    pub fn new(repo: GitRepo, head: ReviewTarget, policy: crate::security::SecurityPolicy) -> Self {
        Self { repo, head, policy }
    }
}

impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().to_string(),
            description: "List entries in a repository directory.".to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path"],
                "properties": {"path": {"type": "string"}}
            }),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.get("path").and_then(Value::as_str).is_some()
            && arguments
                .as_object()
                .is_some_and(|object| object.len() == 1)
        {
            Ok(())
        } else {
            Err("arguments must contain only a string path".to_string())
        }
    }

    fn execute(&self, arguments: &Value) -> ToolResult {
        let path = match arguments.get("path").and_then(Value::as_str) {
            Some(p) => p,
            None => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "missing path argument"}),
                };
            }
        };

        if self.policy.is_path_denied(path) {
            return ToolResult {
                status: ToolStatus::Denied,
                value: json!({"error": "path denied"}),
            };
        }

        let mut entries: Vec<(String, String)> = match &self.head {
            ReviewTarget::Commit(oid) => {
                let git_repo = self.repo.raw_repo();
                let head_tree = match git_repo.find_commit(*oid).and_then(|c| c.tree()) {
                    Ok(t) => t,
                    Err(_) => {
                        return ToolResult {
                            status: ToolStatus::Failed,
                            value: json!({"error": "failed to resolve head tree"}),
                        };
                    }
                };
                if path == "." || path.is_empty() {
                    collect_tree_entries(&head_tree, path, &self.policy)
                } else {
                    match head_tree.get_path(std::path::Path::new(path)) {
                        Ok(entry) if entry.kind() == Some(git2::ObjectType::Tree) => {
                            match entry
                                .to_object(git_repo)
                                .ok()
                                .and_then(|obj| obj.as_tree().cloned())
                            {
                                Some(sub) => collect_tree_entries(&sub, path, &self.policy),
                                None => {
                                    return ToolResult {
                                        status: ToolStatus::Failed,
                                        value: json!({"error": "failed to resolve directory at head"}),
                                    };
                                }
                            }
                        }
                        Ok(_) => {
                            return ToolResult {
                                status: ToolStatus::Failed,
                                value: json!({"error": "path is not a directory at head"}),
                            };
                        }
                        Err(_) => {
                            return ToolResult {
                                status: ToolStatus::Failed,
                                value: json!({"error": "directory not found at head"}),
                            };
                        }
                    }
                }
            }
            ReviewTarget::WorkingDirectory => {
                match collect_workdir_entries(&self.repo, path, &self.policy) {
                    Ok(e) => e,
                    Err(_) => {
                        return ToolResult {
                            status: ToolStatus::Failed,
                            value: json!({"error": "failed to read working directory"}),
                        };
                    }
                }
            }
            ReviewTarget::Index => match collect_index_entries(&self.repo, path, &self.policy) {
                Ok(e) => e,
                Err(_) => {
                    return ToolResult {
                        status: ToolStatus::Failed,
                        value: json!({"error": "failed to read index"}),
                    };
                }
            },
        };

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let limit = self.policy.entries_limit();
        let truncated = entries.len() > limit;
        let entries_json: Vec<Value> = entries
            .into_iter()
            .take(limit)
            .map(|(name, ty)| json!({"name": name, "type": ty}))
            .collect();
        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "entries": entries_json,
                "truncated": truncated,
                "completeness": !truncated,
                "observed_head": self.head.label(&self.repo),
            }),
        }
    }
}

pub struct SearchTextTool {
    repo: GitRepo,
    head: ReviewTarget,
    policy: crate::security::SecurityPolicy,
}

impl SearchTextTool {
    pub fn new(repo: GitRepo, head: ReviewTarget, policy: crate::security::SecurityPolicy) -> Self {
        Self { repo, head, policy }
    }
}

impl Tool for SearchTextTool {
    fn name(&self) -> &str {
        "search_text"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().to_string(),
            description: "Search repository files for a literal text query".to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["query"],
                "properties": {"query": {"type": "string"}}
            }),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.get("query").and_then(Value::as_str).is_some()
            && arguments
                .as_object()
                .is_some_and(|object| object.len() == 1)
        {
            Ok(())
        } else {
            Err("arguments must contain only a string query".to_string())
        }
    }

    fn execute(&self, arguments: &Value) -> ToolResult {
        let query = match arguments.get("query").and_then(Value::as_str) {
            Some(q) => q,
            None => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "missing query argument"}),
                };
            }
        };
        let mut matches: Vec<(String, usize, String)> = match &self.head {
            ReviewTarget::Commit(oid) => {
                let repo = self.repo.raw_repo();
                let tree = match repo.find_commit(*oid).and_then(|c| c.tree()) {
                    Ok(t) => t,
                    Err(_) => {
                        return ToolResult {
                            status: ToolStatus::Failed,
                            value: json!({"error": "failed to resolve head tree"}),
                        };
                    }
                };
                let mut m = Vec::new();
                search_tree_walk(repo, &tree, "", query, &self.policy, &mut m);
                m
            }
            ReviewTarget::WorkingDirectory => {
                let mut m = Vec::new();
                search_workdir_walk(self.repo.root(), "", query, &self.policy, &mut m);
                m
            }
            ReviewTarget::Index => match search_index(&self.repo, query, &self.policy) {
                Ok(m) => m,
                Err(_) => {
                    return ToolResult {
                        status: ToolStatus::Failed,
                        value: json!({"error": "failed to search index"}),
                    };
                }
            },
        };
        matches.sort();
        let limit = self.policy.matches_limit();
        let completeness = matches.len() <= limit;
        let matches_json: Vec<Value> = matches
            .into_iter()
            .take(limit)
            .map(|(path, line, content)| json!({"path": path, "line": line, "content": content}))
            .collect();
        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "matches": matches_json,
                "completeness": completeness,
                "observed_head": self.head.label(&self.repo),
            }),
        }
    }
}

fn search_tree_walk(
    repo: &git2::Repository,
    tree: &git2::Tree,
    prefix: &str,
    query: &str,
    policy: &SecurityPolicy,
    matches: &mut Vec<(String, usize, String)>,
) {
    for entry in tree.iter() {
        let (Some(name), Some(kind)) = (entry.name(), entry.kind()) else {
            continue;
        };
        let rel = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", prefix, name)
        };
        if policy.is_path_denied(&rel) {
            continue;
        }
        match kind {
            git2::ObjectType::Tree => {
                if let Ok(obj) = entry.to_object(repo)
                    && let Some(sub) = obj.as_tree()
                {
                    search_tree_walk(repo, sub, &rel, query, policy, matches);
                }
            }
            git2::ObjectType::Blob => {
                // Skip symlink-mode blobs: their content is a target path, not file text.
                if entry.filemode_raw() & 0o170000 == 0o120000 {
                    continue;
                }
                if let Ok(obj) = entry.to_object(repo)
                    && let Some(blob) = obj.as_blob()
                    && let Ok(content) = String::from_utf8(blob.content().to_vec())
                {
                    for (idx, line) in content.lines().enumerate() {
                        if line.contains(query) {
                            matches.push((rel.clone(), idx + 1, line.to_string()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn search_workdir_walk(
    dir: &std::path::Path,
    prefix: &str,
    query: &str,
    policy: &SecurityPolicy,
    matches: &mut Vec<(String, usize, String)>,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = match entry.file_name().into_string() {
                Ok(n) => n,
                Err(_) => continue,
            };
            if name == ".git" {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };
            if policy.is_path_denied(&rel) {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => {
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                search_workdir_walk(&path, &rel, query, policy, matches);
            } else if let Ok(content) = std::fs::read_to_string(&path) {
                for (idx, line) in content.lines().enumerate() {
                    if line.contains(query) {
                        matches.push((rel.clone(), idx + 1, line.to_string()));
                    }
                }
            }
        }
    }
}

fn search_index(
    repo: &GitRepo,
    query: &str,
    policy: &SecurityPolicy,
) -> Result<Vec<(String, usize, String)>, crate::error::RepoError> {
    let index = repo.index()?;
    let git_repo = repo.raw_repo();
    let mut matches = Vec::new();
    for entry in index.iter() {
        let path = String::from_utf8_lossy(&entry.path).to_string();
        if policy.is_path_denied(&path) {
            continue;
        }
        // Skip symlink entries
        if entry.mode & 0o170000 == 0o120000 {
            continue;
        }
        if let Ok(blob) = git_repo.find_blob(entry.id) {
            if let Ok(content) = String::from_utf8(blob.content().to_vec()) {
                for (idx, line) in content.lines().enumerate() {
                    if line.contains(query) {
                        matches.push((path.clone(), idx + 1, line.to_string()));
                    }
                }
            }
        }
    }
    Ok(matches)
}
