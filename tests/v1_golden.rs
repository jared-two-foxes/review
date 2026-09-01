use std::fs;

use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::ledger::{LedgerEvent, Limits};
use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
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
}

fn build_limits(max_turns: u32) -> Limits {
    Limits {
        max_turns,
        max_tool_calls: 10,
        max_completion_attempts: 10,
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
    }
}

#[test]
fn scenario_1_golden_match() {
    // 1. Set up deterministic sources
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_ids = SequenceIdGenerator::new(["ses-001", "exec-001"]);

    // 2. Build scripted provider for scenario 1
    //    (read_change → completion with no findings)
    let provider = ScriptedModelProvider::new(vec![
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

    // 3. Run through coordinator
    let (result, events) = SessionCoordinator::new(
        ReviewApplication::new_with_sources(clock, app_ids),
        provider,
        coord_ids,
        build_catalog(),
        build_limits(10),
    )
    .run_full(build_request());

    assert_golden(&events, &result, 1);
}

#[test]
fn scenario_2_golden_match() {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_ids = SequenceIdGenerator::new(["ses-001", "exec-001", "exec-002", "exec-003"]);

    // Turn 1: model requests completion before reading → rejected
    // Turn 2: model calls read_change → accepted
    // Turn 3: model requests completion again → accepted → APPROVED
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-1".into(),
                payload: json!({"findings": []}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-2".into(),
                tool: "read_change".into(),
                arguments: json!({}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-3".into(),
                payload: json!({"findings": []}),
            }],
            usage: None,
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

    assert_golden(&events, &result, 2);
}

#[test]
fn scenario_3_golden_match() {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_ids = SequenceIdGenerator::new(["ses-001", "exec-001"]);

    // Turn 1: model calls a tool NOT in the catalog → action_rejected
    // Turn 2: turn > max_turns(1) → exit → Indeterminate (completion never accepted)
    let provider = ScriptedModelProvider::new(vec![CanonicalModelResponse {
        actions: vec![ModelAction::ToolCall {
            action_id: "act-1".into(),
            tool: "nonexistent_tool".into(), // ← not in catalog
            arguments: json!({}),
        }],
        usage: None,
    }]);

    let (result, events) = SessionCoordinator::new(
        ReviewApplication::new_with_sources(clock, app_ids),
        provider,
        coord_ids,
        build_catalog(),
        build_limits(1), // ← exit after 1 turn
    )
    .run_full(build_request());

    assert_golden(&events, &result, 3);
}

#[test]
fn scenario_4_golden_match() {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_ids = SequenceIdGenerator::new(["ses-001", "exec-001"]);

    // Turn 1: model calls read_change with wrong argument type → validate_arguments
    //         fails → action_rejected (execute is NOT called)
    // Turn 2: turn > max_turns(1) → exit → Indeterminate
    let provider = ScriptedModelProvider::new(vec![CanonicalModelResponse {
        actions: vec![ModelAction::ToolCall {
            action_id: "act-1".into(),
            tool: "read_change".into(),        // ← tool EXISTS in catalog
            arguments: json!("not-an-object"), // ← but args fail schema validation
        }],
        usage: None,
    }]);

    let (result, events) = SessionCoordinator::new(
        ReviewApplication::new_with_sources(clock, app_ids),
        provider,
        coord_ids,
        build_catalog(),
        build_limits(1),
    )
    .run_full(build_request());

    assert_golden(&events, &result, 4);
}
