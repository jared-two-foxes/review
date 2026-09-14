# AGENTS.md

## Module summary
`review-app/src` contains the review application composition and the review-specific state machine.

## Contents
_No child subfolders._

## Key modules
- `lib.rs` defines review configuration, review execution entrypoints, state/completion models, the `ReviewApplication`, and the lightweight `ReadChangeTool` used in deterministic scenarios.

## Find existing code
- Review configuration and provider startup: `run_review` and `run_review_with_provider`
- Target resolution and concrete tool registration: `run_review_with_provider`
- Deterministic seams: `run_review_with_sources` and `ReadChangeTool`
- Completion validation and result shaping: `ReviewApplication` methods and review state types

## Add new implementations
Add review-specific state, prompt/context composition, and completion validation here. New repository access or model transport helpers should usually go in `code-agent-runtime` instead.

## Keep in sync
If you change findings, requirements handling, or tool expectations here, verify `review-cli`, evaluation tests, and golden fixtures still match.

## Search hints
Search for `compose_and_run`, `with_requirements`, `validate_completion`, `build_terminal_result`, and `ReadChangeTool`.

## External dependencies used from this level
It uses `code-agent-runtime` for concrete tools and provider wiring, `agent-protocol` for clocks and IDs, and `review-protocol` for public request/result types.

## Usage example
```rust
let (result, events, setup_error) = review_app::run_review(&request, &config);
```
