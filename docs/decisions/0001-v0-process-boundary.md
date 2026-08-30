# ADR 0001: Greenfield repository and subprocess-only Scaffold integration

- Status: Accepted
- Date: 2026-08-18

## Context

This repository is being created as a new standalone Cargo workspace for the Review product. It is not derived from, forked from, or otherwise built on top of Scaffold. The project is intended to establish a clean, independent Rust implementation with its own protocol, validation boundaries, and CLI behavior.

The project must also be able to integrate with Scaffold in the future without creating a brittle dependency on Scaffold internals. A direct embedding or module-sharing relationship would couple the implementation to Scaffold's runtime, language boundaries, and production codebase.

## Decision

1. The repository is greenfield.
   - This is a new standalone Cargo workspace created from scratch.
   - It is not derived from Scaffold or any existing Scaffold codebase.
   - The crate graph is owned and managed by this repository alone.

2. Scaffold integration is subprocess-only.
   - Any future integration with Scaffold happens through a process boundary, not by embedding or importing Scaffold code into the Rust workspace.
   - The Rust product communicates with Scaffold using versioned JSON payloads exchanged over a CLI or subprocess boundary.
   - There is no module sharing, no direct library dependency on Scaffold production code, and no runtime embedding of Scaffold components.

3. No Scaffold production code or foreign-language native bindings are allowed in the workspace.
   - No crate in the workspace depends on Scaffold production modules or implementation code.
   - No crate in the workspace uses Python FFI, native bindings, or other foreign-language bridges as a dependency path.
   - The architecture remains isolated and portable across platforms and toolchains.

## Consequences

- The project remains independent, understandable, and easy to test as a standalone Rust workspace.
- Future Scaffold integration can be added in a controlled, explicit way without creating hidden coupling.
- The boundary is clear: Review is a deliverable process that can be invoked by Scaffold, not a library embedded inside it.
- The repository avoids unsupported architecture patterns such as importing Scaffold production code or introducing native-language interop.
- This keeps the system maintainable during early V0 validation and later product evolution.

## Status

Accepted for the greenfield V0 implementation and treated as a compatibility boundary for future integration work.
