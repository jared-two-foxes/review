use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::SecurityPolicy;
use code_agent_runtime::snapshot::resolve_targets;
use code_agent_runtime::target::ReviewTarget;
use code_agent_runtime::tools::ListDirectoryTool;
use serde_json::json;
use std::process::Command;
use tempfile::TempDir;

fn make_repo_with_directory_entries() -> TempDir {
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

    std::fs::write(dir.path().join("a-file.txt"), "a\n").unwrap();
    std::fs::create_dir(dir.path().join("b-directory")).unwrap();
    std::fs::write(dir.path().join("b-directory/inside.txt"), "inside\n").unwrap();
    std::fs::write(dir.path().join("c-file.txt"), "c\n").unwrap();

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
                "directory entries",
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
fn list_directory_returns_typed_entries_bounded_by_entry_limit() {
    let dir = make_repo_with_directory_entries();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (_, head) = resolve_targets(&repo, "HEAD", "HEAD").unwrap();
    let policy = SecurityPolicy::with_entries_limit(2);
    let tool = ListDirectoryTool::new(repo, head, policy);

    assert_eq!(tool.name(), "list_directory");
    tool.validate_arguments(&json!({"path": "."})).unwrap();
    assert!(std::fs::write(dir.path().join("0-working-only.txt"), "working\n").is_ok());

    let result = (&tool as &dyn Tool).execute(&json!({"path": "."}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(
        result.value["entries"],
        json!([
            {"name": "a-file.txt", "type": "file"},
            {"name": "b-directory", "type": "dir"}
        ])
    );
    assert_eq!(result.value["truncated"], true);
    assert_eq!(result.value["completeness"], false);
}

#[test]
fn list_directory_rejects_denied_paths_before_tree_lookup() {
    let dir = make_repo_with_directory_entries();
    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = ListDirectoryTool::new(
        repo,
        ReviewTarget::Commit(git2::Oid::zero()),
        SecurityPolicy::new(),
    );

    let result = (&tool as &dyn Tool).execute(&json!({"path": ".git"}));

    assert!(matches!(result.status, ToolStatus::Denied));
    assert_eq!(result.value, json!({"error": "path denied"}));
    assert!(result.value.get("entries").is_none());
}

#[test]
fn list_directory_denies_rooted_paths_before_tree_lookup() {
    let dir = make_repo_with_directory_entries();
    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = ListDirectoryTool::new(
        repo,
        ReviewTarget::Commit(git2::Oid::zero()),
        SecurityPolicy::new(),
    );

    // Parent traversal is explicitly rejected by the path-denial predicate.
    // The invalid head makes ordering observable: resolving it first would
    // return Failed rather than the security boundary's Denied result.
    let result = (&tool as &dyn Tool).execute(&json!({"path": "../outside"}));

    assert!(matches!(result.status, ToolStatus::Denied));
    assert_eq!(result.value, json!({"error": "path denied"}));
    assert!(result.value.get("entries").is_none());
}
