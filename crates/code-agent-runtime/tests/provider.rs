use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use agent_kernel::application::{ContextBlock, InstructionBlock};
use agent_kernel::model::{CanonicalModelRequest, ModelAction, ModelProvider, ToolDescription};
use code_agent_runtime::provider::OpenAiProvider;

#[test]
fn openai_provider_round_trips_canonical_request_and_tool_call() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test provider");
    let address = listener.local_addr().expect("read test provider address");
    listener
        .set_nonblocking(true)
        .expect("configure test provider");

    let received = Arc::new(Mutex::new(None::<String>));
    let received_by_server = Arc::clone(&received);
    let server = thread::spawn(move || -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok((mut stream, peer)) => {
                    stream.set_nonblocking(false).ok();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .map_err(|error| format!("configure provider connection: {error}"))?;
                    let mut bytes = Vec::new();
                    let mut chunk = [0_u8; 4096];
                    let body = loop {
                        match stream.read(&mut chunk) {
                            Ok(0) => {
                                return Err(
                                    "provider closed the request before its body arrived".into()
                                );
                            }
                            Ok(count) => {
                                bytes.extend_from_slice(&chunk[..count]);
                                if let Some(headers_end) =
                                    bytes.windows(4).position(|window| window == b"\r\n\r\n")
                                {
                                    let headers = String::from_utf8_lossy(&bytes[..headers_end]);
                                    let content_length = headers
                                        .lines()
                                        .find_map(|line| {
                                            line.strip_prefix("Content-Length:").and_then(|value| {
                                                value.trim().parse::<usize>().ok()
                                            })
                                        })
                                        .ok_or_else(|| {
                                            "request has no Content-Length".to_string()
                                        })?;
                                    let body_start = headers_end + 4;
                                    if bytes.len() >= body_start + content_length {
                                        break String::from_utf8(
                                            bytes[body_start..body_start + content_length].to_vec(),
                                        )
                                        .map_err(
                                            |error| format!("request body is not utf8: {error}"),
                                        )?;
                                    }
                                }
                            }
                            Err(error)
                                if error.kind() == std::io::ErrorKind::TimedOut
                                    || error.kind() == std::io::ErrorKind::WouldBlock =>
                            {
                                return Err(format!("timed out reading provider request: {error}"));
                            }
                            Err(error) => return Err(format!("read provider request: {error}")),
                        }
                    };

                    *received_by_server
                        .lock()
                        .map_err(|_| "record request lock poisoned".to_string())? =
                        Some(body.clone());
                    let request: serde_json::Value = serde_json::from_str(&body)
                        .map_err(|error| format!("provider request must be JSON: {error}"))?;
                    assert_eq!(request["model"], "test-model");
                    assert_eq!(request["messages"][0]["role"], "system");
                    assert_eq!(request["messages"][1]["role"], "user");
                    assert_eq!(request["tools"][0]["function"]["name"], "echo");

                    let response = r#"{"id":"provider-response-9","model":"test-model","choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"provider-call-9","type":"function","function":{"name":"echo","arguments":"{\"value\":\"returned-by-provider\"}"}}]}}]}"#;
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    );
                    stream
                        .write_all(reply.as_bytes())
                        .map_err(|error| format!("write provider response to {peer}: {error}"))?;
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err("adapter made no HTTP request".into());
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(format!("accept provider request: {error}")),
            }
        }
    });

    let request = CanonicalModelRequest {
        instructions: vec![InstructionBlock {
            content: "Follow the review policy.".into(),
        }],
        context: vec![ContextBlock {
            content: "The submitted change adds echo support.".into(),
        }],
        tools: vec![ToolDescription {
            name: "echo".into(),
            description: "Echo a JSON value.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "value": { "type": "string" } }
            }),
        }],
    };

    let mut provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-api-key",
        "test-model",
    );
    let response = provider.generate(&request);
    server
        .join()
        .expect("provider server thread")
        .expect("provider exchange");

    let wire_request = received
        .lock()
        .expect("read recorded request")
        .clone()
        .expect("adapter must send the canonical request to the provider");
    let wire_request: serde_json::Value =
        serde_json::from_str(&wire_request).expect("provider request must be JSON");
    assert_eq!(wire_request["model"], "test-model");
    let messages = wire_request["messages"].as_array().expect("messages array");
    let instruction_message = messages
        .iter()
        .find(|message| message["role"] == "system")
        .expect("instructions must be sent as a system message");
    assert_eq!(instruction_message["content"], "Follow the review policy.");
    let context_message = messages
        .iter()
        .find(|message| message["role"] == "user")
        .expect("context must be sent as a user message");
    assert_eq!(
        context_message["content"],
        "The submitted change adds echo support."
    );
    let tools = wire_request["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|tool| {
        tool["function"]["name"] == "echo"
            && tool["function"]["parameters"]["properties"]["value"]["type"] == "string"
    }));

    assert_eq!(response.actions.len(), 1);
    match &response.actions[0] {
        ModelAction::ToolCall {
            action_id,
            tool,
            arguments,
        } => {
            assert_eq!(action_id, "provider-call-9");
            assert_eq!(tool, "echo");
            assert_eq!(
                arguments,
                &serde_json::json!({ "value": "returned-by-provider" })
            );
        }
        action => panic!("expected parsed tool call, got {action:?}"),
    }
}
