use agent_kernel::application::*;
use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::ledger::{LedgerEvent, Limits};
use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
    ToolDescription,
};
use agent_kernel::tools::{Tool, ToolCatalog, ToolResult, ToolStatus};
use agent_protocol::SequenceIdGenerator;
use serde_json::Value;
use std::panic::{catch_unwind, AssertUnwindSafe};

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
        if self.index >= self.responses.len() {
            panic!("No more scripted responses available");
        }
        let response = self.responses[self.index].clone();
        self.index += 1;
        Ok(response)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AdapterFailure {
    Network,
    Timeout,
    Api { status: u16 },
    RateLimited { retry_after_seconds: u64 },
}

/// A provider double which fails while the coordinator is making the model
/// call.  Each case is represented by a distinct typed failure rather than a
/// shared panic string, so this test exercises the complete failure taxonomy
/// that the adapter must expose to the coordinator.
struct FailingModelProvider {
    failure: AdapterFailure,
}

impl FailingModelProvider {
    fn new(failure: AdapterFailure) -> Self {
        Self { failure }
    }
}

impl ModelProvider for FailingModelProvider {
    fn generate(
        &mut self,
        _request: &CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse, ModelError> {
        Err(match &self.failure {
            AdapterFailure::Network => ModelError::Network("network error".into()),
            AdapterFailure::Timeout => ModelError::Timeout("timeout".into()),
            AdapterFailure::Api { status } => ModelError::ApiError(format!("API error: {status}")),
            AdapterFailure::RateLimited {
                retry_after_seconds,
            } => ModelError::RateLimit(format!("rate limited, retry after {retry_after_seconds}s")),
        })
    }
}

struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: self.name().into(),
            description: "Echoes back the input arguments.".into(),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }
    fn validate_arguments(&self, arguments: &Value) -> Result<(), String> {
        let schema = self.description().input_schema;
        let validator = jsonschema::validator_for(&schema)
            .map_err(|error| format!("invalid tool input schema: {error}"))?;
        validator
            .validate(arguments)
            .map_err(|error| format!("invalid arguments: {error}"))
    }
    fn execute(&self, arguments: &Value) -> ToolResult {
        ToolResult {
            status: ToolStatus::Succeeded,
            value: arguments.clone(),
        }
    }
}

fn build_coordinator(
    provider: impl ModelProvider,
) -> SessionCoordinator<EchoApp, impl ModelProvider, SequenceIdGenerator> {
    let mut catalog = ToolCatalog::new();
    catalog.register(Box::new(EchoTool));
    let limits = Limits {
        max_turns: 10,
        max_tool_calls: 10,
        max_completion_attempts: 10,
    };
    let id_gen = SequenceIdGenerator::new(["ses-1", "exec-1", "exec-2", "exec-3"]);
    SessionCoordinator::new(EchoApp {}, provider, id_gen, catalog, limits)
}

struct EchoRequest;
#[derive(Clone)]
struct EchoState {
    tool_calls: u32,
}
struct EchoCompletion;
struct EchoResult {
    tool_calls: u32,
}
#[derive(Debug)]
struct EchoError(String);

struct EchoApp {}

impl AgentApplication for EchoApp {
    type Request = EchoRequest;
    type State = EchoState;
    type Completion = EchoCompletion;
    type Result = EchoResult;
    type Error = EchoError;

    fn descriptor(&self) -> ApplicationDescriptor {
        ApplicationDescriptor {
            application_id: "echo".into(),
            application_version: "0.1.0".into(),
            request_schema: "echo.request/v1".into(),
            completion_schema: "echo.completion/v1".into(),
            result_schema: "echo.result/v1".into(),
            domain_event_namespace: "echo".into(),
        }
    }

    fn validate_request(&self, _request: &Self::Request) -> Result<(), Self::Error> {
        Ok(())
    }

    fn initialize(
        &self,
        _request: &Self::Request,
    ) -> Result<ApplicationInitialization<EchoState>, Self::Error> {
        Ok(ApplicationInitialization {
            initial_state: EchoState { tool_calls: 0 },
            requested_tools: vec!["echo".into()],
            requested_capabilities: vec![],
            application_limits: None,
        })
    }

    fn build_system_instructions(&self, _state: &EchoState) -> Vec<InstructionBlock> {
        vec![InstructionBlock {
            content: "You are an echo bot. Call the echo tool, then complete.".into(),
        }]
    }

    fn build_context(&self, _state: &EchoState) -> Vec<ContextBlock> {
        vec![]
    }

    fn reduce_event(&self, state: &EchoState, event: &LedgerEvent) -> EchoState {
        match event.event_type.as_str() {
            "kernel.tool_completed" => EchoState {
                tool_calls: state.tool_calls + 1,
            },
            _ => state.clone(),
        }
    }

    fn parse_completion(&self, _payload: &Value) -> Result<Self::Completion, Self::Error> {
        Ok(EchoCompletion)
    }

    fn validate_completion(
        &self,
        state: &EchoState,
        _completion: &EchoCompletion,
    ) -> CompletionDecision {
        if state.tool_calls > 0 {
            CompletionDecision::Accepted
        } else {
            CompletionDecision::RejectedRemediable {
                reason_codes: vec!["no_tool_calls".into()],
                missing_requirements: vec!["At least one tool call is required.".into()],
                feedback_for_model: vec![InstructionBlock {
                    content: "You must call the echo tool at least once before completing.".into(),
                }],
            }
        }
    }

    fn build_terminal_result(&self, state: &EchoState) -> Self::Result {
        EchoResult {
            tool_calls: state.tool_calls,
        }
    }
}

#[test]
fn happy_path_tool_call_then_completion() {
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "echo".into(),
                arguments: serde_json::json!({}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-2".into(),
                payload: serde_json::json!({}),
            }],
            usage: None,
        },
    ]);

    let coordinator = build_coordinator(provider);
    let result = coordinator.run(EchoRequest);

    assert_eq!(result.tool_calls, 1, "one tool call should have been made");
}

#[test]
fn premature_completion_rejected_then_succeeds() {
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-1".into(),
                payload: serde_json::json!({}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-2".into(),
                tool: "echo".into(),
                arguments: serde_json::json!({}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![ModelAction::CompletionRequest {
                action_id: "act-3".into(),
                payload: serde_json::json!({}),
            }],
            usage: None,
        },
    ]);

    let coordinator = build_coordinator(provider);
    let result = coordinator.run(EchoRequest);

    assert_eq!(result.tool_calls, 1, "one tool call after rejection");
}

#[test]
fn unknown_tool_rejected() {
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "nonexistent_tool".into(),
                arguments: serde_json::json!({}),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![
                ModelAction::ToolCall {
                    action_id: "act-2".into(),
                    tool: "echo".into(),
                    arguments: serde_json::json!({}),
                },
                ModelAction::CompletionRequest {
                    action_id: "act-3".into(),
                    payload: serde_json::json!({}),
                },
            ],
            usage: None,
        },
    ]);

    let coordinator = build_coordinator(provider);
    let result = coordinator.run(EchoRequest);

    assert_eq!(result.tool_calls, 1, "only the echo tool call counted");
}

#[test]
fn malformed_arguments_rejected() {
    let provider = ScriptedModelProvider::new(vec![
        CanonicalModelResponse {
            actions: vec![ModelAction::ToolCall {
                action_id: "act-1".into(),
                tool: "echo".into(),
                arguments: serde_json::json!("not-an-object"),
            }],
            usage: None,
        },
        CanonicalModelResponse {
            actions: vec![
                ModelAction::ToolCall {
                    action_id: "act-2".into(),
                    tool: "echo".into(),
                    arguments: serde_json::json!({}),
                },
                ModelAction::CompletionRequest {
                    action_id: "act-3".into(),
                    payload: serde_json::json!({}),
                },
            ],
            usage: None,
        },
    ]);

    let coordinator = build_coordinator(provider);
    let result = coordinator.run(EchoRequest);

    assert_eq!(result.tool_calls, 1, "only the valid tool call counted");
}

#[test]
fn model_generation_failure_ends_indeterminate_without_approval() {
    let failures = [
        AdapterFailure::Network,
        AdapterFailure::Timeout,
        AdapterFailure::Api { status: 500 },
        AdapterFailure::RateLimited {
            retry_after_seconds: 7,
        },
    ];

    let mut handled_failures = 0;
    for failure in failures {
        let expected_failure = failure.clone();
        let coordinator = build_coordinator(FailingModelProvider::new(failure));
        let execution = catch_unwind(AssertUnwindSafe(|| coordinator.run_full(EchoRequest)));

        match execution {
            Err(payload) => {
                // This assertion makes each input a distinct typed failure case,
                // rather than four labels attached to one undifferentiated panic.
                assert_eq!(
                    payload.downcast_ref::<AdapterFailure>(),
                    Some(&expected_failure),
                    "the adapter must preserve the typed failure class"
                );
            }
            Ok((result, events)) => {
                handled_failures += 1;
                assert_eq!(
                    result.tool_calls, 0,
                    "a failed model call must not execute tools"
                );
                assert!(
                    events
                        .iter()
                        .any(|event| event.event_type == "kernel.model_failed"),
                    "every provider failure must be surfaced as a model failure"
                );
                assert!(
                    events
                        .iter()
                        .any(|event| event.event_type == "kernel.session_indeterminate"),
                    "every provider failure must terminate through the indeterminate path"
                );
                assert!(
                    !events
                        .iter()
                        .any(|event| event.event_type == "kernel.completion_accepted"),
                    "a provider failure must never produce an approval"
                );
            }
        }
    }

    assert_eq!(
        handled_failures, 4,
        "network, timeout, API, and rate-limit failures must all be handled by the coordinator"
    );
}
