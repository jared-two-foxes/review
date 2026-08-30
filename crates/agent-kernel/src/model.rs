use serde_json::Value;

use crate::application::{ContextBlock, InstructionBlock};

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

#[derive(Clone)]
pub struct CanonicalModelResponse {
    pub actions: Vec<ModelAction>,
}

pub trait ModelProvider {
    fn generate(&mut self, request: &CanonicalModelRequest) -> CanonicalModelResponse;
}
