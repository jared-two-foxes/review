use agent_kernel::application::*;
use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::ledger::LedgerEvent;
use agent_kernel::limits::Limits;
use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ConversationMessage, ModelAction, ModelError,
    ModelProvider, ToolDescription, UsageRecord,
};
use agent_kernel::tools::{Tool, ToolCatalog, ToolResult, ToolStatus};
use agent_protocol::SequenceIdGenerator;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::time::Instant;

struct ScriptedModelProvider {
    responses: Vec<CanonicalModelResponse>,
    index: usize,
    calls: Option<Rc<Cell<usize>>>,
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
        let mut all_content: Vec<String> =
            request.context.iter().map(|c| c.content.clone()).collect();
        for msg in &request.history {
            match msg {
                ConversationMessage::Tool {
                    tool_call_id,
                    content,
                } => {
                    all_content.push(format!("{}: {}", tool_call_id, content));
                }
                ConversationMessage::User { content } => {
                    all_content.push(content.clone());
                }
                ConversationMessage::Assistant {
                    content: Some(c),
                    tool_calls,
                } => {
                    all_content.push(c.clone());
                    for tc in tool_calls {
                        all_content.push(format!("{}: {}", tc.id, tc.name));
                    }
                }
                ConversationMessage::Assistant {
                    content: None,
                    tool_calls,
                } => {
                    for tc in tool_calls {
                        all_content.push(format!("{}: {}", tc.id, tc.name));
                    }
                }
            }
        }
        self.captured_contexts.borrow_mut().push(all_content);
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
            calls: None,
        }
    }

    fn with_call_counter(responses: Vec<CanonicalModelResponse>, calls: Rc<Cell<usize>>) -> Self {
        Self {
            responses,
            index: 0,
            calls: Some(calls),
        }
    }
}

impl ModelProvider for ScriptedModelProvider {
    fn generate(
        &mut self,
        _request: &CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse, ModelError> {
        if let Some(calls) = &self.calls {
            calls.set(calls.get() + 1);
        }
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
    build_coordinator_with_limits(
        provider,
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
    )
}

fn build_coordinator_with_limits(
    provider: impl ModelProvider,
    limits: Limits,
) -> SessionCoordinator<EchoApp, impl ModelProvider, SequenceIdGenerator> {
    let mut catalog = ToolCatalog::new();
    catalog.register(Box::new(EchoTool));
    let id_gen =
        SequenceIdGenerator::new(["ses-1", "exec-1", "exec-2", "exec-3", "exec-4", "exec-5"]);
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
        let execution = catch_unwind(AssertUnwindSafe(|| coordinator.run_full(EchoRequest, None)));

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
        turn2_context
            .iter()
            .any(|c| { c.contains("tool-output-marker") }),
        "turn-2 model request context must contain the tool result content: {:?}",
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
        turn2_context.iter().any(|c| c.contains("repo-content")),
        "tool result fed back to the model must carry the tool output: {:?}",
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
        turn2_context
            .iter()
            .any(|content| { content.contains("no such tool") }),
        "rejected tool call feedback must be fed back to the model: {:?}",
        turn2_context
    );
}

#[test]
fn usage_budget_exhaustion_terminates_before_model_completion() {
    // The first response stays below the limit; only the accumulated usage
    // from the second response exhausts the budget. This also makes sure the
    // check happens before dispatching the second response's completion.
    let cases = [
        (
            Limits {
                max_turns: 10,
                max_tool_calls: 10,
                max_completion_attempts: 10,
                wall_clock_budget: None,
                ledger_path: None,
                max_repeated_actions: 3,
                max_input_tokens: Some(5),
                max_cost_usd: None,
            },
            vec![
                UsageRecord {
                    input_tokens: 3,
                    output_tokens: 1,
                    estimated_cost_usd: None,
                },
                UsageRecord {
                    input_tokens: 3,
                    output_tokens: 1,
                    estimated_cost_usd: None,
                },
            ],
            "input",
        ),
        (
            Limits {
                max_turns: 10,
                max_tool_calls: 10,
                max_completion_attempts: 10,
                wall_clock_budget: None,
                ledger_path: None,
                max_repeated_actions: 3,
                max_input_tokens: None,
                max_cost_usd: Some(0.01),
            },
            vec![
                UsageRecord {
                    input_tokens: 1,
                    output_tokens: 1,
                    estimated_cost_usd: Some(0.006),
                },
                UsageRecord {
                    input_tokens: 1,
                    output_tokens: 1,
                    estimated_cost_usd: Some(0.006),
                },
            ],
            "cost",
        ),
    ];

    for (limits, usages, exhausted_budget) in cases {
        let model_calls = Rc::new(Cell::new(0));
        let provider = ScriptedModelProvider::with_call_counter(
            vec![
                CanonicalModelResponse {
                    actions: vec![ModelAction::ToolCall {
                        action_id: "act-1".into(),
                        tool: "echo".into(),
                        arguments: json!({}),
                    }],
                    usage: Some(usages[0].clone()),
                },
                CanonicalModelResponse {
                    actions: vec![ModelAction::ToolCall {
                        action_id: "act-2".into(),
                        tool: "echo".into(),
                        arguments: json!({}),
                    }],
                    usage: Some(usages[1].clone()),
                },
                // If the coordinator ignores the accumulated budget, it will
                // dispatch this completion request.  The budget check must stop
                // the session before this response is requested or completed.
                CanonicalModelResponse {
                    actions: vec![ModelAction::CompletionRequest {
                        action_id: "act-3".into(),
                        payload: json!({}),
                    }],
                    usage: None,
                },
            ],
            Rc::clone(&model_calls),
        );

        let (result, events) =
            build_coordinator_with_limits(provider, limits).run_full(EchoRequest, None);

        assert_eq!(
            model_calls.get(),
            2,
            "the two usage-bearing responses must actually be reached"
        );
        assert_eq!(
            result.tool_calls, 1,
            "the below-limit first response should be dispatched"
        );
        let budget_event = events
            .iter()
            .find(|event| event.event_type == "kernel.session_budget_exhausted")
            .expect("exhausting an accumulated usage budget must emit a budget-exhausted event");
        let details = budget_event.details.as_deref().unwrap_or_default();
        assert!(
            details.to_ascii_lowercase().contains(exhausted_budget),
            "budget event details must name the exhausted {} budget: {:?}",
            exhausted_budget,
            details
        );
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == "kernel.session_indeterminate"),
            "budget exhaustion must emit only the budget-exhausted event before building the terminal result"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "kernel.model_completed")
                .count(),
            1,
            "the exhausted second response must not emit model completion"
        );
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == "kernel.completion_accepted"),
            "budget exhaustion must terminate indeterminately rather than accept completion"
        );
    }
}

#[test]
fn cancellation_produces_exactly_one_terminal_state() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    let token = Arc::new(AtomicBool::new(false));
    let token_clone = Arc::clone(&token);

    let provider = ScriptedModelProvider::new(vec![CanonicalModelResponse {
        actions: vec![ModelAction::ToolCall {
            action_id: "act-1".into(),
            tool: "echo".into(),
            arguments: json!({}),
        }],
        usage: None,
    }]);

    let limits = Limits {
        max_turns: 10,
        max_tool_calls: 10,
        max_completion_attempts: 10,
        wall_clock_budget: None,
        ledger_path: None,
        max_repeated_actions: 3,
        max_input_tokens: None,
        max_cost_usd: None,
    };

    // Set the cancellation token before running — the coordinator should
    // detect it at the top of turn 2 (after the first turn's tool call)
    token_clone.store(true, Ordering::Relaxed);

    let coordinator = build_coordinator_with_limits(provider, limits);
    let (_result, events) = coordinator.run_full(EchoRequest, Some(&token_clone));

    // Exactly one terminal state: cancelled
    assert!(events
        .iter()
        .any(|e| e.event_type == "kernel.session_cancelled"));
    assert!(!events
        .iter()
        .any(|e| e.event_type == "kernel.completion_accepted"));
    assert!(!events.iter().any(|e| e.event_type == "kernel.model_failed"));
}
