// Application scaffolding for the end-to-end test.

use agent_kernel::application::{
    AgentApplication, ApplicationDescriptor, ApplicationInitialization, CompletionDecision,
    ContextBlock, InstructionBlock,
};
use agent_kernel::{
    ledger::LedgerEvent,
    model::ToolDescription,
    tools::{Tool, ToolResult, ToolStatus},
};
use agent_protocol::{Clock, IdGenerator};
use review_protocol::{ReviewReason, ReviewRequest, ReviewResult, ReviewStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;

pub fn run_review(request: &ReviewRequest) -> ReviewResult {
    ReviewResult {
        schema: "review.result/v1".to_string(),
        status: ReviewStatus::Indeterminate,
        reason: ReviewReason::ReviewEngineNotAvailable,
        review_id: String::new(),
        completed_at: String::new(),
    }
}

/// Deterministic output seam for replay tests.
///
/// This is deliberately only a compile-time seam until the canonical result
/// envelope is implemented.
pub fn run_review_with_sources<C: Clock, I: IdGenerator>(
    request: &ReviewRequest,
    clock: &C,
    ids: &mut I,
) -> Vec<u8> {
    let result = ReviewResult {
        schema: "review.result/v1".to_string(),
        status: ReviewStatus::Indeterminate,
        reason: ReviewReason::ReviewEngineNotAvailable,
        review_id: ids.next_id(),
        completed_at: clock.now().to_string(),
    };
    serde_json::to_vec(&result).expect("review result is serializable")
}

#[derive(Debug, Clone)]
pub struct ReviewState {
    inspected: bool,
    findings: Vec<Finding>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ReviewCompletion {
    pub findings: Vec<Finding>,
}

#[derive(Clone, Deserialize, Debug)]
pub struct Finding {
    pub blocking: bool,
    pub message: String,
}

#[derive(Debug)]
pub struct ReviewError(String);

pub struct ReviewApplication {
    pending_completion: RefCell<Option<ReviewCompletion>>,
    completion_accepted: RefCell<bool>,
    clock: RefCell<Box<dyn Clock>>,
    id_gen: RefCell<Box<dyn IdGenerator>>,
}

impl ReviewApplication {
    // pub fn new() -> Self {
    //     ReviewApplication {
    //         pending_completion: RefCell::new(None),
    //     }
    // }

    pub fn new_with_sources<C: Clock + 'static, I: IdGenerator + 'static>(
        clock: C,
        id_gen: I,
    ) -> Self {
        ReviewApplication {
            pending_completion: RefCell::new(None),
            completion_accepted: RefCell::new(false),
            clock: RefCell::new(Box::new(clock)),
            id_gen: RefCell::new(Box::new(id_gen)),
        }
    }
}

impl AgentApplication for ReviewApplication {
    type Request = ReviewRequest;
    type State = ReviewState;
    type Completion = ReviewCompletion;
    type Result = ReviewResult;
    type Error = ReviewError;

    fn descriptor(&self) -> ApplicationDescriptor {
        ApplicationDescriptor {
            application_id: "review".into(),
            application_version: "0.1.0".into(),
            request_schema: "review.request/v1".into(),
            completion_schema: "review.completion/v1".into(),
            result_schema: "review.result/v1".into(),
            domain_event_namespace: "review".into(),
        }
    }

    fn initialize(
        &self,
        request: &Self::Request,
    ) -> Result<ApplicationInitialization<Self::State>, Self::Error> {
        Ok(ApplicationInitialization {
            initial_state: ReviewState {
                inspected: false,
                findings: vec![],
            },
            requested_tools: vec!["read_change".into()],
            requested_capabilities: vec![],
            application_limits: None,
        })
    }

    fn build_system_instructions(&self, state: &Self::State) -> Vec<InstructionBlock> {
        vec![]
    }

    fn build_context(&self, state: &Self::State) -> Vec<ContextBlock> {
        vec![]
    }

    fn validate_request(&self, request: &Self::Request) -> Result<(), Self::Error> {
        review_protocol::validate_request(request).map_err(|msg| ReviewError(msg))
    }

    fn validate_completion(
        &self,
        state: &Self::State,
        completion: &Self::Completion,
    ) -> CompletionDecision {
        if !state.inspected {
            CompletionDecision::RejectedRemediable {
                reason_codes: vec!["change_not_inspected".into()],
                missing_requirements: vec!["The change has not been inspected".into()],
                feedback_for_model: vec![InstructionBlock {
                    content: "You must call the read_change tool before completing.".into(),
                }],
            }
        } else {
            *self.pending_completion.borrow_mut() = Some(completion.clone());
            *self.completion_accepted.borrow_mut() = true;
            CompletionDecision::Accepted
        }
    }

    fn reduce_event(&self, state: &Self::State, event: &LedgerEvent) -> Self::State {
        match event.event_type.as_str() {
            "kernel.tool_completed" => ReviewState {
                inspected: true,
                findings: state.findings.clone(),
            },
            _ => state.clone(),
        }
    }

    fn parse_completion(&self, payload: &Value) -> Result<Self::Completion, Self::Error> {
        Ok(serde_json::from_value(payload.clone())
            .map_err(|e| ReviewError(format!("invalid completion payload: {}", e)))?)
    }

    fn build_terminal_result(&self, state: &Self::State) -> Self::Result {
        let accepted = *self.completion_accepted.borrow();
        let pending = self.pending_completion.borrow();
        let empty_binding = vec![];
        let findings = pending
            .as_ref()
            .map(|c| &c.findings)
            .unwrap_or(&empty_binding);
        let has_blocking = findings.iter().any(|f| f.blocking);
        let status = if !accepted {
            ReviewStatus::Indeterminate
        } else if has_blocking {
            ReviewStatus::ChangesRequested
        } else {
            ReviewStatus::Approved
        };

        ReviewResult {
            schema: "review.result/v1".to_string(),
            status,
            reason: if accepted {
                ReviewReason::ReviewCompleted
            } else {
                ReviewReason::ReviewEngineNotAvailable
            },
            review_id: self.id_gen.borrow_mut().next_id(),
            completed_at: self.clock.borrow().now().to_string(),
        }
    }
}

pub struct ReadChangeTool;

impl Tool for ReadChangeTool {
    fn name(&self) -> &str {
        "read_change"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: "read_change".into(),
            description: "Read the change summary for review.".into(),
            input_schema: serde_json::json!({"type":"object", "additionalProperties": false}),
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
            value: json!({"summary": "Modified file.rs: changed
 function signature"}),
        }
    }
}
