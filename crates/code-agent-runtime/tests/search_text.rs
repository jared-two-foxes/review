use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::SecurityPolicy;
use code_agent_runtime::tools::SearchTextTool;
use serde_json::json;
use std::process::Command;
use tempfile::TempDir;

fn make_repo_with_searchable_text() -> TempDir {
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
        dir.path().join("src/one.txt"),
        "first foo.bar match\nfooXbar must not match\nsecond foo.bar match\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/two.txt"),
        "third foo.bar match\nfourth foo.bar match\n",
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
                "search fixtures",
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
fn search_text_returns_literal_structured_matches_with_completeness() {
    let dir = make_repo_with_searchable_text();
    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = SearchTextTool::new(repo, SecurityPolicy::with_matches_limit(2));

    assert_eq!(tool.name(), "search_text");
    tool.validate_arguments(&json!({"query": "foo.bar"}))
        .unwrap();
    assert!(
        tool.validate_arguments(&json!({"query": "foo.bar", "extra": true}))
            .is_err()
    );

    let result = (&tool as &dyn Tool).execute(&json!({"query": "foo.bar"}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(
        result.value["matches"],
        json!([
            {"path": "src/one.txt", "line": 1, "content": "first foo.bar match"},
            {"path": "src/one.txt", "line": 3, "content": "second foo.bar match"}
        ])
    );
    assert_eq!(result.value["completeness"], false);
}

#[test]
fn search_text_bounds_matches_and_reports_incompleteness() {
    let dir = make_repo_with_searchable_text();
    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = SearchTextTool::new(repo, SecurityPolicy::with_matches_limit(3));

    let result = (&tool as &dyn Tool).execute(&json!({"query": "foo.bar"}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(
        result.value["matches"],
        json!([
            {"path": "src/one.txt", "line": 1, "content": "first foo.bar match"},
            {"path": "src/one.txt", "line": 3, "content": "second foo.bar match"},
            {"path": "src/two.txt", "line": 1, "content": "third foo.bar match"}
        ])
    );
    assert_eq!(result.value["completeness"], false);
}
