use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::snapshot::resolve_commits;
use code_agent_runtime::tools::ReadFileTool;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::process::Command;
use tempfile::TempDir;

fn sha256_content_id(content: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(content)))
}

fn make_repo_with_head_file() -> (TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );

    let head_content = "HEAD line 01\nHEAD line 02\nHEAD line 03\nHEAD line 04\n".to_string();
    std::fs::write(dir.path().join("README.md"), &head_content).unwrap();
    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@test.com",
                "commit",
                "-m",
                "head",
            ])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );

    // Make the working tree disagree with HEAD. A read_file result must still
    // be sourced from the requested commit rather than from this file.
    std::fs::write(dir.path().join("README.md"), "working tree content\n").unwrap();
    (dir, head_content)
}

#[test]
fn read_file_returns_head_content_id_and_completeness_metadata() {
    let (dir, head_content) = make_repo_with_head_file();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (_, head) = resolve_commits(&repo, "HEAD", "HEAD").unwrap();
    let tool = ReadFileTool::new(repo, head, usize::MAX);

    assert_eq!(tool.name(), "read_file");
    tool.validate_arguments(&json!({"path": "README.md"}))
        .unwrap();
    assert!(
        tool.validate_arguments(&json!({"path": "README.md", "extra": true}))
            .is_err()
    );

    let result = (&tool as &dyn Tool).execute(&json!({"path": "README.md"}));
    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(result.value["content"], head_content);
    assert_eq!(
        result.value["content_id"],
        sha256_content_id(head_content.as_bytes())
    );
    assert_eq!(result.value["truncated"], false);
    assert_eq!(result.value["completeness"], true);

    let (limited_dir, limited_head_content) = make_repo_with_head_file();
    let limited_repo = GitRepo::open(limited_dir.path()).unwrap();
    let (_, limited_head) = resolve_commits(&limited_repo, "HEAD", "HEAD").unwrap();
    let byte_limit = 17;
    let limited_tool = ReadFileTool::new(limited_repo, limited_head, byte_limit);
    let limited = (&limited_tool as &dyn Tool).execute(&json!({"path": "README.md"}));

    assert!(matches!(limited.status, ToolStatus::Succeeded));
    assert_eq!(limited.value["truncated"], true);
    assert_eq!(limited.value["completeness"], false);
    assert_eq!(
        limited.value["content_id"],
        sha256_content_id(limited_head_content.as_bytes())
    );

    let content = limited.value["content"].as_str().unwrap();
    let marker = format!(
        "\n[truncated: {} of {} bytes]",
        byte_limit,
        limited_head_content.len()
    );
    let retained = content
        .strip_suffix(&marker)
        .expect("truncated output marker");
    assert_eq!(retained, &limited_head_content[..byte_limit]);
    assert!(retained.as_bytes().len() <= byte_limit);
}
