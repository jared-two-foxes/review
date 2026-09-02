use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use agent_kernel::application::{ContextBlock, InstructionBlock};
use agent_kernel::model::{CanonicalModelRequest, ModelError, ModelProvider};
use code_agent_runtime::provider::OpenAiProvider;

#[test]
fn openai_provider_aborts_generation_at_caller_deadline() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind delayed provider");
    let address = listener
        .local_addr()
        .expect("read delayed provider address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept delayed provider request");
        let mut request = [0_u8; 4096];
        let count = stream
            .read(&mut request)
            .expect("read delayed provider request");
        assert!(count > 0, "generation must issue a provider request");

        // The response arrives after the caller's deadline. A deadline-aware
        // adapter must return before this response is available.
        thread::sleep(Duration::from_millis(500));
        let response = r#"{"choices":[{"message":{"role":"assistant","content":"late"}}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        let _ = stream.write_all(reply.as_bytes());
    });

    let request = CanonicalModelRequest {
        instructions: vec![InstructionBlock {
            content: "Answer briefly.".into(),
        }],
        context: vec![ContextBlock {
            content: "This request intentionally has a delayed provider response.".into(),
        }],
        tools: vec![],
    };
    let mut provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-api-key",
        "test-model",
    );

    let deadline = Instant::now() + Duration::from_millis(50);
    let error = provider
        .generate_with_deadline(&request, deadline)
        .expect_err("a response arriving after the caller deadline must be rejected");

    match error {
        ModelError::Timeout(_) => {}
        other => panic!("deadline expiration must be reported as a timeout, got {other:?}"),
    }
    server.join().expect("delayed provider server thread");
}
