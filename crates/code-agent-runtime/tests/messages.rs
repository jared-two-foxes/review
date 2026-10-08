use agent_kernel::application::{ContextBlock, InstructionBlock};
use agent_kernel::model::{
    CanonicalModelRequest, ConversationMessage, ModelAction, ModelError, ModelProvider,
    ToolCallRecord, ToolDescription,
};
use code_agent_runtime::provider::{
    ApiStyle, OpenAiProvider, resolve_provider_route, resolve_provider_route_with_root,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

// Bound both accept and reads so a routing regression fails instead of hanging.
fn probe(response: Value, status: u16) -> (String, thread::JoinHandle<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let root = format!("http://{}/v1/", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "no provider request");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("accept: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let (headers, request_body) = loop {
            let mut chunk = [0; 4096];
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                break (
                    headers,
                    serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
                );
            }
        };
        let body = response.to_string();
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        (headers, request_body)
    });
    (root, server)
}

fn request() -> CanonicalModelRequest {
    CanonicalModelRequest {
        instructions: vec![InstructionBlock {
            content: "Review policy".into(),
        }],
        context: vec![ContextBlock {
            content: "Diff context".into(),
        }],
        history: vec![],
        tools: vec![ToolDescription {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        }],
    }
}

#[test]
fn messages_routes_preserve_overrides_and_other_provider_styles() {
    let direct = resolve_provider_route("AnThRoPiC/claude-sonnet-4-5", Some("direct-key")).unwrap();
    assert_eq!(direct.model, "claude-sonnet-4-5");
    assert_eq!(direct.provider_root, "https://api.anthropic.com/v1");
    assert_eq!(direct.api_key, "direct-key");
    assert_eq!(direct.api_style, ApiStyle::Messages);
    let zen = resolve_provider_route_with_root(
        "opencode/claude-sonnet-4-5",
        Some("zen-key"),
        Some("http://proxy/v1/"),
    )
    .unwrap();
    assert_eq!(zen.api_style, ApiStyle::Messages);
    assert_eq!(zen.provider_root, "http://proxy/v1");
    assert_eq!(zen.api_key, "zen-key");
    for (model, expected) in [
        ("opencode/gpt-6-luna", ApiStyle::Responses),
        ("opencode/zen", ApiStyle::ChatCompletions),
        ("copilot/claude-sonnet-4-5", ApiStyle::ChatCompletions),
    ] {
        assert_eq!(
            resolve_provider_route(model, Some("key"))
                .unwrap()
                .api_style,
            expected
        );
    }
}

#[test]
fn messages_round_trip_tools_history_completion_and_usage() {
    for model in ["anthropic/claude-sonnet-4-5", "opencode/claude-sonnet-4-5"] {
        let response = json!({
            "stop_reason":"tool_use", "content":[
                {"type":"text","text":"Inspecting"},
                {"type":"tool_use","id":"c1","name":"read_file","input":{"path":"a.rs"}},
                {"type":"tool_use","id":"c2","name":"read_file","input":{"path":"b.rs"}}
            ], "usage":{"input_tokens":10,"output_tokens":4,"cache_creation_input_tokens":20,"cache_read_input_tokens":30}
        });
        let (root, server) = probe(response, 200);
        let mut provider = OpenAiProvider::new(
            resolve_provider_route_with_root(model, Some("test-key"), Some(&root)).unwrap(),
        );
        let response = provider.generate(&request()).unwrap();
        let (headers, body) = server.join().unwrap();
        assert!(headers.starts_with("POST /v1/messages HTTP/1.1"));
        let headers = headers.to_ascii_lowercase();
        assert!(headers.contains("x-api-key: test-key"));
        assert!(headers.contains("anthropic-version: 2023-06-01"));
        assert!(!headers.contains("authorization:"));
        assert_eq!(body["model"], "claude-sonnet-4-5");
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["system"], "Review policy");
        assert_eq!(
            body["messages"][0],
            json!({"role":"user","content":[{"type":"text","text":"Diff context"}]})
        );
        assert_eq!(
            body["tools"][0]["input_schema"],
            request().tools[0].input_schema
        );
        assert!(body["tools"][0].get("function").is_none());
        assert_eq!(response.actions.len(), 2);
        let calls = response
            .actions
            .iter()
            .map(|action| match action {
                ModelAction::ToolCall {
                    action_id,
                    tool,
                    arguments,
                } => ToolCallRecord {
                    id: action_id.clone(),
                    name: tool.clone(),
                    arguments: arguments.clone(),
                },
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        let usage = response.usage().unwrap();
        assert_eq!(usage.input_tokens, 60);
        assert_eq!(usage.output_tokens, 4);
        assert_eq!(usage.estimated_cost_usd, None);
        let mut next = request();
        next.history = vec![
            ConversationMessage::Assistant {
                content: Some("Inspecting".into()),
                tool_calls: calls,
            },
            ConversationMessage::Tool {
                tool_call_id: "c1".into(),
                content: "file a".into(),
            },
            ConversationMessage::Tool {
                tool_call_id: "c2".into(),
                content: "file b".into(),
            },
            ConversationMessage::User {
                content: "Finish review".into(),
            },
        ];
        let (root, server) = probe(
            json!({"stop_reason":"end_turn","content":[
                {"type":"thinking","thinking":"ignore"},
                {"type":"text","text":"```json\n{\"findings\":"},
                {"type":"text","text":"[]}\n```"}
            ]}),
            200,
        );
        let mut provider = OpenAiProvider::new(
            resolve_provider_route_with_root(model, Some("test-key"), Some(&root)).unwrap(),
        );
        let response = provider.generate(&next).unwrap();
        let (_, body) = server.join().unwrap();
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);
        assert_eq!(
            body["messages"][1]["content"][1],
            json!({"type":"tool_use","id":"c1","name":"read_file","input":{"path":"a.rs"}})
        );
        assert_eq!(
            body["messages"][2]["content"],
            json!([
                {"type":"tool_result","tool_use_id":"c1","content":"file a"},
                {"type":"tool_result","tool_use_id":"c2","content":"file b"},
                {"type":"text","text":"Finish review"}
            ])
        );
        assert!(
            matches!(&response.actions[..], [ModelAction::CompletionRequest { payload, .. }] if payload == &json!({"findings":[]}))
        );
    }
}

#[test]
fn messages_reject_truncation_refusal_and_malformed_tool_inputs() {
    for response in [
        json!({"stop_reason":"max_tokens","content":[{"type":"text","text":"{\"findings\":[]}"}]}),
        json!({"stop_reason":"refusal","content":[{"type":"text","text":"{\"findings\":[]}"}]}),
        json!({"content":[{"type":"tool_use","id":"c1","name":"read_file","input":123},{"type":"text","text":"{\"findings\":[]}"}]}),
        json!({"content":[{"type":"tool_use","name":"read_file","input":{}}]}),
        json!({"content":[{"type":"text","text":"not JSON"}]}),
        json!({}),
    ] {
        let (root, server) = probe(response, 200);
        let mut provider = OpenAiProvider::new(
            resolve_provider_route_with_root("anthropic/test", Some("key"), Some(&root)).unwrap(),
        );
        assert!(provider.generate(&request()).unwrap().actions.is_empty());
        server.join().unwrap();
    }
}

#[test]
fn messages_use_shared_http_errors_and_deadlines() {
    for status in [429, 500] {
        let (root, server) = probe(json!({"error":{"message":"failure"}}), status);
        let mut provider = OpenAiProvider::new(
            resolve_provider_route_with_root("anthropic/test", Some("key"), Some(&root)).unwrap(),
        );
        let result = provider.generate(&request());
        if status == 429 {
            assert!(matches!(result, Err(ModelError::RateLimit(_))));
        } else {
            assert!(matches!(result, Err(ModelError::ApiError(_))));
        }
        server.join().unwrap();
    }
    let mut provider =
        OpenAiProvider::new(resolve_provider_route("anthropic/test", Some("key")).unwrap());
    assert!(matches!(
        provider.generate_with_deadline(&request(), Instant::now() - Duration::from_secs(1)),
        Err(ModelError::Timeout(_))
    ));
}

#[test]
fn messages_preserve_completion_retry_history_without_tools() {
    let mut request = request();
    request.instructions.push(InstructionBlock {
        content: "Retry policy".into(),
    });
    request.tools.clear();
    request.history = vec![
        ConversationMessage::Assistant {
            content: Some("{\"findings\":[]}".into()),
            tool_calls: vec![],
        },
        ConversationMessage::User {
            content: "Inspect the change first".into(),
        },
    ];
    let (root, server) = probe(json!({"stop_reason":"end_turn","content":[]}), 200);
    let mut provider = OpenAiProvider::new(
        resolve_provider_route_with_root("anthropic/test", Some("key"), Some(&root)).unwrap(),
    );
    provider.generate(&request).unwrap();
    let (_, body) = server.join().unwrap();
    assert_eq!(body["system"], "Review policy\nRetry policy");
    assert!(body.get("tools").is_none());
    assert_eq!(
        body["messages"][1],
        json!({"role":"assistant","content":[{"type":"text","text":"{\"findings\":[]}"}]})
    );
    assert_eq!(
        body["messages"][2],
        json!({"role":"user","content":[{"type":"text","text":"Inspect the change first"}]})
    );
}
