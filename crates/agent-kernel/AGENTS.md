# AGENTS.md

## Module summary
`agent-kernel` is the reusable orchestration core. It defines the application contract, model/provider abstractions, event ledger, tool catalog, and the session coordinator that executes agent turns.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Production kernel modules. |
| `tests/` | Test doubles and focused kernel support tests. |

## Subsystem interaction
`review-app` implements the `AgentApplication` contract defined here, while runtime crates provide `Tool` implementations and model adapters consumed by the coordinator.

## Find existing code
Start with `/home/runner/work/review/review/crates/agent-kernel/src/coordinator.rs` for session flow and `/home/runner/work/review/review/crates/agent-kernel/src/application.rs` for the application interface.

## Add new implementations
Add reusable orchestration behavior here only when it is not specific to review; review-specific policy belongs in `review-app`.

## Keep in sync
Kernel interface changes usually require coordinated updates in `review-app`, runtime tool adapters, and deterministic golden tests.

## Search hints
Search for `AgentApplication`, `SessionCoordinator`, `CompletionDecision`, `UsageRecord`, and `ToolCatalog`.

## External dependencies used from this level
`agent-protocol` for ID generation, `serde`/`serde_json` for payloads, `jsonschema` for validation, and `tracing` for coordinator diagnostics.
