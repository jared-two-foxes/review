use agent_protocol::IdGenerator;
use std::time::Instant;
use tracing;

use crate::application::{AgentApplication, CompletionDecision, ContextBlock};
use crate::ledger::{InMemoryLedger, LedgerEvent, Limits};
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
            ledger: InMemoryLedger::new(),
            limits,
        }
    }

    pub fn run(self, request: A::Request) -> A::Result {
        self.run_full(request).0
    }

    pub fn run_full(mut self, request: A::Request) -> (A::Result, Vec<LedgerEvent>) {
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
            if turn > self.limits.max_turns {
                return (
                    self.app.build_terminal_result(&state, &usage),
                    self.ledger.events().to_vec(),
                );
            }

            // Build model request from app state + available tools.
            let instructions = self.app.build_system_instructions(&state);
            let mut context = self.app.build_context(&state);

            // Truncate history by complete turns when exceeding budget.
            // A "turn" = one Assitant message + all following Tool/User messages
            // until next Assistant message.
            const MAX_HISTORY_MESSAGES: usize = 30;
            while history.len() > MAX_HISTORY_MESSAGES {
                let first_assistant_index = history
                    .iter()
                    .position(|msg| matches!(msg, ConversationMessage::Assistant { .. }))
                    .unwrap_or(0);
                let next_assistant_index = history[first_assistant_index + 1..]
                    .iter()
                    .position(|msg| matches!(msg, ConversationMessage::Assistant { .. }))
                    .map(|i| i + first_assistant_index + 1)
                    .unwrap_or(history.len());
                history.drain(first_assistant_index..next_assistant_index);
            }

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
                        history.push(ConversationMessage::Tool {
                            tool_call_id: action_id.clone(),
                            content: format!(
                                "{} Tool call {} for \"{}\" completed with status {:?} and value: {}",
                                UNTRUSTED_REPOSITORY_CONTENT_MARKER, action_id, tool, result.status, result.value
                            ),
                        });

                        tracing::info!(turn, action_id, tool= %tool, "tool completed");

                        let tool_details =
                            build_tool_event_details(&tool, &arguments, &result.value);
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
    result: &serde_json::Value,
) -> Option<String> {
    let mut summary = serde_json::json!({"tool": tool});
    if let Some(path) = arguments.get("path").and_then(|v| v.as_str()) {
        summary["path"] = serde_json::Value::String(path.into());
    }
    if let Some(query) = arguments.get("query").and_then(|v| v.as_str()) {
        summary["query"] = serde_json::Value::String(query.into());
    }
    if let Some(truncated) = result.get("truncated").and_then(|v| v.as_bool()) {
        summary["truncated"] = serde_json::Value::Bool(truncated);
    }
    if let Some(completeness) = result.get("completeness").and_then(|v| v.as_str()) {
        summary["completeness"] = serde_json::Value::String(completeness.into());
    }
    if tool == "get_changed_files" {
        if let Some(changed_files) = result.get("files").and_then(|f| f.as_array()) {
            let paths: Vec<&str> = changed_files
                .iter()
                .filter_map(|f| f["path"].as_str())
                .collect();
            summary["changed_files"] = serde_json::json!(paths);
        }
    }
    Some(summary.to_string())
}
