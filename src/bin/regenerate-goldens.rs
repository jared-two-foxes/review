use std::fs;

use agent_kernel::{
    coordinator::SessionCoordinator,
    limits::Limits,
    model::{
        CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
        UsageRecord,
    },
    tools::ToolCatalog,
};
use agent_protocol::{FixedClock, SequenceIdGenerator};
use review_app::{ReadChangeTool, ReviewApplication, run_review_with_sources};
use review_protocol::ReviewRequest;
use serde_json::json;

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

fn usage(i: u64, o: u64, c: f64) -> Option<UsageRecord> {
    Some(UsageRecord {
        input_tokens: i,
        output_tokens: o,
        estimated_cost_usd: Some(c),
    })
}

fn write_v0_golden() {
    let request: ReviewRequest = serde_json::from_str(
        &fs::read_to_string("tests/fixtures/v0/minimal-request.json")
            .expect("minimal request fixture should exist"),
    )
    .expect("minimal request fixture should parse");

    let result = run_review_with_sources(
        &request,
        &FixedClock::new("2025-01-01T00:00:00Z"),
        &mut SequenceIdGenerator::new(["result-0001"]),
    );

    fs::write("tests/fixtures/v0/golden-result.json", &result).expect("write v0 golden result");
    println!(
        "wrote tests/fixtures/v0/golden-result.json ({} bytes)",
        result.len()
    );
}

fn run_scenario(
    scenario: u32,
    responses: Vec<CanonicalModelResponse>,
    max_turns: u32,
    coord_ids: Vec<&str>,
) {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    let app_ids = SequenceIdGenerator::new(["rev-001"]);
    let coord_id_gen = SequenceIdGenerator::new(coord_ids);
    let provider = ScriptedModelProvider::new(responses);

    let (result, events) = SessionCoordinator::new(
        ReviewApplication::new_with_sources(clock, app_ids),
        provider,
        coord_id_gen,
        build_catalog(),
        build_limits(max_turns),
    )
    .run_full(build_request(), None);

    let events_json = serde_json::to_vec_pretty(&events).unwrap();
    let result_json = serde_json::to_vec_pretty(&result).unwrap();

    let events_path = format!("tests/fixtures/v1/golden-events-scenario-{scenario}.json");
    let result_path = format!("tests/fixtures/v1/golden-result-scenario-{scenario}.json");

    fs::write(&events_path, &events_json).expect("write golden events");
    fs::write(&result_path, &result_json).expect("write golden result");
    println!("wrote {} ({} bytes)", events_path, events_json.len());
    println!("wrote {} ({} bytes)", result_path, result_json.len());
}

fn main() {
    println!("Regenerating golden fixtures...");

    // v0: replay result (compact JSON, matching run_review_with_sources output)
    write_v0_golden();

    // v1 scenario 1: read_change → completion with no findings → APPROVED
    run_scenario(
        1,
        vec![
            CanonicalModelResponse {
                actions: vec![ModelAction::ToolCall {
                    action_id: "act-1".into(),
                    tool: "get_change_summary".into(),
                    arguments: json!({}),
                }],
                usage: usage(10, 4, 0.01),
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::CompletionRequest {
                    action_id: "act-2".into(),
                    payload: json!({"findings": []}),
                }],
                usage: usage(20, 8, 0.02),
            },
        ],
        10,
        vec!["ses-001", "exec-001"],
    );

    // v1 scenario 2: premature completion → rejected → read_change → completion → APPROVED
    run_scenario(
        2,
        vec![
            CanonicalModelResponse {
                actions: vec![ModelAction::CompletionRequest {
                    action_id: "act-1".into(),
                    payload: json!({"findings": []}),
                }],
                usage: usage(1, 2, 0.1),
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::ToolCall {
                    action_id: "act-2".into(),
                    tool: "get_change_summary".into(),
                    arguments: json!({}),
                }],
                usage: usage(3, 4, 0.2),
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::CompletionRequest {
                    action_id: "act-3".into(),
                    payload: json!({"findings": []}),
                }],
                usage: usage(5, 6, 0.3),
            },
        ],
        10,
        vec!["ses-001", "exec-001", "exec-002", "exec-003"],
    );

    // v1 scenario 3: nonexistent tool → rejected → max_turns(1) → INDETERMINATE
    run_scenario(
        3,
        vec![CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "nonexistent_tool".into(),
                arguments: json!({}),
            }],
            usage: usage(7, 9, 0.7),
        }],
        1,
        vec!["ses-001", "exec-001"],
    );

    // v1 scenario 4: valid tool but invalid args → rejected → max_turns(1) → INDETERMINATE
    run_scenario(
        4,
        vec![CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "get_change_summary".into(),
                arguments: json!("not-an-object"),
            }],
            usage: usage(11, 13, 0.11),
        }],
        1,
        vec!["ses-001", "exec-001"],
    );

    println!("Done. Golden fixtures regenerated.");
}
