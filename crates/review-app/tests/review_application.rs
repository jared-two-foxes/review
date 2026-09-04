use agent_kernel::{
    application::AgentApplication,
    coordinator::SessionCoordinator,
    ledger::Limits,
    model::{
        CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
    },
    tools::ToolCatalog,
};
use agent_protocol::{FixedClock, RandomIdGenerator, SequenceIdGenerator, SystemClock};
use review_app::{ReadChangeTool, ReviewApplication};
use review_protocol::{ReviewRequest, ReviewResult, ReviewStatus};
use serde_json::json;

// ── Scripted provider (same pattern as fake_app) ──
struct ScriptedModelProvider {
    responses: Vec<CanonicalModelResponse>,
    index: usize,
}

impl ScriptedModelProvider {
    fn new(responses: Vec<CanonicalModelResponse>) -> Self {
        Self {
            responses,
            index: 0,
        }
    }
}

impl ModelProvider for ScriptedModelProvider {
    fn generate(
        &mut self,
        _request: &CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse, ModelError> {
        let response = self.responses[self.index].clone();
        self.index += 1;
        Ok(response)
    }

    fn generate_with_deadline(
        &mut self,
        request: &CanonicalModelRequest,
        _deadline: std::time::Instant,
    ) -> Result<CanonicalModelResponse, ModelError> {
        self.generate(request)
    }
}

// ── Helper ──
fn run_review(responses: Vec<CanonicalModelResponse>) -> ReviewResult {
    let provider = ScriptedModelProvider::new(responses);
    let mut catalog = ToolCatalog::new();
    catalog.register(Box::new(ReadChangeTool));
    let limits = Limits {
        max_turns: 10,
        max_tool_calls: 10,
        max_completion_attempts: 10,
        wall_clock_budget: None,
    };
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_id_gen = SequenceIdGenerator::new(["rev-001"]);
    let app = ReviewApplication::new_with_sources(clock, app_id_gen);
    let coord_id_gen = SequenceIdGenerator::new(["ses-1", "exec-1", "exec-2"]);
    let coordinator = SessionCoordinator::new(app, provider, coord_id_gen, catalog, limits);
    let request = ReviewRequest {
        schema: "review.request/v1".into(),
        repository_path: ".".into(),
        base_ref: "HEAD~1".into(),
        head_ref: "HEAD".into(),
    };
    coordinator.run(request)
}

// ── Tests ──

#[test]
fn read_change_then_completion_no_findings_produces_approved() {
    let result = run_review(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "get_change_summary".into(),
                arguments: json!({}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-2".into(),
                payload: json!({"findings": []}),
            }],
            usage: None,
        },
    ]);

    assert!(matches!(result.status, ReviewStatus::Approved));
}

#[test]
fn premature_completion_rejected_then_approved() {
    let result = run_review(vec![
        // Turn 1: try to complete before reading change — should be rejected
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-1".into(),
                payload: json!({"findings": []}),
            }],
            usage: None,
        },
        // Turn 2: read the change
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-2".into(),
                tool: "get_change_summary".into(),
                arguments: json!({}),
            }],
            usage: None,
        },
        // Turn 3: now completion should be accepted
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-3".into(),
                payload: json!({"findings": []}),
            }],
            usage: None,
        },
    ]);

    assert!(matches!(result.status, ReviewStatus::Approved));
}

#[test]
fn blocking_finding_produces_changes_requested() {
    let result = run_review(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "get_change_summary".into(),
                arguments: json!({}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-2".into(),
                payload: json!({
                    "findings": [{"blocking": true, "message": "bug
                found"}]
                }),
            }],
            usage: None,
        },
    ]);

    assert!(matches!(result.status, ReviewStatus::ChangesRequested));
}

#[test]
fn review_application_constructible_with_production_clock_and_id_generator() {
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "get_change_summary".into(),
                arguments: json!({}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-2".into(),
                payload: json!({"findings": []}),
            }],
            usage: None,
        },
    ]);
    let mut catalog = ToolCatalog::new();
    catalog.register(Box::new(ReadChangeTool));
    let limits = Limits {
        max_turns: 10,
        max_tool_calls: 10,
        max_completion_attempts: 10,
        wall_clock_budget: None,
    };
    let app = ReviewApplication::new_with_sources(SystemClock::new(), RandomIdGenerator::new());
    let coordinator =
        SessionCoordinator::new(app, provider, RandomIdGenerator::new(), catalog, limits);
    let request = ReviewRequest {
        schema: "review.request/v1".into(),
        repository_path: ".".into(),
        base_ref: "HEAD~1".into(),
        head_ref: "HEAD".into(),
    };
    let result = coordinator.run(request);

    assert!(matches!(result.status, ReviewStatus::Approved));
    assert!(
        !result.review_id.is_empty(),
        "review_id must be populated by the production id generator"
    );
    assert!(
        !result.completed_at.is_empty(),
        "completed_at must be populated by the production clock"
    );
}

#[test]
fn build_system_instructions_returns_review_policy_naming_tool_and_completion() {
    let app = ReviewApplication::new_with_sources(
        FixedClock::new("2025-01-01T00:00:00Z"),
        SequenceIdGenerator::new(["rev-001"]),
    );
    let request = ReviewRequest {
        schema: "review.request/v1".into(),
        repository_path: ".".into(),
        base_ref: "HEAD~1".into(),
        head_ref: "HEAD".into(),
    };
    let init = app.initialize(&request).expect("initialize");
    let instructions = app.build_system_instructions(&init.initial_state);
    assert!(!instructions.is_empty(), "review policy must be non-empty");
    let content = &instructions[0].content;
    assert!(
        content.contains("get_change_summary"),
        "policy must direct the model to inspect the change: {content}"
    );
    assert!(
        content.contains("completion"),
        "policy must direct the model to issue a completion: {content}"
    );
}
