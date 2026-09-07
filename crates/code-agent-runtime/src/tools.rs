use agent_kernel::model::ToolDescription;
use agent_kernel::tools::{Tool, ToolResult, ToolStatus};
use git2::Oid;
use serde_json::{Value, json};
use sha2::Digest;

use crate::diff::{FileStatus, changed_files, read_diff};
use crate::repo::GitRepo;
use crate::security::{SecurityPolicy, bound_bytes};

/// Minimal API scaffold for the change-summary tool. The implementation is
/// intentionally incomplete; the behavior is supplied by the production work
/// driven by the integration test.
pub struct GetChangeSummaryTool {
    repo: GitRepo,
    base: Oid,
    head: Oid,
}

impl GetChangeSummaryTool {
    pub fn new(repo: GitRepo, base: Oid, head: Oid) -> Self {
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
        let repo = self.repo.raw_repo();
        let base_tree = repo.find_commit(self.base).unwrap().tree().unwrap();
        let head_tree = repo.find_commit(self.head).unwrap().tree().unwrap();
        let mut opts = git2::DiffOptions::new();
        let diff = repo
            .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))
            .unwrap();

        let files = changed_files(&self.repo, self.base, self.head).unwrap_or_default();
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
                "snapshot_id": crate::snapshot::snapshot_id(self.base, self.head),
            }),
        }
    }
}

pub struct GetChangedFilesTool {
    repo: GitRepo,
    base: Oid,
    head: Oid,
}

impl GetChangedFilesTool {
    pub fn new(repo: GitRepo, base: Oid, head: Oid) -> Self {
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
        let changes = match changed_files(&self.repo, self.base, self.head) {
            Ok(c) => c,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "failed to enumerate changes"}),
                };
            }
        };
        let git_repo = self.repo.raw_repo();
        let head_tree = match git_repo.find_commit(self.head).and_then(|c| c.tree()) {
            Ok(t) => t,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "failed to resolve head tree"}),
                };
            }
        };

        let files: Vec<Value> = changes
            .iter()
            .map(|change| {
                let content_id = if change.status == FileStatus::Deleted {
                    Value::Null
                } else {
                    head_tree
                        .get_path(std::path::Path::new(&change.path))
                        .and_then(|entry| entry.to_object(git_repo))
                        .ok()
                        .and_then(|obj| obj.as_blob().map(|blob| blob.content().to_vec()))
                        .map(|content| {
                            let digest = sha2::Sha256::digest(&content);
                            Value::String(format!("sha256:{}", hex::encode(digest)))
                        })
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
                "snapshot_id": crate::snapshot::snapshot_id(self.base, self.head),
            }),
        }
    }
}

/// Additive API scaffold for the read-diff tool exercised by the integration
/// test. Its behavior is intentionally stubbed until the tool implementation
/// is supplied.
pub struct ReadDiffTool {
    repo: GitRepo,
    base: Oid,
    head: Oid,
    byte_limit: usize,
}

impl ReadDiffTool {
    pub fn new(repo: GitRepo, base: Oid, head: Oid, byte_limit: usize) -> Self {
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
        match read_diff(&self.repo, self.base, self.head, path, self.byte_limit) {
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
                        "snapshot_id": crate::snapshot::snapshot_id(self.base, self.head),
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
    head: Oid,
    byte_limit: usize,
}

impl ReadFileTool {
    pub fn new(repo: GitRepo, head: Oid, byte_limit: usize) -> Self {
        Self {
            repo,
            head,
            byte_limit,
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
            description: "Read a file from the repository head commit.".to_string(),
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
        let git_repo = self.repo.raw_repo();
        let head_tree = match git_repo.find_commit(self.head).and_then(|c| c.tree()) {
            Ok(t) => t,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "failed to resolve head tree"}),
                };
            }
        };
        let blob = match head_tree
            .get_path(std::path::Path::new(path))
            .and_then(|entry| entry.to_object(git_repo))
        {
            Ok(obj) => obj,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "file not found at head"}),
                };
            }
        };
        let content_bytes = blob
            .as_blob()
            .map(|b| b.content().to_vec())
            .unwrap_or_default();
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
                "observed_head": self.head.to_string(),
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

/// Additive compile-time scaffold for the directory-listing tool. The tool
/// remains deliberately stubbed so its integration test fails until the
/// bounded, typed listing behavior is implemented.
pub struct ListDirectoryTool {
    repo: GitRepo,
    head: Oid,
    policy: crate::security::SecurityPolicy,
}

impl ListDirectoryTool {
    pub fn new(repo: GitRepo, head: Oid, policy: crate::security::SecurityPolicy) -> Self {
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

        let git_repo = self.repo.raw_repo();
        let head_tree = match git_repo.find_commit(self.head).and_then(|c| c.tree()) {
            Ok(t) => t,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "failed to resolve head tree"}),
                };
            }
        };

        let mut entries: Vec<(String, String)> = if path == "." || path.is_empty() {
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
                "observed_head": self.head.to_string(),
            }),
        }
    }
}

pub struct SearchTextTool {
    repo: GitRepo,
    head: Oid,
    policy: crate::security::SecurityPolicy,
}

impl SearchTextTool {
    pub fn new(repo: GitRepo, head: Oid, policy: crate::security::SecurityPolicy) -> Self {
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
        let repo = self.repo.raw_repo();
        let tree = match repo.find_commit(self.head).and_then(|c| c.tree()) {
            Ok(t) => t,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "failed to resolve head tree"}),
                };
            }
        };
        let mut matches: Vec<(String, usize, String)> = Vec::new();
        search_tree_walk(repo, &tree, "", query, &self.policy, &mut matches);
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
                "observed_head": self.head.to_string(),
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
