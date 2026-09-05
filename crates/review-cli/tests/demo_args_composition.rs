use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::thread;

use serde_json::Value;

#[test]
fn demo_args_run_reaches_composition_root_and_reports_indeterminate() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--repository",
            workspace_root.to_str().unwrap(),
            "--base-ref",
            "HEAD~1",
            "--head-ref",
            "HEAD",
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

    let result: Value = serde_json::from_slice(&output.stdout)
        .expect("demo-args run must emit a JSON review result, not a crash");
    assert_eq!(result["schema"], "review.result/v1");
    assert_eq!(result["status"], "INDETERMINATE");
    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::Indeterminate as i32)
    );
}

#[test]
fn demo_args_accept_format_and_requirements_when_building_review_request() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let requirements = "The change must preserve backwards compatibility.";
    let requirements_path = std::env::temp_dir().join(format!(
        "review-cli-requirements-{}.txt",
        std::process::id()
    ));
    fs::write(&requirements_path, requirements).expect("write requirements fixture");

    // A real local endpoint lets the test inspect the canonical model request.  A
    // provider failure is intentional: the CLI must still have parsed and
    // composed the request far enough to send it.
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test provider");
    let address = listener.local_addr().expect("read test provider address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("CLI should call configured provider");
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let body = loop {
            let count = stream.read(&mut chunk).expect("read provider request");
            assert!(count > 0, "provider request must contain a body");
            bytes.extend_from_slice(&chunk[..count]);
            let headers_end = bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .expect("provider request must contain HTTP headers");
            let headers = String::from_utf8_lossy(&bytes[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .expect("provider request must contain Content-Length");
            let body_start = headers_end + 4;
            if bytes.len() >= body_start + content_length {
                break bytes[body_start..body_start + content_length].to_vec();
            }
        };
        stream
            .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .expect("write provider failure");
        String::from_utf8(body).expect("provider request must be UTF-8")
    });

    let base_url = format!("http://{address}/v1/chat/completions");
    let output = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--repository",
            workspace_root.to_str().unwrap(),
            "--base-ref",
            "HEAD~1",
            "--head-ref",
            "HEAD",
            "--model",
            "demo-model",
            "--format",
            "json",
            "--requirements",
            requirements_path.to_str().unwrap(),
            "--base-url",
            &base_url,
            "--wall-clock-budget-secs",
            "5",
        ])
        .env("OPENAI_API_KEY", "fake-key")
        .output()
        .expect("review CLI should be executable");

    fs::remove_file(&requirements_path).ok();

    let request_body = server.join().expect("provider thread should finish");
    let provider_request: Value =
        serde_json::from_str(&request_body).expect("provider request must be JSON");
    assert_eq!(provider_request["model"], "demo-model");
    assert!(
        request_body.contains(requirements),
        "requirements file content must be included in the composed request: {request_body}"
    );

    let result: Value = serde_json::from_slice(&output.stdout)
        .expect("--format json must produce a JSON review result, not a crash");
    assert_eq!(result["schema"], "review.result/v1");
    assert_eq!(result["status"], "INDETERMINATE");
    assert_eq!(
        output.status.code(),
        Some(cli_common::ExitCode::Indeterminate as i32)
    );

    // JSON is the default output today, so merely observing JSON cannot prove
    // that --format was parsed. An unsupported format must be rejected rather
    // than silently treated as the default.
    let invalid_format = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--repository",
            workspace_root.to_str().unwrap(),
            "--base-ref",
            "HEAD~1",
            "--head-ref",
            "HEAD",
            "--model",
            "demo-model",
            "--format",
            "text",
        ])
        .output()
        .expect("review CLI should validate the format argument");
    let format_error: Value = serde_json::from_slice(&invalid_format.stdout)
        .expect("invalid format must produce a typed JSON error");
    assert_eq!(format_error["schema_version"], "agent.error/v1");
    assert_eq!(format_error["category"], "validation");
    assert_ne!(
        invalid_format.status.code(),
        Some(cli_common::ExitCode::Indeterminate as i32),
        "unsupported --format must not be silently ignored"
    );

    // These probes make the request fields observable rather than merely using
    // valid-looking values.  If the CLI drops the repository or either ref while
    // composing the request, these failures become provider calls instead of the
    // specific setup errors asserted below.
    let invalid_repository = std::env::temp_dir().join(format!(
        "review-cli-not-a-repository-{}",
        std::process::id()
    ));
    fs::create_dir(&invalid_repository).expect("create repository validation fixture");
    let repository_probe = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--repository",
            invalid_repository.to_str().unwrap(),
            "--base-ref",
            "HEAD~1",
            "--head-ref",
            "HEAD",
            "--model",
            "demo-model",
            "--format",
            "json",
            "--emit-events",
            "--base-url",
            "http://127.0.0.1:1/v1/chat/completions",
        ])
        .env("OPENAI_API_KEY", "fake-key")
        .output()
        .expect("review CLI should validate the repository argument");
    fs::remove_dir(&invalid_repository).ok();
    let repository_stderr = String::from_utf8_lossy(&repository_probe.stderr);
    assert!(
        repository_stderr.contains("open repository"),
        "repository argument must reach review composition: {repository_stderr}"
    );

    let ref_probe = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--repository",
            workspace_root.to_str().unwrap(),
            "--base-ref",
            "definitely-invalid-base-ref-for-cli-test",
            "--head-ref",
            "HEAD",
            "--model",
            "demo-model",
            "--format",
            "json",
            "--emit-events",
            "--base-url",
            "http://127.0.0.1:1/v1/chat/completions",
        ])
        .env("OPENAI_API_KEY", "fake-key")
        .output()
        .expect("review CLI should validate the ref arguments");
    let ref_stderr = String::from_utf8_lossy(&ref_probe.stderr);
    assert!(
        ref_stderr.contains("resolve base_ref"),
        "base-ref argument must reach review composition: {ref_stderr}"
    );

    let head_probe = Command::new(env!("CARGO_BIN_EXE_review-cli"))
        .args([
            "run",
            "--repository",
            workspace_root.to_str().unwrap(),
            "--base-ref",
            "HEAD~1",
            "--head-ref",
            "definitely-invalid-head-ref-for-cli-test",
            "--model",
            "demo-model",
            "--format",
            "json",
            "--emit-events",
            "--base-url",
            "http://127.0.0.1:1/v1/chat/completions",
        ])
        .env("OPENAI_API_KEY", "fake-key")
        .output()
        .expect("review CLI should validate the head-ref argument");
    let head_stderr = String::from_utf8_lossy(&head_probe.stderr);
    assert!(
        head_stderr.contains("resolve head_ref"),
        "head-ref argument must reach review composition: {head_stderr}"
    );
}
