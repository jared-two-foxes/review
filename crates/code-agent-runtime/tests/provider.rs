use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use agent_kernel::application::{ContextBlock, InstructionBlock};
use agent_kernel::model::{
    CanonicalModelRequest, ModelAction, ModelError, ModelProvider, ToolDescription, UsageRecord,
};
use code_agent_runtime::provider::{OpenAiProvider, resolve_provider_route};

#[test]
fn model_prefix_routing_normalizes_model_and_provider_defaults() {
    let route = resolve_provider_route("ollama/llama3.2", None, Some("test-key"))
        .expect("ollama prefix should route successfully");
    assert_eq!(route.model, "llama3.2");
    assert_eq!(route.base_url, "http://127.0.0.1:11434/v1/chat/completions");
    assert_eq!(route.api_key, "test-key");
}

#[test]
fn model_prefix_routing_accepts_openai_prefix() {
    let route = resolve_provider_route("openai/gpt-4o", None, Some("test-key"))
        .expect("openai prefix should route successfully");
    assert_eq!(route.model, "gpt-4o");
    assert_eq!(route.base_url, "https://api.openai.com/v1/chat/completions");
    assert_eq!(route.api_key, "test-key");
}

#[test]
fn model_prefix_routing_accepts_opencode_prefix() {
    let route = resolve_provider_route("opencode/zen", None, Some("test-key"))
        .expect("opencode prefix should route successfully");
    assert_eq!(route.model, "zen");
    assert_eq!(route.base_url, "https://api.opencode.ai/v1/chat/completions");
    assert_eq!(route.api_key, "test-key");
}

#[test]
fn model_prefix_routing_honors_explicit_overrides() {
    let route = resolve_provider_route(
        "github-copilot/gpt-4.1",
        Some("http://127.0.0.1:9000/custom"),
        Some("custom-key"),
    )
    .expect("copilot prefix should route successfully");
    assert_eq!(route.model, "gpt-4.1");
    assert_eq!(route.base_url, "http://127.0.0.1:9000/custom");
    assert_eq!(route.api_key, "custom-key");
}

#[test]
fn model_prefix_routing_rejects_unknown_provider_prefixes() {
    let error = resolve_provider_route("anthropic/claude", None, None)
        .expect_err("unknown provider prefix must fail validation");
    assert!(error.contains("unsupported model provider prefix"));
}

#[test]
fn model_prefix_routing_rejects_empty_provider_model_suffixes() {
    let openai_error = resolve_provider_route("openai/", None, None)
        .expect_err("empty provider-qualified model suffix must fail validation");
    assert!(openai_error.contains("requires a non-empty model name"));

    let anthropic_error = resolve_provider_route("anthropic/", None, None)
        .expect_err("empty provider-qualified model suffix must fail validation");
    assert!(anthropic_error.contains("requires a non-empty model name"));
}

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
                                            line.to_ascii_lowercase()
                                                .strip_prefix("content-length:")
                                                .and_then(|value| {
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
        history: vec![],
    };

    let mut provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-api-key",
        "test-model",
    );
    let response = provider
        .generate(&request)
        .expect("provider generate should succeed");
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

#[test]
fn openai_provider_maps_http_failures_to_typed_model_errors() {
    let failures = [
        (
            500_u16,
            r#"{"error":{"message":"upstream unavailable"}}"#,
            false,
        ),
        (429_u16, r#"{"error":{"message":"slow down"}}"#, true),
    ];
    let request = CanonicalModelRequest {
        instructions: vec![],
        context: vec![],
        tools: vec![],
        history: vec![],
    };

    for (status, response_body, rate_limited) in failures {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind failure provider");
        let address = listener
            .local_addr()
            .expect("read failure provider address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("configure failure provider connection");
            let mut request_bytes = [0_u8; 1024];
            let _ = stream.read(&mut request_bytes);
            let reply = format!(
                "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nRetry-After: 9\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(reply.as_bytes())
                .expect("send provider error");
        });

        let mut provider = OpenAiProvider::new(
            format!("http://{address}/v1/chat/completions"),
            "test-api-key",
            "test-model",
        );
        let error = provider
            .generate(&request)
            .expect_err("HTTP provider failures must not become empty successful responses");
        server.join().expect("provider server thread");

        match (rate_limited, error) {
            (false, ModelError::ApiError(message)) => {
                assert!(
                    message.contains("500"),
                    "API error should preserve HTTP status"
                );
            }
            (true, ModelError::RateLimit(message)) => {
                assert!(
                    message.contains("429"),
                    "rate limit should preserve HTTP status"
                );
            }
            (false, other) => panic!("500 response mapped to wrong error: {other:?}"),
            (true, other) => panic!("429 response mapped to wrong error: {other:?}"),
        }
    }
}

#[test]
fn openai_provider_attaches_reported_usage_to_generated_response() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind usage provider");
    let address = listener.local_addr().expect("read usage provider address");
    let response_body = r#"{
        "id":"usage-completion",
        "choices":[{"message":{"role":"assistant","content":"done"}}],
        "usage":{"prompt_tokens":123,"completion_tokens":45,"total_tokens":168}
    }"#;
    let response_body_for_server = response_body.to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept usage provider request");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("configure usage provider connection");
        let mut request = [0_u8; 4096];
        let count = stream
            .read(&mut request)
            .expect("read usage provider request");
        assert!(count > 0, "generate must issue a provider request");
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body_for_server.len(),
            response_body_for_server
        );
        stream
            .write_all(reply.as_bytes())
            .expect("send usage provider response");
    });

    let request = CanonicalModelRequest {
        instructions: vec![],
        context: vec![],
        tools: vec![],
        history: vec![],
    };
    let mut provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-api-key",
        "test-model",
    );
    // Exercise the adapter's provider-wire normalization directly as well as the
    // end-to-end generated response.  This keeps the test red even if the
    // response accessor is accidentally left disconnected from normalization.
    let provider_payload: serde_json::Value =
        serde_json::from_str(response_body).expect("usage fixture must be valid JSON");
    let normalized = OpenAiProvider::normalize_usage(&provider_payload);
    assert_eq!(normalized.input_tokens, 123);
    assert_eq!(normalized.output_tokens, 45);
    assert_eq!(
        normalized.estimated_cost_usd,
        Some(123.0 * 0.000001 + 45.0 * 0.000002),
        "usage cost must use the adapter's documented per-token rates"
    );

    let generated = provider
        .generate(&request)
        .expect("provider generate should succeed");
    server.join().expect("usage provider server thread");

    let usage = generated
        .usage()
        .expect("provider-reported usage must be attached to the canonical response");
    assert_eq!(usage.input_tokens, 123);
    assert_eq!(usage.output_tokens, 45);
    assert_eq!(
        usage.estimated_cost_usd,
        Some(123.0 * 0.000001 + 45.0 * 0.000002),
        "attached usage must preserve the normalized estimated cost"
    );
}

#[cfg(test)]
mod coordinator_failure_tests {
    use super::*;
    use agent_kernel::application::{
        AgentApplication, ApplicationDescriptor, ApplicationInitialization, CompletionDecision,
    };
    use agent_kernel::coordinator::SessionCoordinator;
    use agent_kernel::ledger::LedgerEvent;
    use agent_kernel::limits::Limits;
    use agent_protocol::SequenceIdGenerator;
    use serde_json::Value;

    struct Request;
    #[derive(Clone)]
    struct State;
    struct Completion;
    struct App;

    impl AgentApplication for App {
        type Request = Request;
        type State = State;
        type Completion = Completion;
        type Result = ();
        type Error = String;

        fn descriptor(&self) -> ApplicationDescriptor {
            ApplicationDescriptor {
                application_id: "provider-test".into(),
                application_version: "1".into(),
                request_schema: "request".into(),
                completion_schema: "completion".into(),
                result_schema: "result".into(),
                domain_event_namespace: "provider-test".into(),
            }
        }
        fn validate_request(&self, _: &Request) -> Result<(), String> {
            Ok(())
        }
        fn initialize(&self, _: &Request) -> Result<ApplicationInitialization<State>, String> {
            Ok(ApplicationInitialization {
                initial_state: State,
                requested_tools: vec![],
                requested_capabilities: vec![],
                application_limits: None,
            })
        }
        fn build_system_instructions(&self, _: &State) -> Vec<InstructionBlock> {
            vec![]
        }
        fn build_context(&self, _: &State) -> Vec<ContextBlock> {
            vec![]
        }
        fn reduce_event(&self, state: &State, _: &LedgerEvent) -> State {
            state.clone()
        }
        fn parse_completion(&self, _: &Value) -> Result<Completion, String> {
            Ok(Completion)
        }
        fn validate_completion(&self, _: &State, _: &Completion) -> CompletionDecision {
            CompletionDecision::Accepted
        }
        fn build_terminal_result(&self, _: &State, _: &UsageRecord) {}
    }

    fn run_with_provider<P: ModelProvider>(provider: P) -> Vec<LedgerEvent> {
        let limits = Limits {
            max_turns: 2,
            max_tool_calls: 2,
            max_completion_attempts: 2,
            wall_clock_budget: None,
            ledger_path: None,
            max_repeated_actions: 2,
            max_input_tokens: None,
            max_cost_usd: None,
        };
        let coordinator = SessionCoordinator::new(
            App,
            provider,
            SequenceIdGenerator::new(["session", "execution"]),
            agent_kernel::tools::ToolCatalog::new(),
            limits,
        );
        let (result, events) = coordinator.run_full(Request, None);
        assert_eq!(result, ());
        events
    }

    #[test]
    fn provider_api_failure_terminates_session_indeterminate_without_approval() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind provider");
        let address = listener.local_addr().expect("provider address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            let body = r#"{"error":{"message":"upstream unavailable"}}"#;
            let reply = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(reply.as_bytes())
                .expect("send provider error");
        });

        let provider = OpenAiProvider::new(
            format!("http://{address}/v1/chat/completions"),
            "test-api-key",
            "test-model",
        );
        let events = run_with_provider(provider);
        server.join().expect("provider server thread");

        assert!(
            events
                .iter()
                .any(|event| event.event_type == "kernel.model_failed")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "kernel.session_indeterminate")
        );
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == "kernel.completion_accepted")
        );
    }

    #[test]
    fn provider_connection_failure_terminates_session_indeterminate_without_approval() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve provider address");
        let address = listener.local_addr().expect("provider address");
        drop(listener);

        let provider = OpenAiProvider::new(
            format!("http://{address}/v1/chat/completions"),
            "test-api-key",
            "test-model",
        );
        let events = run_with_provider(provider);

        assert!(
            events
                .iter()
                .any(|event| event.event_type == "kernel.model_failed")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "kernel.session_indeterminate")
        );
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == "kernel.completion_accepted")
        );
    }

    struct RecordingProvider {
        inner: OpenAiProvider,
        saw_timeout: std::sync::Arc<std::sync::Mutex<bool>>,
    }

    impl ModelProvider for RecordingProvider {
        fn generate(
            &mut self,
            request: &CanonicalModelRequest,
        ) -> Result<agent_kernel::model::CanonicalModelResponse, ModelError> {
            let result = self.inner.generate(request);
            if matches!(result, Err(ModelError::Timeout(_))) {
                *self.saw_timeout.lock().expect("record timeout") = true;
            }
            result
        }

        fn generate_with_deadline(
            &mut self,
            request: &CanonicalModelRequest,
            _deadline: Instant,
        ) -> Result<agent_kernel::model::CanonicalModelResponse, ModelError> {
            self.generate(request)
        }
    }

    #[test]
    fn provider_timeout_terminates_session_indeterminate_without_approval() {
        // Keep the real provider connection open past its read deadline. This proves that the
        // adapter made the request, produced a typed timeout, and that the coordinator stopped
        // before any completion validation or approval path could run.
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind hanging provider");
        let address = listener.local_addr().expect("hanging provider address");
        let request_received = std::sync::Arc::new(std::sync::Mutex::new(false));
        let request_received_by_server = std::sync::Arc::clone(&request_received);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept hanging provider request");
            let mut request = [0_u8; 2048];
            let count = stream
                .read(&mut request)
                .expect("read hanging provider request");
            assert!(
                count > 0,
                "the coordinator must reach the provider before timing out"
            );
            *request_received_by_server
                .lock()
                .expect("record provider request") = true;
            thread::sleep(Duration::from_secs(31));
        });
        let saw_timeout = std::sync::Arc::new(std::sync::Mutex::new(false));
        let provider = RecordingProvider {
            inner: OpenAiProvider::new(
                format!("http://{address}/v1/chat/completions"),
                "test-api-key",
                "test-model",
            ),
            saw_timeout: std::sync::Arc::clone(&saw_timeout),
        };
        let events = run_with_provider(provider);
        server.join().expect("hanging provider server thread");

        assert!(
            *request_received.lock().expect("read request observation"),
            "timeout test must exercise a real provider request"
        );
        assert!(
            *saw_timeout.lock().expect("read timeout observation"),
            "OpenAI adapter must convert a deadline expiration into ModelError::Timeout"
        );
        let event_types: Vec<_> = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect();
        assert_eq!(
            event_types,
            [
                "kernel.model_started",
                "kernel.model_failed",
                "kernel.session_indeterminate",
            ],
            "a provider timeout must terminate the session before completion/approval"
        );
    }

    #[test]
    fn provider_rate_limit_terminates_session_indeterminate_without_approval() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind provider");
        let address = listener.local_addr().expect("provider address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            let body = r#"{"error":{"message":"slow down"}}"#;
            let reply = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: 9\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(reply.as_bytes())
                .expect("send provider rate limit");
        });

        let provider = OpenAiProvider::new(
            format!("http://{address}/v1/chat/completions"),
            "test-api-key",
            "test-model",
        );
        let events = run_with_provider(provider);
        server.join().expect("provider server thread");

        assert!(
            events
                .iter()
                .any(|event| event.event_type == "kernel.model_failed")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "kernel.session_indeterminate")
        );
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == "kernel.completion_accepted")
        );
    }
}

#[test]
fn openai_provider_parses_object_shaped_tool_call_arguments() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind object-args provider");
    let address = listener.local_addr().expect("object-args provider address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept object-args provider");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        // Ollama shape: arguments is a JSON object, not a string.
        let response = r#"{"id":"r","model":"test-model","choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"c1","type":"function","function":{"name":"echo","arguments":{"value":"returned-by-provider"}}}]}}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream
            .write_all(reply.as_bytes())
            .expect("send object-args response");
    });

    let request = CanonicalModelRequest {
        instructions: vec![],
        context: vec![],
        tools: vec![],
        history: vec![],
    };
    let mut provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-api-key",
        "test-model",
    );
    let response = provider
        .generate(&request)
        .expect("generate should succeed");
    server.join().expect("object-args server thread");

    assert_eq!(response.actions.len(), 1);
    match &response.actions[0] {
        ModelAction::ToolCall {
            action_id,
            tool,
            arguments,
        } => {
            assert_eq!(action_id, "c1");
            assert_eq!(tool, "echo");
            assert_eq!(
                arguments,
                &serde_json::json!({"value": "returned-by-provider"})
            );
        }
        other => panic!("expected a tool call, got {other:?}"),
    }
}

#[test]
fn openai_provider_rejects_non_object_string_tool_arguments() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind non-object-args provider");
    let address = listener
        .local_addr()
        .expect("non-object-args provider address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept non-object-args provider");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        // arguments is a JSON string that parses to a number, not an object.
        let response = r#"{"id":"r","model":"test-model","choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"c1","type":"function","function":{"name":"echo","arguments":"123"}}]}}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream
            .write_all(reply.as_bytes())
            .expect("send non-object-args response");
    });

    let request = CanonicalModelRequest {
        instructions: vec![],
        context: vec![],
        tools: vec![],
        history: vec![],
    };
    let mut provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-api-key",
        "test-model",
    );
    let response = provider
        .generate(&request)
        .expect("generate should succeed");
    server.join().expect("non-object-args server thread");

    assert_eq!(
        response.actions.len(),
        0,
        "a non-object string argument must be rejected (tool call dropped)"
    );
}

#[test]
fn openai_provider_parses_content_completion_into_completion_request() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind content-completion provider");
    let address = listener
        .local_addr()
        .expect("content-completion provider address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("accept content-completion provider");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        // No tool_calls; the model's findings arrive as message content (a JSON object).
        let response = r#"{"id":"r","model":"test-model","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"{\"findings\":[]}"}}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length:{}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream
            .write_all(reply.as_bytes())
            .expect("send content-completion response");
    });

    let request = CanonicalModelRequest {
        instructions: vec![],
        context: vec![],
        tools: vec![],
        history: vec![],
    };
    let mut provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-api-key",
        "test-model",
    );
    let response = provider
        .generate(&request)
        .expect("generate should succeed");
    server.join().expect("content-completion server thread");

    assert_eq!(response.actions.len(), 1);
    match &response.actions[0] {
        ModelAction::CompletionRequest { action_id, payload } => {
            assert_eq!(action_id, "completion");
            assert_eq!(payload, &serde_json::json!({"findings": []}));
        }
        other => panic!("expected a completion request, got {other:?}"),
    }
}
