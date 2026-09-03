use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[test]
fn binary_runs_composition_root_and_reports_indeterminate_on_provider_failure() {
    // crates/review-cli -> crates -> workspace root (a real git repo with history)
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let request = serde_json::json!({
        "schema": "review.request/v1",
        "repository_path": workspace_root.to_str().unwrap(),
        "base_ref": "HEAD~1",
        "head_ref": "HEAD",
    });
    let request_path = std::env::temp_dir().join("review-smoke-request.json");
    fs::write(&request_path, request.to_string()).expect("write request fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--request",
            request_path.to_str().unwrap(),
            "--model",
            "test-model",
            "--base-url",
            "http://127.0.0.1:1/v1/chat/completions",
            "--wall-clock-budget-secs",
            "5",
        ])
        .env("OPENAI_API_KEY", "fake-key")
        .output()
        .expect("review CLI should be executable");

    fs::remove_file(&request_path).ok();

    // The composition root must run to the provider call without panicking.
    // A wiring bug (repo open, ref resolution, tool construction, coordinator)
    // would crash the binary and produce no valid JSON on stdout.
    let result: Value = serde_json::from_slice(&output.stdout)
        .expect("stdout should contain a JSON review result, not a crash");

    assert_eq!(
        result["schema"], "review.result/v1",
        "result must be a review.result/v1 envelope"
    );
    assert_eq!(
        result["status"],
        "INDETERMINATE",
        "a provider failure must terminate indeterminate, not approved; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::Indeterminate as i32),
        "indeterminate exit code"
    );
}
