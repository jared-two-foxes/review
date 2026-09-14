# AGENTS.md

## Module summary
`review-app/src` contains the review application composition and the review-specific state machine.

## Contents
_No child subfolders._

## Key modules
- `lib.rs` defines review configuration, review execution entrypoints, state/completion models, the `ReviewApplication`, and the lightweight `ReadChangeTool` used in deterministic scenarios.

## Subsystem interaction
The review application builds the tool catalog, resolves base/head targets, reads optional requirements, and hands execution to the `agent-kernel` coordinator.

## External dependencies used from this level
It uses `code-agent-runtime` for concrete tools and provider wiring, `agent-protocol` for clocks and IDs, and `review-protocol` for public request/result types.

## Usage example
```rust
let (result, events, setup_error) = review_app::run_review(&request, &config);
```
