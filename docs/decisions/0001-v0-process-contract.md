# ADR 0001: V0 process contract and integration boundary

- Status: Accepted
- Date: 2026-08-18

## Context

This repository is a greenfield Rust workspace for the Review CLI and protocol boundary. In V0, the binary is intentionally a process contract and validation shell rather than a full review engine. There is no model provider integration, no repository access, and no production dependency on Scaffold code.

The CLI must still provide a stable process contract for automation and script callers. That contract must be documented, fixed, and safe for future expansion. The same document also records the architectural decision that any future Scaffold integration must remain subprocess-only so the workspace remains independent and free of direct Scaffold production imports or foreign-language native bindings.

## Decision

We will standardize the process exit-code mapping as a fixed public contract and record the greenfield/subprocess-only integration boundary in the same ADR.

### Exit code mapping

The six terminal exit codes are fixed and must not change once released:

| Numeric value | Meaning | Notes |
| --- | --- | --- |
| 0 | approved | Reserved for a future positive review outcome. |
| 1 | changes-requested | Reserved for a future negative review outcome. |
| 2 | invalid-request | Returned for malformed requests, unsupported schema major versions, validation failures, and invalid repository paths. |
| 3 | indeterminate | Returned when the review cannot be completed deterministically in V0. |
| 4 | internal-failure | Reserved for unhandled internal or runtime failures. |
| 5 | cancellation | Reserved for user-initiated cancellation or interrupted execution. |

The mapping is intentionally stable and must be treated as part of the public CLI contract. Existing automation may rely on these values, and they must not be renumbered or reused for different meanings.

### V0 production boundary

In V0, only two exit codes are producible:

- invalid-request = 2
- indeterminate = 3

The V0 implementation is deliberately limited to validation and deterministic synthetic processing. It does not produce approval or rejection verdicts, and it does not claim a substantive review engine result. Any later review outcomes such as approved, changes-requested, internal-failure, or cancellation are reserved for future V1+ capability and must not be emitted by the current V0 binary.

### Stability policy

These exit code values are fixed for the life of the CLI contract:

- Numeric values are part of the compatibility surface.
- They are not allowed to change even when the implementation grows.
- A future state may be added to the mapping only if the numeric value remains unchanged.
- The CLI must not reinterpret an existing code for a new meaning.
- The public documentation and tests must be updated together with any behavior change.

This keeps automation safe, preserves scripts and CI gates, and prevents silent breaking changes.

### Greenfield repository decision

The repository is a greenfield workspace. It is created from scratch as a standalone Rust project, not as a fork or extension of an existing Scaffold codebase. The purpose of V0 is to establish the review process contract and validation boundary before any model or platform integration is added.

### Scaffold integration policy

Any future integration with Scaffold is subprocess-only. The Review binary remains a standalone process that can be invoked by Scaffold, but no crate in this workspace may import Scaffold production modules or rely on foreign-language native bindings.

Concretely:

- Integration occurs via CLI invocation over process boundaries.
- Data crossing the boundary is serialized JSON or similar protocol payloads.
- Dependency direction remains downward and isolated.
- Review crates do not take a direct dependency on Scaffold runtime code.
- No native-language bridge is introduced into the workspace.

This preserves a clean architecture boundary and allows the Review system to evolve without coupling the implementation to Scaffold internals.

## Consequences

- The CLI has a stable, documented exit-code contract for automation and CI.
- V0 is explicit about its operational limits: it validates input and emits deterministic indeterminate or invalid-request outcomes only.
- Future review verdicts can be added without breaking semantics, as long as the numeric values remain unchanged.
- The repository remains architecture-clean and can integrate with Scaffold only via a narrow subprocess boundary rather than direct code coupling.
- The workspace remains portable, testable, and easy to reason about as a standalone greenfield implementation.

## Status

Accepted for V0 and treated as a compatibility contract moving forward.
