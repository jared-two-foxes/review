use agent_protocol::IdGenerator;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tracing;

use crate::application::{AgentApplication, CompletionDecision};
use crate::ledger::{InMemoryLedger, LedgerEvent};
use crate::limits::Limits;
use crate::model::{
    CanonicalModelRequest, ConversationMessage, ModelAction, ModelProvider, ToolCallRecord,
    UsageRecord,
};
use crate::tools::ToolCatalog;

const UNTRUSTED_REPOSITORY_CONTENT_MARKER: &str = "[untrusted repository content]";

pub struct SessionCoordinator<A, P, I> {
    app: A,
    provider: P,
    id_gen: I,
    catalog: ToolCatalog,
    ledger: InMemoryLedger,
    limits: Limits,
}

impl<A, P, I> SessionCoordinator<A, P, I>
where
    A: AgentApplication,
    P: ModelProvider,
    I: IdGenerator,
{
    pub fn new(app: A, provider: P, id_gen: I, catalog: ToolCatalog, limits: Limits) -> Self {
        Self {
            app,
            provider,
            id_gen,
            catalog,
            ledger: InMemoryLedger::with_path(limits.ledger_path.as_ref()),
            limits,
        }
    }

    pub fn run(self, request: A::Request) -> A::Result {
        self.run_full(request, None).0
    }

    pub fn run_full(
        mut self,
        request: A::Request,
        cancel: Option<&AtomicBool>,
    ) -> (A::Result, Vec<LedgerEvent>) {
        self.app
            .validate_request(&request)
            .expect("request validation failed");

        let init = self
            .app
            .initialize(&request)
            .expect("initialization failed");
        let mut state = init.initial_state;

        // Resolve requested tools against the catalog
        let tool_descriptions: Vec<_> = init
            .requested_tools
            .iter()
            .filter_map(|name| self.catalog.get(name).map(|t| t.description()))
            .collect();

        let mut latest_tool_call_ids: HashMap<(String, String), String> = HashMap::new();
        let mut repeat_counts: HashMap<(String, String), u32> = HashMap::new();

        // -- Budget counter --
        let mut turn = 0u32;
        let mut tool_call_count = 0u32;
        let mut completion_attempt_count = 0u32;
        let session_id = self.id_gen.next_id();
        let session_deadline = self
            .limits
            .wall_clock_budget
            .map(|budget| Instant::now() + budget);

        let mut history: Vec<ConversationMessage> = Vec::new();
        let mut usage = UsageRecord {
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: None,
        };

        // Main Loop
        loop {
            turn += 1;

            // Check for cancellation before starting a new turn
            if let Some(token) = cancel {
                if token.load(Ordering::Relaxed) {
                    tracing::warn!(turn, "session cancelled by external request");
                    self.append_event_with_details(
                        &session_id,
                        turn,
                        "",
                        "kernel.session_cancelled",
                        Some("cancelled by external request".into()),
                    );
                    return (
                        self.app.build_terminal_result(&state, &usage),
                        self.ledger.events().to_vec(),
                    );
                }
            }

            if turn > self.limits.max_turns {
                return (
                    self.app.build_terminal_result(&state, &usage),
                    self.ledger.events().to_vec(),
                );
            }

            // Build model request from app state + available tools.
            let instructions = self.app.build_system_instructions(&state);
            let context = self.app.build_context(&state);

            // Truncate history by complete turns when exceeding budget.
            // A "turn" = one Assitant message + all following Tool/User messages
            // until next Assistant message.
            compact_history(&mut history);

            let model_request = CanonicalModelRequest {
                instructions,
                context,
                tools: tool_descriptions.clone(),
                history: history.clone(),
            };

            self.append_event(&session_id, turn, "", "kernel.model_started");
            tracing::info!(turn, "model started");

            // Call the model
            let generation = match session_deadline {
                Some(deadline) => self
                    .provider
                    .generate_with_deadline(&model_request, deadline),
                None => self.provider.generate(&model_request),
            };
            let response = match generation {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(turn, error = ?e, "model failed");
                    self.append_event_with_details(
                        &session_id,
                        turn,
                        "",
                        "kernel.model_failed",
                        Some(format!("{:?}", e)),
                    );
                    self.append_event(&session_id, turn, "", "kernel.session_indeterminate");
                    return (
                        self.app.build_terminal_result(&state, &usage),
                        self.ledger.events().to_vec(),
                    );
                }
            };

            if let Some(record) = response.usage() {
                usage.input_tokens += record.input_tokens;
                usage.output_tokens += record.output_tokens;
                usage.estimated_cost_usd =
                    match (usage.estimated_cost_usd, record.estimated_cost_usd) {
                        (Some(total), Some(cost)) => Some(total + cost),
                        (None, cost) => cost,
                        (total, None) => total,
                    };
            }

            let mut exhausted_budgets = Vec::new();
            if self
                .limits
                .max_input_tokens
                .is_some_and(|limit| usage.input_tokens > limit)
            {
                exhausted_budgets.push("input_tokens exceeded max_input_tokens");
            }
            if let (Some(limit), Some(cost)) = (self.limits.max_cost_usd, usage.estimated_cost_usd)
            {
                if cost > limit {
                    exhausted_budgets.push("estimated_cost_usd exceeded max_cost_usd");
                }
            }
            if !exhausted_budgets.is_empty() {
                self.append_event_with_details(
                    &session_id,
                    turn,
                    "",
                    "kernel.session_budget_exhausted",
                    Some(exhausted_budgets.join("; ")),
                );
                return (
                    self.app.build_terminal_result(&state, &usage),
                    self.ledger.events().to_vec(),
                );
            }

            tracing::info!(turn, "model completed");
            self.append_event(&session_id, turn, "", "kernel.model_completed");

            // Record the assistant's actions in conversation history.
            let tool_calls: Vec<ToolCallRecord> = response
                .actions
                .iter()
                .filter_map(|action| match action {
                    ModelAction::ToolCall {
                        action_id,
                        tool,
                        arguments,
                    } => Some(ToolCallRecord {
                        id: action_id.clone(),
                        name: tool.clone(),
                        arguments: arguments.clone(),
                    }),
                    _ => None,
                })
                .collect();
            let completion_content: Option<String> =
                response.actions.iter().find_map(|action| match action {
                    ModelAction::CompletionRequest { payload, .. } => {
                        Some(serde_json::to_string(payload).unwrap_or_default())
                    }
                    _ => None,
                });
            if !tool_calls.is_empty() || completion_content.is_some() {
                history.push(ConversationMessage::Assistant {
                    content: completion_content,
                    tool_calls,
                });
            }

            // Dispatch each action sequentially
            for action in response.actions {
                match action {
                    ModelAction::ToolCall {
                        action_id,
                        tool,
                        arguments,
                    } => {
                        tool_call_count += 1;
                        if tool_call_count > self.limits.max_tool_calls {
                            return (
                                self.app.build_terminal_result(&state, &usage),
                                self.ledger.events().to_vec(),
                            );
                        }

                        // Check for cancellation before executing a tool
                        if let Some(token) = cancel {
                            if token.load(std::sync::atomic::Ordering::Relaxed) {
                                tracing::warn!(turn, "session cancelled during tool dispatch");
                                self.append_event_with_details(
                                    &session_id,
                                    turn,
                                    "",
                                    "kernel.session_cancelled",
                                    Some("cancelled by external request".into()),
                                );
                                return (
                                    self.app.build_terminal_result(&state, &usage),
                                    self.ledger.events().to_vec(),
                                );
                            }
                        }
                        let tool_impl = match self.catalog.get(&tool) {
                            Some(t) => t,
                            None => {
                                history.push(ConversationMessage::Tool {
                                    tool_call_id: action_id.clone(),
                                    content: format!(
                                        "{}, Tool call {} for \"{}\" was rejected: no such tool is available.",
                                        UNTRUSTED_REPOSITORY_CONTENT_MARKER, action_id, tool
                                    ),
                                });
                                tracing::warn!(turn, action_id, tool = %tool, "tool rejected: no such tool is available");
                                self.append_event(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.action_rejected",
                                );
                                continue;
                            }
                        };

                        if let Err(msg) = tool_impl.validate_arguments(&arguments) {
                            history.push(ConversationMessage::Tool {
                                tool_call_id: action_id.clone(),
                                content: format!(
                                    "{}, Tool call {} for \"{}\" was rejected: invalid arguments: {}",
                                    UNTRUSTED_REPOSITORY_CONTENT_MARKER, action_id, tool, msg
                                ),
                            });
                            tracing::warn!(turn, action_id, tool= %tool, "tool rejected: invalid arguments");
                            self.append_event_with_details(
                                &session_id,
                                turn,
                                &action_id,
                                "kernel.action_rejected",
                                Some(format!("invalid arguments: {}", msg)),
                            );
                            // Do NOT call execute
                            continue;
                        }

                        let result = tool_impl.execute(&arguments);
                        let call_key = (
                            tool.clone(),
                            serde_json::to_string(&arguments).unwrap_or_default(),
                        );

                        // If this tool was called before with teh same arguments, compact the
                        // earlier result to a pointer.  The latest result stays in full.
                        if let Some(prev_id) = latest_tool_call_ids.get(&call_key) {
                            for msg in history.iter_mut() {
                                if let ConversationMessage::Tool {
                                    tool_call_id,
                                    content,
                                } = msg
                                {
                                    if tool_call_id == prev_id {
                                        *content = format!(
                                            "[deduplicated: superseded by a later call to \"{}\" with the same arguments.]",
                                            tool
                                        );
                                        break;
                                    }
                                }
                            }

                            let count = repeat_counts.entry(call_key.clone()).or_insert(0);
                            *count += 1;
                            if *count >= self.limits.max_repeated_actions {
                                tracing::warn!(
                                    turn,
                                    tool = %tool,
                                    repeated = *count,
                                    "session stalled: repeated identical tool calls"
                                );
                                self.append_event_with_details(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.session_stalled",
                                    Some(format!(
                                        "repeated identical call to \"{}\" {} times",
                                        tool, *count,
                                    )),
                                );
                                return (
                                    self.app.build_terminal_result(&state, &usage),
                                    self.ledger.events().to_vec(),
                                );
                            }
                        }

                        latest_tool_call_ids.insert(call_key, action_id.clone());

                        history.push(ConversationMessage::Tool {
                            tool_call_id: action_id.clone(),
                            content: format!(
                                "{} Tool call {} for \"{}\" completed with status {:?} and value: {}",
                                UNTRUSTED_REPOSITORY_CONTENT_MARKER, action_id, tool, result.status, result.value
                            ),
                        });

                        tracing::info!(turn, action_id, tool= %tool, "tool completed");

                        let tool_details = build_tool_event_details(&tool, &arguments, &result);
                        let event = self.append_event_with_details(
                            &session_id,
                            turn,
                            &action_id,
                            "kernel.tool_completed",
                            tool_details,
                        );

                        state = self.app.reduce_event(&state, &event);
                    }
                    ModelAction::CompletionRequest { action_id, payload } => {
                        completion_attempt_count += 1;
                        if completion_attempt_count > self.limits.max_completion_attempts {
                            return (
                                self.app.build_terminal_result(&state, &usage),
                                self.ledger.events().to_vec(),
                            );
                        }

                        let completion = match self.app.parse_completion(&payload) {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::warn!(turn, action_id, "completion rejected: parse error");
                                history.push(ConversationMessage::User {
                                    content: format!("Completion rejected. Parse error: {:?}", e),
                                });
                                self.append_event_with_details(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.completion_rejected",
                                    Some(format!("{:?}", e)),
                                );
                                continue;
                            }
                        };

                        match self.app.validate_completion(&state, &completion) {
                            CompletionDecision::Accepted => {
                                tracing::info!(turn, action_id, "completion accepted");
                                self.append_event(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.completion_accepted",
                                );
                                return (
                                    self.app.build_terminal_result(&state, &usage),
                                    self.ledger.events().to_vec(),
                                );
                            }
                            CompletionDecision::RejectedRemediable {
                                feedback_for_model, ..
                            } => {
                                for block in &feedback_for_model {
                                    history.push(ConversationMessage::User {
                                        content: format!(
                                            "Completion rejected. Feedback: {}",
                                            block.content
                                        ),
                                    });
                                }
                                tracing::warn!(turn, action_id, "completion rejected: remediable");
                                self.append_event(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.completion_rejected",
                                );
                                // Loop continues - feedback_for_model would be
                                // injected into next turn's context
                                // Note: completion_attempt_count was incremented,
                                // turn and tool_call_count are NOT reset
                            }
                            CompletionDecision::RejectedTerminal { .. } => {
                                tracing::warn!(turn, action_id, "completion rejected: terminal");
                                self.append_event(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.completion_rejected",
                                );
                                return (
                                    self.app.build_terminal_result(&state, &usage),
                                    self.ledger.events().to_vec(),
                                );
                            }
                            CompletionDecision::Paused => {
                                // V1: Not supported, end session
                                return (
                                    self.app.build_terminal_result(&state, &usage),
                                    self.ledger.events().to_vec(),
                                );
                            }
                        }
                    }

                    // V1: reject unsupported action kinds
                    ModelAction::PauseRequest { action_id, .. } => {
                        self.append_event(&session_id, turn, &action_id, "kernel.action_rejected");
                    }
                    ModelAction::ApplicationAction { action_id, .. } => {
                        self.append_event(&session_id, turn, &action_id, "kernel.action_rejected");
                    }
                }
            }
        }
    }

    fn append_event(
        &mut self,
        session_id: &str,
        turn: u32,
        action_id: &str,
        event_type: &str,
    ) -> LedgerEvent {
        self.append_event_with_details(session_id, turn, action_id, event_type, None)
    }

    fn append_event_with_details(
        &mut self,
        session_id: &str,
        turn: u32,
        action_id: &str,
        event_type: &str,
        details: Option<String>,
    ) -> LedgerEvent {
        let execution_id = self.id_gen.next_id();
        self.ledger.append(LedgerEvent {
            event_type: event_type.into(),
            sequence: 0,
            session_id: session_id.into(),
            turn,
            action_id: action_id.into(),
            execution_id,
            details,
            prev_hash: None,
        })
    }

    // Expose the ledger for test assertions
    pub fn ledger(&self) -> &InMemoryLedger {
        &self.ledger
    }
}

/// Build a JSON summary of a tall call fo rthe event details field.
/// This lets 'reduce_event` reconstruct what was inspected without access to
/// the conversation history.
fn build_tool_event_details(
    tool: &str,
    arguments: &serde_json::Value,
    result: &crate::tools::ToolResult,
) -> Option<String> {
    let mut summary = serde_json::json!({"tool": tool});
    summary["status"] = serde_json::Value::String(
        match result.status {
            crate::tools::ToolStatus::Succeeded => "Succeeded",
            crate::tools::ToolStatus::Failed => "Failed",
            crate::tools::ToolStatus::Denied => "Denied",
        }
        .into(),
    );
    if let Some(path) = arguments.get("path").and_then(|v| v.as_str()) {
        summary["path"] = serde_json::Value::String(path.into());
    }
    if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
        summary["query"] = serde_json::Value::String(query.into());
    }
    if let Some(content_id) = result.value.get("content_id").and_then(|v| v.as_str()) {
        summary["content_id"] = serde_json::Value::String(content_id.into());
    }
    if let Some(truncated) = result.value.get("truncated").and_then(|v| v.as_bool()) {
        summary["truncated"] = serde_json::Value::Bool(truncated);
    }
    if let Some(completeness) = result.value.get("completeness").and_then(|v| v.as_bool()) {
        summary["completeness"] = serde_json::Value::Bool(completeness);
    }
    if tool == "get_changed_files" || tool == "get_change_summary" {
        if let Some(changed_files) = result.value.get("files").and_then(|f| f.as_array()) {
            let paths: Vec<&str> = changed_files
                .iter()
                .filter_map(|f| f["path"].as_str())
                .collect();
            summary["changed_files"] = serde_json::json!(paths);
        }
    }
    Some(summary.to_string())
}

/// Maximum total byte count o tool result content before truncation
/// ~80KB ~= 20K tokens, leaving room for system prompt and model output,
const MAX_HISTORY_BYTES: usize = 80_000;

/// Tool results larger than this are eligibale for truncation.
const TRUNCATION_THRESHOLD_BYTES: usize = 2_000;

/// How many bytes to keep when truncating a large tool result.
const TRUNCATION_KEEP_BYTES: usize = 1_000;

/// Bytes of recent history to preserve in full (the model's working set).
const PRESERVED_RECENT_BYTES: usize = 20_000;

/// Compact conversation hustory by truncating old, large tool results.
///
/// Only Tool messages older than the preserved zone and larger than the
/// truncation threshold are truncated.  Assistant messages, User message,
/// dedupliated results, and recent results are left untouched
fn compact_history(history: &mut Vec<ConversationMessage>) {
    let total_bytes: usize = history
        .iter()
        .map(|msg| match msg {
            ConversationMessage::Tool { content, .. } => content.len(),
            _ => 0,
        })
        .sum();

    if total_bytes <= MAX_HISTORY_BYTES {
        return;
    }

    // Find the cutoff: iterate from the end, preserving the most recent
    // bytes up to PRESERVED_RECENT_BYTES.
    let mut accumulated = 0usize;
    let mut cutoff = history.len();
    for (i, msg) in history.iter().enumerate().rev() {
        if accumulated >= PRESERVED_RECENT_BYTES {
            cutoff = i + 1;
            break;
        }
        if let ConversationMessage::Tool { content, .. } = msg {
            accumulated += content.len();
        }
        cutoff = i;
    }

    // Truncate large Tool messages before the cutoff.
    for msg in history[..cutoff].iter_mut() {
        if let ConversationMessage::Tool { content, .. } = msg {
            if content.len() > TRUNCATION_THRESHOLD_BYTES && !content.starts_with("[deduplicated:")
            {
                let limit = content.len().min(TRUNCATION_KEEP_BYTES);
                let mut keep_end = limit;
                while !content.is_char_boundary(keep_end) {
                    keep_end -= 1;
                }
                let original_len = content.len();
                *content = format!(
                    "{}\n[truncated: {} more bytes - re-call this tool to see full content]",
                    &content[..keep_end],
                    original_len.saturating_sub(keep_end)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_tool_event_details, compact_history, TRUNCATION_KEEP_BYTES};
    use crate::tools::{ToolResult, ToolStatus};
    use serde_json::json;

    #[test]
    fn change_summary_event_details_include_changed_files() {
        let details = build_tool_event_details(
            "get_change_summary",
            &json!({}),
            &ToolResult {
                status: ToolStatus::Succeeded,
                value: json!({
                    "files": [
                        {"path": "src/main.rs"},
                        {"path": "src/lib.rs"}
                    ]
                }),
            },
        )
        .expect("tool details");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&details).expect("json"),
            json!({
                "status": "Succeeded",
                "tool": "get_change_summary",
                "changed_files": ["src/main.rs", "src/lib.rs"]
            })
        );
    }

    #[test]
    fn compact_history_truncates_old_large_tool_results() {
        use crate::model::ConversationMessage;

        // 10 large old messages (10K each = 100K total) + 1 recent (5K).
        // Total = 105K > MAX_HISTORY_BYTES (80K), so truncation triggers.
        // Preserved zone (20K from end) covers the last 3 messages
        // (5K + 10K + 10K = 25K), so messages 0-7 are eligible.
        let mut history: Vec<ConversationMessage> = (0..10)
            .map(|i| ConversationMessage::Tool {
                tool_call_id: format!("old-{i}"),
                content: "x".repeat(10_000),
            })
            .collect();
        history.push(ConversationMessage::Tool {
            tool_call_id: "recent-1".into(),
            content: "z".repeat(5_000),
        });

        compact_history(&mut history);

        // Old messages (before preserved zone) should be truncated.
        assert!(
            matches!(&history[0], ConversationMessage::Tool { content, .. } if content.contains("[truncated:"))
        );
        assert!(
            matches!(&history[1], ConversationMessage::Tool { content, .. } if content.contains("[truncated:"))
        );
        // Recent message (within preserved zone) should be unchanged.
        assert!(
            matches!(&history[10], ConversationMessage::Tool { content, .. } if content.len() == 5_000 && !content.contains("[truncated:"))
        );
    }

    #[test]
    fn compact_history_preserves_short_results() {
        use crate::model::ConversationMessage;

        // 1 short old message + 9 large old messages (10K each) + 1 recent large.
        // Total = 100.5K > MAX_HISTORY_BYTES (80K), so truncation triggers.
        // Preserved zone (20K from end) covers the last 2 messages
        // (10K + 10K = 20K), so messages 0-9 are eligible.
        let mut history = vec![ConversationMessage::Tool {
            tool_call_id: "short-old".into(),
            content: "small result".into(),
        }];
        for i in 0..9 {
            history.push(ConversationMessage::Tool {
                tool_call_id: format!("large-old-{i}"),
                content: "x".repeat(10_000),
            });
        }
        history.push(ConversationMessage::Tool {
            tool_call_id: "recent-1".into(),
            content: "z".repeat(10_000),
        });

        compact_history(&mut history);

        // Short result should be unchanged (under TRUNCATION_THRESHOLD_BYTES).
        assert!(
            matches!(&history[0], ConversationMessage::Tool { content, .. } if content == "small result")
        );
        // Large old result should be truncated.
        assert!(
            matches!(&history[1], ConversationMessage::Tool { content, .. } if content.contains("[truncated:"))
        );
        // Recent message should be unchanged.
        assert!(
            matches!(&history[10], ConversationMessage::Tool { content, .. } if content.len() == 10_000 && !content.contains("[truncated:"))
        );
    }

    #[test]
    fn compact_history_skips_deduplicated_results() {
        use crate::model::ConversationMessage;

        let mut history = vec![
            ConversationMessage::Tool {
                tool_call_id: "dedup-1".into(),
                content: "[deduplicated: superseded by a later call]".repeat(500),
            },
            ConversationMessage::Tool {
                tool_call_id: "large-1".into(),
                content: "x".repeat(90_000),
            },
        ];

        compact_history(&mut history);

        // Deduplicated result should not be double-truncated.
        assert!(
            matches!(&history[0], ConversationMessage::Tool { content, .. } if !content.contains("[truncated:"))
        );
    }

    #[test]
    fn compact_history_noop_when_under_threshold() {
        use crate::model::ConversationMessage;

        let mut history = vec![ConversationMessage::Tool {
            tool_call_id: "1".into(),
            content: "small".into(),
        }];

        compact_history(&mut history);

        assert!(
            matches!(&history[0], ConversationMessage::Tool { content, .. } if content == "small")
        );
    }

    #[test]
    fn compact_history_truncates_at_byte_boundary_not_char_boundary() {
        use crate::model::ConversationMessage;

        // 4-byte UTF-8 characters (🎉 = U+1F389, 4 bytes in UTF-8).
        // 30,000 of them = 120,000 bytes. With the old char_indices().nth()
        // approach, the 1,000th character would be at byte offset 4,000,
        // keeping ~4KB instead of the intended ~1KB.
        let large_multi_byte: String = "🎉".repeat(30_000);

        // A second recent message pushes the large one outside the
        // preserved zone so it becomes eligible for truncation.
        let mut history = vec![
            ConversationMessage::Tool {
                tool_call_id: "old-multi-byte".into(),
                content: large_multi_byte,
            },
            ConversationMessage::Tool {
                tool_call_id: "recent".into(),
                content: "z".repeat(20_001),
            },
        ];

        compact_history(&mut history);

        if let ConversationMessage::Tool { content, .. } = &history[0] {
            let kept = content.split("\n[truncated:").next().unwrap();
            assert!(
                kept.len() <= TRUNCATION_KEEP_BYTES,
                "kept {} bytes, expected at most {}",
                kept.len(),
                TRUNCATION_KEEP_BYTES
            );
        }
    }
}
