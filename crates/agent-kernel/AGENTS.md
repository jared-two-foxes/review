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

## External dependencies used from this level
`agent-protocol` for ID generation, `serde`/`serde_json` for payloads, `jsonschema` for validation, and `tracing` for coordinator diagnostics.
