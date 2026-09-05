use serde_json::Value;

use crate::ledger::{LedgerEvent, Limits};
use crate::model::UsageRecord;

pub struct ApplicationDescriptor {
    pub application_id: String,
    pub application_version: String,
    pub request_schema: String,
    pub completion_schema: String,
    pub result_schema: String,
    pub domain_event_namespace: String,
}

pub struct ApplicationInitialization<S> {
    pub initial_state: S,
    pub requested_tools: Vec<String>,
    pub requested_capabilities: Vec<String>,
    pub application_limits: Option<Limits>,
}

#[derive(Clone, Debug)]
pub struct InstructionBlock {
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ContextBlock {
    pub content: String,
}

pub enum CompletionDecision {
    Accepted,
    RejectedRemediable {
        reason_codes: Vec<String>,
        missing_requirements: Vec<String>,
        feedback_for_model: Vec<InstructionBlock>,
    },
    RejectedTerminal {
        reason: String,
    },
    Paused,
}

pub trait AgentApplication {
    type Request;
    type State: Clone;
    type Completion;
    type Result;
    type Error: std::fmt::Debug;

    fn descriptor(&self) -> ApplicationDescriptor;
    fn validate_request(&self, request: &Self::Request) -> Result<(), Self::Error>;
    fn initialize(
        &self,
        request: &Self::Request,
    ) -> Result<ApplicationInitialization<Self::State>, Self::Error>;
    fn build_system_instructions(&self, state: &Self::State) -> Vec<InstructionBlock>;
    fn build_context(&self, state: &Self::State) -> Vec<ContextBlock>;
    fn reduce_event(&self, state: &Self::State, event: &LedgerEvent) -> Self::State;
    fn parse_completion(&self, payload: &Value) -> Result<Self::Completion, Self::Error>;
    fn validate_completion(
        &self,
        state: &Self::State,
        completion: &Self::Completion,
    ) -> CompletionDecision;
    fn build_terminal_result(&self, state: &Self::State, usage: &UsageRecord) -> Self::Result;
}
