use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LedgerEvent {
    pub event_type: String,
    pub sequence: u64,
    pub session_id: String,
    pub turn: u32,
    pub action_id: String,
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
}

pub struct InMemoryLedger {
    events: Vec<LedgerEvent>,
    file: Option<std::fs::File>,
    last_hash: Option<String>,
}

impl InMemoryLedger {
    pub fn new() -> Self {
        Self::with_path(None)
    }

    pub fn with_path(ledger_path: Option<&PathBuf>) -> Self {
        let file = ledger_path.map(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(path)
                .expect("open ledger file")
        });
        Self {
            events: Vec::new(),
            file,
            last_hash: None,
        }
    }
    pub fn append(&mut self, mut event: LedgerEvent) -> LedgerEvent {
        event.sequence = self.events.len() as u64;
        event.prev_hash = self.last_hash.clone();

        // Compute hash of teh event (including prev_hash) for the next link
        let event_json = serde_json::to_string(&event).unwrap_or_default();
        let hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(event_json.as_bytes()))
        );
        self.last_hash = Some(hash);

        // Write to JSONL file if configured
        if let Some(file) = &mut self.file {
            writeln!(file, "{}", event_json).expect("write ledger event");
            file.flush().expect("flush ledger file");
        }

        self.events.push(event.clone());
        event
    }

    pub fn events(&self) -> &[LedgerEvent] {
        &self.events
    }
}

impl Default for InMemoryLedger {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Limits {
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_completion_attempts: u32,
    pub wall_clock_budget: Option<Duration>,
    pub ledger_path: Option<PathBuf>,
    pub max_repeated_actions: u32,
    pub max_input_tokens: Option<u64>,
    pub max_cost_usd: Option<f64>,
}
