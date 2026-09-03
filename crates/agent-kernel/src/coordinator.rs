use agent_protocol::IdGenerator;
use std::time::Instant;

use crate::application::{AgentApplication, CompletionDecision};
use crate::ledger::{InMemoryLedger, LedgerEvent, Limits};
use crate::model::{CanonicalModelRequest, ModelAction, ModelProvider};
use crate::tools::ToolCatalog;

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

        // Main Loop
        loop {
            turn += 1;
            if turn > self.limits.max_turns {
                return (
                    self.app.build_terminal_result(&state),
                    self.ledger.events().to_vec(),
                );
            }

            // Build model request from app state + available tools.
            let instructions = self.app.build_system_instructions(&state);
            let context = self.app.build_context(&state);
            let model_request = CanonicalModelRequest {
                instructions,
                context,
                tools: tool_descriptions.clone(),
            };

            self.append_event(&session_id, turn, "", "kernel.model_started");

            // Call the model
            let generation = match session_deadline {
                Some(deadline) => self
                    .provider
                    .generate_with_deadline(&model_request, deadline),
                None => self.provider.generate(&model_request),
            };
            let response = match generation {
                Ok(r) => r,
                Err(_) => {
                    self.append_event(&session_id, turn, "", "kernel.model_failed");
                    self.append_event(&session_id, turn, "", "kernel.session_indeterminate");
                    return (
                        self.app.build_terminal_result(&state),
                        self.ledger.events().to_vec(),
                    );
                }
            };

            self.append_event(&session_id, turn, "", "kernel.model_completed");

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
                                self.app.build_terminal_result(&state),
                                self.ledger.events().to_vec(),
                            );
                        }

                        let tool_impl = match self.catalog.get(&tool) {
                            Some(t) => t,
                            None => {
                                self.append_event(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.action_rejected",
                                );
                                continue;
                            }
                        };

                        if let Err(_msg) = tool_impl.validate_arguments(&arguments) {
                            self.append_event(
                                &session_id,
                                turn,
                                &action_id,
                                "kernel.action_rejected",
                            );
                            // Do NOT call execute
                            continue;
                        }

                        let _result = tool_impl.execute(&arguments);

                        let event = self.append_event(
                            &session_id,
                            turn,
                            &action_id,
                            "kernel.tool_completed",
                        );

                        state = self.app.reduce_event(&state, &event);
                    }
                    ModelAction::CompletionRequest { action_id, payload } => {
                        completion_attempt_count += 1;
                        if completion_attempt_count > self.limits.max_completion_attempts {
                            return (
                                self.app.build_terminal_result(&state),
                                self.ledger.events().to_vec(),
                            );
                        }

                        let completion = match self.app.parse_completion(&payload) {
                            Ok(c) => c,
                            Err(_) => {
                                self.append_event(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.completion_rejected",
                                );
                                continue;
                            }
                        };

                        match self.app.validate_completion(&state, &completion) {
                            CompletionDecision::Accepted => {
                                self.append_event(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.completion_accepted",
                                );
                                return (
                                    self.app.build_terminal_result(&state),
                                    self.ledger.events().to_vec(),
                                );
                            }
                            CompletionDecision::RejectedRemediable { .. } => {
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
                                self.append_event(
                                    &session_id,
                                    turn,
                                    &action_id,
                                    "kernel.completion_rejected",
                                );
                                return (
                                    self.app.build_terminal_result(&state),
                                    self.ledger.events().to_vec(),
                                );
                            }
                            CompletionDecision::Paused => {
                                // V1: Not supported, end session
                                return (
                                    self.app.build_terminal_result(&state),
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
        let execution_id = self.id_gen.next_id();
        self.ledger.append(LedgerEvent {
            event_type: event_type.into(),
            sequence: 0,
            session_id: session_id.into(),
            turn,
            action_id: action_id.into(),
            execution_id,
        })
    }

    // Expose the ledger for test assertions
    pub fn ledger(&self) -> &InMemoryLedger {
        &self.ledger
    }
}
