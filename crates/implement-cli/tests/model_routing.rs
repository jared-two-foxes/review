use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::thread;

fn spawn_provider_probe() -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test provider");
    let address = listener.local_addr().expect("read test provider address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("CLI should call provider");
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let body = loop {
            let count = stream.read(&mut chunk).expect("read provider request");
            assert!(count > 0, "provider request must contain a body");
            bytes.extend_from_slice(&chunk[..count]);
            let Some(headers_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
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
            .write_all(
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .expect("write provider failure");
        String::from_utf8(body).expect("provider request must be UTF-8")
    });
    (format!("http://{address}/v1/chat/completions"), handle)
}

#[test]
fn openai_model_prefix_routes_and_strips_provider_name() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let (base_url, server) = spawn_provider_probe();

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
            "--model",
            "openai/gpt-4o",
            "--base-url",
            &base_url,
            "--wall-clock-budget-secs",
            "5",
        ])
        .env("OPENAI_API_KEY", "fake-openai-key")
        .output()
        .expect("implement CLI should be executable");

    let body = server.join().expect("provider thread should finish");
    let provider_request: Value = serde_json::from_str(&body).expect("provider request must be JSON");
    assert_eq!(provider_request["model"], "gpt-4o");

    let result: Value = serde_json::from_slice(&output.stdout).expect("CLI output should be JSON");
    assert_eq!(result["status"], "Indeterminate");
}

#[test]
fn unknown_model_provider_prefix_is_rejected() {
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
            "--model",
            "anthropic/claude-sonnet",
        ])
        .output()
        .expect("implement CLI should be executable");

    let error: Value = serde_json::from_slice(&output.stdout)
        .expect("unknown provider prefix should emit a JSON error");
    assert_eq!(error["schema_version"], "agent.error/v1");
    assert_eq!(error["code"], "INVALID_ARGUMENTS");
    assert_eq!(output.status.code(), Some(cli_common::ExitCode::InvalidRequest as i32));
}
