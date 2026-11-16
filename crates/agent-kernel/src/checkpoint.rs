//! Resumable session checkpoints.
//!
//! A checkpoint captures everything the coordinator needs to continue a
//! session that stopped early (tool-call limit, turn limit, budget, stall,
//! cancellation, crash): the ledger, the conversation history, the budget
//! counters and the repeat-detection bookkeeping.  Application state is *not*
//! stored; it is rebuilt by replaying the ledger through the application's
//! `reduce_event`, so the ledger remains the single source of truth.
//!
//! Limits are deliberately not stored either.  A resumed session is governed
//! by whatever limits the caller supplies, compared against the *cumulative*
//! counters, so raising a limit grants exactly the additional headroom.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::ledger::LedgerEvent;
use crate::model::{ConversationMessage, UsageRecord};

pub const CHECKPOINT_VERSION: u32 = 1;

/// Ledger event types that end a session.  They are kept in the ledger for
/// the audit trail but skipped when rebuilding application state, otherwise
/// the terminal reason of the previous attempt would leak into the resumed one.
pub const TERMINAL_EVENT_TYPES: &[&str] = &[
    "kernel.session_limit_exceeded",
    "kernel.session_budget_exhausted",
    "kernel.session_stalled",
    "kernel.session_cancelled",
    "kernel.session_indeterminate",
    "kernel.model_failed",
];

/// Stable identifier for a request, used to bind a checkpoint to it.
pub fn fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

#[derive(Debug)]
pub enum CheckpointError {
    Io(String),
    Parse(String),
    VersionMismatch { found: u32, expected: u32 },
    FingerprintMismatch,
}

impl std::fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "checkpoint I/O error: {e}"),
            Self::Parse(e) => write!(f, "checkpoint is not valid: {e}"),
            Self::VersionMismatch { found, expected } => write!(
                f,
                "checkpoint version {found} is not supported (expected {expected})"
            ),
            Self::FingerprintMismatch => write!(
                f,
                "checkpoint was created for a different request and cannot be resumed"
            ),
        }
    }
}

impl std::error::Error for CheckpointError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallKey {
    pub tool: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestCall {
    pub key: CallKey,
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatCount {
    pub key: CallKey,
    pub count: u32,
}

/// Cheap, human/wrapper-readable summary for deciding whether a resume with a
/// larger limit is worthwhile.  Informational only; never read on resume.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Progress {
    pub turns: u32,
    pub tool_calls: u32,
    /// Distinct (tool, arguments) pairs executed: repeated calls add nothing.
    pub distinct_tool_calls: u32,
    pub completion_attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    /// Identifies the request this checkpoint belongs to.
    pub fingerprint: String,
    pub session_id: String,
    pub turn: u32,
    pub tool_call_count: u32,
    pub completion_attempt_count: u32,
    pub usage: UsageRecord,
    pub history: Vec<ConversationMessage>,
    pub latest_tool_calls: Vec<LatestCall>,
    pub repeat_counts: Vec<RepeatCount>,
    pub events: Vec<LedgerEvent>,
    /// Event type that ended the run that wrote this checkpoint, if it ended
    /// (as opposed to a mid-session save).
    #[serde(default)]
    pub stopped_by: Option<String>,
    #[serde(default)]
    pub progress: Progress,
}

impl Checkpoint {
    pub fn load(path: &Path, fingerprint: &str) -> Result<Self, CheckpointError> {
        let bytes = std::fs::read(path).map_err(|e| CheckpointError::Io(format!("{}: {e}", path.display())))?;
        let checkpoint: Checkpoint =
            serde_json::from_slice(&bytes).map_err(|e| CheckpointError::Parse(e.to_string()))?;
        if checkpoint.version != CHECKPOINT_VERSION {
            return Err(CheckpointError::VersionMismatch {
                found: checkpoint.version,
                expected: CHECKPOINT_VERSION,
            });
        }
        if checkpoint.fingerprint != fingerprint {
            return Err(CheckpointError::FingerprintMismatch);
        }
        Ok(checkpoint)
    }

    /// Write atomically (temp file + rename) so a crash mid-write can never
    /// destroy the previous good checkpoint.
    pub fn save(&self, path: &Path) -> Result<(), CheckpointError> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| CheckpointError::Parse(e.to_string()))?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp, json).map_err(|e| CheckpointError::Io(format!("{}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path).map_err(|e| CheckpointError::Io(format!("{}: {e}", path.display())))
    }
}

/// Give every assistant tool call that has no recorded result a synthetic
/// one.  A session can stop between an assistant message and the execution of
/// its calls (limit hit mid-response, cancellation, stall); providers reject
/// histories with unanswered tool calls.
pub fn repair_dangling_tool_calls(history: &mut Vec<ConversationMessage>) {
    use std::collections::HashSet;
    let answered: HashSet<String> = history
        .iter()
        .filter_map(|m| match m {
            ConversationMessage::Tool { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect();
    let mut repaired = Vec::with_capacity(history.len());
    for message in history.drain(..) {
        let pending: Vec<(String, String)> = match &message {
            ConversationMessage::Assistant { tool_calls, .. } => tool_calls
                .iter()
                .filter(|c| !answered.contains(&c.id))
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect(),
            _ => Vec::new(),
        };
        repaired.push(message);
        for (id, name) in pending {
            repaired.push(ConversationMessage::Tool {
                content: format!(
                    "[untrusted repository content] Tool call {id} for \"{name}\" was not executed: the session stopped before it ran. Call it again if you still need it."
                ),
                tool_call_id: id,
            });
        }
    }
    *history = repaired;
}
