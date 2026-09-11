use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::snapshot::resolve_targets;
use code_agent_runtime::tools::GetChangedFilesTool;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::process::Command;
use tempfile::TempDir;

fn sha256_content_id(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    format!("sha256:{}", hex::encode(digest))
}

fn make_repo_with_head_changes() -> TempDir {
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

    std::fs::write(dir.path().join("modified.txt"), "base\n").unwrap();
    std::fs::write(dir.path().join("deleted.txt"), "removed from head\n").unwrap();
    std::fs::write(
        dir.path().join("renamed-old.txt"),
        "same content after rename\n",
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
                "initial",
            ])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );

    std::fs::write(dir.path().join("modified.txt"), "head content\n").unwrap();
    std::fs::write(dir.path().join("added.txt"), "new head file\n").unwrap();
    std::fs::remove_file(dir.path().join("deleted.txt")).unwrap();
    std::fs::rename(
        dir.path().join("renamed-old.txt"),
        dir.path().join("renamed-new.txt"),
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", "-A"])
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
                "changes",
            ])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );

    // Keep HEAD's contents distinct from the working tree. The tool must read
    // content IDs from the requested head commit, not from these modifications.
    std::fs::write(dir.path().join("modified.txt"), "working tree content\n").unwrap();
    std::fs::write(dir.path().join("added.txt"), "working tree replacement\n").unwrap();
    std::fs::write(
        dir.path().join("renamed-new.txt"),
        "working tree rename content\n",
    )
    .unwrap();

    dir
}

#[test]
fn get_changed_files_reports_head_content_ids_and_statuses() {
    let dir = make_repo_with_head_changes();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (base, head) = resolve_targets(&repo, "HEAD~1", "HEAD").unwrap();
    let tool = GetChangedFilesTool::new(repo, base, head);

    assert_eq!(tool.name(), "get_changed_files");
    tool.validate_arguments(&json!({})).unwrap();
    let result = (&tool as &dyn Tool).execute(&json!({}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    let files = result.value["files"].as_array().unwrap();
    assert_eq!(files.len(), 4, "every changed file must be returned");

    let file = |path: &str| {
        files
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("missing changed file {path}: {files:?}"))
    };

    assert_eq!(
        file("added.txt"),
        &json!({
            "path": "added.txt",
            "status": "added",
            "content_id": sha256_content_id(b"new head file\n")
        })
    );
    assert_eq!(
        file("modified.txt"),
        &json!({
            "path": "modified.txt",
            "status": "modified",
            "content_id": sha256_content_id(b"head content\n")
        })
    );
    assert_eq!(
        file("renamed-new.txt"),
        &json!({
            "path": "renamed-new.txt",
            "status": "renamed",
            "content_id": sha256_content_id(b"same content after rename\n")
        })
    );
    assert_eq!(
        file("deleted.txt"),
        &json!({
            "path": "deleted.txt",
            "status": "deleted",
            "content_id": null
        })
    );
}
