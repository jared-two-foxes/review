//! Checkpoint / resume behaviour of the session coordinator.

use agent_kernel::application::*;
use agent_kernel::checkpoint::{Checkpoint, CheckpointError};
use agent_kernel::coordinator::SessionCoordinator;
use agent_kernel::ledger::LedgerEvent;
use agent_kernel::limits::Limits;
use agent_kernel::model::*;
use agent_kernel::tools::{Tool, ToolCatalog, ToolResult, ToolStatus};
use agent_protocol::RandomIdGenerator;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

struct CountingTool(Rc<Cell<u32>>);

impl Tool for CountingTool {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn description(&self) -> ToolDescription {
        ToolDescription {
            name: "echo".into(),
            description: "echo".into(),
            input_schema: json!({"type": "object"}),
        }
    }
    fn validate_arguments(&self, _: &Value) -> Result<(), String> {
        Ok(())
    }
    fn execute(&self, arguments: &Value) -> ToolResult {
        self.0.set(self.0.get() + 1);
        ToolResult { status: ToolStatus::Succeeded, value: arguments.clone() }
    }
}

struct Script {
    cancel_on_generate: Option<Arc<AtomicBool>>,
    responses: Vec<CanonicalModelResponse>,
    seen_histories: Rc<RefCell<Vec<Vec<ConversationMessage>>>>,
}

impl ModelProvider for Script {
    fn generate(&mut self, request: &CanonicalModelRequest) -> Result<CanonicalModelResponse, ModelError> {
        self.seen_histories.borrow_mut().push(request.history.clone());
        if let Some(flag) = &self.cancel_on_generate {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(self.responses.remove(0))
    }
    fn generate_with_deadline(&mut self, request: &CanonicalModelRequest, _: Instant) -> Result<CanonicalModelResponse, ModelError> {
        self.generate(request)
    }
}

#[derive(Clone)]
struct St {
    tool_calls: u32,
    terminal: Option<String>,
}
struct App;
#[derive(Debug)]
struct Err_(#[allow(dead_code)] String);

impl AgentApplication for App {
    type Request = ();
    type State = St;
    type Completion = ();
    type Result = St;
    type Error = Err_;
    fn descriptor(&self) -> ApplicationDescriptor {
        ApplicationDescriptor {
            application_id: "t".into(),
            application_version: "0".into(),
            request_schema: "".into(),
            completion_schema: "".into(),
            result_schema: "".into(),
            domain_event_namespace: "".into(),
        }
    }
    fn validate_request(&self, _: &()) -> Result<(), Err_> {
        Ok(())
    }
    fn initialize(&self, _: &()) -> Result<ApplicationInitialization<St>, Err_> {
        Ok(ApplicationInitialization {
            initial_state: St { tool_calls: 0, terminal: None },
            requested_tools: vec!["echo".into()],
            requested_capabilities: vec![],
            application_limits: None,
        })
    }
    fn build_system_instructions(&self, _: &St) -> Vec<InstructionBlock> {
        vec![]
    }
    fn build_context(&self, _: &St) -> Vec<ContextBlock> {
        vec![]
    }
    fn reduce_event(&self, state: &St, event: &LedgerEvent) -> St {
        let mut next = state.clone();
        match event.event_type.as_str() {
            "kernel.tool_completed" => next.tool_calls += 1,
            t if t.starts_with("kernel.session_") => next.terminal = Some(t.into()),
            _ => {}
        }
        next
    }
    fn parse_completion(&self, _: &Value) -> Result<(), Err_> {
        Ok(())
    }
    fn validate_completion(&self, _: &St, _: &()) -> CompletionDecision {
        CompletionDecision::Accepted
    }
    fn build_terminal_result(&self, state: &St, _: &UsageRecord) -> St {
        state.clone()
    }
}

fn call(id: &str, n: u32) -> ModelAction {
    ModelAction::ToolCall { action_id: id.into(), tool: "echo".into(), arguments: json!({"n": n}) }
}

fn respond(actions: Vec<ModelAction>) -> CanonicalModelResponse {
    CanonicalModelResponse { actions, usage: None, reasoning: None }
}

fn limits(max_tool_calls: u32, ledger: Option<std::path::PathBuf>) -> Limits {
    Limits {
        max_turns: 10,
        max_tool_calls,
        max_completion_attempts: 3,
        wall_clock_budget: None,
        ledger_path: ledger,
        max_repeated_actions: 3,
        max_input_tokens: None,
        max_cost_usd: None,
    }
}

fn coordinator(
    responses: Vec<CanonicalModelResponse>,
    executed: &Rc<Cell<u32>>,
    seen: &Rc<RefCell<Vec<Vec<ConversationMessage>>>>,
    limits: Limits,
) -> SessionCoordinator<App, Script, RandomIdGenerator> {
    let mut catalog = ToolCatalog::new();
    catalog.register(Box::new(CountingTool(executed.clone())));
    SessionCoordinator::new(
        App,
        Script { cancel_on_generate: None, responses, seen_histories: seen.clone() },
        RandomIdGenerator::new(),
        catalog,
        limits,
    )
}

#[test]
fn resumes_from_tool_limit_without_redoing_work_and_with_valid_history() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("cp.json");
    let ledger = dir.path().join("ledger.jsonl");
    let executed = Rc::new(Cell::new(0));
    let seen = Rc::new(RefCell::new(Vec::new()));

    // Attempt 1: one response carries three calls but only two are allowed.
    let first = coordinator(
        vec![respond(vec![call("a1", 1), call("a2", 2), call("a3", 3)])],
        &executed,
        &seen,
        limits(2, Some(ledger.clone())),
    )
    .with_checkpoint(cp.clone(), "fp".into());
    let (result, events) = first.run_full((), None);
    assert_eq!(result.tool_calls, 2);
    assert_eq!(result.terminal.as_deref(), Some("kernel.session_limit_exceeded"));
    assert_eq!(executed.get(), 2);
    let session_id = events[0].session_id.clone();

    let saved = Checkpoint::load(&cp, "fp").expect("checkpoint written");
    assert_eq!(saved.stopped_by.as_deref(), Some("kernel.session_limit_exceeded"));
    // The over-limit call was never executed and must not be charged.
    assert_eq!(saved.tool_call_count, 2);
    assert_eq!(saved.progress.distinct_tool_calls, 2);

    // Attempt 2: higher limit.  The model re-issues the dropped call, then completes.
    seen.borrow_mut().clear();
    let second = coordinator(
        vec![respond(vec![call("b1", 3)]), respond(vec![ModelAction::CompletionRequest { action_id: "c".into(), payload: json!({}) }])],
        &executed,
        &seen,
        limits(3, Some(ledger.clone())),
    )
    .with_checkpoint(cp.clone(), "fp".into())
    .resume_from(saved);
    let (result, events) = second.run_full((), None);

    // Prior work was kept and the earlier terminal reason did not leak.
    assert_eq!(result.tool_calls, 3);
    assert_eq!(executed.get(), 3, "earlier calls must not be re-executed");
    assert_ne!(result.terminal.as_deref(), Some("kernel.session_limit_exceeded"));
    assert!(events.iter().all(|e| e.session_id == session_id));
    assert!(events.iter().any(|e| e.event_type == "kernel.session_resumed"));
    assert!(!cp.exists(), "completed session removes its checkpoint");

    // The history handed to the model has a result for every tool call.
    let first_history = seen.borrow()[0].clone();
    let calls: Vec<String> = first_history
        .iter()
        .flat_map(|m| match m {
            ConversationMessage::Assistant { tool_calls, .. } => tool_calls.iter().map(|c| c.id.clone()).collect(),
            _ => vec![],
        })
        .collect();
    assert_eq!(calls, ["a1", "a2", "a3"]);
    for id in &calls {
        assert!(
            first_history.iter().any(|m| matches!(m, ConversationMessage::Tool { tool_call_id, .. } if tool_call_id == id)),
            "dangling tool call {id}"
        );
    }

    // The ledger file holds the whole chain with contiguous sequence numbers.
    let lines: Vec<LedgerEvent> = std::fs::read_to_string(&ledger)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), events.len());
    assert!(lines.iter().enumerate().all(|(i, e)| e.sequence == i as u64));
}

#[test]
fn turn_limit_resume_continues_turn_numbering() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("cp.json");
    let executed = Rc::new(Cell::new(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut l = limits(10, None);
    l.max_turns = 2;
    let (r, _) = coordinator(vec![respond(vec![call("a", 1)]), respond(vec![call("b", 2)])], &executed, &seen, l)
        .with_checkpoint(cp.clone(), "fp".into())
        .run_full((), None);
    assert_eq!(r.terminal.as_deref(), Some("kernel.session_limit_exceeded"));
    let saved = Checkpoint::load(&cp, "fp").unwrap();
    assert_eq!(saved.turn, 2, "the refused third turn is not charged");

    let mut l = limits(10, None);
    l.max_turns = 3;
    let (r, events) = coordinator(vec![respond(vec![ModelAction::CompletionRequest { action_id: "c".into(), payload: json!({}) }])], &executed, &seen, l)
        .with_checkpoint(cp.clone(), "fp".into())
        .resume_from(saved)
        .run_full((), None);
    assert_eq!(r.tool_calls, 2);
    assert_eq!(events.last().unwrap().turn, 3);
    assert_eq!(executed.get(), 2);
}

#[test]
fn checkpoint_for_a_different_request_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("cp.json");
    let executed = Rc::new(Cell::new(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    coordinator(vec![respond(vec![call("a", 1)]), respond(vec![call("b", 2)])], &executed, &seen, limits(1, None))
        .with_checkpoint(cp.clone(), "fp-one".into())
        .run_full((), None);
    assert!(matches!(Checkpoint::load(&cp, "fp-two"), Err(CheckpointError::FingerprintMismatch { .. })));
    assert!(matches!(Checkpoint::load(&dir.path().join("missing"), "x"), Err(CheckpointError::Io(_))));
}

fn tool_message<'a>(history: &'a [ConversationMessage], id: &str) -> Option<&'a str> {
    history.iter().find_map(|m| match m {
        ConversationMessage::Tool { tool_call_id, content } if tool_call_id == id => Some(content.as_str()),
        _ => None,
    })
}

#[test]
fn stalled_call_is_recorded_not_reported_as_unexecuted() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("cp.json");
    let executed = Rc::new(Cell::new(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut l = limits(10, None);
    l.max_repeated_actions = 1;
    let (r, events) = coordinator(vec![respond(vec![call("a1", 1), call("a2", 1)])], &executed, &seen, l)
        .with_checkpoint(cp.clone(), "fp".into())
        .run_full((), None);
    assert_eq!(executed.get(), 2);
    assert_eq!(r.tool_calls, 2, "the executed repeat is reduced into state");
    let completed = events.iter().filter(|e| e.event_type == "kernel.tool_completed").count();
    assert_eq!(completed, 2);
    assert_eq!(events.last().unwrap().event_type, "kernel.session_stalled");

    let saved = Checkpoint::load(&cp, "fp").unwrap();
    let result = tool_message(&saved.history, "a2").expect("result recorded");
    assert!(!result.contains("was not executed"), "{result}");
    assert!(result.contains("completed with status"));
}

#[test]
fn cancelled_call_is_not_counted() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("cp.json");
    let executed = Rc::new(Cell::new(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let flag = Arc::new(AtomicBool::new(false));
    let mut catalog = ToolCatalog::new();
    catalog.register(Box::new(CountingTool(executed.clone())));
    let (result, events) = SessionCoordinator::new(
        App,
        Script {
            cancel_on_generate: Some(flag.clone()),
            responses: vec![respond(vec![call("a1", 1)])],
            seen_histories: seen.clone(),
        },
        RandomIdGenerator::new(),
        catalog,
        limits(5, None),
    )
    .with_checkpoint(cp.clone(), "fp".into())
    .run_full((), Some(flag.as_ref()));
    assert_eq!(events.last().unwrap().event_type, "kernel.session_cancelled");
    // The terminal event reaches application state, so the result can say why.
    assert_eq!(result.terminal.as_deref(), Some("kernel.session_cancelled"));
    assert_eq!(executed.get(), 0);
    let saved = Checkpoint::load(&cp, "fp").unwrap();
    assert_eq!(saved.tool_call_count, 0, "cancelled call must not consume budget");
}

#[test]
fn restored_usage_over_budget_stops_before_another_generation() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("cp.json");
    let executed = Rc::new(Cell::new(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut l = limits(10, None);
    l.max_input_tokens = Some(50);
    let mut expensive = respond(vec![call("a1", 1)]);
    expensive.usage = Some(UsageRecord { input_tokens: 100, output_tokens: 1, estimated_cost_usd: None });
    let (first, events) = coordinator(vec![expensive], &executed, &seen, l)
        .with_checkpoint(cp.clone(), "fp".into())
        .run_full((), None);
    assert_eq!(events.last().unwrap().event_type, "kernel.session_budget_exhausted");
    assert_eq!(first.terminal.as_deref(), Some("kernel.session_budget_exhausted"));
    let saved = Checkpoint::load(&cp, "fp").unwrap();

    seen.borrow_mut().clear();
    let mut l = limits(10, None);
    l.max_input_tokens = Some(50);
    // No scripted responses: any provider call would panic.
    let (second, events) = coordinator(vec![], &executed, &seen, l)
        .with_checkpoint(cp.clone(), "fp".into())
        .resume_from(saved)
        .run_full((), None);
    assert_eq!(events.last().unwrap().event_type, "kernel.session_budget_exhausted");
    assert_eq!(second.terminal.as_deref(), Some("kernel.session_budget_exhausted"));
    assert!(seen.borrow().is_empty(), "no model request may be made");
}

#[test]
fn a_refused_resume_names_the_inputs_that_differ() {
    use agent_kernel::checkpoint::RequestFingerprint;
    use std::collections::BTreeMap;
    let parts = |identity: &str| -> BTreeMap<String, String> {
        [("base", "b1"), ("resume_identity", identity)].map(|(k, v)| (k.to_string(), v.to_string())).into()
    };
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("cp.json");
    let first = RequestFingerprint::new("fp-one", parts("id-1"));
    let mut c = Checkpoint {
        version: agent_kernel::checkpoint::CHECKPOINT_VERSION,
        fingerprint: first.value.clone(),
        fingerprint_parts: first.parts.clone(),
        session_id: "s".into(),
        turn: 0,
        tool_call_count: 0,
        completion_attempt_count: 0,
        usage: UsageRecord { input_tokens: 0, output_tokens: 0, estimated_cost_usd: None },
        history: vec![],
        latest_tool_calls: vec![],
        repeat_counts: vec![],
        events: vec![],
        stopped_by: None,
        progress: Default::default(),
    };
    c.save(&cp).unwrap();

    let err = Checkpoint::load(&cp, RequestFingerprint::new("fp-two", parts("id-2"))).unwrap_err();
    match &err {
        CheckpointError::FingerprintMismatch { differing } => {
            assert_eq!(differing.len(), 1);
            assert_eq!(differing[0].name, "resume_identity");
            assert_eq!(differing[0].checkpoint.as_deref(), Some("id-1"));
            assert_eq!(differing[0].current.as_deref(), Some("id-2"));
        }
        other => panic!("unexpected {other:?}"),
    }
    let msg = err.to_string();
    assert!(msg.starts_with("checkpoint was created for a different request and cannot be resumed"));
    assert!(msg.contains("resume_identity") && msg.contains("id-1") && msg.contains("id-2"));
    assert!(!msg.contains("base"));

    // A checkpoint written without recorded parts keeps the original message exactly.
    c.fingerprint_parts.clear();
    c.save(&cp).unwrap();
    let old = Checkpoint::load(&cp, RequestFingerprint::new("fp-two", parts("id-2"))).unwrap_err();
    assert_eq!(old.to_string(), "checkpoint was created for a different request and cannot be resumed");
    // Parts never decide acceptance: same combined value with different parts still loads.
    assert!(Checkpoint::load(&cp, RequestFingerprint::new("fp-one", parts("zzz"))).is_ok());
}
