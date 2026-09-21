use std::fs;
use std::path::Path;
use std::process::Command;

use jsonschema::JSONSchema;
use serde_json::Value;

#[test]
fn valid_minimal_request_produces_indeterminate_result() {
    // crates/review-cli -> crates/ -> workspace root
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let fixture = workspace_root.join("tests/fixtures/v0/minimal-request.json");
    let schema_path = workspace_root.join("schemas/review/review.result.v1.json");

    let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--request",
            fixture.to_str().unwrap(),
            "--format",
            "json",
        ])
        .env("OPENAI_API_KEY", "")
        .output()
        .expect("review CLI should be executable");

    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::Indeterminate as i32),
        "indeterminate exit code"
    );
    assert!(output.stderr.is_empty(), "JSON mode must not log to stderr");

    let result: Value =
        serde_json::from_slice(&output.stdout).expect("stdout should contain a JSON review result");
    assert_review_result(&result, &schema_path);
}

#[test]
fn demo_args_produce_schema_valid_indeterminate_result() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let schema_path = workspace_root.join("schemas/review/review.result.v1.json");

    let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--repository",
            workspace_root.to_str().unwrap(),
            "--base-ref",
            "HEAD~1",
            "--head-ref",
            "HEAD",
            "--format",
            "json",
        ])
        .env("OPENAI_API_KEY", "")
        .output()
        .expect("review CLI should be executable");

    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::Indeterminate as i32),
        "indeterminate exit code"
    );
    assert!(output.stderr.is_empty(), "JSON mode must not log to stderr");

    let result: Value =
        serde_json::from_slice(&output.stdout).expect("stdout should contain a JSON review result");
    assert_review_result(&result, &schema_path);
}

#[test]
fn root_help_lists_run_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .arg("--help")
        .output()
        .expect("review CLI should be executable");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "help should not write to stderr");

    let stdout = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(stdout.contains("review-cli"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("Run review requests against a repository diff"));
}

#[test]
fn run_help_lists_review_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args(["run", "--help"])
        .output()
        .expect("review CLI should be executable");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "help should not write to stderr");

    let stdout = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(stdout.contains("--request"));
    assert!(stdout.contains("--repository"));
    assert!(stdout.contains("--head-ref"));
    assert!(stdout.contains("--emit-events"));
}

fn assert_review_result(result: &Value, schema_path: &Path) {
    let schema: Value = serde_json::from_slice(
        &fs::read(schema_path).expect("checked-in review result schema should exist"),
    )
    .expect("checked-in review result schema should be valid JSON");
    let compiled_schema = JSONSchema::compile(&schema)
        .expect("checked-in review result schema should be a valid JSON Schema");
    let validation_errors = compiled_schema.validate(result).err().map(|errors| {
        errors
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    });
    assert!(
        validation_errors.is_none(),
        "CLI output must validate against review.result/v1: {}",
        validation_errors.unwrap_or_default()
    );

    assert_eq!(result["schema"], "review.result/v1");
    assert_eq!(result["status"], "INDETERMINATE");
    assert_eq!(result["reason"], "REVIEW_ENGINE_NOT_AVAILABLE");
}
