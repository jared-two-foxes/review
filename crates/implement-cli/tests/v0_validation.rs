use serde_json::Value;
use std::process::Command;

#[test]
fn missing_required_demo_args_are_rejected_as_typed_validation_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args(["run", "--repository", ".", "--target-path", "README.md"])
        .output()
        .expect("implement CLI should be executable");

    let error: Value = serde_json::from_slice(&output.stdout)
        .expect("missing required args should emit a JSON error envelope");
    assert_eq!(error["schema_version"], "agent.error/v1");
    assert_eq!(error["category"], "validation");
    assert_eq!(error["code"], "MISSING_EXPECTED_CONTENT");
    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::InvalidRequest as i32)
    );
}

#[test]
fn invalid_json_request_is_rejected_as_typed_validation_error() {
    let request_path = std::env::temp_dir().join(format!(
        "implement-cli-invalid-request-{}.json",
        std::process::id()
    ));
    std::fs::write(&request_path, "not json").expect("write invalid request fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args([
            "run",
            "--request",
            request_path.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("implement CLI should be executable");
    std::fs::remove_file(&request_path).ok();

    let error: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid request should emit a JSON error envelope");
    assert_eq!(error["schema_version"], "agent.error/v1");
    assert_eq!(error["category"], "validation");
    assert_eq!(error["code"], "REQUEST_PARSE_FAILED");
    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::InvalidRequest as i32)
    );
}

#[test]
fn invalid_numeric_flags_are_rejected_as_typed_validation_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args([
            "run",
            "--repository",
            ".",
            "--target-path",
            "README.md",
            "--expected-content",
            "before",
            "--desired-content",
            "after",
            "--max-turns",
            "not-a-number",
        ])
        .output()
        .expect("implement CLI should be executable");

    let error: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid numeric flags should emit a JSON error envelope");
    assert_eq!(error["schema_version"], "agent.error/v1");
    assert_eq!(error["category"], "validation");
    assert_eq!(error["code"], "INVALID_MAX_TURNS");
    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::InvalidRequest as i32)
    );
}

#[test]
fn unknown_arguments_are_rejected_as_typed_validation_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args([
            "run",
            "--repository",
            ".",
            "--target-path",
            "README.md",
            "--expected-content",
            "before",
            "--desired-content",
            "after",
            "--repositry",
            ".",
        ])
        .output()
        .expect("implement CLI should be executable");

    let error: Value = serde_json::from_slice(&output.stdout)
        .expect("unknown arguments should emit a JSON error envelope");
    assert_eq!(error["schema_version"], "agent.error/v1");
    assert_eq!(error["category"], "validation");
    assert_eq!(error["code"], "UNRECOGNIZED_ARGUMENT");
    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::InvalidRequest as i32)
    );
}

#[test]
fn missing_api_key_uses_non_validation_failure_exit_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_implement-cli"))
        .args([
            "run",
            "--repository",
            ".",
            "--target-path",
            "README.md",
            "--expected-content",
            "before",
            "--desired-content",
            "after",
        ])
        .env_remove("OPENAI_API_KEY")
        .output()
        .expect("implement CLI should be executable");

    let error: Value =
        serde_json::from_slice(&output.stdout).expect("missing API key should emit JSON");
    assert_eq!(error["schema_version"], "agent.error/v1");
    assert_eq!(error["category"], "configuration");
    assert_eq!(error["code"], "MISSING_API_KEY");
    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::InternalFailure as i32)
    );
}
