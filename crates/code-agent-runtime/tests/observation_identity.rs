use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::SecurityPolicy;
use code_agent_runtime::snapshot::{resolve_commits, snapshot_id};
use code_agent_runtime::tools::{
    GetChangeSummaryTool, GetChangedFilesTool, ListDirectoryTool, ReadDiffTool, ReadFileTool,
    SearchTextTool,
};
use serde_json::json;
use std::process::Command;
use tempfile::TempDir;

fn make_base_head_repo_with_dirt() -> (TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=t@t.com"])
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
                .status
                .success(),
            "git {args:?} failed"
        );
    };
    git(&["init"]);
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() { setup(); }\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "base"]);
    // head commit: change main.rs, add search.rs
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() { setup(); retry(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/search.rs"),
        "pub fn find() { /* marker */ }\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "head"]);
    // working-directory dirt: untracked file the head tree must NOT see
    std::fs::write(dir.path().join("src/untracked.rs"), "// dirt\n").unwrap();

    let repo = GitRepo::open(dir.path()).unwrap();
    let (base, head) = resolve_commits(&repo, "HEAD~1", "HEAD").unwrap();
    (dir, base.to_string(), head.to_string())
}

fn open_repo(dir: &TempDir) -> GitRepo {
    GitRepo::open(dir.path()).unwrap()
}

fn assert_success(result: agent_kernel::tools::ToolResult) -> serde_json::Value {
    assert!(
        matches!(result.status, ToolStatus::Succeeded),
        "tool failed: {}",
        result.value
    );
    result.value
}

#[test]
fn all_six_tools_report_consistent_observation_identity() {
    let (dir, base_hex, head_hex) = make_base_head_repo_with_dirt();
    let base_oid: git2::Oid = base_hex.parse().unwrap();
    let head_oid: git2::Oid = head_hex.parse().unwrap();
    let expected_snapshot = snapshot_id(base_oid, head_oid);

    // Range tools: snapshot_id == snapshot_id(base, head)
    let summary = GetChangeSummaryTool::new(open_repo(&dir), oid(&base_hex), oid(&head_hex));
    let value = assert_success(summary.execute(&json!({})));
    assert_eq!(
        value["snapshot_id"], expected_snapshot,
        "get_change_summary identity"
    );

    let files = GetChangedFilesTool::new(open_repo(&dir), oid(&base_hex), oid(&head_hex));
    let value = assert_success(files.execute(&json!({})));
    assert_eq!(
        value["snapshot_id"], expected_snapshot,
        "get_changed_files identity"
    );

    let diff = ReadDiffTool::new(open_repo(&dir), oid(&base_hex), oid(&head_hex), 65_536);
    let value = assert_success(diff.execute(&json!({"path": "src/main.rs"})));
    assert_eq!(
        value["snapshot_id"], expected_snapshot,
        "read_diff identity"
    );

    // Head-only tools: observed_head == head hex, and dirt is invisible
    let read = ReadFileTool::new(open_repo(&dir), oid(&head_hex), 65_536);
    let value = assert_success(read.execute(&json!({"path": "src/main.rs"})));
    assert_eq!(value["observed_head"], head_hex, "read_file identity");

    let list = ListDirectoryTool::new(open_repo(&dir), oid(&head_hex), SecurityPolicy::new());
    let value = assert_success(list.execute(&json!({"path": "src"})));
    assert_eq!(value["observed_head"], head_hex, "list_directory identity");
    let names: Vec<&str> = value["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        names.contains(&"main.rs") && names.contains(&"search.rs"),
        "head-tree entries: {names:?}"
    );
    assert!(
        !names.contains(&"untracked.rs"),
        "working-directory dirt must be invisible: {names:?}"
    );

    let search = SearchTextTool::new(open_repo(&dir), oid(&head_hex), SecurityPolicy::new());
    let value = assert_success(search.execute(&json!({"query": "marker"})));
    assert_eq!(value["observed_head"], head_hex, "search_text identity");
    let paths: Vec<&str> = value["matches"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["path"].as_str())
        .collect();
    assert!(
        paths.contains(&"src/search.rs"),
        "committed match found: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains("untracked")),
        "dirt must not match: {paths:?}"
    );
}

fn oid(hex: &str) -> git2::Oid {
    hex.parse().unwrap()
}
