use serde_json::Value;
use std::time::Instant;

use crate::application::{ContextBlock, InstructionBlock};

#[derive(Debug, Clone)]
pub enum ModelError {
    Network(String),
    Timeout(String),
    ApiError(String),
    RateLimit(String),
}

/// Provider-independent accounting for tokens consumed by a model request.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRecord {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Clone)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct CanonicalModelRequest {
    pub instructions: Vec<InstructionBlock>,
    pub context: Vec<ContextBlock>,
    pub tools: Vec<ToolDescription>,
}

#[derive(Clone, Debug)]
pub enum ModelAction {
    ToolCall {
        action_id: String,
        tool: String,
        arguments: Value,
    },
    CompletionRequest {
        action_id: String,
        payload: Value,
    },
    PauseRequest {
        action_id: String,
        payload: Value,
    },
    ApplicationAction {
        action_id: String,
        payload: Value,
    },
}

#[derive(Clone, Debug)]
pub struct CanonicalModelResponse {
    pub actions: Vec<ModelAction>,
    pub usage: Option<UsageRecord>,
}

impl CanonicalModelResponse {
    /// Additive access point for provider accounting; populated by the runtime adapter.
    pub fn usage(&self) -> Option<&UsageRecord> {
        self.usage.as_ref()
    }
}

pub trait ModelProvider {
    fn generate(
        &mut self,
        request: &CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse, ModelError>;

    fn generate_with_deadline(
        &mut self,
        request: &CanonicalModelRequest,
        deadline: Instant,
    ) -> Result<CanonicalModelResponse, ModelError>;
}
