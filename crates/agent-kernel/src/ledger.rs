use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LedgerEvent {
    pub event_type: String,
    pub sequence: u64,
    pub session_id: String,
    pub turn: u32,
    pub action_id: String,
    pub execution_id: String,
}

#[derive(Default)]
pub struct InMemoryLedger {
    events: Vec<LedgerEvent>,
}

impl InMemoryLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, mut event: LedgerEvent) -> LedgerEvent {
        event.sequence = self.events.len() as u64;
        self.events.push(event.clone());
        event
    }

    pub fn events(&self) -> &[LedgerEvent] {
        &self.events
    }
}

pub struct Limits {
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_completion_attempts: u32,
}
