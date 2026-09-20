use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::limits::Limits;
use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
    UsageRecord,
};
use agent_kernel::tools::{Tool, ToolCatalog, ToolResult, ToolStatus};
use agent_protocol::SequenceIdGenerator;
use code_agent_runtime::identity::content_id_for_bytes;
use implement_app::{
    ImplementApplication, ImplementConfig, ImplementReason, ImplementRequest, ImplementStatus,
    run_implement, run_implement_with_provider,
};
use serde_json::{Value, json};
use std::process::Command;
use std::time::Instant;

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
        _deadline: Instant,
    ) -> Result<CanonicalModelResponse, ModelError> {
        self.generate(request)
    }
}

struct FakeReadFileTool;

impl Tool for FakeReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> agent_kernel::model::ToolDescription {
        agent_kernel::model::ToolDescription {
            name: self.name().into(),
            description: "Read a file".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
            }),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.get("path").and_then(Value::as_str).is_some() {
            Ok(())
        } else {
            Err("missing path".into())
        }
    }

    fn execute(&self, arguments: &Value) -> ToolResult {
        let replacement = if arguments["path"] == "src/app.txt" {
            "after"
        } else {
            "desired content"
        };
        let content_id = if arguments["path"] == "src/app.txt" {
            content_id_for_bytes(replacement.as_bytes())
        } else {
            content_id_for_bytes("desired content".as_bytes())
        };
        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "content": replacement,
                "content_id": content_id,
                "truncated": false,
                "completeness": true,
                "observed_head": "working",
            }),
        }
    }
}

struct FakeReplaceTool;

impl Tool for FakeReplaceTool {
    fn name(&self) -> &str {
        "replace_file_content"
    }

    fn description(&self) -> agent_kernel::model::ToolDescription {
        agent_kernel::model::ToolDescription {
            name: self.name().into(),
            description: "Replace file content".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path", "expected", "replacement"],
                "properties": {
                    "path": { "type": "string" },
                    "expected": { "type": "string" },
                    "replacement": { "type": "string" }
                }
            }),
        }
    }

    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        if arguments.get("path").and_then(Value::as_str).is_some()
            && arguments.get("expected").and_then(Value::as_str).is_some()
            && arguments
                .get("replacement")
                .and_then(Value::as_str)
                .is_some()
        {
            Ok(())
        } else {
            Err("invalid arguments".into())
        }
    }

    fn execute(&self, arguments: &Value) -> ToolResult {
        let replacement = arguments["replacement"].as_str().unwrap();
        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({
                "path": arguments["path"].as_str().unwrap(),
                "content_id": content_id_for_bytes(replacement.as_bytes()),
                "bytes_written": replacement.len(),
                "observed_head": "working",
            }),
        }
    }
}

fn scripted_usage() -> Option<UsageRecord> {
    Some(UsageRecord {
        input_tokens: 2,
        output_tokens: 1,
        estimated_cost_usd: Some(0.01),
    })
}

#[test]
fn scripted_second_consumer_reaches_candidate_ready() {
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "read_file".into(),
                arguments: json!({"path": "src/app.txt"}),
            }],
            usage: scripted_usage(),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-2".into(),
                tool: "replace_file_content".into(),
                arguments: json!({
                    "path": "src/app.txt",
                    "expected": "before",
                    "replacement": "after"
                }),
            }],
            usage: scripted_usage(),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-3".into(),
                tool: "read_file".into(),
                arguments: json!({"path": "src/app.txt"}),
            }],
            usage: scripted_usage(),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-4".into(),
                payload: json!({
                    "ready": true,
                    "summary": "Updated src/app.txt with the requested replacement."
                }),
            }],
            usage: scripted_usage(),
        },
    ]);

    let mut catalog = ToolCatalog::new();
    catalog.register(Box::new(FakeReadFileTool));
    catalog.register(Box::new(FakeReplaceTool));

    let coordinator = SessionCoordinator::new(
        ImplementApplication::new_with_sources(
            agent_protocol::FixedClock::new("2025-01-01T00:00:00Z"),
            SequenceIdGenerator::new(["impl-001"]),
        ),
        provider,
        SequenceIdGenerator::new([
            "session-001",
            "exec-001",
            "exec-002",
            "exec-003",
            "exec-004",
        ]),
        catalog,
        Limits {
            max_turns: 10,
            max_tool_calls: 10,
            max_completion_attempts: 10,
            wall_clock_budget: None,
            ledger_path: None,
            max_repeated_actions: 3,
            max_input_tokens: None,
            max_cost_usd: None,
        },
    );

    let (result, events) = coordinator.run_full(
        ImplementRequest {
            repository_path: ".".into(),
            target_path: "src/app.txt".into(),
            expected_content: "before".into(),
            desired_content: "after".into(),
        },
        None,
    );

    assert_eq!(result.status, ImplementStatus::CandidateReady);
    assert_eq!(result.reason, ImplementReason::CandidateReady);
    assert!(result.applied);
    assert!(result.verified);
    assert_eq!(
        result.summary.as_deref(),
        Some("Updated src/app.txt with the requested replacement.")
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "kernel.tool_completed")
            .count(),
        3
    );
}

fn make_repo_with_target() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/app.txt"), "before\n").unwrap();
    dir
}

#[test]
fn temporary_repository_flow_applies_bounded_mutation() {
    let repo = make_repo_with_target();
    let request = ImplementRequest {
        repository_path: repo.path().to_string_lossy().into_owned(),
        target_path: "src/app.txt".into(),
        expected_content: "before\n".into(),
        desired_content: "after\n".into(),
    };
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "inspect".into(),
                tool: "read_file".into(),
                arguments: json!({"path": "src/app.txt"}),
            }],
            usage: scripted_usage(),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "mutate".into(),
                tool: "replace_file_content".into(),
                arguments: json!({
                    "path": "src/app.txt",
                    "expected": "before\n",
                    "replacement": "after\n"
                }),
            }],
            usage: scripted_usage(),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "verify".into(),
                tool: "read_file".into(),
                arguments: json!({"path": "src/app.txt"}),
            }],
            usage: scripted_usage(),
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "complete".into(),
                payload: json!({
                    "ready": true,
                    "summary": "Applied the requested replacement to src/app.txt."
                }),
            }],
            usage: scripted_usage(),
        },
    ]);

    let (result, events) =
        run_implement_with_provider(&request, &ImplementConfig::default(), provider, None)
            .expect("implementor flow should succeed");

    assert_eq!(result.status, ImplementStatus::CandidateReady);
    assert!(result.applied);
    assert!(result.verified);
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/app.txt")).unwrap(),
        "after\n"
    );
    assert!(events.iter().any(|event| {
        event.event_type == "kernel.completion_accepted" && !event.execution_id.is_empty()
    }));
}

#[test]
fn run_implement_rejects_unknown_provider_prefix() {
    let request = ImplementRequest {
        repository_path: ".".into(),
        target_path: "src/app.txt".into(),
        expected_content: "before".into(),
        desired_content: "after".into(),
    };
    let config = ImplementConfig {
        model: "anthropic/claude".into(),
        ..ImplementConfig::default()
    };

    let error = run_implement(&request, &config, None)
        .expect_err("unknown provider prefixes must be rejected");
    assert!(error.contains("unsupported model provider prefix"));
}
