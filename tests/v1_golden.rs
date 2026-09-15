use std::fs;

use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::ledger::{LedgerEvent, Limits};
use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
    UsageRecord,
};
use agent_kernel::tools::ToolCatalog;
use agent_protocol::{FixedClock, SequenceIdGenerator};
use review_app::{ReadChangeTool, ReviewApplication};
use review_protocol::{ReviewRequest, ReviewResult};
use serde_json::json;

fn assert_golden(events: &[LedgerEvent], result: &ReviewResult, scenario: u32) {
    let actual_events = serde_json::to_vec_pretty(events).unwrap();
    let golden_events = fs::read(format!(
        "tests/fixtures/v1/golden-events-scenario-{scenario}.json"
    ))
    .unwrap();
    assert_eq!(
        actual_events, golden_events,
        "events mismatch for scenario {scenario}"
    );

    let actual_result = serde_json::to_vec_pretty(result).unwrap();
    let golden_result = fs::read(format!(
        "tests/fixtures/v1/golden-result-scenario-{scenario}.json"
    ))
    .unwrap();
    assert_eq!(
        actual_result, golden_result,
        "result mismatch for scenario {scenario}"
    );
}

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

fn build_limits(max_turns: u32) -> Limits {
    Limits {
        max_turns,
        max_tool_calls: 10,
        max_completion_attempts: 10,
        wall_clock_budget: None,
        ledger_path: None,
        max_repeated_actions: 3,
        max_input_tokens: None,
        max_cost_usd: None,
    }
}

fn build_catalog() -> ToolCatalog {
    let mut catalog = ToolCatalog::new();
    catalog.register(Box::new(ReadChangeTool));
    catalog
}

fn build_request() -> ReviewRequest {
    ReviewRequest {
        schema: "review.request/v1".into(),
        repository_path: ".".into(),
        base_ref: "HEAD~1".into(),
        head_ref: "HEAD".into(),
        requirements: None,
    }
}

#[test]
fn scenario_1_golden_match() {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_ids = SequenceIdGenerator::new(["ses-001", "exec-001"]);
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "get_change_summary".into(),
                arguments: json!({}),
            }],
            usage: Some(UsageRecord {
                input_tokens: 10,
                output_tokens: 4,
                estimated_cost_usd: Some(0.01),
            }),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-2".into(),
                payload: json!({"findings": []}),
            }],
            usage: Some(UsageRecord {
                input_tokens: 20,
                output_tokens: 8,
                estimated_cost_usd: Some(0.02),
            }),
        },
    ]);
    let (result, events) = SessionCoordinator::new(
        ReviewApplication::new_with_sources(clock, app_ids),
        provider,
        coord_ids,
        build_catalog(),
        build_limits(10),
    )
    .run_full(build_request());
    assert_eq!(result.usage.input_tokens, 30);
    assert_eq!(result.usage.output_tokens, 12);
    assert_eq!(result.usage.estimated_cost_usd, Some(0.03));
    assert_golden(&events, &result, 1);
}

#[test]
fn scenario_2_golden_match() {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_ids = SequenceIdGenerator::new(["ses-001", "exec-001", "exec-002", "exec-003"]);
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-1".into(),
                payload: json!({"findings": []}),
            }],
            usage: Some(UsageRecord {
                input_tokens: 1,
                output_tokens: 2,
                estimated_cost_usd: Some(0.1),
            }),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-2".into(),
                tool: "get_change_summary".into(),
                arguments: json!({}),
            }],
            usage: Some(UsageRecord {
                input_tokens: 3,
                output_tokens: 4,
                estimated_cost_usd: Some(0.2),
            }),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-3".into(),
                payload: json!({"findings": []}),
            }],
            usage: Some(UsageRecord {
                input_tokens: 5,
                output_tokens: 6,
                estimated_cost_usd: Some(0.3),
            }),
        },
    ]);
    let (result, events) = SessionCoordinator::new(
        ReviewApplication::new_with_sources(clock, app_ids),
        provider,
        coord_ids,
        build_catalog(),
        build_limits(10),
    )
    .run_full(build_request());
    assert_eq!(result.usage.input_tokens, 9);
    assert_eq!(result.usage.output_tokens, 12);
    let cost = result
        .usage
        .estimated_cost_usd
        .expect("usage cost should be present");
    assert!((cost - 0.6).abs() < f64::EPSILON * 4.0);
    assert_golden(&events, &result, 2);
}

#[test]
fn scenario_3_golden_match() {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_ids = SequenceIdGenerator::new(["ses-001", "exec-001"]);
    let provider = ScriptedModelProvider::new(vec![CanonicalModelResponse {
        actions: vec![ModelAction::ToolCall {
            action_id: "act-1".into(),
            tool: "nonexistent_tool".into(),
            arguments: json!({}),
        }],
        usage: Some(UsageRecord {
            input_tokens: 7,
            output_tokens: 9,
            estimated_cost_usd: Some(0.7),
        }),
    }]);
    let (result, events) = SessionCoordinator::new(
        ReviewApplication::new_with_sources(clock, app_ids),
        provider,
        coord_ids,
        build_catalog(),
        build_limits(1),
    )
    .run_full(build_request());
    assert_eq!(result.usage.input_tokens, 7);
    assert_eq!(result.usage.output_tokens, 9);
    assert_eq!(result.usage.estimated_cost_usd, Some(0.7));
    assert_golden(&events, &result, 3);
}

#[test]
fn scenario_4_golden_match() {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_ids = SequenceIdGenerator::new(["ses-001", "exec-001"]);
    let provider = ScriptedModelProvider::new(vec![CanonicalModelResponse {
        actions: vec![ModelAction::ToolCall {
            action_id: "act-1".into(),
            tool: "get_change_summary".into(),
            arguments: json!("not-an-object"),
        }],
        usage: Some(UsageRecord {
            input_tokens: 11,
            output_tokens: 13,
            estimated_cost_usd: Some(0.11),
        }),
    }]);
    let (result, events) = SessionCoordinator::new(
        ReviewApplication::new_with_sources(clock, app_ids),
        provider,
        coord_ids,
        build_catalog(),
        build_limits(1),
    )
    .run_full(build_request());
    assert_eq!(result.usage.input_tokens, 11);
    assert_eq!(result.usage.output_tokens, 13);
    assert_eq!(result.usage.estimated_cost_usd, Some(0.11));
    assert_golden(&events, &result, 4);
}
