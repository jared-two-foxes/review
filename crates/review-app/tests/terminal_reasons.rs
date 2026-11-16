//! A session that stops early must say why in the result.  The coordinator has
//! to feed each terminal event to the application's reducer; these tests drive
//! the real coordinator and assert the reason a caller actually receives.
use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider, UsageRecord,
};
use review_app::{ReviewConfig, run_review_with_provider};
use review_protocol::{ReviewRequest, ReviewResult, ReviewStatus};
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git").current_dir(path).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@example.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    std::fs::write(dir.path().join("a.txt"), "old\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "base"]);
    std::fs::write(dir.path().join("a.txt"), "new\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "change"]);
    dir
}

fn request(path: &Path) -> ReviewRequest {
    ReviewRequest {
        schema: "review.request/v1".into(),
        repository_path: path.to_str().unwrap().into(),
        base_ref: "HEAD~1".into(),
        head_ref: "HEAD".into(),
        requirements: None,
    }
}

fn summary_call() -> ModelAction {
    ModelAction::ToolCall { action_id: "c1".into(), tool: "get_change_summary".into(), arguments: json!({}) }
}

/// Fails every generation with the given error.
struct Failing(fn() -> ModelError);
impl ModelProvider for Failing {
    fn generate(&mut self, _: &CanonicalModelRequest) -> Result<CanonicalModelResponse, ModelError> {
        Err((self.0)())
    }
    fn generate_with_deadline(&mut self, r: &CanonicalModelRequest, _: std::time::Instant) -> Result<CanonicalModelResponse, ModelError> {
        self.generate(r)
    }
}

/// Answers with one change-summary call, optionally reporting usage and/or
/// raising a cancellation flag while the request is in flight.
struct OneCall {
    usage: Option<UsageRecord>,
    cancel: Option<Arc<AtomicBool>>,
}
impl ModelProvider for OneCall {
    fn generate(&mut self, _: &CanonicalModelRequest) -> Result<CanonicalModelResponse, ModelError> {
        if let Some(flag) = &self.cancel {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(CanonicalModelResponse { actions: vec![summary_call()], usage: self.usage.clone(), reasoning: None })
    }
    fn generate_with_deadline(&mut self, r: &CanonicalModelRequest, _: std::time::Instant) -> Result<CanonicalModelResponse, ModelError> {
        self.generate(r)
    }
}

fn reason(result: &ReviewResult) -> String {
    serde_json::to_value(&result.reason).unwrap().as_str().unwrap().to_string()
}

fn run(provider: impl ModelProvider, config: ReviewConfig, cancel: Option<&AtomicBool>) -> ReviewResult {
    let dir = fixture();
    let (result, _events) = run_review_with_provider(&request(dir.path()), &config, provider, cancel).unwrap();
    assert!(matches!(result.status, ReviewStatus::Indeterminate));
    result
}

#[test]
fn model_failures_report_their_typed_reason() {
    let cases: [(fn() -> ModelError, &str); 5] = [
        (|| ModelError::Authentication("no".into()), "AUTHENTICATION_FAILURE"),
        (|| ModelError::Network("down".into()), "NETWORK_FAILURE"),
        (|| ModelError::Timeout("slow".into()), "NETWORK_FAILURE"),
        (|| ModelError::RateLimit("slow down".into()), "RATE_LIMITED"),
        (|| ModelError::ApiError("500".into()), "PROVIDER_FAILURE"),
    ];
    for (error, expected) in cases {
        let result = run(Failing(error), ReviewConfig::default(), None);
        assert_eq!(reason(&result), expected);
    }
}

#[test]
fn cancellation_before_the_first_turn_reports_cancelled() {
    let flag = AtomicBool::new(true);
    let result = run(OneCall { usage: None, cancel: None }, ReviewConfig::default(), Some(&flag));
    assert_eq!(reason(&result), "CANCELLED");
}

#[test]
fn cancellation_during_tool_dispatch_reports_cancelled() {
    let flag = Arc::new(AtomicBool::new(false));
    let result = run(
        OneCall { usage: None, cancel: Some(flag.clone()) },
        ReviewConfig::default(),
        Some(flag.as_ref()),
    );
    assert_eq!(reason(&result), "CANCELLED");
}

#[test]
fn exhausting_the_token_budget_reports_budget_exhausted() {
    let config = ReviewConfig { max_input_tokens: Some(50), ..Default::default() };
    let usage = UsageRecord { input_tokens: 100, output_tokens: 1, estimated_cost_usd: None };
    let result = run(OneCall { usage: Some(usage), cancel: None }, config, None);
    assert_eq!(reason(&result), "BUDGET_EXHAUSTED");
}

#[test]
fn exhausting_the_cost_budget_reports_budget_exhausted() {
    let config = ReviewConfig { max_cost_usd: Some(0.01), ..Default::default() };
    let usage = UsageRecord { input_tokens: 1, output_tokens: 1, estimated_cost_usd: Some(1.0) };
    let result = run(OneCall { usage: Some(usage), cancel: None }, config, None);
    assert_eq!(reason(&result), "BUDGET_EXHAUSTED");
}

#[test]
fn limits_and_stalls_keep_reporting_their_reasons() {
    let config = ReviewConfig { max_tool_calls: 1, max_turns: 10, ..Default::default() };
    // The second identical-or-not call exceeds the single allowed tool call.
    struct Twice(u32);
    impl ModelProvider for Twice {
        fn generate(&mut self, _: &CanonicalModelRequest) -> Result<CanonicalModelResponse, ModelError> {
            self.0 += 1;
            let call = ModelAction::ToolCall { action_id: format!("c{}", self.0), tool: "get_changed_files".into(), arguments: json!({}) };
            Ok(CanonicalModelResponse { actions: vec![call], usage: None, reasoning: None })
        }
        fn generate_with_deadline(&mut self, r: &CanonicalModelRequest, _: std::time::Instant) -> Result<CanonicalModelResponse, ModelError> {
            self.generate(r)
        }
    }
    assert_eq!(reason(&run(Twice(0), config, None)), "LIMIT_EXCEEDED");
}
