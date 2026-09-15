use code_agent_runtime::provider::OpenAiProvider;

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
