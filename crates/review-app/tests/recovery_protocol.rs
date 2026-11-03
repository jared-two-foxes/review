use agent_kernel::application::AgentApplication;
use agent_kernel::ledger::LedgerEvent;
use agent_kernel::model::UsageRecord;
use review_app::{ReviewApplication, ReviewConfig, ReviewState};
use review_protocol::{ReviewReason, ReviewRequest, ReviewStatus};

#[test]
fn setup_failure_has_a_valid_inspectable_run_identity() {
    let request = ReviewRequest {
        schema: "review.request/v1".into(),
        repository_path: "/definitely-not-a-review-repository".into(),
        base_ref: "HEAD".into(),
        head_ref: ":working".into(),
        requirements: None,
    };
    let (result, _, setup_error) = review_app::run_review(&request, &ReviewConfig::default(), None);
    assert!(matches!(result.status, ReviewStatus::Indeterminate));
    assert!(!result.review_id.is_empty());
    assert!(!result.completed_at.is_empty());
    assert!(setup_error.is_some());
}

#[test]
fn typed_provider_failures_survive_the_kernel_boundary_without_secret_details() {
    for (category, expected) in [
        ("authentication", ReviewReason::AuthenticationFailure),
        ("network", ReviewReason::NetworkFailure),
        ("provider", ReviewReason::ProviderFailure),
        ("rate_limit", ReviewReason::RateLimited),
    ] {
        let app = ReviewApplication::new_with_sources(agent_protocol::SystemClock::new(), agent_protocol::RandomIdGenerator::new());
        let state = app.reduce_event(&ReviewState::default(), &LedgerEvent {
            event_type: "kernel.model_failed".into(),
            details: Some(serde_json::json!({"category": category}).to_string()),
            ..Default::default()
        });
        let result = app.build_terminal_result(&state, &UsageRecord {
            input_tokens: 10, output_tokens: 0, estimated_cost_usd: Some(0.01),
        });
        assert_eq!(serde_json::to_value(&result.reason).unwrap(),
                   serde_json::to_value(expected).unwrap());
        assert!(matches!(result.status, ReviewStatus::Indeterminate));
        assert_eq!(result.usage.input_tokens, 10);
    }
}
