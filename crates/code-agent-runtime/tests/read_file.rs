use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::snapshot::resolve_targets;
use code_agent_runtime::tools::ReadFileTool;
use code_agent_runtime::target::ReviewTarget;
use code_agent_runtime::{repo::GitRepo, security::SecurityPolicy};
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
    let (_, head) = resolve_targets(&repo, "HEAD", "HEAD").unwrap();
    let tool = ReadFileTool::new(repo, head, usize::MAX, SecurityPolicy::new());

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
    let (_, limited_head) = resolve_targets(&limited_repo, "HEAD", "HEAD").unwrap();
    let byte_limit = 17;
    let limited_tool = ReadFileTool::new(
        limited_repo,
        limited_head,
        byte_limit,
        SecurityPolicy::new(),
    );
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

fn working_tool(content: &[u8], byte_limit: usize) -> (TempDir, ReadFileTool) {
    let (dir, _) = make_repo_with_head_file();
    std::fs::write(dir.path().join("README.md"), content).unwrap();
    let tool = ReadFileTool::new(GitRepo::open(dir.path()).unwrap(), ReviewTarget::WorkingDirectory, byte_limit, SecurityPolicy::new());
    (dir, tool)
}

#[test]
fn read_file_range_schema_and_validation_agree() {
    let (_dir, tool) = working_tool(b"one\ntwo\n", usize::MAX);
    let schema = tool.description().input_schema;
    assert_eq!(schema["required"], json!(["path"]));
    assert_eq!(schema["additionalProperties"], false);
    for key in ["offset", "limit"] {
        assert_eq!(schema["properties"][key]["type"], "integer");
        assert_eq!(schema["properties"][key]["minimum"], 1);
        assert_eq!(schema["properties"][key]["maximum"], json!(u64::MAX));
        for value in [json!(0), json!(-1), json!(1.5), json!(1.0), json!("2"), json!(null), json!(true), json!([]), json!({}), json!(18446744073709551616.0_f64)] {
            let mut arguments = json!({"path": "README.md"});
            arguments[key] = value;
            assert!(tool.validate_arguments(&arguments).is_err(), "{arguments}");
            let result = tool.execute(&arguments);
            assert!(matches!(result.status, ToolStatus::Failed), "{arguments}");
            assert!(result.value["error"].is_string());
        }
    }
    for arguments in [json!({"path": "README.md"}), json!({"path": "README.md", "offset": 2}), json!({"path": "README.md", "limit": 2}), json!({"path": "README.md", "offset": 1, "limit": u64::MAX})] {
        tool.validate_arguments(&arguments).unwrap();
    }
    for arguments in [json!(null), json!([]), json!({}), json!({"path": 2}), json!({"path": "README.md", "extra": 1})] {
        assert!(tool.validate_arguments(&arguments).is_err());
        assert!(matches!(tool.execute(&arguments).status, ToolStatus::Failed));
    }
}

#[test]
fn read_file_selects_committed_lines_and_preserves_full_file_identity() {
    let (dir, full_content) = make_repo_with_head_file();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (_, head) = resolve_targets(&repo, "HEAD", "HEAD").unwrap();
    let expected_head = head.label(&repo);
    let tool = ReadFileTool::new(repo, head, usize::MAX, SecurityPolicy::new());
    for (arguments, expected) in [
        (json!({"path": "README.md", "offset": 2, "limit": 2}), "HEAD line 02\nHEAD line 03\n"),
        (json!({"path": "README.md", "offset": 3}), "HEAD line 03\nHEAD line 04\n"),
        (json!({"path": "README.md", "limit": 1}), "HEAD line 01\n"),
        (json!({"path": "README.md", "offset": 1}), full_content.as_str()),
    ] {
        let result = tool.execute(&arguments);
        assert!(matches!(result.status, ToolStatus::Succeeded));
        assert_eq!(result.value["content"], expected);
        assert_eq!(result.value["content_id"], sha256_content_id(full_content.as_bytes()));
        assert_eq!(result.value["observed_head"], expected_head);
        assert_eq!(result.value["truncated"], false);
        assert_eq!(result.value["completeness"], true);
    }
}

#[test]
fn read_file_ranges_preserve_line_endings_blank_lines_and_unterminated_lines() {
    for (content, offset, limit, expected) in [
        ("one\r\n\r\nthree\r\nfour", 2, 2, "\r\nthree\r\n"),
        ("one\r\n\r\nthree\r\nfour", 4, 1, "four"),
        ("one\ntwo\n", 2, 1, "two\n"),
        ("one\ntwo", 2, 5, "two"),
        ("\n\nlast", 1, 2, "\n\n"),
        ("one\rtwo", 1, 1, "one\rtwo"),
        ("one\nλ雪\nlast", 2, 1, "λ雪\n"),
    ] {
        let (_dir, tool) = working_tool(content.as_bytes(), usize::MAX);
        let result = tool.execute(&json!({"path": "README.md", "offset": offset, "limit": limit}));
        assert!(matches!(result.status, ToolStatus::Succeeded));
        assert_eq!(result.value["content"], expected);
    }
}

#[test]
fn read_file_ranges_handle_empty_files_and_eof() {
    for (content, offset, expected) in [("", 1, ""), ("", 2, ""), ("one", 1, "one"), ("one", 2, ""), ("one\n", 2, ""), ("one\ntwo\n", 2, "two\n"), ("one\ntwo\n", 3, ""), ("one\ntwo\n", 100, "")] {
        let (_dir, tool) = working_tool(content.as_bytes(), usize::MAX);
        let result = tool.execute(&json!({"path": "README.md", "offset": offset, "limit": 10}));
        assert!(matches!(result.status, ToolStatus::Succeeded));
        assert_eq!(result.value["content"], expected);
        assert_eq!(result.value["truncated"], false);
        assert_eq!(result.value["completeness"], true);
    }
}

#[test]
fn read_file_ranges_accept_large_values_without_overflow() {
    let (_dir, tool) = working_tool(b"one\ntwo\n", usize::MAX);
    for (offset, limit, expected) in [(1, u64::MAX, "one\ntwo\n"), (2, u64::MAX, "two\n"), (u64::MAX, 1, ""), (u64::MAX, u64::MAX, "")] {
        let arguments = json!({"path": "README.md", "offset": offset, "limit": limit});
        tool.validate_arguments(&arguments).unwrap();
        let result = tool.execute(&arguments);
        assert!(matches!(result.status, ToolStatus::Succeeded));
        assert_eq!(result.value["content"], expected);
    }
}

#[test]
fn read_file_byte_cap_applies_after_line_selection_and_preserves_utf8_boundaries() {
    let content = "a long skipped line\nλ雪tail\nlast";
    let (_dir, tool) = working_tool(content.as_bytes(), 4);
    let result = tool.execute(&json!({"path": "README.md", "offset": 2, "limit": 1}));
    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(result.value["content"], "λ\n[truncated: 2 of 10 bytes]");
    assert_eq!(result.value["truncated"], true);
    assert_eq!(result.value["completeness"], false);
    assert_eq!(result.value["content_id"], sha256_content_id(content.as_bytes()));
    let last = tool.execute(&json!({"path": "README.md", "offset": 3, "limit": 1}));
    assert_eq!(last.value["content"], "last");
    assert_eq!(last.value["truncated"], false);
    assert_eq!(last.value["completeness"], true);
    let (_raw_dir, raw_tool) = working_tool(b"skip\n\xffraw\nend", usize::MAX);
    let raw = raw_tool.execute(&json!({"path": "README.md", "offset": 2, "limit": 1}));
    assert_eq!(raw.value["content"], "�raw\n");
    assert_eq!(raw.value["content_id"], sha256_content_id(b"skip\n\xffraw\nend"));
    let (_zero_dir, zero_tool) = working_tool(b"skip\nline", 0);
    let zero = zero_tool.execute(&json!({"path": "README.md", "offset": 2}));
    assert_eq!(zero.value["content"], "\n[truncated: 0 of 4 bytes]");
    assert_eq!(zero.value["truncated"], true);
    let empty = zero_tool.execute(&json!({"path": "README.md", "offset": 3}));
    assert_eq!(empty.value["content"], "");
    assert_eq!(empty.value["completeness"], true);
}

#[test]
fn read_file_ranges_respect_working_and_staged_targets() {
    let (dir, working) = working_tool(b"staged one\nstaged two\n", usize::MAX);
    assert!(Command::new("git").args(["add", "README.md"]).current_dir(dir.path()).output().unwrap().status.success());
    std::fs::write(dir.path().join("README.md"), b"working one\nworking two\n").unwrap();
    let staged = ReadFileTool::new(GitRepo::open(dir.path()).unwrap(), ReviewTarget::Index, usize::MAX, SecurityPolicy::new());
    let arguments = json!({"path": "README.md", "offset": 2, "limit": 1});
    let work_result = working.execute(&arguments);
    let staged_result = staged.execute(&arguments);
    assert!(matches!(work_result.status, ToolStatus::Succeeded));
    assert!(matches!(staged_result.status, ToolStatus::Succeeded));
    assert_eq!(work_result.value["content"], "working two\n");
    assert_eq!(staged_result.value["content"], "staged two\n");
    assert_eq!(work_result.value["observed_head"], "working");
    assert_ne!(work_result.value["content_id"], staged_result.value["content_id"]);
}

#[test]
fn read_file_ranges_retain_path_denial_and_missing_file_errors() {
    let (_dir, tool) = working_tool(b"one\ntwo\n", usize::MAX);
    for path in [".env", ".git/config", "../outside"] {
        let result = tool.execute(&json!({"path": path, "offset": 2, "limit": 1}));
        assert!(matches!(result.status, ToolStatus::Denied), "{path}: {:?}", result);
    }
    // Absolute paths must retain the path-only rejection, including its status.
    let absolute = tool.execute(&json!({"path": "/etc/passwd", "offset": 2, "limit": 1}));
    let legacy_absolute = tool.execute(&json!({"path": "/etc/passwd"}));
    assert!(!matches!(absolute.status, ToolStatus::Succeeded));
    assert_eq!(std::mem::discriminant(&absolute.status), std::mem::discriminant(&legacy_absolute.status));
    assert_eq!(absolute.value, legacy_absolute.value);
    assert!(absolute.value.get("content").is_none());
    let missing = tool.execute(&json!({"path": "missing.txt", "offset": 2, "limit": 1}));
    assert!(matches!(missing.status, ToolStatus::Failed));
    assert!(missing.value["error"].as_str().unwrap().contains("failed to read"));
}
