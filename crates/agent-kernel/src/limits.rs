use std::path::PathBuf;
use std::time::Duration;

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
