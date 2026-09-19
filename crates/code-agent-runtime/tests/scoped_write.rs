use agent_kernel::tools::Tool;
use code_agent_runtime::capabilities::{CodeToolCatalog, ReadOnly, ScopedWrite};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::SecurityPolicy;
use code_agent_runtime::tools::{ReadFileTool, ReplaceFileContentTool};
use std::process::Command;

fn make_repo() -> tempfile::TempDir {
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
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/app.txt"), "before\n").unwrap();
    dir
}

#[test]
fn scoped_write_tool_replaces_file_content_when_precondition_matches() {
    let dir = make_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = ReplaceFileContentTool::new(repo, SecurityPolicy::new());
    tool.validate_arguments(&serde_json::json!({
        "path": "src/app.txt",
        "expected": "before\n",
        "replacement": "after\n"
    }))
    .unwrap();

    let result = (&tool as &dyn Tool).execute(&serde_json::json!({
        "path": "src/app.txt",
        "expected": "before\n",
        "replacement": "after\n"
    }));

    assert!(matches!(
        result.status,
        agent_kernel::tools::ToolStatus::Succeeded
    ));
    assert_eq!(result.value["path"], "src/app.txt");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/app.txt")).unwrap(),
        "after\n"
    );
}

#[test]
fn typed_code_tool_catalog_allows_read_only_and_scoped_write_separately() {
    let dir = make_repo();
    let read_repo = GitRepo::open(dir.path()).unwrap();
    let write_repo = GitRepo::open(dir.path()).unwrap();

    let mut read_only = CodeToolCatalog::<ReadOnly>::new();
    read_only.register(ReadFileTool::new(
        read_repo,
        code_agent_runtime::target::ReviewTarget::WorkingDirectory,
        usize::MAX,
        SecurityPolicy::new(),
    ));
    assert_eq!(read_only.into_inner().descriptions().len(), 1);

    let mut scoped_write = CodeToolCatalog::<ScopedWrite>::new();
    scoped_write.register(ReplaceFileContentTool::new(
        write_repo,
        SecurityPolicy::new(),
    ));
    assert_eq!(scoped_write.into_inner().descriptions().len(), 1);
}
