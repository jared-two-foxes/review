use std::io::Write;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use code_agent_runtime::provider::{ApiStyle, OpenAiProvider, ProviderRoute};
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
    //listener.set_nonblocking(true).expect("configure listener");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept provider request");
        let response = r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"secret provider response"}}], "usage":{}}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream.write_all(reply.as_bytes()).expect("write response");
    });
    (address, server)
}

#[test]
fn openai_provider_configures_content_tracing_from_environment() {
    let previous = std::env::var("REVIEW_TRACE_CONTENT").ok();
    let _restore_environment = TraceContentEnvironmentRestore(previous);

    unsafe {
        std::env::remove_var("REVIEW_TRACE_CONTENT");
    }
    let unset_provider = OpenAiProvider::new(ProviderRoute {
        provider_root: "http://localhost".into(),
        provider: "openai".into(),
        api_key: "test-key".into(),
        model: "test-model".into(),
        api_style: ApiStyle::ChatCompletions,
    });
    assert!(!unset_provider.trace_content);

    unsafe {
        std::env::set_var("REVIEW_TRACE_CONTENT", "");
    }
    let empty_provider = OpenAiProvider::new(ProviderRoute {
        provider_root: "http://localhost".into(),
        provider: "openai".into(),
        api_key: "test-key".into(),
        model: "test-model".into(),
        api_style: ApiStyle::ChatCompletions,
    });
    assert!(!empty_provider.trace_content);

    unsafe {
        std::env::set_var("REVIEW_TRACE_CONTENT", "enabled");
    }
    let opted_in_provider = OpenAiProvider::new(ProviderRoute {
        provider_root: "http://localhost".into(),
        provider: "openai".into(),
        api_key: "test-key".into(),
        model: "test-model".into(),
        api_style: ApiStyle::ChatCompletions,
    });
    assert!(opted_in_provider.trace_content);
}
