# AGENTS.md

## Module summary
`agent-kernel/src` contains the core building blocks for generic agent execution.

## Contents
_No child subfolders._

## Key modules
- `application.rs` defines the application contract, state initialization, and completion decisions.
- `coordinator.rs` runs the turn loop, invokes providers and tools, and records ledger events.
- `ledger.rs` stores in-memory session events and execution limits.
- `model.rs` defines canonical model requests, responses, tool calls, and usage accounting.
- `tools.rs` defines the tool trait, results, and catalog.
- `lib.rs` re-exports the crate modules.

## Find existing code
- Session flow and rejection behavior: `coordinator.rs`
- App-facing interfaces: `application.rs`
- Event and budget types: `ledger.rs`
- Provider payloads and history shapes: `model.rs`
- Tool registration and lookup: `tools.rs`

## Add new implementations
Add shared turn-loop, ledger, or provider abstractions here; avoid putting review-domain logic in this directory.

## Keep in sync
Changes to `ApplicationDescriptor`, `CompletionDecision`, model request/response types, or tool traits can ripple into `review-app`, `code-agent-runtime`, and golden tests.

## Search hints
Search for `run_full`, `validate_completion`, `CanonicalModelRequest`, `ConversationMessage`, and `ToolResult`.

## External dependencies used from this level
`agent-protocol` provides ID generation, while `serde_json` and `tracing` support payload handling and observability.
