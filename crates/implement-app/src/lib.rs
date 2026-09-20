use agent_kernel::application::{
    AgentApplication, ApplicationDescriptor, ApplicationInitialization, CompletionDecision,
    ContextBlock, InstructionBlock,
};
use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::ledger::LedgerEvent;
use agent_kernel::limits::Limits;
use agent_kernel::model::{ModelProvider, UsageRecord};
use agent_protocol::{Clock, IdGenerator, RandomIdGenerator, SystemClock};
use code_agent_runtime::capabilities::{CodeToolCatalog, ScopedWrite};
use code_agent_runtime::identity::content_id_for_bytes;
use code_agent_runtime::provider::{OpenAiProvider, resolve_provider_route};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::SecurityPolicy;
use code_agent_runtime::target::ReviewTarget;
use code_agent_runtime::tools::{ReadFileTool, ReplaceFileContentTool};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub struct ImplementConfig {
    pub api_key: Option<String>,
    pub model: String,
    pub base_url: Option<String>,
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_completion_attempts: u32,
    pub wall_clock_budget: Option<Duration>,
    pub ledger_path: Option<PathBuf>,
    pub max_repeated_actions: u32,
    pub max_input_tokens: Option<u64>,
    pub max_cost_usd: Option<f64>,
}

impl Default for ImplementConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: "gpt-4o".to_string(),
            base_url: None,
            max_turns: 10,
            max_tool_calls: 10,
            max_completion_attempts: 3,
            wall_clock_budget: Some(Duration::from_secs(60)),
            ledger_path: None,
            max_repeated_actions: 3,
            max_input_tokens: None,
            max_cost_usd: None,
        }
    }
}

pub fn run_implement(
    request: &ImplementRequest,
    config: &ImplementConfig,
    cancel: Option<&AtomicBool>,
) -> Result<(ImplementResult, Vec<LedgerEvent>), String> {
    let route = resolve_provider_route(
        config.model.as_str(),
        config.base_url.as_deref(),
        config.api_key.as_deref(),
    )?;
    if route.api_key.is_empty() {
        return Err("missing API key".into());
    }
    let provider = OpenAiProvider::new(route.base_url, route.api_key, route.model);
    run_implement_with_provider(request, config, provider, cancel)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImplementRequest {
    pub repository_path: String,
    pub target_path: String,
    pub expected_content: String,
    pub desired_content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImplementCompletion {
    pub ready: bool,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImplementStatus {
    CandidateReady,
    Indeterminate,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImplementReason {
    CandidateReady,
    CandidateNotReady,
    ModelFailure,
    BudgetExhausted,
    SessionStalled,
    LimitExceeded,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ImplementUsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImplementResult {
    pub status: ImplementStatus,
    pub reason: ImplementReason,
    pub implementation_id: String,
    pub completed_at: String,
    pub target_path: String,
    pub summary: Option<String>,
    pub applied: bool,
    pub verified: bool,
    pub usage: ImplementUsageSummary,
}

#[derive(Clone, Debug)]
pub struct ImplementState {
    target_path: String,
    expected_content: String,
    desired_content: String,
    desired_content_id: String,
    inspected: bool,
    mutation_before_inspection: bool,
    mutation_applied: bool,
    verified: bool,
    terminal_reason: Option<ImplementReason>,
}

#[derive(Debug)]
pub struct ImplementError(pub String);

pub struct ImplementApplication {
    pending_completion: RefCell<Option<ImplementCompletion>>,
    completion_accepted: RefCell<bool>,
    clock: RefCell<Box<dyn Clock>>,
    id_gen: RefCell<Box<dyn IdGenerator>>,
}

impl ImplementApplication {
    pub fn new_with_sources<C: Clock + 'static, I: IdGenerator + 'static>(
        clock: C,
        id_gen: I,
    ) -> Self {
        Self {
            pending_completion: RefCell::new(None),
            completion_accepted: RefCell::new(false),
            clock: RefCell::new(Box::new(clock)),
            id_gen: RefCell::new(Box::new(id_gen)),
        }
    }
}

pub fn run_implement_with_provider<P: ModelProvider>(
    request: &ImplementRequest,
    config: &ImplementConfig,
    provider: P,
    cancel: Option<&AtomicBool>,
) -> Result<(ImplementResult, Vec<LedgerEvent>), String> {
    let path = Path::new(&request.repository_path);
    let head = ReviewTarget::WorkingDirectory;

    let mut catalog = CodeToolCatalog::<ScopedWrite>::new();
    catalog.register(ReadFileTool::new(
        open_repo(path)?,
        head.clone(),
        65_536,
        SecurityPolicy::new(),
    ));
    catalog.register(ReplaceFileContentTool::new(
        open_repo(path)?,
        SecurityPolicy::new(),
    ));

    let limits = Limits {
        max_turns: config.max_turns,
        max_tool_calls: config.max_tool_calls,
        max_completion_attempts: config.max_completion_attempts,
        wall_clock_budget: config.wall_clock_budget,
        ledger_path: config.ledger_path.clone(),
        max_repeated_actions: config.max_repeated_actions,
        max_input_tokens: config.max_input_tokens,
        max_cost_usd: config.max_cost_usd,
    };

    let app = ImplementApplication::new_with_sources(SystemClock::new(), RandomIdGenerator::new());
    let coordinator = SessionCoordinator::new(
        app,
        provider,
        RandomIdGenerator::new(),
        catalog.into_inner(),
        limits,
    );
    Ok(coordinator.run_full(request.clone(), cancel))
}

fn open_repo(path: &Path) -> Result<GitRepo, String> {
    GitRepo::open(path).map_err(|e| format!("open repository: {e:?}"))
}

impl AgentApplication for ImplementApplication {
    type Request = ImplementRequest;
    type State = ImplementState;
    type Completion = ImplementCompletion;
    type Result = ImplementResult;
    type Error = ImplementError;

    fn descriptor(&self) -> ApplicationDescriptor {
        ApplicationDescriptor {
            application_id: "implement".into(),
            application_version: "0.1.0".into(),
            request_schema: "implement.request/v0".into(),
            completion_schema: "implement.completion/v0".into(),
            result_schema: "implement.result/v0".into(),
            domain_event_namespace: "implement".into(),
        }
    }

    fn validate_request(&self, request: &Self::Request) -> Result<(), Self::Error> {
        if !Path::new(&request.repository_path).exists() {
            return Err(ImplementError(format!(
                "repository path does not exist: {}",
                request.repository_path
            )));
        }
        if request.target_path.trim().is_empty() {
            return Err(ImplementError("target_path must not be empty".into()));
        }
        if request.expected_content == request.desired_content {
            return Err(ImplementError(
                "expected_content and desired_content must differ".into(),
            ));
        }
        Ok(())
    }

    fn initialize(
        &self,
        request: &Self::Request,
    ) -> Result<ApplicationInitialization<Self::State>, Self::Error> {
        self.validate_request(request)?;
        Ok(ApplicationInitialization {
            initial_state: ImplementState {
                target_path: request.target_path.clone(),
                expected_content: request.expected_content.clone(),
                desired_content: request.desired_content.clone(),
                desired_content_id: content_id_for_bytes(request.desired_content.as_bytes()),
                inspected: false,
                mutation_before_inspection: false,
                mutation_applied: false,
                verified: false,
                terminal_reason: None,
            },
            requested_tools: vec!["read_file".into(), "replace_file_content".into()],
            requested_capabilities: vec![],
            application_limits: None,
        })
    }

    fn build_system_instructions(&self, _state: &Self::State) -> Vec<InstructionBlock> {
        vec![InstructionBlock {
            content: "You are an implementation assistant working inside a bounded repository sandbox. First inspect the target file with read_file. If the current content matches the request precondition, apply exactly one bounded replacement with replace_file_content. Then read the file again to verify the resulting content before requesting completion. When ready, emit a JSON completion payload of the form {\"ready\": true, \"summary\": \"<what changed>\"}.".into(),
        }]
    }

    fn build_context(&self, state: &Self::State) -> Vec<ContextBlock> {
        vec![ContextBlock {
            content: format!(
                "Implementation target data: modify `{}` only. Expected current content:\n{}\nDesired final content:\n{}\nCandidate readiness requires one successful bounded mutation and one post-mutation verification read whose content matches the requested target state.",
                state.target_path, state.expected_content, state.desired_content
            ),
        }]
    }

    fn reduce_event(&self, state: &Self::State, event: &LedgerEvent) -> Self::State {
        let terminal_reason = match event.event_type.as_str() {
            "kernel.model_failed" => Some(ImplementReason::ModelFailure),
            "kernel.session_budget_exhausted" => Some(ImplementReason::BudgetExhausted),
            "kernel.session_stalled" => Some(ImplementReason::SessionStalled),
            "kernel.session_limit_exceeded" => Some(ImplementReason::LimitExceeded),
            "kernel.session_cancelled" => Some(ImplementReason::Cancelled),
            _ => state.terminal_reason.clone(),
        };

        let mut next = state.clone();
        next.terminal_reason = terminal_reason;

        if event.event_type.as_str() != "kernel.tool_completed" {
            return next;
        }

        if let Some(details) = &event.details
            && let Ok(info) = serde_json::from_str::<Value>(details)
        {
            let tool = info["tool"].as_str().unwrap_or_default();
            let path = info["path"].as_str().unwrap_or_default();
            let status = info["status"].as_str().unwrap_or_default();
            let content_id = info["content_id"].as_str().unwrap_or_default();

            if path == state.target_path && tool == "read_file" && status == "Succeeded" {
                if !state.mutation_applied {
                    next.inspected = true;
                }
                if state.inspected
                    && state.mutation_applied
                    && content_id == state.desired_content_id
                {
                    next.verified = true;
                }
            }

            if path == state.target_path && tool == "replace_file_content" && status == "Succeeded"
            {
                if !state.inspected {
                    next.mutation_before_inspection = true;
                }
                if content_id == state.desired_content_id {
                    next.mutation_applied = true;
                }
            }
        }

        next
    }

    fn parse_completion(&self, payload: &Value) -> Result<Self::Completion, Self::Error> {
        serde_json::from_value(payload.clone())
            .map_err(|error| ImplementError(format!("invalid completion payload: {}", error)))
    }

    fn validate_completion(
        &self,
        state: &Self::State,
        completion: &Self::Completion,
    ) -> CompletionDecision {
        if state.mutation_before_inspection {
            return CompletionDecision::RejectedTerminal {
                reason: "mutation_applied_before_inspection".into(),
            };
        }

        if !state.inspected {
            return CompletionDecision::RejectedRemediable {
                reason_codes: vec!["target_not_inspected".into()],
                missing_requirements: vec!["The target file has not been inspected.".into()],
                feedback_for_model: vec![InstructionBlock {
                    content: "You must inspect the target file with read_file before completing."
                        .into(),
                }],
            };
        }

        if !state.mutation_applied {
            return CompletionDecision::RejectedRemediable {
                reason_codes: vec!["mutation_not_applied".into()],
                missing_requirements: vec!["The bounded mutation has not been applied.".into()],
                feedback_for_model: vec![InstructionBlock {
                    content:
                        "Apply the requested change with replace_file_content before completing."
                            .into(),
                }],
            };
        }

        if !state.verified {
            return CompletionDecision::RejectedRemediable {
                reason_codes: vec!["candidate_not_verified".into()],
                missing_requirements: vec![
                    "The candidate state has not been verified with a follow-up read.".into(),
                ],
                feedback_for_model: vec![InstructionBlock {
                    content: "Read the target file again after mutation and confirm the requested state before completing.".into(),
                }],
            };
        }

        if !completion.ready {
            return CompletionDecision::RejectedRemediable {
                reason_codes: vec!["ready_false".into()],
                missing_requirements: vec!["Completion must assert candidate readiness.".into()],
                feedback_for_model: vec![InstructionBlock {
                    content: "Only complete with ready=true once the candidate is ready.".into(),
                }],
            };
        }

        if completion.summary.trim().is_empty() {
            return CompletionDecision::RejectedRemediable {
                reason_codes: vec!["summary_missing".into()],
                missing_requirements: vec!["Completion summary must not be empty.".into()],
                feedback_for_model: vec![InstructionBlock {
                    content: "Include a short non-empty summary in the completion payload.".into(),
                }],
            };
        }

        *self.pending_completion.borrow_mut() = Some(completion.clone());
        *self.completion_accepted.borrow_mut() = true;
        CompletionDecision::Accepted
    }

    fn build_terminal_result(&self, state: &Self::State, usage: &UsageRecord) -> Self::Result {
        let accepted = *self.completion_accepted.borrow();
        let pending = self.pending_completion.borrow();
        ImplementResult {
            status: if accepted {
                ImplementStatus::CandidateReady
            } else {
                ImplementStatus::Indeterminate
            },
            reason: if accepted {
                ImplementReason::CandidateReady
            } else {
                state
                    .terminal_reason
                    .clone()
                    .unwrap_or(ImplementReason::CandidateNotReady)
            },
            implementation_id: self.id_gen.borrow_mut().next_id(),
            completed_at: self.clock.borrow().now(),
            target_path: state.target_path.clone(),
            summary: pending
                .as_ref()
                .map(|completion| completion.summary.clone())
                .or_else(|| {
                    state
                        .mutation_before_inspection
                        .then_some("Ordering violation: mutation_applied_before_inspection".into())
                }),
            applied: state.mutation_applied,
            verified: state.verified,
            usage: ImplementUsageSummary {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                estimated_cost_usd: usage.estimated_cost_usd,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_kernel::application::AgentApplication;
    use agent_kernel::ledger::LedgerEvent;
    use agent_protocol::{FixedClock, SequenceIdGenerator};
    use serde_json::json;

    fn request() -> ImplementRequest {
        ImplementRequest {
            repository_path: ".".into(),
            target_path: "src/app.txt".into(),
            expected_content: "before".into(),
            desired_content: "after".into(),
        }
    }

    fn tool_completed_event(tool: &str, path: &str, status: &str, content_id: &str) -> LedgerEvent {
        LedgerEvent {
            event_type: "kernel.tool_completed".into(),
            details: Some(
                json!({
                    "tool": tool,
                    "path": path,
                    "status": status,
                    "content_id": content_id,
                })
                .to_string(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn reduce_event_marks_verification_only_after_pre_mutation_inspection() {
        let app = ImplementApplication::new_with_sources(
            FixedClock::new("2025-01-01T00:00:00Z"),
            SequenceIdGenerator::new(["impl-001"]),
        );
        let mut state = app.initialize(&request()).unwrap().initial_state;

        state = app.reduce_event(
            &state,
            &tool_completed_event("read_file", "src/app.txt", "Succeeded", "before-id"),
        );
        assert!(state.inspected);
        assert!(!state.verified);

        state = app.reduce_event(
            &state,
            &tool_completed_event(
                "replace_file_content",
                "src/app.txt",
                "Succeeded",
                &content_id_for_bytes("after".as_bytes()),
            ),
        );
        assert!(state.mutation_applied);
        assert!(!state.verified);

        state = app.reduce_event(
            &state,
            &tool_completed_event(
                "read_file",
                "src/app.txt",
                "Succeeded",
                &content_id_for_bytes("after".as_bytes()),
            ),
        );
        assert!(state.inspected);
        assert!(state.verified);
    }

    #[test]
    fn reduce_event_does_not_count_post_mutation_read_as_inspection() {
        let app = ImplementApplication::new_with_sources(
            FixedClock::new("2025-01-01T00:00:00Z"),
            SequenceIdGenerator::new(["impl-001"]),
        );
        let mut state = app.initialize(&request()).unwrap().initial_state;

        state = app.reduce_event(
            &state,
            &tool_completed_event(
                "replace_file_content",
                "src/app.txt",
                "Succeeded",
                &content_id_for_bytes("after".as_bytes()),
            ),
        );
        assert!(state.mutation_applied);
        assert!(!state.inspected);

        state = app.reduce_event(
            &state,
            &tool_completed_event(
                "read_file",
                "src/app.txt",
                "Succeeded",
                &content_id_for_bytes("after".as_bytes()),
            ),
        );
        assert!(!state.inspected);
        assert!(!state.verified);
    }

    #[test]
    fn validate_completion_rejects_mutation_before_inspection_as_terminal() {
        let app = ImplementApplication::new_with_sources(
            FixedClock::new("2025-01-01T00:00:00Z"),
            SequenceIdGenerator::new(["impl-001"]),
        );
        let state = app.reduce_event(
            &app.initialize(&request()).unwrap().initial_state,
            &tool_completed_event(
                "replace_file_content",
                "src/app.txt",
                "Succeeded",
                &content_id_for_bytes("after".as_bytes()),
            ),
        );

        let decision = app.validate_completion(
            &state,
            &ImplementCompletion {
                ready: true,
                summary: "Updated src/app.txt".into(),
            },
        );

        assert!(matches!(
            decision,
            CompletionDecision::RejectedTerminal { ref reason }
            if reason == "mutation_applied_before_inspection"
        ));
    }

    #[test]
    fn build_terminal_result_surfaces_terminal_completion_failure_in_summary() {
        let app = ImplementApplication::new_with_sources(
            FixedClock::new("2025-01-01T00:00:00Z"),
            SequenceIdGenerator::new(["impl-001"]),
        );
        let state = app.reduce_event(
            &app.initialize(&request()).unwrap().initial_state,
            &tool_completed_event(
                "replace_file_content",
                "src/app.txt",
                "Succeeded",
                &content_id_for_bytes("after".as_bytes()),
            ),
        );

        let decision = app.validate_completion(
            &state,
            &ImplementCompletion {
                ready: true,
                summary: "Updated src/app.txt".into(),
            },
        );
        assert!(matches!(
            decision,
            CompletionDecision::RejectedTerminal { .. }
        ));

        let result = app.build_terminal_result(
            &state,
            &UsageRecord {
                input_tokens: 0,
                output_tokens: 0,
                estimated_cost_usd: None,
            },
        );

        assert_eq!(result.reason, ImplementReason::CandidateNotReady);
        assert_eq!(
            result.summary.as_deref(),
            Some("Ordering violation: mutation_applied_before_inspection")
        );
    }
}
