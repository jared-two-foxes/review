use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::snapshot::resolve_commits;
use code_agent_runtime::tools::ReadDiffTool;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::process::Command;
use tempfile::TempDir;

fn make_repo_with_changed_file() -> TempDir {
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
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    println!(\"base\");\n}\n",
    )
    .unwrap();
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
                "base",
            ])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );

    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    println!(\"changed\");\n    println!(\"added\");\n}\n",
    )
    .unwrap();
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
    dir
}

#[test]
fn read_diff_returns_line_numbered_content_id_and_bounded_output() {
    let complete_dir = make_repo_with_changed_file();
    let complete_repo = GitRepo::open(complete_dir.path()).unwrap();
    let (base, head) = resolve_commits(&complete_repo, "HEAD~1", "HEAD").unwrap();

    let complete_tool = ReadDiffTool::new(complete_repo, base, head, usize::MAX);
    assert_eq!(complete_tool.name(), "read_diff");
    complete_tool
        .validate_arguments(&json!({"path": "src/main.rs"}))
        .unwrap();
    assert!(
        complete_tool
            .validate_arguments(&json!({"path": "src/main.rs", "extra": true}))
            .is_err()
    );

    let complete = (&complete_tool as &dyn Tool).execute(&json!({"path": "src/main.rs"}));
    assert!(matches!(complete.status, ToolStatus::Succeeded));
    assert_eq!(complete.value["path"], "src/main.rs");
    assert_eq!(complete.value["truncated"], false);
    assert!(
        complete.value["content"]
            .as_str()
            .unwrap()
            .contains("@@ -1,3 +1,4 @@")
    );
    let complete_content = complete.value["content"].as_str().unwrap();
    let expected_id = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(complete_content.as_bytes()))
    );
    assert_eq!(complete.value["content_id"], expected_id);

    let limit = complete_content
        .find("+    println!(\"added\");")
        .map(|offset| offset + 5)
        .unwrap();
    let limited_dir = make_repo_with_changed_file();
    let limited_repo = GitRepo::open(limited_dir.path()).unwrap();
    let (limited_base, limited_head) = resolve_commits(&limited_repo, "HEAD~1", "HEAD").unwrap();
    let limited_tool = ReadDiffTool::new(limited_repo, limited_base, limited_head, limit);
    let limited = (&limited_tool as &dyn Tool).execute(&json!({"path": "src/main.rs"}));
    assert!(matches!(limited.status, ToolStatus::Succeeded));
    assert_eq!(limited.value["truncated"], true);

    let limited_content = limited.value["content"].as_str().unwrap();
    let marker = format!("\n[truncated at {} bytes]", limit);
    assert!(limited_content.ends_with(&marker));
    let bounded_content = limited_content.strip_suffix(&marker).unwrap();
    assert!(bounded_content.as_bytes().len() <= limit);

    let expected_limited_id = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(limited_content.as_bytes()))
    );
    assert_eq!(limited.value["content_id"], expected_limited_id);
}
