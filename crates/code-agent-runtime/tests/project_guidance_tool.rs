use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::SecurityPolicy;
use code_agent_runtime::target::ReviewTarget;
use code_agent_runtime::tools::GetProjectGuidanceTool;
use serde_json::json;
use std::process::Command;

fn init_repo(path: &std::path::Path) {
    let status = Command::new("git")
        .args(["init"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(status.success());
}

#[test]
fn get_project_guidance_reads_root_documents() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("README.md"), "repo readme").unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "root guidance").unwrap();
    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = GetProjectGuidanceTool::new(
        repo,
        ReviewTarget::WorkingDirectory,
        SecurityPolicy::new(),
        65_536,
    );

    let result = (&tool as &dyn Tool).execute(&json!({"path":"."}));
    assert!(matches!(result.status, ToolStatus::Succeeded));
    let documents = result.value["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0]["kind"], "readme");
    assert_eq!(documents[1]["kind"], "agents");
}

#[test]
fn get_project_guidance_scopes_new_file_paths_to_parent_directory() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "root guidance").unwrap();
    std::fs::write(dir.path().join("src/AGENTS.md"), "src guidance").unwrap();
    std::fs::write(dir.path().join("src/nested/AGENTS.md"), "nested guidance").unwrap();

    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = GetProjectGuidanceTool::new(
        repo,
        ReviewTarget::WorkingDirectory,
        SecurityPolicy::new(),
        65_536,
    );
    let result = (&tool as &dyn Tool).execute(&json!({"path":"src/nested/new_file.rs"}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(result.value["scope"], "src/nested");
    let documents = result.value["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0]["path"], "src/nested/AGENTS.md");
}
