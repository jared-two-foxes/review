use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::SecurityPolicy;
use code_agent_runtime::snapshot::resolve_commits;
use code_agent_runtime::tools::SearchTextTool;
use git2::Oid;
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
    let (_, head) = resolve_commits(&repo, "HEAD", "HEAD").unwrap();
    let tool = SearchTextTool::new(repo, head, SecurityPolicy::with_matches_limit(2));

    assert_eq!(tool.name(), "search_text");
    tool.validate_arguments(&json!({"query": "foo.bar"}))
        .unwrap();
    assert!(
        tool.validate_arguments(&json!({"query": "foo.bar", "extra": true}))
            .is_err()
    );
    std::fs::write(
        dir.path().join("src/one.txt"),
        "working tree foo.bar match\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/working-only.txt"),
        "working foo.bar match\n",
    )
    .unwrap();

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
    let (_, head) = resolve_commits(&repo, "HEAD", "HEAD").unwrap();
    let tool = SearchTextTool::new(repo, head, SecurityPolicy::with_matches_limit(3));

    std::fs::write(
        dir.path().join("src/one.txt"),
        "working tree foo.bar match\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/working-only.txt"),
        "working foo.bar match\n",
    )
    .unwrap();

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

#[test]
fn search_text_ignores_symlink_targets() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "git {:?}: {:?}", args, output);
        output
    };

    git(&["init"]);
    std::fs::write(dir.path().join("needle.txt"), "ordinary content\n").unwrap();
    // The blob content is a target path containing the query. Install it in
    // the index with Git's symlink mode so the fixture does not need an OS
    // filesystem symlink (and remains portable on Windows).
    std::fs::write(dir.path().join("link.txt"), "needle.txt\n").unwrap();
    git(&["add", "."]);
    let blob_oid = String::from_utf8(git(&["hash-object", "link.txt"]).stdout)
        .unwrap()
        .trim()
        .to_string();
    git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("120000,{},link.txt", blob_oid),
    ]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@test.com",
        "commit",
        "-m",
        "symlink fixture",
    ]);

    let repo = GitRepo::open(dir.path()).unwrap();
    let (_, head) = resolve_commits(&repo, "HEAD", "HEAD").unwrap();
    let tool = SearchTextTool::new(repo, head, SecurityPolicy::default());

    let result = (&tool as &dyn Tool).execute(&json!({"query": "needle.txt"}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(result.value["matches"], json!([]));
}

#[test]
fn search_text_returns_failed_result_when_head_tree_cannot_be_resolved() {
    let dir = make_repo_with_searchable_text();
    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = SearchTextTool::new(repo, Oid::zero(), SecurityPolicy::default());

    let result = (&tool as &dyn Tool).execute(&json!({"query": "foo.bar"}));

    assert!(matches!(result.status, ToolStatus::Failed));
    assert_eq!(result.value, json!({"error": "failed to resolve head tree"}));
}
