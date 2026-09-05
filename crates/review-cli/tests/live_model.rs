use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Opt-in live smoke test for the acceptance criterion.
///
/// Run with `OPENAI_API_KEY` (and optionally `REVIEW_MODEL` and
/// `REVIEW_BASE_URL`) set. Keeping this opt-in prevents ordinary test runs
/// from depending on credentials or a network service while providing a
/// reproducible check against a real repository and model.
#[test]
fn live_binary_emits_verdict_with_findings() {
    if std::env::var("REVIEW_LIVE_TEST").is_err() {
        eprintln!("skipping live model test: set REVIEW_LIVE_TEST=1 to run");
        return;
    }
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("REVIEW_LIVE_TEST is set but OPENAI_API_KEY is not");

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
    let request_path =
        std::env::temp_dir().join(format!("review-live-request-{}.json", std::process::id()));
    fs::write(&request_path, request.to_string()).expect("write live request");

    let model = std::env::var("REVIEW_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
    let base_url = std::env::var("REVIEW_BASE_URL").ok();
    let mut args = vec![
        "run".to_string(),
        "--request".to_string(),
        request_path.to_str().unwrap().to_string(),
        "--model".to_string(),
        model,
    ];
    if let Some(url) = base_url {
        args.extend(["--base-url".to_string(), url]);
    }

    let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args(&args)
        .env("OPENAI_API_KEY", api_key)
        .output()
        .expect("review CLI should be executable");
    fs::remove_file(&request_path).ok();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("live CLI did not emit JSON: {error}; stdout={stdout}"));

    assert_ne!(
        result["status"], "INDETERMINATE",
        "live review must reach a verdict; stdout={stdout}"
    );
    assert!(
        result["findings"]
            .as_array()
            .is_some_and(|findings| !findings.is_empty()),
        "live review must emit at least one finding; stdout={stdout}"
    );
}
