use serde_json::Value;

use crate::model::ToolDescription;

#[derive(Clone, Debug)]
pub enum ToolStatus {
    Succeeded,
    Failed,
    Denied,
}

#[derive(Clone, Debug)]
pub struct ToolResult {
    pub status: ToolStatus,
    pub value: Value,
}

// Object-safe trait for tool handers.  The coordinator calls
// 'validate_arguments' first; only if it passes does it call 'execute'.
pub trait Tool {
    fn name(&self) -> &str;
    fn description(&self) -> ToolDescription;
    fn validate_arguments(&self, arguments: &Value) -> Result<(), String>;
    fn execute(&self, arguments: &Value) -> ToolResult;
}

use std::collections::HashMap;

pub struct ToolCatalog {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl Default for ToolCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolCatalog {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|b| b.as_ref())
    }

    pub fn descriptions(&self) -> Vec<ToolDescription> {
        self.tools.values().map(|tool| tool.description()).collect()
    }
}
