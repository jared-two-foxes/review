use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[test]
fn valid_request_file_produces_json_result() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let request_path =
        std::env::temp_dir().join(format!("implement-cli-request-{}.json", std::process::id()));
    fs::write(
        &request_path,
        format!(
            r#"{{
  "repository_path": "{}",
  "target_path": "README.md",
  "expected_content": "before",
  "desired_content": "after"
}}"#,
            workspace_root.display()
        ),
    )
    .expect("write request fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args([
            "run",
            "--request",
            request_path.to_str().unwrap(),
            "--format",
            "json",
        ])
        .env("OPENAI_API_KEY", "fake-openai-key")
        .env("RUST_LOG", "error")
        .output()
        .expect("implement CLI should be executable");
    fs::remove_file(&request_path).ok();

    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::Indeterminate as i32)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(result["status"], "Indeterminate");
    assert_eq!(result["target_path"], "README.md");
    assert!(result.get("implementation_id").is_some());
    assert!(result.get("completed_at").is_some());
}

#[test]
fn demo_args_produce_json_result() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args([
            "run",
            "--repository",
            workspace_root.to_str().unwrap(),
            "--target-path",
            "README.md",
            "--expected-content",
            "before",
            "--desired-content",
            "after",
            "--format",
            "json",
        ])
        .env("OPENAI_API_KEY", "fake-openai-key")
        .output()
        .expect("implement CLI should be executable");

    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::Indeterminate as i32)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(result["status"], "Indeterminate");
    assert_eq!(result["target_path"], "README.md");
}
