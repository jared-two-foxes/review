//! Ranged inspection must preserve full-file mutation and identity rules.
use agent_kernel::model::{CanonicalModelRequest, CanonicalModelResponse, ConversationMessage, ModelAction, ModelError, ModelProvider};
use code_agent_runtime::identity::content_id_for_bytes;
use implement_app::{ImplementConfig, ImplementRequest, ImplementStatus, run_implement_with_provider};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
struct Probe { actions: VecDeque<ModelAction>, seen: Arc<Mutex<Vec<Value>>> }
impl ModelProvider for Probe {
    fn generate(&mut self, request: &CanonicalModelRequest) -> Result<CanonicalModelResponse, ModelError> {
        let history: Vec<_> = request.history.iter().filter_map(|m| match m {
            ConversationMessage::Tool { tool_call_id, content } => Some(json!({"id":tool_call_id,"content":content})),
            _ => None,
        }).collect();
        self.seen.lock().unwrap().push(json!({
            "tools":request.tools.iter().map(|t| json!({"name":t.name,"schema":t.input_schema})).collect::<Vec<_>>(),
            "instructions":request.instructions.iter().map(|b| &b.content).collect::<Vec<_>>(),
            "history":history
        }));
        Ok(CanonicalModelResponse { actions: vec![self.actions.pop_front().expect("unexpected extra model turn")], usage: None, reasoning: None })
    }
    fn generate_with_deadline(&mut self, r: &CanonicalModelRequest, _: std::time::Instant) -> Result<CanonicalModelResponse, ModelError> { self.generate(r) }
}
fn call(id: &str, tool: &str, arguments: Value) -> ModelAction {
    ModelAction::ToolCall { action_id: id.into(), tool: tool.into(), arguments }
}
fn tool_value(seen: &[Value], id: &str) -> Value {
    for observation in seen {
        for item in observation["history"].as_array().unwrap() {
            if item["id"] == id {
                if let Some(body) = item["content"].as_str().unwrap().split(" and value: ").nth(1) { return serde_json::from_str(body).unwrap(); }
            }
        }
    }
    panic!("missing result {id}")
}
fn exercise(sliced_precondition: bool) {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("git").current_dir(dir.path()).args(["init", "-q"]).output().unwrap();
    assert!(output.status.success());
    let before = "one\nbefore\nthree\nfour\n";
    let after = "one\nafter\nthree\nfour\n";
    std::fs::write(dir.path().join("a.txt"), before).unwrap();
    let request = ImplementRequest { repository_path: dir.path().to_str().unwrap().into(), target_path: "a.txt".into(), expected_content: before.into(), desired_content: after.into() };
    let mut actions = vec![];
    for offset in 1..=4 { actions.push(call(&format!("range{offset}"), "read_file", json!({"path":"a.txt","offset":offset,"limit":1}))); }
    actions.push(call("mutate", "replace_file_content", json!({"path":"a.txt","expected":if sliced_precondition { "before\n" } else { before },"replacement":after})));
    actions.push(call("verify", "read_file", json!({"path":"a.txt","offset":2,"limit":1})));
    actions.push(ModelAction::CompletionRequest { action_id: "complete".into(), payload: json!({"ready":true,"summary":"Updated second line; full-file identity verified."}) });
    let config = ImplementConfig { max_turns: actions.len() as u32, max_tool_calls: 10, ..Default::default() };
    let seen = Arc::new(Mutex::new(vec![]));
    let (result, events) = run_implement_with_provider(&request, &config, Probe { actions: actions.into(), seen: seen.clone() }, None).unwrap();
    let expected = if sliced_precondition { before } else { after };
    assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), expected);
    assert_eq!(result.applied, !sliced_precondition);
    assert_eq!(result.verified, !sliced_precondition);
    assert_eq!(matches!(result.status, ImplementStatus::CandidateReady), !sliced_precondition);
    if sliced_precondition { assert!(matches!(result.status, ImplementStatus::Indeterminate)); }
    let seen = seen.lock().unwrap();
    let tools = seen[0]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 3);
    for name in ["get_project_guidance", "read_file", "replace_file_content"] { assert!(names.contains(&name)); }
    assert!(!names.contains(&"search_text"));
    let schema = &tools.iter().find(|t| t["name"] == "read_file").unwrap()["schema"];
    for name in ["offset", "limit"] {
        assert_eq!(schema["properties"][name]["type"], "integer");
        assert!(!schema["required"].as_array().unwrap().iter().any(|v| v == name));
    }
    let instruction = seen[0]["instructions"].to_string();
    assert!(instruction.contains("1-based starting line"));
    assert!(instruction.contains("entire file"));
    for (i, text) in ["one\n", "before\n", "three\n", "four\n"].iter().enumerate() {
        let value = tool_value(&seen, &format!("range{}", i + 1));
        assert_eq!(value["content"], *text);
        assert_eq!(value["content_id"], content_id_for_bytes(before.as_bytes()));
    }
    let verified = tool_value(&seen, "verify");
    assert_eq!(verified["content"], if sliced_precondition { "before\n" } else { "after\n" });
    assert_eq!(verified["content_id"], content_id_for_bytes(expected.as_bytes()));
    assert_ne!(verified["content_id"], content_id_for_bytes(verified["content"].as_str().unwrap().as_bytes()));
    let metadata: Vec<Value> = events.iter().filter(|e| e.event_type == "kernel.tool_completed").map(|e| serde_json::from_str(e.details.as_deref().unwrap()).unwrap()).collect();
    let mutations: Vec<_> = metadata.iter().filter(|v| v["tool"] == "replace_file_content").collect();
    assert_eq!(mutations.len(), 1);
    assert_eq!(mutations[0]["status"], if sliced_precondition { "Failed" } else { "Succeeded" });
    let reads: Vec<_> = metadata.iter().filter(|v| v["tool"] == "read_file").collect();
    assert_eq!(reads.len(), 5);
    for read in reads { assert_eq!(read["scope"], "file_range"); assert_eq!(read["limit"], 1); }
}
#[test]
fn production_ranged_reads_and_full_file_mutation_reach_candidate_ready() { exercise(false); }
#[test]
fn production_sliced_precondition_cannot_mutate_or_reach_candidate_ready() { exercise(true); }
