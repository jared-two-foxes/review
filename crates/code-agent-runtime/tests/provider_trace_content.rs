use std::io::Write;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use agent_kernel::application::InstructionBlock;
use agent_kernel::model::{CanonicalModelRequest, ModelProvider};
use code_agent_runtime::provider::OpenAiProvider;
use tracing_subscriber::fmt::MakeWriter;

struct TraceContentEnvironmentRestore(Option<String>);

impl Drop for TraceContentEnvironmentRestore {
    fn drop(&mut self) {
        unsafe {
            match &self.0 {
                Some(value) => std::env::set_var("REVIEW_TRACE_CONTENT", value),
                None => std::env::remove_var("REVIEW_TRACE_CONTENT"),
            }
        }
    }
}

#[derive(Clone)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriterGuard;

    fn make_writer(&'a self) -> Self::Writer {
        SharedWriterGuard(Arc::clone(&self.0))
    }
}

struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

impl Write for SharedWriterGuard {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn spawn_provider_server() -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test provider");
    let address = listener.local_addr().expect("read test provider address");
    listener.set_nonblocking(true).expect("configure listener");
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let response = r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"secret provider response"}}],"usage":{}}"#;
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    );
                    stream.write_all(reply.as_bytes()).expect("write response");
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        panic!("provider request timed out");
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept provider request: {error}"),
            }
        }
    });
    (address, server)
}

#[test]
fn openai_provider_does_not_trace_response_content_without_opt_in() {
    let previous = std::env::var("REVIEW_TRACE_CONTENT").ok();
    let _restore_environment = TraceContentEnvironmentRestore(previous);
    unsafe {
        std::env::remove_var("REVIEW_TRACE_CONTENT");
    }

    let output = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(SharedWriter(Arc::clone(&output)))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let request = CanonicalModelRequest {
        instructions: vec![InstructionBlock {
            content: "Return a completion.".into(),
        }],
        context: vec![],
        tools: vec![],
        history: vec![],
    };
    let (address, server) = spawn_provider_server();
    let mut provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-key",
        "test-model",
    );
    provider.generate(&request).expect("provider request succeeds");
    server.join().expect("provider server thread");

    let logs_without_opt_in = String::from_utf8(output.lock().unwrap().clone()).expect("logs are utf8");
    assert!(!logs_without_opt_in.contains("secret provider response"));
    assert!(!logs_without_opt_in.contains("content_preview"));

    unsafe {
        std::env::set_var("REVIEW_TRACE_CONTENT", "enabled");
    }
    let (address, server) = spawn_provider_server();
    let mut opted_in_provider = OpenAiProvider::new(
        format!("http://{address}/v1/chat/completions"),
        "test-key",
        "test-model",
    );
    opted_in_provider
        .generate(&request)
        .expect("opted-in provider request succeeds");
    server.join().expect("opted-in provider server thread");

    let logs_with_opt_in = String::from_utf8(output.lock().unwrap().clone()).expect("logs are utf8");
    assert!(logs_with_opt_in.contains("secret provider response"));
    assert!(logs_with_opt_in.contains("content_preview"));
}

#[test]
fn openai_provider_configures_content_tracing_from_environment() {
    let previous = std::env::var("REVIEW_TRACE_CONTENT").ok();
    let _restore_environment = TraceContentEnvironmentRestore(previous);

    unsafe {
        std::env::remove_var("REVIEW_TRACE_CONTENT");
    }
    let unset_provider = OpenAiProvider::new("http://localhost", "test-key", "test-model");
    assert!(!unset_provider.trace_content);

    unsafe {
        std::env::set_var("REVIEW_TRACE_CONTENT", "");
    }
    let empty_provider = OpenAiProvider::new("http://localhost", "test-key", "test-model");
    assert!(!empty_provider.trace_content);

    unsafe {
        std::env::set_var("REVIEW_TRACE_CONTENT", "enabled");
    }
    let opted_in_provider = OpenAiProvider::new("http://localhost", "test-key", "test-model");
    assert!(opted_in_provider.trace_content);
}
