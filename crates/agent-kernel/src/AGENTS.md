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

## Subsystem interaction
These modules are tightly coupled: the coordinator uses the application trait, model types, ledger, and tool catalog to execute one review session.

## External dependencies used from this level
`agent-protocol` provides ID generation, while `serde_json` and `tracing` support payload handling and observability.
