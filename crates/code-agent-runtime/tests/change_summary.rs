use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::snapshot::resolve_targets;
use code_agent_runtime::tools::GetChangeSummaryTool;
use serde_json::json;
use std::process::Command;
use tempfile::TempDir;

fn make_repo_with_changes() -> TempDir {
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
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("src/obsolete.rs"), "pub fn obsolete() {}\n").unwrap();
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
                "initial",
            ])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );

    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/new.rs"), "pub fn new_fn() {}\n").unwrap();
    std::fs::remove_file(dir.path().join("src/obsolete.rs")).unwrap();
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
                "second",
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
fn get_change_summary_reports_totals_and_file_statuses() {
    let dir = make_repo_with_changes();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (base, head) = resolve_targets(&repo, "HEAD~1", "HEAD").unwrap();
    let tool = GetChangeSummaryTool::new(repo, base, head);

    assert_eq!(tool.name(), "get_change_summary");
    tool.validate_arguments(&json!({})).unwrap();

    // Invoke the Tool trait object itself so this verifies the public tool
    // execution path rather than reproducing the summary with lower-level APIs.
    let result = (&tool as &dyn Tool).execute(&json!({}));
    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(result.value["changed_file_count"], 3);
    assert_eq!(result.value["total_insertions"], 2);
    assert_eq!(result.value["total_deletions"], 2);
    assert_eq!(
        result.value["files"],
        json!([
            {"path": "src/main.rs", "status": "modified"},
            {"path": "src/new.rs", "status": "added"},
            {"path": "src/obsolete.rs", "status": "deleted"}
        ])
    );
}
