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
            "context":request.context.iter().map(|b| &b.content).collect::<Vec<_>>(),
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
fn assert_context_ids(seen: &[Value], before: &str, after: &str) {
    assert!(!seen.is_empty());
    for observation in seen {
        let context = observation["context"].to_string();
        assert!(context.contains(&format!("Expected full-file content_id: {}", content_id_for_bytes(before.as_bytes()))));
        assert!(context.contains(&format!("Desired full-file content_id: {}", content_id_for_bytes(after.as_bytes()))));
    }
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
    assert_context_ids(&seen, before, after);
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

#[test]
fn production_byte_truncation_then_narrow_retry_preserves_full_file_mutation() {
    let dir = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("git").current_dir(dir.path()).args(["init", "-q"]).output().unwrap().status.success());
    let padding = "padding\n".repeat(10_000);
    let before = format!("one\nbefore\n{padding}");
    let after = format!("one\nafter\n{padding}");
    assert_eq!(before.len(), 80_011);
    std::fs::write(dir.path().join("a.txt"), &before).unwrap();
    let request = ImplementRequest { repository_path: dir.path().to_str().unwrap().into(), target_path: "a.txt".into(), expected_content: before.clone(), desired_content: after.clone() };
    let actions = vec![
        call("large", "read_file", json!({"path":"a.txt"})),
        call("retry", "read_file", json!({"path":"a.txt","offset":2,"limit":1})),
        call("mutate", "replace_file_content", json!({"path":"a.txt","expected":before,"replacement":after})),
        call("verify", "read_file", json!({"path":"a.txt","offset":2,"limit":1})),
        ModelAction::CompletionRequest { action_id: "complete".into(), payload: json!({"ready":true,"summary":"Retried bounded read and verified full-file replacement."}) },
    ];
    let config = ImplementConfig { max_turns: actions.len() as u32, ..Default::default() };
    let seen = Arc::new(Mutex::new(vec![]));
    let (result, events) = run_implement_with_provider(&request, &config, Probe { actions: actions.into(), seen: seen.clone() }, None).unwrap();
    assert!(matches!(result.status, ImplementStatus::CandidateReady));
    assert!(result.applied && result.verified);
    assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), after);
    let seen = seen.lock().unwrap();
    assert_context_ids(&seen, &before, &after);
    let mut tools: Vec<_> = seen[0]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    tools.sort_unstable();
    assert_eq!(tools, vec!["get_project_guidance", "read_file", "replace_file_content"]);
    let initial = tool_value(&seen, "large");
    assert_eq!(initial["truncated"], true);
    assert_eq!(initial["completeness"], false);
    assert_eq!(initial["content_id"], content_id_for_bytes(before.as_bytes()));
    assert_eq!(initial["content"], format!("{}\n[truncated: 65536 of 80011 bytes]", &before[..65_536]));
    for (id, bytes, full) in [("retry", "before\n", &before), ("verify", "after\n", &after)] {
        let value = tool_value(&seen, id);
        assert_eq!(value["content"], bytes);
        assert_eq!(value["truncated"], false);
        assert_eq!(value["completeness"], true);
        assert_eq!(value["content_id"], content_id_for_bytes(full.as_bytes()));
        assert_ne!(value["content_id"], content_id_for_bytes(bytes.as_bytes()));
    }
    let metadata: Vec<Value> = events.iter().filter(|e| e.event_type == "kernel.tool_completed").map(|e| serde_json::from_str(e.details.as_deref().unwrap()).unwrap()).collect();
    assert_eq!(metadata.len(), 4);
    let action_ids: Vec<_> = events.iter().filter(|e| e.event_type == "kernel.tool_completed").map(|e| e.action_id.as_str()).collect();
    assert_eq!(action_ids, vec!["large", "retry", "mutate", "verify"]);
    for i in [0, 1] { assert_eq!(metadata[i]["content_id"], content_id_for_bytes(before.as_bytes())); }
    assert_eq!(metadata[3]["content_id"], content_id_for_bytes(after.as_bytes()));
    assert_eq!(metadata[0]["scope"], "file");
    assert_eq!(metadata[0]["has_content"], true);
    assert_eq!(metadata[0]["truncated"], true);
    assert_eq!(metadata[0]["completeness"], false);
    assert_eq!(metadata[2]["tool"], "replace_file_content");
    for item in &metadata { assert_eq!(item["status"], "Succeeded"); }
    for i in [1, 3] {
        assert_eq!(metadata[i]["scope"], "file_range");
        assert_eq!(metadata[i]["offset"], 2);
        assert_eq!(metadata[i]["limit"], 1);
        assert_eq!(metadata[i]["has_content"], true);
        assert_eq!(metadata[i]["completeness"], true);
        assert_eq!(metadata[i]["truncated"], false);
    }
}

#[test]
fn production_empty_reads_require_full_file_empty_identity_for_inspection() {
    for (name, before, after, inspection, ready) in [
        ("past EOF of nonempty file", "one\nbefore\n", "one\nafter\n",
            json!({"path": "a.txt", "offset": 999, "limit": 1}), false),
        ("whole empty file", "", "after\n", json!({"path": "a.txt"}), true),
        ("ranged genuinely empty file", "", "after\n",
            json!({"path": "a.txt", "offset": 999, "limit": 1}), true),
        ("empty desired file", "before\n", "",
            json!({"path": "a.txt", "offset": 1, "limit": 1}), true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new("git").current_dir(dir.path())
            .args(["init", "-q"]).output().unwrap();
        assert!(output.status.success());
        std::fs::write(dir.path().join("a.txt"), before).unwrap();
        let request = ImplementRequest {
            repository_path: dir.path().to_str().unwrap().into(),
            target_path: "a.txt".into(),
            expected_content: before.into(), desired_content: after.into(),
        };
        let actions = vec![
            call("inspect", "read_file", inspection),
            call("mutate", "replace_file_content", json!({
                "path": "a.txt", "expected": before, "replacement": after,
            })),
            call("verify", "read_file", json!({"path": "a.txt", "offset": 1, "limit": 1})),
            ModelAction::CompletionRequest { action_id: "complete".into(),
                payload: json!({"ready": true, "summary": "Requested full-file state reached."}) },
        ];
        let config = ImplementConfig { max_turns: actions.len() as u32, ..Default::default() };
        let seen = Arc::new(Mutex::new(vec![]));
        let (result, events) = run_implement_with_provider(&request, &config,
            Probe { actions: actions.into(), seen: seen.clone() }, None).unwrap();
        // Tool preconditions still govern mutation; the application separately rejects
        // readiness if the model mutated without meaningful pre-mutation inspection.
        assert!(result.applied, "{name}");
        assert_eq!(result.verified, ready, "{name}");
        assert_eq!(matches!(result.status, ImplementStatus::CandidateReady), ready, "{name}");
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), after, "{name}");
        let seen = seen.lock().unwrap();
        assert_context_ids(&seen, before, after);
        let inspected = tool_value(&seen, "inspect");
        assert_eq!(inspected["content_id"], content_id_for_bytes(before.as_bytes()), "{name}");
        assert_eq!(inspected["completeness"], true, "{name}");
        if before.is_empty() || !ready { assert_eq!(inspected["content"], "", "{name}"); }
        assert_eq!(tool_value(&seen, "verify")["content_id"], content_id_for_bytes(after.as_bytes()), "{name}");
        let metadata: Vec<Value> = events.iter().filter(|e| e.event_type == "kernel.tool_completed")
            .map(|e| serde_json::from_str(e.details.as_deref().unwrap()).unwrap()).collect();
        assert_eq!(metadata.len(), 3, "{name}");
        assert_eq!(metadata[0]["has_content"], !before.is_empty() && ready, "{name}");
        assert_eq!(metadata[1]["tool"], "replace_file_content", "{name}");
        assert_eq!(metadata[1]["status"], "Succeeded", "{name}");
        if !ready {
            assert!(matches!(result.status, ImplementStatus::Indeterminate), "{name}");
            assert_eq!(result.summary.as_deref(), Some("Ordering violation: mutation_applied_before_inspection"));
        }
    }
}
