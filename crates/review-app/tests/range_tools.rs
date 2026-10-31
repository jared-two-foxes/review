//! Production-tool regressions; not claims about autonomous model efficiency.
use agent_kernel::ledger::LedgerEvent;
use agent_kernel::model::{CanonicalModelRequest, CanonicalModelResponse, ConversationMessage, ModelAction, ModelError, ModelProvider};
use code_agent_runtime::identity::content_id_for_bytes;
use review_app::{ReviewConfig, run_review_with_provider};
use review_protocol::{ReviewRequest, ReviewStatus};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

struct Probe { actions: VecDeque<ModelAction>, seen: Arc<Mutex<Vec<Value>>> }
impl ModelProvider for Probe {
    fn generate(&mut self, request: &CanonicalModelRequest) -> Result<CanonicalModelResponse, ModelError> {
        let history: Vec<_> = request.history.iter().filter_map(|m| match m {
            ConversationMessage::Tool { tool_call_id, content } => Some(json!({"id":tool_call_id,"content":content})),
            ConversationMessage::User { content } => Some(json!({"feedback":content})),
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
fn completion(id: &str, path: Option<&str>) -> ModelAction {
    ModelAction::CompletionRequest { action_id: id.into(), payload: json!({"findings":[{"blocking":false,"message":"missing_symbol not found","path":path,"severity":"low"}]}) }
}
fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git").current_dir(path).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}
fn fixture(large_change: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Range Test"]);
    git(dir.path(), &["config", "core.autocrlf", "false"]);
    std::fs::write(dir.path().join("a.txt"), "old\n").unwrap();
    std::fs::write(dir.path().join("large.txt"), format!("{}\n", "x".repeat(70_000))).unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "base"]);
    std::fs::write(dir.path().join("a.txt"), "one\nneedle\nthree\nfour\n").unwrap();
    if large_change {
        for i in 0..21 { std::fs::write(dir.path().join(format!("changed{i}.rs")), "pub const N: u8 = 1;\n").unwrap(); }
    }
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "change"]);
    dir
}
fn request(path: &Path) -> ReviewRequest {
    ReviewRequest { schema: "review.request/v1".into(), repository_path: path.to_str().unwrap().into(), base_ref: "HEAD~1".into(), head_ref: "HEAD".into(), requirements: None }
}
fn summaries(events: &[LedgerEvent]) -> Vec<Value> {
    events.iter().filter(|e| e.event_type == "kernel.tool_completed").map(|e| serde_json::from_str(e.details.as_deref().unwrap()).unwrap()).collect()
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
#[test]
fn production_review_ranges_keep_identity_and_require_same_query_global_evidence() {
    let dir = fixture(false);
    let seen = Arc::new(Mutex::new(vec![]));
    let mut actions = vec![
        call("summary", "get_change_summary", json!({})),
        call("files", "get_changed_files", json!({})),
        call("diff", "read_diff", json!({"path":"a.txt"})),
        call("failed", "read_file", json!({"path":"missing.txt","offset":2,"limit":1})),
        call("denied", "read_file", json!({"path":".env","offset":2,"limit":1})),
        call("truncated", "read_file", json!({"path":"large.txt","offset":1,"limit":1})),
    ];
    for offset in 1..=4 { actions.push(call(&format!("range{offset}"), "read_file", json!({"path":"a.txt","offset":offset,"limit":1}))); }
    actions.extend([
        call("selected-match", "search_text", json!({"query":"needle","path":"a.txt","offset":2,"limit":1})),
        call("selected-clear", "search_text", json!({"query":"needle"})),
        call("scoped", "search_text", json!({"query":"missing_symbol","path":"a.txt","offset":2,"limit":1})),
        completion("premature", Some("a.txt")),
        call("broad", "search_text", json!({"query":"missing_symbol"})),
        completion("final", Some("a.txt")),
    ]);
    let config = ReviewConfig { max_turns: 25, max_tool_calls: 25, max_repeated_actions: 1, ..Default::default() };
    let (result, events) = run_review_with_provider(&request(dir.path()), &config, Probe { actions: actions.into(), seen: seen.clone() }, None).unwrap();
    assert!(matches!(result.status, ReviewStatus::Approved));
    assert_eq!(events.iter().filter(|e| e.event_type == "kernel.completion_rejected").count(), 1);
    let seen = seen.lock().unwrap();
    let tools = seen[0]["tools"].as_array().unwrap();
    for name in ["read_file", "search_text"] {
        let schema = &tools.iter().find(|t| t["name"] == name).unwrap()["schema"];
        for parameter in ["offset", "limit"] {
            assert_eq!(schema["properties"][parameter]["type"], "integer");
            assert!(!schema["required"].as_array().unwrap().iter().any(|v| v == parameter));
        }
    }
    // General skill is a system instruction; base orientation is user context.
    for role in ["instructions", "context"] {
        assert!(seen[0][role].as_array().unwrap().iter().any(|v| {
            v.as_str().unwrap().contains("offset is a 1-based starting line")
        }), "shared range guidance missing from {role}");
    }
    let expected_id = content_id_for_bytes(b"one\nneedle\nthree\nfour\n");
    for (i, text) in ["one\n", "needle\n", "three\n", "four\n"].iter().enumerate() {
        let value = tool_value(&seen, &format!("range{}", i + 1));
        assert_eq!(value["content"], *text);
        assert_eq!(value["content_id"], expected_id);
        assert_eq!(value["completeness"], true);
    }
    let matched = tool_value(&seen, "selected-match");
    assert_eq!(matched["matches"].as_array().unwrap().len(), 1);
    assert_eq!(matched["matches"][0]["path"], "a.txt");
    assert_eq!(matched["matches"][0]["line"], 2);
    assert!(tool_value(&seen, "scoped")["matches"].as_array().unwrap().is_empty());
    assert_eq!(tool_value(&seen, "scoped")["completeness"], true);
    assert_eq!(tool_value(&seen, "truncated")["truncated"], true);
    assert!(seen.iter().any(|v| v["history"].to_string().contains("without path, offset or limit")));
    let metadata = summaries(&events);
    let ranges: Vec<_> = metadata.iter().filter(|v| v["tool"] == "read_file" && v["path"] == "a.txt").collect();
    assert_eq!(ranges.len(), 4);
    for (i, range) in ranges.iter().enumerate() {
        assert_eq!(range["scope"], "file_range");
        assert_eq!(range["offset"], i + 1);
        assert_eq!(range["limit"], 1);
        assert_eq!(range["has_content"], true);
    }
    for (path, status) in [("missing.txt", "Failed"), (".env", "Denied")] {
        let failed = metadata.iter().find(|v| v["path"] == path).unwrap();
        assert_eq!(failed["status"], status);
        assert!(failed.get("has_content").is_none());
    }
    let truncated = metadata.iter().find(|v| v["path"] == "large.txt").unwrap();
    assert_eq!(truncated["scope"], "file_range");
    assert_eq!(truncated["completeness"], false);
    assert!(seen.last().unwrap()["context"].to_string().contains("Full-file read paths: []"));
}
#[test]
fn global_same_query_recovery_does_not_bypass_original_review_gates() {
    for case in ["coverage", "finding_path", "requirements", "exploration", "skill"] {
        let dir = fixture(matches!(case, "exploration" | "skill"));
        let mut req = request(dir.path());
        if case == "requirements" { req.requirements = Some("exponential resilience".into()); }
        let mut actions = vec![call("summary", "get_change_summary", json!({})), call("files", "get_changed_files", json!({}))];
        if case != "coverage" { actions.push(call("diff", "read_diff", json!({"path":"a.txt"}))); }
        if case == "skill" {
            actions.push(call("read", "read_file", json!({"path":"a.txt","offset":1,"limit":1})));
            actions.push(call("list", "list_directory", json!({"path":"."})));
        }
        actions.extend([call("scoped", "search_text", json!({"query":"missing_symbol","path":"a.txt"})), call("broad", "search_text", json!({"query":"missing_symbol"})), completion("attempt", if case == "finding_path" { Some("uninspected.txt") } else { None })]);
        let turns = actions.len() as u32;
        let seen = Arc::new(Mutex::new(vec![]));
        let config = ReviewConfig { max_turns: turns + 1, max_tool_calls: 20, max_completion_attempts: 1, ..Default::default() };
        actions.push(completion("budget-stop", None));
        let (result, events) = run_review_with_provider(&req, &config, Probe { actions: actions.into(), seen: seen.clone() }, None).unwrap();
        assert!(matches!(result.status, ReviewStatus::Indeterminate), "{case}");
        assert_eq!(events.iter().filter(|e| e.event_type == "kernel.completion_rejected").count(), 1, "{case}");
        let feedback = seen.lock().unwrap().last().unwrap()["history"].to_string();
        let expected = match case {
            "coverage" => "You must read all changed files",
            "finding_path" => "must only reference files that have been inspected",
            "requirements" => "must reference the provided requirements",
            "exploration" => "Call list_directory",
            "skill" => "Skill-specific requirements not met",
            _ => unreachable!(),
        };
        assert!(feedback.contains(expected), "{case}: {feedback}");
    }
}
