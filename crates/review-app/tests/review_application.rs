use agent_kernel::{
    application::AgentApplication,
    coordinator::SessionCoordinator,
    ledger::{LedgerEvent, Limits},
    model::{
        CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
        ToolDescription,
    },
    tools::{Tool, ToolCatalog, ToolResult, ToolStatus},
};
use agent_protocol::{FixedClock, SequenceIdGenerator};
use review_app::{ReadChangeTool, ReviewApplication};
use review_protocol::{ReviewRequest, ReviewResult, ReviewStatus};
use serde_json::{Value, json};

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
    };
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_id_gen = SequenceIdGenerator::new(["rev-001"]);
    let app = ReviewApplication::new_with_sources(clock, app_id_gen);
    let coord_id_gen = SequenceIdGenerator::new(["ses-1", "exec-1", "exec-2"]);
    let coordinator = SessionCoordinator::new(app, provider, coord_id_gen, catalog, limits);
    let request = ReviewRequest {
        schema: "review.request/v1".into(),
        repository_path: ".".into(),
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
                tool: "read_change".into(),
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
                tool: "read_change".into(),
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
                tool: "read_change".into(),
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
