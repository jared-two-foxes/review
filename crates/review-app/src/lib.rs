// Application scaffolding for the end-to-end test.

use agent_kernel::application::{
    AgentApplication, ApplicationDescriptor, ApplicationInitialization, CompletionDecision,
    ContextBlock, InstructionBlock,
};
use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::tools::ToolCatalog;
use agent_kernel::{
    ledger::{LedgerEvent, Limits},
    model::ToolDescription,
    tools::{Tool, ToolResult, ToolStatus},
};
use agent_protocol::{Clock, IdGenerator, RandomIdGenerator, SystemClock};
use code_agent_runtime::provider::OpenAiProvider;
use code_agent_runtime::tools::{
    GetChangeSummaryTool, GetChangedFilesTool, ListDirectoryTool, ReadDiffTool, ReadFileTool,
    SearchTextTool,
};
use code_agent_runtime::{repo::GitRepo, security::SecurityPolicy};
use review_protocol::{ReviewReason, ReviewRequest, ReviewResult, ReviewStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

pub struct ReviewConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_completion_attempts: u32,
    pub wall_clock_budget: Option<Duration>,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "gpt-4o".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            max_turns: 10,
            max_tool_calls: 10,
            max_completion_attempts: 3,
            wall_clock_budget: Some(Duration::from_secs(60)),
        }
    }
}

pub fn run_review(
    request: &ReviewRequest,
    config: &ReviewConfig,
) -> (ReviewResult, Vec<LedgerEvent>, Option<String>) {
    match compose_and_run(request, config) {
        Ok((result, events)) => (result, events, None),
        Err(reason) => (
            ReviewResult {
                schema: "review.result/v1".to_string(),
                status: ReviewStatus::Indeterminate,
                reason: ReviewReason::ReviewEngineNotAvailable,
                review_id: String::new(),
                completed_at: String::new(),
            },
            vec![],
            Some(reason),
        ),
    }
}

fn open_repo(path: &Path) -> Result<GitRepo, String> {
    GitRepo::open(path).map_err(|e| format!("open repository: {e:?}"))
}

fn compose_and_run(
    request: &ReviewRequest,
    config: &ReviewConfig,
) -> Result<(ReviewResult, Vec<LedgerEvent>), String> {
    if config.api_key.is_empty() {
        return Err("missing API key".into());
    }
    let path = Path::new(&request.repository_path);
    let ref_repo = open_repo(path)?;
    let base = ref_repo
        .revparse_single(&request.base_ref)
        .map_err(|e| format!("resolve base_ref: {e:?}"))?
        .id();
    let head = ref_repo
        .revparse_single(&request.head_ref)
        .map_err(|e| format!("resolve head_ref: {e:?}"))?
        .id();
    drop(ref_repo);

    let byte_limit = 65_536usize;
    let mut catalog = ToolCatalog::new();

    catalog.register(Box::new(GetChangeSummaryTool::new(
        open_repo(path)?,
        base,
        head,
    )));
    catalog.register(Box::new(GetChangedFilesTool::new(
        open_repo(path)?,
        base,
        head,
    )));
    catalog.register(Box::new(ReadDiffTool::new(
        open_repo(path)?,
        base,
        head,
        byte_limit,
    )));
    catalog.register(Box::new(ReadFileTool::new(
        open_repo(path)?,
        head,
        byte_limit,
    )));
    catalog.register(Box::new(ListDirectoryTool::new(
        open_repo(path)?,
        SecurityPolicy::new(),
    )));
    catalog.register(Box::new(SearchTextTool::new(
        open_repo(path)?,
        SecurityPolicy::new(),
    )));

    let provider = OpenAiProvider::new(
        config.base_url.as_str(),
        config.api_key.as_str(),
        config.model.as_str(),
    );
    let app = ReviewApplication::new_with_sources(SystemClock::new(), RandomIdGenerator::new());
    let limits = Limits {
        max_turns: config.max_turns,
        max_tool_calls: config.max_tool_calls,
        max_completion_attempts: config.max_completion_attempts,
        wall_clock_budget: config.wall_clock_budget,
    };
    let coordinator =
        SessionCoordinator::new(app, provider, RandomIdGenerator::new(), catalog, limits);
    let (result, events) = coordinator.run_full(request.clone());
    Ok((result, events))
}

/// Deterministic output seam for replay tests.
///
/// This is deliberately only a compile-time seam until the canonical result
/// envelope is implemented.
pub fn run_review_with_sources<C: Clock, I: IdGenerator>(
    _request: &ReviewRequest,
    clock: &C,
    ids: &mut I,
) -> Vec<u8> {
    let result = ReviewResult {
        schema: "review.result/v1".to_string(),
        status: ReviewStatus::Indeterminate,
        reason: ReviewReason::ReviewEngineNotAvailable,
        review_id: ids.next_id(),
        completed_at: clock.now(),
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

#[allow(dead_code)]
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
        _request: &Self::Request,
    ) -> Result<ApplicationInitialization<Self::State>, Self::Error> {
        Ok(ApplicationInitialization {
            initial_state: ReviewState {
                inspected: false,
                findings: vec![],
            },
            requested_tools: vec![
                "get_change_summary".into(),
                "get_changed_files".into(),
                "read_diff".into(),
                "read_file".into(),
                "list_directory".into(),
                "search_text".into(),
            ],
            requested_capabilities: vec![],
            application_limits: None,
        })
    }

    fn build_system_instructions(&self, _state: &Self::State) -> Vec<InstructionBlock> {
        vec![InstructionBlock {
            content: "You are a code reviewer. Inspect the change by calling get_change_summary, then read_diff, read_file, list_directory, or search_text as needed to understand it.  WHen you have enough information, issue a completion with a JSON payload of the format {\"findings\": [{\"blocking\": <bool>, \"message\":\"<<string>\"}]}.  A blocking finding means the change must not be approved.  If you find no issues, complete with an empty findings array.".into(),
        }]
    }

    fn build_context(&self, _state: &Self::State) -> Vec<ContextBlock> {
        vec![
            ContextBlock {
                content: "Review the code change in this repository. Begin by calling get_change_summary to inspect the change, then call read_diff, read_file, list_directory, or search_text as needed. When you have enough information, issue a completion with your findings as a JSON object of the form {\"findings\":[{\"blocking\": <bool>, \"message\": \"<string>\"}]}.".into()
            }
        ]
    }

    fn validate_request(&self, request: &Self::Request) -> Result<(), Self::Error> {
        review_protocol::validate_request(request).map_err(ReviewError)
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
                    content: "You must call the get_change_summary tool before completing.".into(),
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
        serde_json::from_value(payload.clone())
            .map_err(|e| ReviewError(format!("invalid completion payload: {}", e)))
    }

    fn build_terminal_result(&self, _state: &Self::State) -> Self::Result {
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
            completed_at: self.clock.borrow().now(),
        }
    }
}

pub struct ReadChangeTool;

impl Tool for ReadChangeTool {
    fn name(&self) -> &str {
        "get_change_summary"
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: "get_change_summary".into(),
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

    fn execute(&self, _arguments: &Value) -> ToolResult {
        ToolResult {
            status: ToolStatus::Succeeded,
            value: json!({"summary": "Modified file.rs: changed
 function signature"}),
        }
    }
}
