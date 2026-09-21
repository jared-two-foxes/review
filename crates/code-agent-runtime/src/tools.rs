use agent_kernel::model::ToolDescription;
use agent_kernel::tools::{Tool, ToolResult, ToolStatus};
use serde_json::{Value, json};
use std::io::Write;
use tempfile::NamedTempFile;

use crate::diff::{FileStatus, changed_files, compute_diff, read_diff};
use crate::identity::content_id_for_bytes;
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
                        .map(|content| content_id_for_bytes(&content))
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
                let content_id = content_id_for_bytes(diff.content.as_bytes());
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
        let content_id = format!("{}", content_id_for_bytes(&content_bytes));
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

pub struct ReplaceFileContentTool {
    repo: GitRepo,
    policy: SecurityPolicy,
}

impl ReplaceFileContentTool {
    pub fn new(repo: GitRepo, policy: SecurityPolicy) -> Self {
        Self { repo, policy }
    }
}

impl Tool for ReplaceFileContentTool {
    fn name(&self) -> &str {
        "replace_file_content"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().to_string(),
            description: "Replace the full content of one repository file when the current content exactly matches an expected precondition.".to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path", "expected", "replacement"],
                "properties": {
                    "path": {"type": "string"},
                    "expected": {"type": "string"},
                    "replacement": {"type": "string"}
                }
            }),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.get("path").and_then(Value::as_str).is_some()
            && arguments.get("expected").and_then(Value::as_str).is_some()
            && arguments
                .get("replacement")
                .and_then(Value::as_str)
                .is_some()
            && arguments
                .as_object()
                .is_some_and(|object| object.len() == 3)
        {
            Ok(())
        } else {
            Err(
                "arguments must contain only string path, expected, and replacement fields"
                    .to_string(),
            )
        }
    }

    fn execute(&self, arguments: &Value) -> ToolResult {
        let path = match arguments.get("path").and_then(Value::as_str) {
            Some(path) => path,
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
        let expected = match arguments.get("expected").and_then(Value::as_str) {
            Some(expected) => expected,
            None => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "missing expected argument"}),
                };
            }
        };
        let replacement = match arguments.get("replacement").and_then(Value::as_str) {
            Some(replacement) => replacement,
            None => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "missing replacement argument"}),
                };
            }
        };
        let full_path = match self.policy.validate_path(&self.repo, path) {
            Ok(path) => path,
            Err(_) => {
                return ToolResult {
                    status: ToolStatus::Denied,
                    value: json!({"error": "path denied"}),
                };
            }
        };
        let current_bytes = match std::fs::read(&full_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": format!("failed to read file: {}", error)}),
                };
            }
        };
        if current_bytes != expected.as_bytes() {
            return ToolResult {
                status: ToolStatus::Failed,
                value: json!({
                    "error": "content precondition did not match",
                    "path": path,
                    "content_id": content_id_for_bytes(&current_bytes),
                }),
            };
        }
        let parent = match full_path.parent() {
            Some(parent) => parent,
            None => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "failed to derive parent directory"}),
                };
            }
        };
        let permissions = match std::fs::metadata(&full_path) {
            Ok(metadata) => metadata.permissions(),
            Err(error) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": format!("failed to read file metadata: {}", error)}),
                };
            }
        };
        let mut temp_file = match NamedTempFile::new_in(parent) {
            Ok(temp_file) => temp_file,
            Err(error) => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": format!("failed to create temp file: {}", error)}),
                };
            }
        };
        if let Err(error) = temp_file
            .write_all(replacement.as_bytes())
            .and_then(|_| temp_file.as_file_mut().sync_all())
            .and_then(|_| std::fs::set_permissions(temp_file.path(), permissions))
        {
            return ToolResult {
                status: ToolStatus::Failed,
                value: json!({"error": format!("failed to write file: {}", error)}),
            };
        }
        if let Err(error) = temp_file.persist(&full_path) {
            return ToolResult {
                status: ToolStatus::Failed,
                value: json!({"error": format!("failed to write file: {}", error.error)}),
            };
        }
        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "path": path,
                "content_id": content_id_for_bytes(replacement.as_bytes()),
                "bytes_written": replacement.len(),
                "observed_head": "working",
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
            ReviewTarget::Empty => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "cannot list directory in empty review target"}),
                };
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
                "observed_head": self.head.label(&self.repo),
            }),
        }
    }
}

pub struct GetProjectGuidanceTool {
    repo: GitRepo,
    head: ReviewTarget,
    policy: SecurityPolicy,
    byte_limit: usize,
}

impl GetProjectGuidanceTool {
    pub fn new(
        repo: GitRepo,
        head: ReviewTarget,
        policy: SecurityPolicy,
        byte_limit: usize,
    ) -> Self {
        Self {
            repo,
            head,
            policy,
            byte_limit,
        }
    }
}

impl Tool for GetProjectGuidanceTool {
    fn name(&self) -> &str {
        "get_project_guidance"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().to_string(),
            description: "Read repository guidance documents (README and AGENTS) relevant to a path. Use this to decide whether to explore additional directories.".to_string(),
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
        let requested_path = match arguments.get("path").and_then(Value::as_str) {
            Some(path) => path,
            None => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "missing path argument"}),
                };
            }
        };

        if self.policy.is_path_denied(requested_path) {
            return ToolResult {
                status: ToolStatus::Denied,
                value: json!({"error": "path denied"}),
            };
        }

        let guidance_scope = guidance_scope_for_path(&self.repo, &self.head, requested_path);
        if self.policy.is_path_denied(&guidance_scope) {
            return ToolResult {
                status: ToolStatus::Denied,
                value: json!({"error": "path denied"}),
            };
        }

        let mut documents = vec![];
        for readme in ["README.md", "README", "readme.md", "readme"] {
            if let Some(document) = read_guidance_document(
                &self.repo,
                &self.head,
                &self.policy,
                self.byte_limit,
                "readme",
                readme,
            ) {
                documents.push(document);
                break;
            }
        }

        for scope in guidance_scopes_to_root(&guidance_scope) {
            for candidate in guidance_agents_candidates(&scope) {
                if let Some(document) = read_guidance_document(
                    &self.repo,
                    &self.head,
                    &self.policy,
                    self.byte_limit,
                    "agents",
                    &candidate,
                ) {
                    documents.push(document);
                    break;
                }
            }
        }

        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "path": requested_path,
                "scope": guidance_scope,
                "documents": documents,
                "observed_head": self.head.label(&self.repo),
            }),
        }
    }
}

fn guidance_scope_for_path(repo: &GitRepo, head: &ReviewTarget, path: &str) -> String {
    if path == "." || path.is_empty() {
        return ".".to_string();
    }
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return ".".to_string();
    }

    if head.read_file(repo, path).is_ok() {
        return parent_scope_or_root(path);
    }

    if repo.root().join(path).is_dir() {
        return path.to_string();
    }

    parent_scope_or_root(path)
}

fn guidance_scopes_to_root(scope: &str) -> Vec<String> {
    if scope == "." || scope.is_empty() {
        return vec![".".to_string()];
    }

    let mut scopes = vec![".".to_string()];
    let mut current = std::path::PathBuf::new();
    for component in std::path::Path::new(scope).components() {
        if let std::path::Component::Normal(part) = component {
            current.push(part);
            scopes.push(current.to_string_lossy().replace('\\', "/"));
        }
    }
    scopes
}

fn guidance_agents_candidates(scope: &str) -> Vec<String> {
    if scope == "." || scope.is_empty() {
        vec!["AGENTS.md".to_string(), "agents.md".to_string()]
    } else {
        vec![format!("{scope}/AGENTS.md"), format!("{scope}/agents.md")]
    }
}

fn parent_scope_or_root(path: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .and_then(|parent| parent.to_str())
        .filter(|parent| !parent.is_empty())
        .map(|parent| parent.to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn read_guidance_document(
    repo: &GitRepo,
    head: &ReviewTarget,
    policy: &SecurityPolicy,
    byte_limit: usize,
    kind: &str,
    path: &str,
) -> Option<Value> {
    if policy.is_path_denied(path) {
        return None;
    }
    let bytes = head.read_file(repo, path).ok()?;
    let content_id = content_id_for_bytes(&bytes);
    let bounded = bound_bytes(&bytes, byte_limit);
    Some(json!({
        "kind": kind,
        "path": path,
        "content": bounded.content,
        "truncated": bounded.truncated,
        "completeness": bounded.completeness,
        "content_id": content_id,
    }))
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
                match search_with_ripgrep(&self.repo, query, &self.policy) {
                    Ok(m) => m,
                    Err(_) => {
                        return ToolResult {
                            status: ToolStatus::Failed,
                            value: json!({"error": "failed to search working directory"}),
                        };
                    }
                }
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
            ReviewTarget::Empty => {
                return ToolResult {
                    status: ToolStatus::Failed,
                    value: json!({"error": "cannot search in empty review target"}),
                };
            }
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
        if let Ok(blob) = git_repo.find_blob(entry.id)
            && let Ok(content) = String::from_utf8(blob.content().to_vec())
        {
            for (idx, line) in content.lines().enumerate() {
                if line.contains(query) {
                    matches.push((path.clone(), idx + 1, line.to_string()));
                }
            }
        }
    }
    Ok(matches)
}

fn search_with_ripgrep(
    repo: &GitRepo,
    query: &str,
    policy: &SecurityPolicy,
) -> Result<Vec<(String, usize, String)>, crate::error::RepoError> {
    use ripgrep_api::SearchBuilder;

    let escaped_query = regex::escape(query);
    let root = repo.root();

    let matches: Vec<_> = SearchBuilder::new(&escaped_query)
        .path(root)
        .build()
        .map_err(|e| crate::error::RepoError::Other(format!("search failed: {}", e)))?
        .collect();

    let root_str = root.to_string_lossy();
    let mut results = Vec::new();
    for mat in matches {
        let abs_path = mat.path.to_string_lossy().to_string();
        let rel_path = abs_path
            .strip_prefix(&*root_str)
            .unwrap_or(&abs_path)
            .trim_start_matches(['/', '\\'])
            .to_string();

        if policy.is_path_denied(&rel_path) {
            continue;
        }

        results.push((
            rel_path,
            mat.line.unwrap_or(0) as usize,
            String::from_utf8_lossy(mat.text.as_ref()).into_owned(),
        ));
    }
    Ok(results)
}
