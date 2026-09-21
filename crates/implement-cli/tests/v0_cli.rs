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
    let repository_path = serde_json::to_string(workspace_root).unwrap();
    let request_path =
        std::env::temp_dir().join(format!("implement-cli-request-{}.json", std::process::id()));
    fs::write(
        &request_path,
        format!(
            r#"{{
  "repository_path": {},
  "target_path": "README.md",
  "expected_content": "before",
  "desired_content": "after"
}}"#,
            repository_path
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

#[test]
fn request_file_takes_precedence_over_inline_flags() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let repository_path = serde_json::to_string(workspace_root).unwrap();
    let request_path = std::env::temp_dir().join(format!(
        "implement-cli-request-precedence-{}.json",
        std::process::id()
    ));
    fs::write(
        &request_path,
        format!(
            r#"{{
  "repository_path": {},
  "target_path": "README.md",
  "expected_content": "before",
  "desired_content": "after"
}}"#,
            repository_path
        ),
    )
    .expect("write request fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args([
            "run",
            "--request",
            request_path.to_str().unwrap(),
            "--repository",
            "/definitely/ignored",
            "--target-path",
            "ignored.txt",
            "--expected-content",
            "ignored-before",
            "--desired-content",
            "ignored-after",
        ])
        .env("OPENAI_API_KEY", "fake-openai-key")
        .output()
        .expect("implement CLI should be executable");
    fs::remove_file(&request_path).ok();

    let result: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(result["status"], "Indeterminate");
    assert_eq!(result["target_path"], "README.md");
}

#[test]
fn root_help_lists_run_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .arg("--help")
        .output()
        .expect("implement CLI should be executable");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "help should not write to stderr");

    let stdout = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(stdout.contains("implement-cli"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("Run implementation requests against a repository"));
}

#[test]
fn run_help_lists_implement_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args(["run", "--help"])
        .output()
        .expect("implement CLI should be executable");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "help should not write to stderr");

    let stdout = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(stdout.contains("--request"));
    assert!(stdout.contains("--target-path"));
    assert!(stdout.contains("--desired-content"));
    assert!(stdout.contains("--emit-events"));
}

#[test]
fn run_help_is_recognized_after_other_run_flags() {
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
            "--help",
        ])
        .output()
        .expect("implement CLI should be executable");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "help should not write to stderr");

    let stdout = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(stdout.contains("Execute an implementation request"));
    assert!(stdout.contains("--repository"));
}

#[test]
fn help_aliases_render_root_and_run_help() {
    let root_output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .arg("help")
        .output()
        .expect("implement CLI should be executable");

    assert_eq!(root_output.status.code(), Some(0));
    assert!(
        root_output.stderr.is_empty(),
        "help should not write to stderr"
    );

    let root_stdout = String::from_utf8(root_output.stdout).expect("help output should be UTF-8");
    assert!(root_stdout.contains("Run implementation requests against a repository"));

    let run_output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args(["help", "run"])
        .output()
        .expect("implement CLI should be executable");

    assert_eq!(run_output.status.code(), Some(0));
    assert!(
        run_output.stderr.is_empty(),
        "help should not write to stderr"
    );

    let run_stdout = String::from_utf8(run_output.stdout).expect("help output should be UTF-8");
    assert!(run_stdout.contains("Execute an implementation request"));
    assert!(run_stdout.contains("--request"));
}
