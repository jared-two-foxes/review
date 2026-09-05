use agent_kernel::application::*;
use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::ledger::{LedgerEvent, Limits};
use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
    ToolDescription, UsageRecord,
};
use agent_kernel::tools::{Tool, ToolCatalog, ToolResult, ToolStatus};
use agent_protocol::SequenceIdGenerator;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::time::Instant;

struct ScriptedModelProvider {
    responses: Vec<CanonicalModelResponse>,
    index: usize,
}

struct RecordingProvider {
    responses: Vec<CanonicalModelResponse>,
    index: usize,
    captured_contexts: Rc<RefCell<Vec<Vec<String>>>>,
}

impl ModelProvider for RecordingProvider {
    fn generate(
        &mut self,
        request: &CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse, ModelError> {
        self.captured_contexts
            .borrow_mut()
            .push(request.context.iter().map(|c| c.content.clone()).collect());
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

    fn generate_with_deadline(
        &mut self,
        request: &CanonicalModelRequest,
        _instant: Instant,
    ) -> Result<CanonicalModelResponse, ModelError> {
        self.generate(request)
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

    fn generate_with_deadline(
        &mut self,
        request: &CanonicalModelRequest,
        _deadline: Instant,
    ) -> Result<CanonicalModelResponse, ModelError> {
        self.generate(request)
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
        wall_clock_budget: None,
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

    fn build_terminal_result(&self, state: &EchoState, _usage: &UsageRecord) -> Self::Result {
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

#[test]
fn tool_result_is_fed_back_into_next_model_request() {
    let captured: Rc<RefCell<Vec<Vec<String>>>> = Rc::new(RefCell::new(vec![]));
    let provider = RecordingProvider {
        responses: vec![
            CanonicalModelResponse {
                actions: vec![ModelAction::ToolCall {
                    action_id: "act-1".into(),
                    tool: "echo".into(),
                    arguments: json!({"marker": "tool-output-marker"}),
                }],
                usage: None,
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::CompletionRequest {
                    action_id: "act-2".into(),
                    payload: json!({}),
                }],
                usage: None,
            },
        ],
        index: 0,
        captured_contexts: Rc::clone(&captured),
    };
    let coordinator = build_coordinator(provider);
    let _ = coordinator.run(EchoRequest);

    let captured = captured.borrow();
    assert!(
        captured.len() >= 2,
        "expected at least 2 model calls, got {}",
        captured.len()
    );
    let turn2_context = &captured[1];
    assert!(
        turn2_context.iter().any(|c| {
            c.contains("tool-output-marker") && c.contains("[untrusted repository content]")
        }),
        "turn-2 model request context must contain the tool result content with its untrusted label: {:?}",
        turn2_context
    );
}

#[test]
fn rejection_and_richer_feedback_are_fed_back_into_next_model_request() {
    let captured: Rc<RefCell<Vec<Vec<String>>>> = Rc::new(RefCell::new(vec![]));
    let provider = RecordingProvider {
        responses: vec![
            CanonicalModelResponse {
                actions: vec![ModelAction::ToolCall {
                    action_id: "act-1".into(),
                    tool: "echo".into(),
                    arguments: json!({"marker": "ok"}),
                }],
                usage: None,
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::ToolCall {
                    action_id: "act-2".into(),
                    tool: "nonexistent".into(),
                    arguments: json!({}),
                }],
                usage: None,
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::CompletionRequest {
                    action_id: "act-3".into(),
                    payload: json!({}),
                }],
                usage: None,
            },
        ],
        index: 0,
        captured_contexts: Rc::clone(&captured),
    };
    let coordinator = build_coordinator(provider);
    let _ = coordinator.run(EchoRequest);

    let captured = captured.borrow();
    assert!(
        captured.len() >= 3,
        "expected at least 3 model calls, got {}",
        captured.len()
    );
    let turn2 = &captured[1];
    assert!(
        turn2
            .iter()
            .any(|c| c.contains("act-1") && c.contains("Succeeded")),
        "turn-2 context must contain richer successful tool feedback (action id + status): {:?}",
        turn2
    );
    let turn3 = &captured[2];
    assert!(
        turn3.iter().any(|c| c.contains("no such tool")),
        "turn-3 context must contain corrective feedback for the rejected tool call: {:?}",
        turn3
    );
}

#[test]
fn untrusted_label_appears_in_tool_result_feedback() {
    let captured: Rc<RefCell<Vec<Vec<String>>>> = Rc::new(RefCell::new(vec![]));
    let provider = RecordingProvider {
        responses: vec![
            CanonicalModelResponse {
                actions: vec![ModelAction::ToolCall {
                    action_id: "act-1".into(),
                    tool: "echo".into(),
                    arguments: json!({"data": "repo-content"}),
                }],
                usage: None,
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::CompletionRequest {
                    action_id: "act-2".into(),
                    payload: json!({}),
                }],
                usage: None,
            },
        ],
        index: 0,
        captured_contexts: Rc::clone(&captured),
    };
    let coordinator = build_coordinator(provider);
    let _ = coordinator.run(EchoRequest);

    let captured = captured.borrow();
    assert!(captured.len() >= 2, "expected at least 2 model calls");
    let turn2_context = &captured[1];
    assert!(
        turn2_context
            .iter()
            .any(|c| c.contains("[untrusted repository content]")),
        "tool result fed back to the model must carry an untrusted label: {:?}",
        turn2_context
    );
}

#[test]
fn rejected_tool_result_is_labeled_as_untrusted_repository_content() {
    let captured: Rc<RefCell<Vec<Vec<String>>>> = Rc::new(RefCell::new(vec![]));
    let provider = RecordingProvider {
        responses: vec![
            CanonicalModelResponse {
                actions: vec![ModelAction::ToolCall {
                    action_id: "act-1".into(),
                    tool: "missing-tool".into(),
                    arguments: json!({}),
                }],
                usage: None,
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::ToolCall {
                    action_id: "act-2".into(),
                    tool: "echo".into(),
                    arguments: json!({}),
                }],
                usage: None,
            },
            CanonicalModelResponse {
                actions: vec![ModelAction::CompletionRequest {
                    action_id: "act-3".into(),
                    payload: json!({}),
                }],
                usage: None,
            },
        ],
        index: 0,
        captured_contexts: Rc::clone(&captured),
    };

    let coordinator = build_coordinator(provider);
    let _ = coordinator.run(EchoRequest);

    let captured = captured.borrow();
    assert!(captured.len() >= 2, "expected at least 2 model calls");
    let turn2_context = &captured[1];
    assert!(
        turn2_context.iter().any(|content| {
            content.contains("no such tool")
                && content.starts_with("[untrusted repository content]")
        }),
        "every tool-result context entry, including rejected calls, must be marked as untrusted repository content: {:?}",
        turn2_context
    );
}
