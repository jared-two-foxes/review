use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const INVALID_REQUEST_EXIT_CODE: i32 = 2;

fn unique_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("review-validation-{name}-{unique}"))
}

fn request_file(name: &str, contents: &str) -> PathBuf {
    let path = unique_path(name).with_extension("json");
    fs::write(&path, contents).expect("request fixture should be writable");
    path
}

#[test]
fn malformed_requests_are_rejected_as_typed_validation_errors() {
    let repository = unique_path("repository");
    fs::create_dir(&repository).expect("repository fixture directory should be creatable");
    let repository_json = serde_json::to_string(&repository.to_string_lossy().to_string())
        .expect("repository path should be JSON-encodable");

    let cases = [
        (
            "unknown-field",
            format!(
                r#"{{"schema":"review.request/v1","repository_path":{repository_json},"unexpected":true}}"#
            ),
        ),
        (
            "unsupported-version",
            format!(r#"{{"schema":"review.request/v2","repository_path":{repository_json}}}"#),
        ),
        (
            "invalid-repository",
            format!(
                r#"{{"schema":"review.request/v1","repository_path":{}}}"#,
                serde_json::to_string(
                    &unique_path("definitely-not-a-real-repository")
                        .to_string_lossy()
                        .to_string()
                )
                .expect("repository path should be JSON-encodable")
            ),
        ),
        (
            "duplicate-key",
            format!(
                r#"{{"schema":"review.request/v1","schema":"review.request/v1","repository_path":{repository_json}}}"#
            ),
        ),
    ];

    for (name, contents) in cases {
        let path = request_file(name, &contents);
        let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
            .args([
                "run",
                "--request",
                path.to_str()
                    .expect("temporary request path should be valid UTF-8"),
                "--format",
                "json",
            ])
            .output()
            .expect("review CLI should be executable");
        fs::remove_file(&path).expect("temporary request should be removable");

        let exit_code = output.status.code().expect("CLI should exit normally");
        assert_eq!(
            exit_code, INVALID_REQUEST_EXIT_CODE,
            "{name} must use the invalid-request exit code"
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{name} must emit a JSON error envelope"));
        assert_eq!(error["schema_version"], "agent.error/v1", "{name}");
        assert_eq!(error["category"], "validation", "{name}");
        assert_eq!(error["retryable"], false, "{name}");
        let code = error["code"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} must provide a typed error code"));
        assert!(
            !code.is_empty(),
            "{name} must provide a non-empty typed error code"
        );
        let message = error["message"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} must provide a safe error message"));

        for (location, text) in [
            ("JSON error output", stdout.as_ref()),
            ("stderr", stderr.as_ref()),
        ] {
            for leaked_detail in ["panicked at", "stack backtrace", "serde_json::"] {
                assert!(
                    !text.contains(leaked_detail),
                    "{name} must not expose {leaked_detail} in {location}"
                );
            }
        }
        for leaked_detail in ["panicked at", "stack backtrace", "serde_json::"] {
            assert!(
                !message.contains(leaked_detail),
                "{name} must not expose {leaked_detail} in its error message"
            );
        }
    }

    fs::remove_dir(&repository).expect("repository fixture directory should be removable");
}
